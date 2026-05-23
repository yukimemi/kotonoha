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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use tokio::sync::Mutex;
use voicevox_dyn::{AccelerationMode, VoiceVox};

/// Default speaker id — 春日部つむぎ (ノーマル). Middle-school-aged
/// female character, child-safe, fits the "Japanese English teacher"
/// persona kotonoha is built around.
pub const DEFAULT_SPEAKER_ID: u32 = 8;

/// Coarse-grained progress events emitted during [`Tts::load`].
///
/// Used by `kotonoha setup-voicevox` to drive an indicatif progress
/// bar; the FFI download itself doesn't surface bytes-transferred,
/// so the bar between `EngineReady` events is a spinner and the
/// per-speaker bar is incremented one tick per [`SpeakerLoaded`].
#[derive(Debug, Clone, Copy)]
pub enum LoadEvent {
    /// Core library + ONNX runtime downloaded (if needed) and the
    /// engine has finished `init`. Speaker model loads start next.
    EngineReady,
    /// A speaker model finished loading.
    SpeakerLoaded { id: u32 },
}

pub type LoadEventCallback = std::sync::Arc<dyn Fn(LoadEvent) + Send + Sync>;

#[derive(Clone)]
pub struct TtsConfig {
    /// Speakers (numeric ids) to pre-load on engine init. Loading
    /// extra speakers on demand still works; pre-loaded ones just
    /// avoid the first-call latency.
    pub speaker_ids: Vec<u32>,
    /// Optional callback invoked from the blocking load worker on
    /// each [`LoadEvent`]. Implementations must be cheap + Send +
    /// Sync since the worker is a tokio blocking task.
    pub on_event: Option<LoadEventCallback>,
}

