//! kotonoha-tts — text-to-speech backends for kotonoha.
//!
//! Two engines are exposed side by side:
//!
//! - [`kokoro`]: Kokoro 82M ONNX, pure-Rust phonemizer (misaki-lean).
//!   Excellent English voices (`af_heart`, `jf_alpha`, ...); the
//!   Japanese voices exist as models but the lean phonemizer can't
//!   feed them Japanese text — they sound silent on JA input.
//! - [`voicevox`]: VOICEVOX, dynamic-loaded native engine. Japanese
//!   characters (春日部つむぎ, 四国めたん, ずんだもん, ...) with
//!   per-character licenses; the engine + ONNX models are downloaded
//!   at runtime via `voicevox-dyn` so the binary itself stays small.
//!
//! Both produce a self-contained WAV byte blob suitable for shipping
//! straight to the browser. Pick which one to call by sentence
//! language (English → kokoro, Japanese → voicevox) — see
//! `kotonoha-server` for the routing.

pub mod kokoro;
pub mod voicevox;

// Backward-compat re-exports — earlier code (server, examples) talks
// about plain `Tts` + `TtsConfig` meaning the Kokoro pair. Keep the
// names as aliases so we don't churn the call sites here.
pub use kokoro::{DEFAULT_VOICE, SAMPLE_RATE, Tts, TtsConfig};
