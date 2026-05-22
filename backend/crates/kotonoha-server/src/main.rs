use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use axum::extract::State;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use clap::{Parser, Subcommand};
use tokio::sync::OnceCell;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use kotonoha_core::Config;
use kotonoha_tts::kokoro::Tts as KokoroTts;
use kotonoha_tts::voicevox::Tts as VoicevoxTts;

mod setup_tts;
mod setup_voicevox;
mod web;
mod ws;

#[derive(Debug, Parser)]
#[command(name = "kotonoha", version, about)]
struct Cli {
    /// Path to kotonoha.toml.
    #[arg(
        long,
        env = "KOTONOHA_CONFIG",
        default_value = "./configs/kotonoha.toml",
        global = true
    )]
    config: PathBuf,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Run the server (default when no subcommand is given).
    Serve,
    /// Download the Kokoro 82M ONNX model + curated voice files into
    /// the directories specified by `[voice.kokoro]` in the config.
    ///
    /// Equivalent to `bun run scripts/setup-tts.ts` from the source
    /// tree, but works for users who installed via `cargo install`
    /// and don't have bun / the repo checked out.
    SetupTts(setup_tts::SetupTtsArgs),
    /// Download the VOICEVOX core library + ONNX runtime + curated
    /// speaker models into the binary's directory. Required for
    /// Japanese-language TTS (Kokoro's misaki-lean phonemizer is
    /// English-only).
    SetupVoicevox(setup_voicevox::SetupVoicevoxArgs),
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    /// Lazily-loaded Kokoro engine. First /api/tts request pays the
    /// model-load cost (~200ms-1s); subsequent requests are warm.
    pub kokoro: Arc<OnceCell<KokoroTts>>,
    /// Lazily-loaded VOICEVOX engine. First call to the JA path
    /// pays the core-library download (if not already cached) +
    /// model init; subsequent calls are warm.
    pub voicevox: Arc<OnceCell<VoicevoxTts>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,kotonoha=debug,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let config = Config::load(&cli.config)
        .with_context(|| format!("load config {}", cli.config.display()))?;

    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => run_serve(config).await,
        Cmd::SetupTts(args) => setup_tts::run(&config, args).await,
        Cmd::SetupVoicevox(args) => setup_voicevox::run(&config, args).await,
    }
}

async fn run_serve(config: Config) -> anyhow::Result<()> {
    let bind: SocketAddr = config.server.bind.parse()?;
    let avatars_dir = config.avatars_dir();
    let state = AppState {
        config: Arc::new(config),
        kokoro: Arc::new(OnceCell::new()),
        voicevox: Arc::new(OnceCell::new()),
    };

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        .allow_origin(Any);

    let app = Router::new()
        .route("/api/info", get(info))
        .route("/api/tts", post(tts))
        .route("/ws/chat", get(ws::ws_handler))
        .nest_service("/avatars", ServeDir::new(&avatars_dir))
        // `fallback` runs after every other route fails — covers `/`
        // and the SPA's client-side routes via rust-embed.
        .fallback(web::serve)
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    // Print a click-through URL the browser can actually resolve.
    // The bind address can be `0.0.0.0` / `[::]` (listen on every
    // interface), which browsers don't resolve as a destination —
    // swap that for `localhost` so Ctrl+click in a terminal opens
    // the dashboard. Keep the literal `bind=` for ops who need it.
    //
    // IPv6 literals must be bracketed in URLs (`http://[::1]:7400/`)
    // per RFC 3986 § 3.2.2; the bare `bind.ip().to_string()` form
    // would render as `http://::1:7400/` which browsers refuse.
    let click_host = if bind.ip().is_unspecified() {
        "localhost".to_string()
    } else if bind.is_ipv6() {
        format!("[{}]", bind.ip())
    } else {
        bind.ip().to_string()
    };
    tracing::info!(
        "kotonoha-server listening on http://{click_host}:{port}/ (bind={bind}), avatars from {avatars}",
        port = bind.port(),
        avatars = avatars_dir
            .canonicalize()
            .unwrap_or_else(|_| avatars_dir.clone())
            .display(),
    );
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// Subcommand impl lives in `setup_tts.rs` for length.

#[axum::debug_handler]
async fn info(State(state): State<AppState>) -> axum::Json<serde_json::Value> {
    let backends: Vec<&str> = state.config.backend.keys().map(String::as_str).collect();
    let lessons: Vec<&str> = state.config.lesson.keys().map(String::as_str).collect();
    let avatars: Vec<String> = list_avatars(&state.config.avatars_dir());
    let kokoro_voices: Vec<String> = state
        .config
        .voice
        .kokoro
        .as_ref()
        .map(|k| list_voice_bins(std::path::Path::new(&k.voices_dir)))
        .unwrap_or_default();
    let kokoro_default = state
        .config
        .voice
        .kokoro
        .as_ref()
        .map(|k| k.default_voice.clone());
    let voicevox_default = state
        .config
        .voice
        .voicevox
        .as_ref()
        .map(|v| v.default_speaker_id);
    let voicevox_speakers = state
        .config
        .voice
        .voicevox
        .as_ref()
        .map(|v| v.preload_speakers.clone())
        .unwrap_or_default();

    axum::Json(serde_json::json!({
        "backends": backends,
        "lessons":  lessons,
        "avatars":  avatars,
        "defaults": {
            "backend": state.config.default_backend_name(),
            "lesson":  state.config.default_lesson_name(),
            "avatar":  state.config.avatars.default,
        },
        "voice": {
            "stt": state.config.voice.stt,
            "tts": state.config.voice.tts,
            "kokoro_voices":   kokoro_voices,
            "kokoro_default":  kokoro_default,
            "voicevox_default": voicevox_default,
            "voicevox_speakers": voicevox_speakers,
        },
    }))
}

/// Crude language detector used to route `/api/tts` between Kokoro
/// (English) and VOICEVOX (Japanese). Any hiragana / katakana /
/// CJK ideograph in the text → Japanese. Otherwise English. This
/// is intentionally simple — the frontend can also pass an explicit
/// `lang` on the request to override.
fn detect_lang(text: &str) -> &'static str {
    let has_ja = text.chars().any(|c| {
        matches!(c,
            '\u{3040}'..='\u{309f}'   // hiragana
            | '\u{30a0}'..='\u{30ff}'  // katakana
            | '\u{4e00}'..='\u{9fff}'  // CJK unified ideographs
        )
    });
    if has_ja { "ja" } else { "en" }
}

