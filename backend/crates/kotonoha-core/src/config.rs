use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use teravars::{Context, Engine, extract_vars, resolve, system_context};

use crate::lesson::Lesson;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub vars: BTreeMap<String, toml::Value>,
    pub server: ServerConfig,
    #[serde(default)]
    pub avatars: AvatarsConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default)]
    pub backend: BTreeMap<String, BackendConfig>,
    #[serde(default)]
    pub lesson: BTreeMap<String, LessonRef>,

    #[serde(skip)]
    pub root_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub bind: String,
    #[serde(default = "default_cors")]
    pub cors_allow_origins: Vec<String>,
}

fn default_cors() -> Vec<String> {
    vec!["*".into()]
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AvatarsConfig {
    #[serde(default = "default_avatars_dir")]
    pub dir: String,
    #[serde(default)]
    pub default: String,
}

fn default_avatars_dir() -> String {
    "./avatars".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceConfig {
    #[serde(default = "default_browser")]
    pub stt: String,
    #[serde(default = "default_browser")]
    pub tts: String,
    /// Optional kokoro section — only required when `tts = "kokoro"`.
    pub kokoro: Option<KokoroConfig>,
    /// Optional voicevox section — used when a request asks for
    /// Japanese synthesis (lang=ja or auto-detected).
    pub voicevox: Option<VoicevoxConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KokoroConfig {
    pub model_path: String,
    pub voices_dir: String,
    #[serde(default = "default_kokoro_voice")]
    pub default_voice: String,
    #[serde(default = "default_kokoro_speed")]
    pub speed: f32,
}

fn default_browser() -> String {
    "browser".into()
}

fn default_kokoro_voice() -> String {
    "af_heart".into()
}

fn default_kokoro_speed() -> f32 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoicevoxConfig {
    /// Numeric VOICEVOX speaker id (e.g. 8 = 春日部つむぎ ノーマル).
    /// See `kotonoha-tts::voicevox` docs for the curated subset and
    /// each character's license requirements.
    #[serde(default = "default_voicevox_speaker")]
    pub default_speaker_id: u32,
    /// Speaker ids to pre-load on engine init. On-demand loading
    /// still works for everything else; this just hides the
    /// first-call latency for the speakers you expect to use.
    #[serde(default = "default_voicevox_preload")]
    pub preload_speakers: Vec<u32>,
}

fn default_voicevox_speaker() -> u32 {
    8
}

fn default_voicevox_preload() -> Vec<u32> {
    vec![2, 3, 8] // 四国めたん, ずんだもん, 春日部つむぎ
}

/// Backend configuration — either a local CLI subprocess (claude /
/// gemini / codex shipped binaries) or a direct HTTP API call.
///
/// Discriminated by which fields are present (untagged), so existing
/// CLI entries like `[backend.claude] cmd = "..." args = [...]` still
/// parse without adding a `type = "cli"` line.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BackendConfig {
    Api(ApiBackendConfig),
    Cli(CliBackendConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub struct CliBackendConfig {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiBackendConfig {
    /// Provider key — currently `"google"` (Gemini).  Future:
    /// `"anthropic"` / `"openai"`.
    pub provider: String,
    /// Model identifier passed to the provider.  Examples:
    ///   - google:    `"gemini-2.5-flash"`, `"gemini-2.5-pro"`
    ///   - anthropic: `"claude-sonnet-4-6"`
    ///   - openai:    `"gpt-4o-mini"`
    pub model: String,
    /// Name of the env var holding the API key (e.g. `"GEMINI_API_KEY"`).
    /// Resolving at request time means a missing key only breaks API
    /// backends — CLI backends keep working zero-config.
    pub api_key_env: String,
    /// Optional per-call temperature (provider default if omitted).
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LessonRef {
    pub extends: String,
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let rendered = render_toml(path)?;
        let mut cfg: Config = toml::from_str(&rendered)?;
        cfg.root_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(cfg)
    }

    pub fn load_lesson(&self, name: &str) -> anyhow::Result<Lesson> {
        let lesson_ref = self
            .lesson
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown lesson: {name}"))?;
        let lesson_path = self.root_dir.join(&lesson_ref.extends);
        Lesson::load(&lesson_path)
    }

    pub fn default_lesson_name(&self) -> &str {
        self.vars
            .get("default_lesson")
            .and_then(|v| v.as_str())
            .unwrap_or("elementary-low")
    }

    pub fn default_backend_name(&self) -> &str {
        self.vars
            .get("default_backend")
            .and_then(|v| v.as_str())
            .unwrap_or("claude")
    }

    /// The avatars directory.  Resolved relative to the current
    /// working directory (where the server was launched), or used
    /// as-is if absolute.  Intentionally NOT relative to the config
    /// file — users expect `./avatars` in the TOML to mean
    /// `<project_root>/avatars` when running from the project root.
    pub fn avatars_dir(&self) -> PathBuf {
        PathBuf::from(&self.avatars.dir)
    }
}

/// Read a TOML file, run teravars's `[vars]` self-resolve, then
/// render Tera placeholders against the resolved vars + system
/// context. The result is plain TOML ready for `toml::from_str`.
pub fn render_toml(path: &Path) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let mut engine = Engine::new();
    let mut vars = extract_vars(&raw)?;
    // Best-effort: if cross-refs don't converge, keep going with
    // partially-resolved values rather than failing the whole load.
    let _ = resolve(&mut vars, &mut engine);

    let mut ctx: Context = system_context();
    ctx.insert("vars", &vars);

    let rendered = engine.render(&raw, &ctx)?;
    Ok(rendered)
}
