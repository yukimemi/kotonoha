//! VOICEVOX 0.16.x native FFI binding.
//!
//! voicevox-dyn 0.3 (MIT, (c) 2023 chronicl,
//! <https://github.com/chronicl/voicevox-dyn>) targeted the legacy
//! flat layout (`exe_dir/voicevox_core.dll`) and the 0.14-era
//! single-shot `voicevox_initialize` entrypoint. The 0.16 downloader
//! lays assets out under `c_api/lib/`, `onnxruntime/lib/`, `dict/`,
//! and `models/vvms/`, and the runtime now wants a multi-step setup
//! (`voicevox_onnxruntime_load_once` -> `voicevox_open_jtalk_rc_new`
//! -> `voicevox_synthesizer_new` -> load each `.vvm`). We pull all of
//! that ourselves via libloading so kotonoha keeps its single-binary
//! distribution story.
//!
//! The C-API symbol names and signatures are facts read straight off
//! the `voicevox_core.h` that ships with each release — not
//! copyrightable. The load() control flow was adapted from
//! voicevox-dyn 0.3 (MIT) per its license terms.
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

use std::ffi::{CString, c_void};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use libloading::{Library, Symbol};
use tokio::sync::Mutex;

/// Default speaker id — 春日部つむぎ (ノーマル). Middle-school-aged
/// female character, child-safe, fits the "Japanese English teacher"
/// persona kotonoha is built around.
pub const DEFAULT_SPEAKER_ID: u32 = 8;

/// Coarse-grained progress events emitted during [`Tts::load`].
#[derive(Debug, Clone, Copy)]
pub enum LoadEvent {
    /// ONNX runtime loaded + JTalk dictionary opened + synthesizer
    /// created. Voice-model loads start next.
    EngineReady,
    /// A speaker model finished loading.
    SpeakerLoaded { id: u32 },
}

pub type LoadEventCallback = Arc<dyn Fn(LoadEvent) + Send + Sync>;

#[derive(Clone)]
pub struct TtsConfig {
    /// Speakers (numeric ids) the UI cares about — used only for
    /// firing per-id [`LoadEvent::SpeakerLoaded`] events for the
    /// progress bar. We always load every `.vvm` we find, so any
    /// speaker can still be addressed at synthesis time.
    pub speaker_ids: Vec<u32>,
    /// Optional callback invoked from the blocking load worker on
    /// each [`LoadEvent`].
    pub on_event: Option<LoadEventCallback>,
    /// Caller has presented the VOICEVOX licenses to the user and
    /// received explicit consent (e.g. interactive `y` from the
    /// `kotonoha setup-voicevox` prompt, or `--accept-license` from
    /// a scripted run). Required to auto-pipe `y\n` to the
    /// downloader's agreement prompt on first run. If false and
    /// the assets aren't already downloaded, `load()` errors out
    /// pointing at `kotonoha setup-voicevox`.
    pub license_accepted: bool,
}

impl std::fmt::Debug for TtsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsConfig")
            .field("speaker_ids", &self.speaker_ids)
            .field("on_event", &self.on_event.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

// ─── FFI types ───────────────────────────────────────────────────

#[repr(C)]
struct VoicevoxLoadOnnxruntimeOptions {
    filename: *const c_char,
}

#[repr(i32)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum VoicevoxAccelerationMode {
    Auto = 0,
    Cpu = 1,
    Gpu = 2,
}

#[repr(C)]
struct VoicevoxInitializeOptions {
    acceleration_mode: VoicevoxAccelerationMode,
    cpu_num_threads: u16,
}

#[repr(C)]
struct VoicevoxTtsOptions {
    enable_interrogative_upspeak: bool,
}

type VoicevoxResultCode = i32;
type VoicevoxStyleId = u32;

#[allow(dead_code)]
type VoicevoxOnnxruntime = c_void;
#[allow(dead_code)]
type OpenJtalkRc = c_void;
#[allow(dead_code)]
type VoicevoxSynthesizer = c_void;
#[allow(dead_code)]
type VoicevoxVoiceModelFile = c_void;

