//! kotonoha-tts — wrapper around `kokoro-en` that returns WAV bytes
//! ready to ship over HTTP.
//!
//! Why a wrapper:
//! - keeps the model path / voice dir configuration in one place
//! - converts `Vec<f32>` PCM samples to a self-contained WAV blob
//! - hides whether `kokoro-en`'s API shape changes between versions

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use kokoro_en::{KokoroTts, Voice};

/// Default voice (American female, "heart" — bright + warm tone).
pub const DEFAULT_VOICE: &str = "af_heart";

/// Sample rate produced by Kokoro 82M.
pub const SAMPLE_RATE: u32 = 24_000;

#[derive(Clone)]
pub struct Tts {
    inner: Arc<KokoroTts>,
}

#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// Path to the `model.onnx` (or quantized variant) on disk.
    pub model_path: PathBuf,
    /// Directory containing `<voice>.bin` files.
    pub voices_dir: PathBuf,
}

impl Tts {
    /// Load the Kokoro engine from disk.  This does NOT download
    /// anything — the caller is expected to have placed the model
    /// and voice bins under the configured paths (see scripts/).
    pub async fn load(cfg: &TtsConfig) -> anyhow::Result<Self> {
        ensure_path(&cfg.model_path, "model")?;
        ensure_path(&cfg.voices_dir, "voices dir")?;
        tracing::info!(
            "loading kokoro model from {} (voices: {})",
            cfg.model_path.display(),
            cfg.voices_dir.display()
        );
        let tts = KokoroTts::new(&cfg.model_path, &cfg.voices_dir)
            .await
            .context("KokoroTts::new")?;
        Ok(Self { inner: Arc::new(tts) })
    }

    /// Synthesize `text` with the given voice id (e.g. `"af_heart"`)
    /// and return a self-contained WAV byte blob (16-bit PCM, 24kHz).
    pub async fn synthesize_wav(&self, text: &str, voice: &str, speed: f32) -> anyhow::Result<Vec<u8>> {
        let voice = Voice::new(voice).with_speed(speed);
        let (samples, took) = self.inner.synth(text, voice).await
            .map_err(|e| anyhow::anyhow!("kokoro synth: {e}"))?;
        tracing::debug!("kokoro synthesized {} samples in {:?}", samples.len(), took);
        f32_samples_to_wav(&samples, SAMPLE_RATE)
    }
}

fn ensure_path(p: &Path, what: &str) -> anyhow::Result<()> {
    if !p.exists() {
        anyhow::bail!("{what} not found at {} — run `bun run setup:tts` to download", p.display());
    }
    Ok(())
}

fn f32_samples_to_wav(samples: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::<u8>::with_capacity(samples.len() * 2 + 44));
    {
        let mut w = hound::WavWriter::new(&mut buf, spec)?;
        for &s in samples {
            // Clip + scale to i16.
            let clipped = s.clamp(-1.0, 1.0);
            w.write_sample((clipped * i16::MAX as f32) as i16)?;
        }
        w.finalize()?;
    }
    Ok(buf.into_inner())
}