#[derive(Debug, serde::Deserialize)]
struct TtsRequest {
    text: String,
    /// Kokoro voice name (e.g. "jf_alpha", "af_heart").
    #[serde(default)]
    voice: Option<String>,
    /// VOICEVOX speaker id (numeric).
    #[serde(default)]
    speaker_id: Option<u32>,
    /// Playback rate for Kokoro (VOICEVOX uses its own pacing).
    #[serde(default)]
    speed: Option<f32>,
    /// Routing hint: "en" → Kokoro, "ja" → VOICEVOX, "auto" or
    /// missing → script-based detection (hiragana / katakana /
    /// kanji present → ja, else en).
    #[serde(default)]
    lang: Option<String>,
}

async fn tts(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<TtsRequest>,
) -> Result<Response, (StatusCode, String)> {
    let lang = req
        .lang
        .as_deref()
        .filter(|l| matches!(*l, "en" | "ja"))
        .unwrap_or_else(|| detect_lang(&req.text));

    match lang {
        "ja" => synth_voicevox(&state, &req).await,
        _ => synth_kokoro(&state, &req).await,
    }
}

async fn synth_kokoro(
    state: &AppState,
    req: &TtsRequest,
) -> Result<Response, (StatusCode, String)> {
    let kokoro_cfg = state.config.voice.kokoro.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "kokoro not configured".into(),
    ))?;

    let tts = state
        .kokoro
        .get_or_try_init(|| async {
            KokoroTts::load(&kotonoha_tts::kokoro::TtsConfig {
                model_path: kokoro_cfg.model_path.clone().into(),
                voices_dir: kokoro_cfg.voices_dir.clone().into(),
            })
            .await
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("kokoro init: {e:#}"),
            )
        })?;

    // `synthesize_wav` borrows the voice — no need to clone the
    // config default when no override was supplied.
    let voice = req.voice.as_deref().unwrap_or(&kokoro_cfg.default_voice);
    let speed = req.speed.unwrap_or(kokoro_cfg.speed);
    let wav = tts
        .synthesize_wav(&req.text, voice, speed)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("kokoro synth: {e:#}"),
            )
        })?;

    Ok(([(header::CONTENT_TYPE, "audio/wav")], wav).into_response())
}

async fn synth_voicevox(
    state: &AppState,
    req: &TtsRequest,
) -> Result<Response, (StatusCode, String)> {
    let vv_cfg = state.config.voice.voicevox.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "voicevox not configured — add `[voice.voicevox]` to configs/kotonoha.toml \
         and run `kotonoha setup-voicevox` first"
            .into(),
    ))?;

    let tts = state
        .voicevox
        .get_or_try_init(|| async {
            VoicevoxTts::load(&kotonoha_tts::voicevox::TtsConfig {
                speaker_ids: vv_cfg.preload_speakers.clone(),
                on_event: None,
            })
            .await
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("voicevox init: {e:#}"),
            )
        })?;

    let speaker_id = req.speaker_id.unwrap_or(vv_cfg.default_speaker_id);
    let wav = tts
        .synthesize_wav(&req.text, speaker_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("voicevox synth: {e:#}"),
            )
        })?;

    Ok(([(header::CONTENT_TYPE, "audio/wav")], wav).into_response())
}

fn list_voice_bins(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".bin").map(|s| s.to_string())
        })
        .collect();
    out.sort();
    out
}

fn list_avatars(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.to_lowercase().ends_with(".vrm")).then_some(name)
        })
        .collect();
    out.sort();
    out
}