type FnLoadOnnxruntime = unsafe extern "C" fn(
    options: VoicevoxLoadOnnxruntimeOptions,
    out_onnxruntime: *mut *const VoicevoxOnnxruntime,
) -> VoicevoxResultCode;
type FnOpenJtalkNew = unsafe extern "C" fn(
    open_jtalk_dic_dir: *const c_char,
    out_open_jtalk: *mut *mut OpenJtalkRc,
) -> VoicevoxResultCode;
type FnOpenJtalkDelete = unsafe extern "C" fn(open_jtalk: *mut OpenJtalkRc);
type FnMakeDefaultInitOptions = unsafe extern "C" fn() -> VoicevoxInitializeOptions;
type FnSynthesizerNew = unsafe extern "C" fn(
    onnxruntime: *const VoicevoxOnnxruntime,
    open_jtalk: *const OpenJtalkRc,
    options: VoicevoxInitializeOptions,
    out_synthesizer: *mut *mut VoicevoxSynthesizer,
) -> VoicevoxResultCode;
type FnSynthesizerDelete = unsafe extern "C" fn(synthesizer: *mut VoicevoxSynthesizer);
type FnVoiceModelOpen = unsafe extern "C" fn(
    path: *const c_char,
    out_model: *mut *mut VoicevoxVoiceModelFile,
) -> VoicevoxResultCode;
type FnVoiceModelDelete = unsafe extern "C" fn(model: *mut VoicevoxVoiceModelFile);
type FnSynthesizerLoadVoiceModel = unsafe extern "C" fn(
    synthesizer: *const VoicevoxSynthesizer,
    model: *const VoicevoxVoiceModelFile,
) -> VoicevoxResultCode;
type FnMakeDefaultTtsOptions = unsafe extern "C" fn() -> VoicevoxTtsOptions;
type FnSynthesizerTts = unsafe extern "C" fn(
    synthesizer: *const VoicevoxSynthesizer,
    text: *const c_char,
    style_id: VoicevoxStyleId,
    options: VoicevoxTtsOptions,
    output_wav_length: *mut usize,
    output_wav: *mut *mut u8,
) -> VoicevoxResultCode;
type FnWavFree = unsafe extern "C" fn(wav: *mut u8);
type FnErrorMessage = unsafe extern "C" fn(code: VoicevoxResultCode) -> *const c_char;

struct Fns {
    load_onnxruntime: FnLoadOnnxruntime,
    open_jtalk_new: FnOpenJtalkNew,
    open_jtalk_delete: FnOpenJtalkDelete,
    make_default_init_options: FnMakeDefaultInitOptions,
    synthesizer_new: FnSynthesizerNew,
    synthesizer_delete: FnSynthesizerDelete,
    voice_model_open: FnVoiceModelOpen,
    voice_model_delete: FnVoiceModelDelete,
    synthesizer_load_voice_model: FnSynthesizerLoadVoiceModel,
    make_default_tts_options: FnMakeDefaultTtsOptions,
    synthesizer_tts: FnSynthesizerTts,
    wav_free: FnWavFree,
    error_message: FnErrorMessage,
}

impl Fns {
    /// libloading binds symbols to the Library lifetime; we copy the
    /// raw fn-pointer out so we can drop the Symbol wrapper and keep
    /// the typed function around. Safe because the owning [`Inner`]
    /// holds the Library for as long as we use these pointers.
    unsafe fn resolve(lib: &Library) -> anyhow::Result<Self> {
        unsafe fn sym<T: Copy>(lib: &Library, name: &[u8]) -> anyhow::Result<T> {
            let s: Symbol<T> = unsafe { lib.get(name) }
                .with_context(|| format!("missing symbol {}", String::from_utf8_lossy(name)))?;
            Ok(*s)
        }
        Ok(Self {
            load_onnxruntime: unsafe { sym(lib, b"voicevox_onnxruntime_load_once\0")? },
            open_jtalk_new: unsafe { sym(lib, b"voicevox_open_jtalk_rc_new\0")? },
            open_jtalk_delete: unsafe { sym(lib, b"voicevox_open_jtalk_rc_delete\0")? },
            make_default_init_options: unsafe {
                sym(lib, b"voicevox_make_default_initialize_options\0")?
            },
            synthesizer_new: unsafe { sym(lib, b"voicevox_synthesizer_new\0")? },
            synthesizer_delete: unsafe { sym(lib, b"voicevox_synthesizer_delete\0")? },
            voice_model_open: unsafe { sym(lib, b"voicevox_voice_model_file_open\0")? },
            voice_model_delete: unsafe { sym(lib, b"voicevox_voice_model_file_delete\0")? },
            synthesizer_load_voice_model: unsafe {
                sym(lib, b"voicevox_synthesizer_load_voice_model\0")?
            },
            make_default_tts_options: unsafe { sym(lib, b"voicevox_make_default_tts_options\0")? },
            synthesizer_tts: unsafe { sym(lib, b"voicevox_synthesizer_tts\0")? },
            wav_free: unsafe { sym(lib, b"voicevox_wav_free\0")? },
            error_message: unsafe { sym(lib, b"voicevox_error_result_to_message\0")? },
        })
    }
}

