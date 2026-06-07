//! Google Gemini API backend — streams responses via the
//! `streamGenerateContent` endpoint.
//!
//! Endpoint shape (v1beta, May 2026):
//!   POST https://generativelanguage.googleapis.com/v1beta/models/<MODEL>:streamGenerateContent?alt=sse&key=<KEY>
//!
//! Response is Server-Sent Events; each event payload is a JSON object
//! whose `candidates[0].content.parts[*].text` carries the next chunk.

use anyhow::{Context as _, anyhow};
use async_stream::try_stream;
use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use kotonoha_core::{ApiBackendConfig, Backend, CompletionRequest, ReplyStream, Turn};

use crate::sse::find_event_boundary;

const ENDPOINT_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Debug, Clone)]
pub struct GeminiBackend {
    client: reqwest::Client,
    model: String,
    api_key: String,
    temperature: Option<f32>,
}

impl GeminiBackend {
    pub fn new(cfg: &ApiBackendConfig) -> anyhow::Result<Self> {
        let api_key = std::env::var(&cfg.api_key_env).map_err(|_| {
            anyhow!(
                "env var `{}` not set — gemini API backend cannot start. \
                 Get a free key at https://aistudio.google.com/apikey",
                cfg.api_key_env
            )
        })?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            client,
            model: cfg.model.clone(),
            api_key,
            temperature: cfg.temperature,
        })
    }

    fn build_body(&self, req: &CompletionRequest) -> GeminiRequest {
        // Gemini's `contents` is the running transcript.  `role` is
        // either `"user"` (student) or `"model"` (previous teacher).
        // The system prompt goes into `systemInstruction`.
        //
        // Edge case: if the transcript is empty (very first turn), we
        // synthesize a single user message asking the teacher to greet
        // the student — otherwise the model has nothing to respond to.
        let contents: Vec<GeminiContent> = if req.turns.is_empty() {
            vec![GeminiContent {
                role: "user",
                parts: vec![GeminiPart {
                    text: "(Begin the lesson — greet the student warmly in English and ask an opening question.)".into(),
                }],
            }]
        } else {
            req.turns
                .iter()
                .map(|t| match t {
                    Turn::Student(text) => GeminiContent {
                        role: "user",
                        parts: vec![GeminiPart { text: text.clone() }],
                    },
                    Turn::Teacher(text) => GeminiContent {
                        role: "model",
                        parts: vec![GeminiPart { text: text.clone() }],
                    },
                })
                .collect()
        };

        GeminiRequest {
            system_instruction: Some(GeminiContent {
                role: "system",
                parts: vec![GeminiPart {
                    text: req.system_prompt.clone(),
                }],
            }),
            contents,
            generation_config: self.temperature.map(|t| GeminiGenerationConfig {
                temperature: Some(t),
            }),
        }
    }
}

#[async_trait::async_trait]
impl Backend for GeminiBackend {
    async fn complete(&self, req: CompletionRequest) -> anyhow::Result<ReplyStream> {
        let url = format!(
            "{ENDPOINT_BASE}/{model}:streamGenerateContent?alt=sse&key={key}",
            model = self.model,
            key = self.api_key,
        );
        let body = self.build_body(&req);
        tracing::debug!(
            target: "kotonoha::llm",
            "gemini request: model={} turns={}", self.model, req.turns.len()
        );

        let t_send = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("send gemini request")?;
        let t_headers = t_send.elapsed();

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        tracing::info!(
            target: "kotonoha::llm",
            "gemini headers in {:.0}ms: status={status} content-type={content_type}",
            t_headers.as_secs_f64() * 1000.0
        );

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("gemini http {status}: {text}"));
        }

        let mut byte_stream = resp.bytes_stream();
        let stream = try_stream! {
            let mut buffer: Vec<u8> = Vec::new();
            let mut total_bytes: usize = 0;
            let mut yielded: usize = 0;
            let mut first_chunk_at: Option<std::time::Duration> = None;
            let mut first_yield_at: Option<std::time::Duration> = None;
            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk.context("stream chunk")?;
                if first_chunk_at.is_none() {
                    first_chunk_at = Some(t_send.elapsed());
                }
                total_bytes += chunk.len();
                buffer.extend_from_slice(&chunk);
                // Process any complete SSE events in the buffer.  SSE
                // delimiter is a blank line (`\n\n` per spec, but some
                // proxies inject `\r\n\r\n` — accept both).
                while let Some((pos, sep_len)) = find_event_boundary(&buffer) {
                    let event_bytes = buffer.drain(..pos + sep_len).collect::<Vec<u8>>();
                    let event = String::from_utf8_lossy(&event_bytes);
                    for line in event.lines() {
                        let Some(payload) = line.strip_prefix("data:") else { continue; };
                        let payload = payload.trim();
                        if payload.is_empty() || payload == "[DONE]" { continue; }
                        match serde_json::from_str::<GeminiStreamEvent>(payload) {
                            Ok(ev) => {
                                if let Some(text) = ev.first_text() {
                                    if first_yield_at.is_none() {
                                        first_yield_at = Some(t_send.elapsed());
                                    }
                                    yielded += text.len();
                                    yield text;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(target: "kotonoha::llm",
                                    "gemini parse skipped: {e} on `{payload}`");
                            }
                        }
                    }
                }
            }
            // Tail flush — final event may not end with a blank line.
            if !buffer.is_empty() {
                let event = String::from_utf8_lossy(&buffer);
                for line in event.lines() {
                    let Some(payload) = line.strip_prefix("data:") else { continue; };
                    let payload = payload.trim();
                    if payload.is_empty() || payload == "[DONE]" { continue; }
                    if let Ok(ev) = serde_json::from_str::<GeminiStreamEvent>(payload) {
                        if let Some(text) = ev.first_text() {
                            yielded += text.len();
                            yield text;
                        }
                    }
                }
            }
            let total = t_send.elapsed();
            tracing::info!(
                target: "kotonoha::llm",
                "gemini stream done: total={:.0}ms ttfb={:.0}ms ttft={:.0}ms bytes={total_bytes} chars={yielded}",
                total.as_secs_f64() * 1000.0,
                first_chunk_at.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
                first_yield_at.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            );
            if yielded == 0 {
                // No text came through.  Most likely either the model
                // name is wrong or the body was a non-streaming
                // response.  Surface this rather than letting the chat
                // sit silent.
                Err(anyhow!(
                    "gemini stream produced no text ({total_bytes} bytes received). \
                     Check model name and that the endpoint returned SSE."
                ))?;
            }
        };
        Ok(Box::pin(stream))
    }
}

// ---- Gemini request/response shapes ----------------------------------

#[derive(Debug, Serialize)]
struct GeminiRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: &'static str,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamEvent {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

impl GeminiStreamEvent {
    fn first_text(&self) -> Option<String> {
        let cand = self.candidates.first()?;
        let part = cand.content.parts.first()?;
        part.text.clone()
    }
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: GeminiCandidateContent,
}

#[derive(Debug, Default, Deserialize)]
struct GeminiCandidateContent {
    #[serde(default)]
    parts: Vec<GeminiCandidatePart>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidatePart {
    text: Option<String>,
}
