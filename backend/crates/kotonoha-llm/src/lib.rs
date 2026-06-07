//! Direct HTTP API backends — bypasses the CLI subprocess overhead
//! (Node.js cold-start, ~2-5s) and gives us native streaming.

pub mod gemini;
pub mod openai_compat;
mod sse;

use anyhow::anyhow;
use kotonoha_core::ApiBackendConfig;
use kotonoha_core::Backend;

/// Build a concrete `Backend` from an `ApiBackendConfig`.  The
/// `provider` string drives the dispatch; future providers (anthropic)
/// plug in here.
pub fn build_backend(cfg: &ApiBackendConfig) -> anyhow::Result<Box<dyn Backend>> {
    match cfg.provider.as_str() {
        "google" | "gemini" => Ok(Box::new(gemini::GeminiBackend::new(cfg)?)),
        // Known Chat Completions providers, plus anything with an
        // explicit `base_url` (Ollama, LM Studio, vLLM, ...).
        p if openai_compat::default_base_url(p).is_some() || cfg.base_url.is_some() => {
            Ok(Box::new(openai_compat::OpenAiCompatBackend::new(cfg)?))
        }
        other => Err(anyhow!(
            "unsupported LLM provider `{other}` — kotonoha-llm supports: \
             google, openrouter, openai, deepseek, or any OpenAI-compatible \
             server via `base_url`"
        )),
    }
}