struct Inner {
    voice_models: Vec<*mut VoicevoxVoiceModelFile>,
    synthesizer: *mut VoicevoxSynthesizer,
    open_jtalk: *mut OpenJtalkRc,
    fns: Fns,
    // ONNX runtime is a process-global singleton — no delete API.
    _onnxruntime: *const VoicevoxOnnxruntime,
    _core_lib: Library,
}

// Raw FFI handles. We serialize access via tokio Mutex so the engine
// (not documented as reentrant) is only touched one task at a time.
// Only `Send` is needed — `tokio::sync::Mutex<T>` is `Sync` for any
// `T: Send`, and every FFI call goes through `blocking_lock()`, so
// `Inner` itself never needs to be shared across threads without the
// mutex guard.
unsafe impl Send for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            for &m in &self.voice_models {
                (self.fns.voice_model_delete)(m);
            }
            if !self.synthesizer.is_null() {
                (self.fns.synthesizer_delete)(self.synthesizer);
            }
            if !self.open_jtalk.is_null() {
                (self.fns.open_jtalk_delete)(self.open_jtalk);
            }
        }
    }
}

#[derive(Clone)]
pub struct Tts {
    inner: Arc<Mutex<Inner>>,
}

impl Tts {
    /// Pre-download the ~700 MB of VOICEVOX assets to disk (no-op
    /// if all four marker dirs are already present next to the
    /// executable). Separated from [`Tts::load`] so callers can
    /// run it before starting any progress UI of their own —
    /// `voicevox_downloader` paints its own license pager + DL
    /// progress bar, and an indicatif spinner painting on top of
    /// that fight for the same terminal lines and corrupt each
    /// other on scroll.
    ///
    /// `license_accepted` works the same as in [`TtsConfig`]: if
    /// false and assets are missing, this errors out pointing at
    /// `kotonoha setup-voicevox`.
    pub async fn ensure_assets(license_accepted: bool) -> anyhow::Result<()> {
        ensure_voicevox_assets(license_accepted).await
    }

    /// Load the engine. Ensures the ~700 MB of assets are on disk
    /// (running the official downloader on first call), opens
    /// `c_api/lib/voicevox_core.dll`, walks `models/vvms/*.vvm` and
    /// registers each model with the synthesizer.
    pub async fn load(cfg: &TtsConfig) -> anyhow::Result<Self> {
        tracing::info!(
            "loading voicevox (preloading speakers {:?})",
            cfg.speaker_ids
        );

        ensure_voicevox_assets(cfg.license_accepted).await?;

        let speaker_ids = cfg.speaker_ids.clone();
        let on_event = cfg.on_event.clone();
        let inner = tokio::task::spawn_blocking(move || -> anyhow::Result<Inner> {
            build_inner(speaker_ids, on_event)
        })
        .await
        .context("spawn_blocking voicevox load")??;

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// Synthesize Japanese `text` with the given speaker id. Caller
    /// must ensure that `speaker_id` belongs to a voice model that
    /// was loaded at `Tts::load` time (we load every `.vvm` in
    /// `models/vvms/`).
    pub async fn synthesize_wav(&self, text: &str, speaker_id: u32) -> anyhow::Result<Vec<u8>> {
        let inner = self.inner.clone();
        let text = text.to_string();
        let wav = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let inner = inner.blocking_lock();
            let text_cstr = CString::new(text).context("text contains NUL")?;
            let opts = unsafe { (inner.fns.make_default_tts_options)() };
            let mut wav_len: usize = 0;
            let mut wav_ptr: *mut u8 = std::ptr::null_mut();
            let code = unsafe {
                (inner.fns.synthesizer_tts)(
                    inner.synthesizer,
                    text_cstr.as_ptr(),
                    speaker_id,
                    opts,
                    &mut wav_len,
                    &mut wav_ptr,
                )
            };
            // Defensive: the C-API doesn't promise wav_ptr is null
            // on error, and we don't want to leak a partial buffer
            // when we bail out below.
            if code != 0 {
                if !wav_ptr.is_null() {
                    unsafe { (inner.fns.wav_free)(wav_ptr) };
                }
                check_code(&inner.fns, code, "voicevox_synthesizer_tts")?;
            }
            if wav_ptr.is_null() || wav_len == 0 {
                if !wav_ptr.is_null() {
                    unsafe { (inner.fns.wav_free)(wav_ptr) };
                }
                anyhow::bail!("voicevox_synthesizer_tts returned an empty wav");
            }
            // SAFETY: ptr/len returned by voicevox_synthesizer_tts;
            // we copy into a Vec and immediately free via wav_free.
            let bytes = unsafe { std::slice::from_raw_parts(wav_ptr, wav_len) }.to_vec();
            unsafe { (inner.fns.wav_free)(wav_ptr) };
            Ok(bytes)
        })
        .await
        .context("spawn_blocking voicevox tts")??;
        Ok(wav)
    }
}

