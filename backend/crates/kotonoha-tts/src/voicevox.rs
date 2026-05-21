//! VOICEVOX TTS — dynamic-loaded native engine for Japanese voices.
//!
//! `voicevox-dyn` downloads the platform core library + ONNX runtime
//! into the directory of the running executable on first call to
//! `VoiceVox::load()`. After that the same call reuses the cached
//! files. `kotonoha setup-voicevox` just runs the download eagerly
//! so the first `kotonoha serve` doesn't block waiting for the
//! ~200 MB DL.
//!
//! ## Speaker IDs (default subset)
//!
//! | id | character | style | child-safe? |
//! |----|-----------|-------|-------------|
//! |  8 | 春日部つむぎ | ノーマル | ✅ |
//! |  2 | 四国めたん   | ノーマル | ✅ |
//! |  3 | ずんだもん   | ノーマル | ✅ |
//!
//! Each character carries its own license that this project's UI
//! must credit ("VOICEVOX:<character>"). Full enumeration is
//! available via the core API at runtime.

use std::sync::Arc;

use anyhow::Context as _;
use tokio::sync::Mutex;
use voicevox_dyn::{AccelerationMode, VoiceVox};

/// Default speaker id — 春日部つむぎ (ノーマル). Middle-school-aged
/// female character, child-safe, fits the "Japanese English teacher"
/// persona kotonoha is built around.
pub const DEFAULT_SPEAKER_ID: u32 = 8;

#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// Speakers (numeric ids) to pre-load on engine init. Loading
    /// extra speakers on demand still works; pre-loaded ones just
    /// avoid the first-call latency.
    pub speaker_ids: Vec<u32>,
}

/// Thin handle. `voicevox-dyn` uses interior FFI state; we wrap
/// `VoiceVox` in a `Mutex` so concurrent `synthesize_wav` calls
/// serialize through the engine (it's not documented as reentrant).
#[derive(Clone)]
pub struct Tts {
    inner: Arc<Mutex<VoiceVox>>,
}

impl Tts {
    /// Load the engine. Downloads core + ONNX runtime into the
    /// executable's directory on first call; reuses the cache on
    /// subsequent calls. Errors if the network is unavailable AND
    /// no cache exists yet — point users at `kotonoha setup-voicevox`.
    pub async fn load(cfg: &TtsConfig) -> anyhow::Result<Self> {
        tracing::info!(
            "loading voicevox-dyn (preloading speakers {:?})",
            cfg.speaker_ids
        );

        // `VoiceVox::load` is sync FFI; run on a blocking worker.
        let speaker_ids = cfg.speaker_ids.clone();
        let vv = tokio::task::spawn_blocking(move || -> anyhow::Result<VoiceVox> {
            let mut vv = VoiceVox::load().map_err(|e| anyhow::anyhow!("VoiceVox::load: {e:?}"))?;
            let threads = std::thread::available_parallelism()
                .map(|n| n.get() as u16)
                .unwrap_or(2);
            vv.init(AccelerationMode::Auto, threads, false)
                .map_err(|e| anyhow::anyhow!("VoiceVox::init: {e:?}"))?;
            for id in &speaker_ids {
                vv.load_model(*id)
                    .map_err(|e| anyhow::anyhow!("load speaker {id}: {e:?}"))?;
            }
            Ok(vv)
        })
        .await
        .context("spawn_blocking voicevox load")??;

        Ok(Self {
            inner: Arc::new(Mutex::new(vv)),
        })
    }

    /// Synthesize Japanese `text` with the given speaker id. Returns
    /// a self-contained WAV blob. Speaker is loaded on demand
    /// (idempotent — re-loading an already-loaded model is a noop
    /// in `voicevox-dyn`).
    pub async fn synthesize_wav(&self, text: &str, speaker_id: u32) -> anyhow::Result<Vec<u8>> {
        let inner = self.inner.clone();
        let text = text.to_string();
        let wav = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let vv = inner.blocking_lock();
            // load_model is idempotent — just call it; cheap if
            // already loaded, downloads + loads if not.
            vv.load_model(speaker_id)
                .map_err(|e| anyhow::anyhow!("load speaker {speaker_id}: {e:?}"))?;
            let wav = vv
                .tts(&text, speaker_id, Default::default())
                .map_err(|e| anyhow::anyhow!("voicevox tts (speaker {speaker_id}): {e:?}"))?;
            Ok(wav.as_slice().to_vec())
        })
        .await
        .context("spawn_blocking voicevox tts")??;
        Ok(wav)
    }
}
