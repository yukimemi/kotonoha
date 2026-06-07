//! OpenAI-compatible Chat Completions backend — covers OpenRouter,
//! OpenAI itself, DeepSeek, and any other server speaking the same
//! dialect (Ollama, LM Studio, ...) via `base_url`.
//!
//! Endpoint shape:
//!   POST <base_url>/chat/completions
//!   Authorization: Bearer <KEY>
//!   body: { model, messages, stream: true, temperature? }
//!
//! Response is Server-Sent Events; each event payload is a JSON object
//! whose `choices[0].delta.content` carries the next chunk, terminated
//! by a literal `data: [DONE]` event.

use anyhow::{Context as _, anyhow};
use async_stream::try_stream;
use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use kotonoha_core::{ApiBackendConfig, Backend, CompletionRequest, ReplyStream, Turn};

use crate::sse::find_event_boundary;

/// Default Chat Completions base URL for each known provider key.
/// `base_url` in the config overrides these; an unknown provider key
/// with no `base_url` is rejected in `build_backend`.
pub(crate) fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "openai" => Some("https://api.openai.com/v1"),
        "deepseek" => Some("https://api.deepseek.com/v1"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatBackend {
    client: reqwest::Client,
    provider: String,
    base_url: String,
    model: String,
    api_key: String,
    temperature: Option<f32>,
}

impl OpenAiCompatBackend {
    pub fn new(cfg: &ApiBackendConfig) -> anyhow::Result<Self> {
        let api_key = std::env::var(&cfg.api_key_env).map_err(|_| {
            anyhow!(
                "env var `{}` not set — `{}` API backend cannot start",
                cfg.api_key_env,
                cfg.provider,
            )
        })?;
        let base_url = cfg
            .base_url
            .clone()
            .or_else(|| default_base_url(&cfg.provider).map(str::to_owned))
            .ok_or_else(|| {
                anyhow!(
                    "provider `{}` has no default base URL — set `base_url` \
                     in the backend config",
                    cfg.provider
                )
            })?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            client,
            provider: cfg.provider.clone(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            model: cfg.model.clone(),
            api_key,
            temperature: cfg.temperature,
        })
    }

    fn build_body(&self, req: &CompletionRequest) -> ChatRequest {
        // The system prompt is the first message; the transcript maps
        // student → `user`, teacher → `assistant`.
        //
        // Edge case: if the transcript is empty (very first turn), we
        // synthesize a single user message asking the teacher to greet
        // the student — otherwise the model has nothing to respond to.
        let mut messages = Vec::with_capacity(req.turns.len() + 2);
        messages.push(ChatMessage {
            role: "system",
            content: req.system_prompt.clone(),
        });
        if req.turns.is_empty() {
            messages.push(ChatMessage {
                role: "user",
                content: "(Begin the lesson — greet the student warmly in English and ask an opening question.)".into(),
            });
        } else {
            messages.extend(req.turns.iter().map(|t| match t {
                Turn::Student(text) => ChatMessage {
                    role: "user",
                    content: text.clone(),
                },
                Turn::Teacher(text) => ChatMessage {
                    role: "assistant",
                    content: text.clone(),
                },
            }));
        }

        ChatRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            temperature: self.temperature,
        }
    }
}

#[async_trait::async_trait]
impl Backend for OpenAiCompatBackend {
    async fn complete(&self, req: CompletionRequest) -> anyhow::Result<ReplyStream> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = self.build_body(&req);
        let provider = self.provider.clone();
        tracing::debug!(
            target: "kotonoha::llm",
            "{provider} request: model={} turns={}", self.model, req.turns.len()
        );

        let mut request = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body);
        if provider == "openrouter" {
            // App attribution — shows up on the OpenRouter activity
            // dashboard.  Harmless for everyone else, so gated anyway
            // to keep requests minimal.
            request = request
                .header("HTTP-Referer", "https://github.com/yukimemi/kotonoha")
                .header("X-Title", "kotonoha");
        }

        let t_send = std::time::Instant::now();
        let resp = request
            .send()
            .await
            .with_context(|| format!("send {provider} request"))?;
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
            "{provider} headers in {:.0}ms: status={status} content-type={content_type}",
            t_headers.as_secs_f64() * 1000.0
        );

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("{provider} http {status}: {text}"));
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
                // Process any complete SSE events in the buffer.  Lines
                // not starting with `data:` (OpenRouter's `: PROCESSING`
                // keepalive comments, `event:` fields) fall through the
                // strip_prefix and are skipped.
                while let Some((pos, sep_len)) = find_event_boundary(&buffer) {
                    let event_bytes = buffer.drain(..pos + sep_len).collect::<Vec<u8>>();
                    let event = String::from_utf8_lossy(&event_bytes);
                    for line in event.lines() {
                        let Some(payload) = line.strip_prefix("data:") else { continue; };
                        let payload = payload.trim();
                        if payload.is_empty() || payload == "[DONE]" { continue; }
                        match serde_json::from_str::<ChatStreamEvent>(payload) {
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
                                    "{provider} parse skipped: {e} on `{payload}`");
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
                    if let Ok(ev) = serde_json::from_str::<ChatStreamEvent>(payload) {
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
                "{provider} stream done: total={:.0}ms ttfb={:.0}ms ttft={:.0}ms bytes={total_bytes} chars={yielded}",
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
                    "{provider} stream produced no text ({total_bytes} bytes received). \
                     Check model name and that the endpoint returned SSE."
                ))?;
            }
        };
        Ok(Box::pin(stream))
    }
}

// ---- Chat Completions request/response shapes -------------------------

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatStreamEvent {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

impl ChatStreamEvent {
    fn first_text(&self) -> Option<String> {
        let choice = self.choices.first()?;
        match choice.delta.content.as_deref() {
            // Role-only / empty deltas (the leading `{"role":"assistant"}`
            // frame, finish_reason frames) carry no text — skip them.
            None | Some("") => None,
            Some(text) => Some(text.to_owned()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    delta: ChatDelta,
}

#[derive(Debug, Default, Deserialize)]
struct ChatDelta {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_have_default_base_urls() {
        assert!(default_base_url("openrouter").is_some());
        assert!(default_base_url("openai").is_some());
        assert!(default_base_url("deepseek").is_some());
        assert!(default_base_url("nonsense").is_none());
    }

    #[test]
    fn stream_event_extracts_delta_content() {
        let ev: ChatStreamEvent =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":"Hi!"},"index":0}]}"#).unwrap();
        assert_eq!(ev.first_text().as_deref(), Some("Hi!"));
    }

    #[test]
    fn stream_event_skips_role_only_delta() {
        let ev: ChatStreamEvent =
            serde_json::from_str(r#"{"choices":[{"delta":{"role":"assistant"},"index":0}]}"#)
                .unwrap();
        assert_eq!(ev.first_text(), None);
    }

    #[test]
    fn stream_event_skips_finish_frame() {
        let ev: ChatStreamEvent =
            serde_json::from_str(r#"{"choices":[{"delta":{},"index":0,"finish_reason":"stop"}]}"#)
                .unwrap();
        assert_eq!(ev.first_text(), None);
    }
}