fn build_inner(
    speaker_ids: Vec<u32>,
    on_event: Option<LoadEventCallback>,
) -> anyhow::Result<Inner> {
    let exe_dir = current_exe_dir()?;
    let core_dll = exe_dir
        .join("c_api")
        .join("lib")
        .join(if cfg!(target_os = "windows") {
            "voicevox_core.dll"
        } else if cfg!(target_os = "macos") {
            "libvoicevox_core.dylib"
        } else {
            "libvoicevox_core.so"
        });
    let onnx_dll = exe_dir
        .join("onnxruntime")
        .join("lib")
        .join(if cfg!(target_os = "windows") {
            "voicevox_onnxruntime.dll"
        } else if cfg!(target_os = "macos") {
            "libvoicevox_onnxruntime.dylib"
        } else {
            "libvoicevox_onnxruntime.so"
        });

    tracing::info!("loading voicevox_core from {}", core_dll.display());
    let lib = unsafe { Library::new(&core_dll) }
        .with_context(|| format!("Library::new {}", core_dll.display()))?;
    let fns = unsafe { Fns::resolve(&lib)? };

    // ONNX runtime load — supply the full path so libloading inside
    // voicevox_core picks up the bundled .dll rather than searching
    // PATH (where it might be missing or stale). The ONNX runtime
    // is a process-global singleton (no per-instance delete API), so
    // failing here doesn't leave anything to clean up.
    let onnx_path_cstr = path_to_cstring(&onnx_dll)?;
    let load_opts = VoicevoxLoadOnnxruntimeOptions {
        filename: onnx_path_cstr.as_ptr(),
    };
    let mut onnxruntime: *const VoicevoxOnnxruntime = std::ptr::null();
    let code = unsafe { (fns.load_onnxruntime)(load_opts, &mut onnxruntime) };
    check_code(&fns, code, "voicevox_onnxruntime_load_once")?;

    // Construct the owning Inner *before* the open_jtalk + synthesizer
    // allocations so any `?` from here on hands cleanup to
    // `Drop for Inner`. Previously a failure in synthesizer_new
    // leaked the open_jtalk handle.
    let mut inner = Inner {
        voice_models: Vec::new(),
        synthesizer: std::ptr::null_mut(),
        open_jtalk: std::ptr::null_mut(),
        fns,
        _onnxruntime: onnxruntime,
        _core_lib: lib,
    };

    // Open JTalk dictionary.
    let jtalk_dir = exe_dir.join("dict").join("open_jtalk_dic_utf_8-1.11");
    let jtalk_cstr = path_to_cstring(&jtalk_dir)?;
    let code = unsafe { (inner.fns.open_jtalk_new)(jtalk_cstr.as_ptr(), &mut inner.open_jtalk) };
    check_code(&inner.fns, code, "voicevox_open_jtalk_rc_new")?;

    // Synthesizer with default options (Auto acceleration, env-derived thread count).
    let init_opts = unsafe { (inner.fns.make_default_init_options)() };
    let code = unsafe {
        (inner.fns.synthesizer_new)(
            onnxruntime,
            inner.open_jtalk,
            init_opts,
            &mut inner.synthesizer,
        )
    };
    check_code(&inner.fns, code, "voicevox_synthesizer_new")?;

    if let Some(cb) = &on_event {
        cb(LoadEvent::EngineReady);
    }

    // Load every .vvm under models/vvms/ so any style id in the
    // bundled set is usable at runtime. Failures on individual
    // models are logged + skipped — a corrupt single .vvm shouldn't
    // bring down the whole engine init.
    let vvms_dir = exe_dir.join("models").join("vvms");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&vvms_dir)
        .with_context(|| format!("read_dir {}", vvms_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .map(|x| x.eq_ignore_ascii_case("vvm"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();
    for path in entries {
        let path_cstr = match path_to_cstring(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("skip {}: {e}", path.display());
                continue;
            }
        };
        let mut model: *mut VoicevoxVoiceModelFile = std::ptr::null_mut();
        let code = unsafe { (inner.fns.voice_model_open)(path_cstr.as_ptr(), &mut model) };
        if code != 0 {
            tracing::warn!(
                "voicevox_voice_model_file_open {}: {}",
                path.display(),
                code_message(&inner.fns, code)
            );
            continue;
        }
        let code = unsafe { (inner.fns.synthesizer_load_voice_model)(inner.synthesizer, model) };
        if code != 0 {
            tracing::warn!(
                "voicevox_synthesizer_load_voice_model {}: {}",
                path.display(),
                code_message(&inner.fns, code)
            );
            unsafe { (inner.fns.voice_model_delete)(model) };
            continue;
        }
        inner.voice_models.push(model);
    }

    if inner.voice_models.is_empty() {
        // `Drop for Inner` cleans up synthesizer + open_jtalk for us.
        anyhow::bail!(
            "no voice models loaded from {} — run `kotonoha setup-voicevox` to download them",
            vvms_dir.display()
        );
    }

    // Fire SpeakerLoaded for each id the UI is tracking. The model
    // load above is per-.vvm, not per-speaker-id, so this is just
    // driving the bar to "done".
    if let Some(cb) = &on_event {
        for id in &speaker_ids {
            cb(LoadEvent::SpeakerLoaded { id: *id });
        }
    }

    tracing::info!(
        "voicevox ready: {} voice models loaded from {}",
        inner.voice_models.len(),
        vvms_dir.display()
    );

    Ok(inner)
}

fn current_exe_dir() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_exe()
        .context("locating current exe")?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("exe has no parent dir"))?
        .to_path_buf())
}

