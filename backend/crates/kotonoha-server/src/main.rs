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
use kotonoha_tts::Tts;

mod setup_tts;
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
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    /// Lazily-loaded Kokoro engine. First /api/tts request pays the
    /// model-load cost (~200ms-1s); subsequent requests are warm.
    pub tts: Arc<OnceCell<Tts>>,
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
    }
}

async fn run_serve(config: Config) -> anyhow::Result<()> {
    let bind: SocketAddr = config.server.bind.parse()?;
    let avatars_dir = config.avatars_dir();
    let state = AppState {
        config: Arc::new(config),
        tts: Arc::new(OnceCell::new()),
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
    let click_host = if bind.ip().is_unspecified() {
        "localhost".to_string()
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
            "kokoro_voices":  kokoro_voices,
            "kokoro_default": kokoro_default,
        },
    }))
}

#[derive(Debug, serde::Deserialize)]
struct TtsRequest {
    text: String,
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    speed: Option<f32>,
}

async fn tts(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<TtsRequest>,
) -> Result<Response, (StatusCode, String)> {
    let kokoro_cfg = state.config.voice.kokoro.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "kokoro not configured".into(),
    ))?;

    // First request initializes; subsequent requests reuse the warm engine.
    let tts = state
        .tts
        .get_or_try_init(|| async {
            Tts::load(&kotonoha_tts::TtsConfig {
                model_path: kokoro_cfg.model_path.clone().into(),
                voices_dir: kokoro_cfg.voices_dir.clone().into(),
            })
            .await
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("tts init: {e:#}"),
            )
        })?;

    let voice = req
        .voice
        .unwrap_or_else(|| kokoro_cfg.default_voice.clone());
    let speed = req.speed.unwrap_or(kokoro_cfg.speed);
    let wav = tts
        .synthesize_wav(&req.text, &voice, speed)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("synth: {e:#}")))?;

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
