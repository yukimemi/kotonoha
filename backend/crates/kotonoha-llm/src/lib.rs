//! Direct HTTP API backends — bypasses the CLI subprocess overhead
//! (Node.js cold-start, ~2-5s) and gives us native streaming.

pub mod gemini;

use anyhow::anyhow;
use kotonoha_core::ApiBackendConfig;
use kotonoha_core::Backend;

/// Build a concrete `Backend` from an `ApiBackendConfig`.  The
/// `provider` string drives the dispatch; future providers (anthropic,
/// openai) plug in here.
pub fn build_backend(cfg: &ApiBackendConfig) -> anyhow::Result<Box<dyn Backend>> {
    match cfg.provider.as_str() {
        "google" | "gemini" => Ok(Box::new(gemini::GeminiBackend::new(cfg)?)),
        other => Err(anyhow!(
            "unsupported LLM provider `{other}` — kotonoha-llm currently \
             supports: google"
        )),
    }
}
