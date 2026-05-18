//! `kotonoha setup-tts` — download the Kokoro 82M model + voices.
//!
//! This is the Rust port of `scripts/setup-tts.ts` (the bun script
//! that source-checked-out users have run since v0.1.0). It exists
//! so users who installed via `cargo install kotonoha-server` —
//! without bun or the source tree — can also populate the TTS assets
//! straight from the binary.
//!
//! By default the destination paths come from `[voice.kokoro]` in
//! `configs/kotonoha.toml` so the resulting layout drops straight
//! into the server's expected locations.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};
use clap::Args;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;

use kotonoha_core::Config;

const HF_REPO: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main";

/// Curated voice set — same as `scripts/setup-tts.ts` keeps in sync.
/// Bias: "Japanese English teacher" persona — jf_alpha first, plus a
/// few `af_*` for variety + `bf_emma` for a British alternative.
const DEFAULT_VOICES: &[&str] = &[
    "jf_alpha",
    "af_heart",
    "af_bella",
    "af_nicole",
    "af_sky",
    "bf_emma",
];

/// Full v1.0 voice list (mirrors `scripts/setup-tts.ts`). Used when
/// `--all-voices` is passed.
const ALL_VOICES: &[&str] = &[
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
    "ef_dora",
    "em_alex",
    "em_santa",
    "ff_siwis",
    "hf_alpha",
    "hf_beta",
    "hm_omega",
    "hm_psi",
    "if_sara",
    "im_nicola",
    "jf_alpha",
    "jf_gongitsune",
    "jf_nezumi",
    "jf_tebukuro",
    "jm_kumo",
    "pf_dora",
    "pm_alex",
    "pm_santa",
    "zf_xiaobei",
    "zf_xiaoni",
    "zf_xiaoxiao",
    "zf_xiaoyi",
    "zm_yunjian",
    "zm_yunxi",
    "zm_yunxia",
    "zm_yunyang",
];

#[derive(Debug, Args)]
pub struct SetupTtsArgs {
    /// Download the full-precision model (~325 MB) instead of the
    /// default q4f16 quantized variant (~92 MB).
    #[arg(long)]
    pub full: bool,

    /// Download all 54 voices instead of the curated 6.
    #[arg(long)]
    pub all_voices: bool,

    /// Override the model file destination. Defaults to
    /// `[voice.kokoro].model_path` from the config.
    #[arg(long, value_name = "FILE")]
    pub model: Option<PathBuf>,

    /// Override the voices directory. Defaults to
    /// `[voice.kokoro].voices_dir` from the config.
    #[arg(long, value_name = "DIR")]
    pub voices_dir: Option<PathBuf>,

    /// Re-download files even if they already exist on disk.
    #[arg(long)]
    pub force: bool,
}

pub async fn run(config: &Config, args: SetupTtsArgs) -> anyhow::Result<()> {
    // Per CodeRabbit / Gemini PR #29 feedback: don't hard-require
    // `[voice.kokoro]` when both `--model` and `--voices-dir` are
    // given on the CLI. The config is only consulted to fill in the
    // missing side, and the error message names the specific knob.
    let kokoro = config.voice.kokoro.as_ref();
    // `args` is owned and the fields aren't used past this point, so
    // move rather than clone (Gemini PR #30 nit).
    let model_dest = args
        .model
        .or_else(|| kokoro.map(|k| PathBuf::from(&k.model_path)))
        .ok_or_else(|| {
            anyhow!(
                "model path not specified — add `[voice.kokoro] model_path` \
                 to configs/kotonoha.toml, or pass `--model <FILE>`."
            )
        })?;
    let voices_dir = args
        .voices_dir
        .or_else(|| kokoro.map(|k| PathBuf::from(&k.voices_dir)))
        .ok_or_else(|| {
            anyhow!(
                "voices dir not specified — add `[voice.kokoro] voices_dir` \
                 to configs/kotonoha.toml, or pass `--voices-dir <DIR>`."
            )
        })?;

    let model_url = if args.full {
        format!("{HF_REPO}/onnx/model.onnx")
    } else {
        format!("{HF_REPO}/onnx/model_q4f16.onnx")
    };

    let voices: Vec<&str> = if args.all_voices {
        ALL_VOICES.to_vec()
    } else {
        DEFAULT_VOICES.to_vec()
    };

    eprintln!("model → {}", model_dest.display());
    eprintln!("voices → {} ({} files)", voices_dir.display(), voices.len());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    download_if_missing(&client, &model_url, &model_dest, args.force).await?;
    for v in &voices {
        let url = format!("{HF_REPO}/voices/{v}.bin");
        let dest = voices_dir.join(format!("{v}.bin"));
        download_if_missing(&client, &url, &dest, args.force).await?;
    }

    eprintln!();
    eprintln!("Done. Switch the server to Kokoro TTS by setting in configs/kotonoha.toml:");
    eprintln!("    [voice]");
    eprintln!("    tts = \"kokoro\"");
    Ok(())
}