fn path_to_cstring(p: &Path) -> anyhow::Result<CString> {
    CString::new(p.to_string_lossy().into_owned())
        .with_context(|| format!("path {} contains NUL byte", p.display()))
}

fn check_code(fns: &Fns, code: VoicevoxResultCode, context: &str) -> anyhow::Result<()> {
    if code == 0 {
        return Ok(());
    }
    anyhow::bail!(
        "{context} failed: {} (code {code})",
        code_message(fns, code)
    )
}

fn code_message(fns: &Fns, code: VoicevoxResultCode) -> String {
    let ptr = unsafe { (fns.error_message)(code) };
    if ptr.is_null() {
        return format!("code {code}");
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

// ─── Asset bootstrapping (kept from v0.1.12) ────────────────────

/// Make sure the c_api / onnxruntime / models / dict asset
/// directories already exist next to the executable. If any are
/// missing, run the official `voicevox_downloader` ourselves with
/// `y` piped on stdin so the license-agreement prompt doesn't
/// stall it forever.
async fn ensure_voicevox_assets(license_accepted: bool) -> anyhow::Result<()> {
    let exe_dir = current_exe_dir()?;

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

    // Refuse to silently auto-accept on the user's behalf. The
    // downloader's agreement prompt represents a real license the
    // caller's user has to opt into. The `kotonoha setup-voicevox`
    // CLI gathers that consent up front (or honors --accept-license
    // in scripted runs) and sets `license_accepted: true`. The
    // server's idle-init path passes `false` so it can never trip
    // the download itself — it errors out and points the user at
    // the setup command instead.
    if !license_accepted {
        anyhow::bail!(
            "VOICEVOX 利用規約への同意が未確認のため自動 download を中断しました。\n\
             先に `kotonoha setup-voicevox` を実行して規約に同意してください。\n\
             (CI / scripted セットアップでは `kotonoha setup-voicevox --accept-license`)"
        );
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
            cmd.env("MINUS_PAGER", "cat")
                .env("TERM", "dumb")
                .stdin(Stdio::piped())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            let mut child = cmd.spawn().context("spawning voicevox_downloader")?;
            if let Some(mut stdin) = child.stdin.take() {
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
    if cfg!(target_os = "windows") {
        let alt = exe_dir.join("voicevox_downloader");
        if tokio::fs::try_exists(&alt).await.unwrap_or(false) {
            tokio::fs::copy(&alt, &target)
                .await
                .context("copy voicevox_downloader -> .exe")?;
            return Ok(target);
        }
    }

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