impl std::fmt::Debug for TtsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsConfig")
            .field("speaker_ids", &self.speaker_ids)
            .field("on_event", &self.on_event.as_ref().map(|_| "<callback>"))
            .finish()
    }
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

        // Pre-step: voicevox-dyn delegates the actual ~700 MB asset
        // download to a `voicevox_downloader` helper binary that
        // *prompts for license agreement on stdin* on first run.
        // voicevox-dyn spawns the helper without piping a "y" and
        // hangs forever on Windows where the prompt is interactive.
        // We do the download ourselves with `y` piped in so by the
        // time `VoiceVox::load()` runs, every asset is already on
        // disk and it short-circuits without spawning anything.
        ensure_voicevox_assets().await?;

        // `VoiceVox::load` is sync FFI; run on a blocking worker.
        let speaker_ids = cfg.speaker_ids.clone();
        let on_event = cfg.on_event.clone();
        let vv = tokio::task::spawn_blocking(move || -> anyhow::Result<VoiceVox> {
            let mut vv = VoiceVox::load().map_err(|e| anyhow::anyhow!("VoiceVox::load: {e:?}"))?;
            let threads = std::thread::available_parallelism()
                .map(|n| n.get() as u16)
                .unwrap_or(2);
            vv.init(AccelerationMode::Auto, threads, false)
                .map_err(|e| anyhow::anyhow!("VoiceVox::init: {e:?}"))?;
            if let Some(cb) = &on_event {
                cb(LoadEvent::EngineReady);
            }
            for id in &speaker_ids {
                vv.load_model(*id)
                    .map_err(|e| anyhow::anyhow!("load speaker {id}: {e:?}"))?;
                if let Some(cb) = &on_event {
                    cb(LoadEvent::SpeakerLoaded { id: *id });
                }
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

/// Make sure the c_api / onnxruntime / models / dict asset
/// directories already exist next to the executable. If any are
/// missing, run the official `voicevox_downloader` ourselves with
/// `y` piped on stdin so the license-agreement prompt doesn't
/// stall it forever.
async fn ensure_voicevox_assets() -> anyhow::Result<()> {
    let exe_dir = std::env::current_exe()
        .context("locating current exe")?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("exe has no parent dir"))?
        .to_path_buf();

    // voicevox_downloader lays out four sibling dirs. If they're
    // all here we know the previous setup-voicevox finished. The
    // check runs through tokio::fs so we don't block an executor
    // thread on a sync stat() per dir.
    let marker_dirs = ["c_api", "onnxruntime", "models", "dict"];
    let mut all_present = true;
    for d in &marker_dirs {
        let meta = tokio::fs::metadata(exe_dir.join(d)).await;
        if !matches!(meta, Ok(m) if m.is_dir()) {
            all_present = false;
            break;
        }
    }
    if all_present {
        tracing::info!(
            "voicevox assets already present at {}, skipping downloader",
            exe_dir.display()
        );
        return Ok(());
    }

    let downloader = ensure_downloader_binary(&exe_dir).await?;
    tracing::info!(
        "running voicevox_downloader via {} (this fetches ~700 MB)",
        downloader.display()
    );

    let exe_dir_for_blocking = exe_dir.clone();
    let downloader_clone = downloader.clone();
    let status =
        tokio::task::spawn_blocking(move || -> anyhow::Result<std::process::ExitStatus> {
            use std::io::Write;
            use std::process::{Command, Stdio};

            let (cpu_arch, os_tag) = downloader_target();
            let mut cmd = Command::new(&downloader_clone);
            cmd.arg("-o")
                .arg(&exe_dir_for_blocking)
                .arg("--devices")
                .arg("cpu")
                .arg("--cpu-arch")
                .arg(cpu_arch)
                .arg("--os")
                .arg(os_tag);
            // voicevox_downloader uses the `minus` pager to display the
            // license text — minus crashes on Japanese char boundaries.
            // Force it into plain-cat mode and a dumb terminal so it
            // writes the agreement to stdout/stderr and proceeds to the
            // y/n prompt without trying to invoke a pager.
            cmd.env("MINUS_PAGER", "cat")
                .env("TERM", "dumb")
                .stdin(Stdio::piped())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            let mut child = cmd.spawn().context("spawning voicevox_downloader")?;
            if let Some(mut stdin) = child.stdin.take() {
                // Two agreement prompts (audio model + ONNX runtime
                // licenses) plus a buffer in case the downloader adds
                // more in the future.
                let _ = stdin.write_all(b"y\ny\ny\ny\n");
            }
            child.wait().context("waiting voicevox_downloader")
        })
        .await
        .context("spawn_blocking voicevox_downloader")??;

    if !status.success() {
        anyhow::bail!("voicevox_downloader exited with {status}");
    }
    tracing::info!("voicevox assets downloaded to {}", exe_dir.display());
    Ok(())
}

/// Make sure a runnable `voicevox_downloader` is sitting next to
/// the executable. voicevox-dyn used to drop the binary without a
/// `.exe` suffix on Windows; if that's what we find, copy it under
/// the canonical name. Otherwise, fetch the matching release asset
/// from VOICEVOX/voicevox_core directly.
async fn ensure_downloader_binary(exe_dir: &Path) -> anyhow::Result<PathBuf> {
    let canonical_name = if cfg!(target_os = "windows") {
        "voicevox_downloader.exe"
    } else {
        "voicevox_downloader"
    };
    let target = exe_dir.join(canonical_name);
    if tokio::fs::try_exists(&target).await.unwrap_or(false) {
        return Ok(target);
    }

    // Fallback A (Windows): voicevox-dyn occasionally stashes the
    // binary with no extension. Copy to the canonical name so the
    // OS recognizes it as executable.
    if cfg!(target_os = "windows") {
        let alt = exe_dir.join("voicevox_downloader");
        if tokio::fs::try_exists(&alt).await.unwrap_or(false) {
            tokio::fs::copy(&alt, &target)
                .await
                .context("copy voicevox_downloader -> .exe")?;
            return Ok(target);
        }
    }

    // Fallback B: fetch from GitHub releases. Released as
    // `download-<os>-<arch>[.exe]` under VOICEVOX/voicevox_core.
    // downloader_asset_name returns Err for platforms we haven't
    // mapped so users see a clear "your platform isn't supported"
    // message instead of an obscure "download linux x64 won't
    // execute on your osx-arm64" failure later.
    let asset_name = downloader_asset_name()?;
    tracing::info!(
        "fetching {} from VOICEVOX/voicevox_core releases",
        asset_name
    );
    let client = reqwest::Client::builder()
        .user_agent(format!("kotonoha-tts/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build reqwest client")?;
    let mut req = client.get("https://api.github.com/repos/VOICEVOX/voicevox_core/releases/latest");
    // GitHub anonymous rate limit is 60/h — pick up a token if one's
    // sitting in the environment so kicking the tires repeatedly
    // doesn't get throttled.
    if let Ok(tok) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        if !tok.is_empty() {
            req = req.header("Authorization", format!("Bearer {tok}"));
        }
    }
    let release: serde_json::Value = req
        .send()
        .await
        .context("GET latest release")?
        .error_for_status()
        .context("GitHub releases status")?
        .json()
        .await
        .context("parse release json")?;
    let assets = release["assets"]
        .as_array()
        .context("no assets array on release")?;
    let asset_url = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(asset_name))
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| anyhow::anyhow!("asset {} not found in latest release", asset_name))?;

    let bytes = client
        .get(asset_url)
        .send()
        .await
        .context("GET downloader asset")?
        .error_for_status()
        .context("downloader asset status")?
        .bytes()
        .await
        .context("read downloader bytes")?;
    tokio::fs::write(&target, &bytes)
        .await
        .context("write downloader to disk")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&target).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&target, perms).await?;
    }
    Ok(target)
}

fn downloader_asset_name() -> anyhow::Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "download-windows-x64.exe",
        ("linux", "x86_64") => "download-linux-x64",
        ("linux", "aarch64") => "download-linux-arm64",
        ("macos", "x86_64") => "download-osx-x64",
        ("macos", "aarch64") => "download-osx-arm64",
        (os, arch) => anyhow::bail!(
            "no voicevox_downloader release asset for {os}/{arch}; supported: \
             windows/x86_64, linux/x86_64, linux/aarch64, macos/x86_64, macos/aarch64"
        ),
    })
}

fn downloader_target() -> (&'static str, &'static str) {
    let cpu_arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => "x64",
    };
    let os_tag = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        _ => "linux",
    };
    (cpu_arch, os_tag)
}