async fn download_if_missing(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    force: bool,
) -> anyhow::Result<()> {
    // Atomic download: stream to `<dest>.partial`, fsync, then rename.
    //
    // Per Gemini / CodeRabbit PR #29: the previous "is the file
    // larger than 1KB" heuristic skipped subsequent runs even when
    // the file on disk was a truncated, half-downloaded asset from
    // a Ctrl+C'd or network-dropped earlier run.  Writing to a
    // `.partial` sibling and renaming on success means a successful
    // run can never observe a corrupt `dest`, and a failed run
    // leaves the partial out of the way of the skip check.
    // `Path::exists` is synchronous blocking I/O — use
    // `tokio::fs::metadata` so we don't pin a tokio worker thread on
    // a syscall when a slow filesystem (network mount etc.) responds
    // late (Gemini PR #30 nit). NotFound short-circuits to download.
    if !force {
        match tokio::fs::metadata(dest).await {
            Ok(meta) => {
                let size = meta.len();
                eprintln!(
                    "✓ already have {} ({:.1} MB)",
                    short(dest),
                    size as f64 / 1024.0 / 1024.0
                );
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => { /* download */ }
            Err(e) => return Err(e).with_context(|| format!("stat {}", dest.display())),
        }
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp_dest = dest.with_extension("partial");
    eprintln!("↓ {url}");
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} for {url}", resp.status());
    }
    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&tmp_dest)
        .await
        .with_context(|| format!("create {}", tmp_dest.display()))?;
    let mut downloaded: u64 = 0;
    let mut last_log = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("stream chunk")?;
        downloaded += chunk.len() as u64;
        file.write_all(&chunk).await?;
        // Progress every ~1s so big files don't look hung.
        if last_log.elapsed().as_millis() > 1000 {
            let pct = total
                .map(|t| format!(" / {:.0}%", downloaded as f64 / t as f64 * 100.0))
                .unwrap_or_default();
            eprintln!("  {:.1} MB{pct}", downloaded as f64 / 1024.0 / 1024.0);
            last_log = std::time::Instant::now();
        }
    }
    // `sync_all` (not `flush`) actually fsyncs to disk — flush on
    // tokio::fs::File is a no-op since the writer isn't buffered.
    file.sync_all()
        .await
        .with_context(|| format!("fsync {}", tmp_dest.display()))?;
    drop(file);
    tokio::fs::rename(&tmp_dest, dest)
        .await
        .with_context(|| format!("rename {} -> {}", tmp_dest.display(), dest.display()))?;
    eprintln!(
        "  saved {} ({:.1} MB)",
        short(dest),
        downloaded as f64 / 1024.0 / 1024.0
    );
    Ok(())
}

/// Strip the CWD prefix so the message is readable.
fn short(p: &Path) -> String {
    if let Ok(cwd) = std::env::current_dir()
        && let Ok(rel) = p.strip_prefix(&cwd)
    {
        return format!("./{}", rel.display());
    }
    p.display().to_string()
}
