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
    #[serde(default)]
    pub update: UpdateConfig,

    #[serde(skip)]
    pub root_dir: PathBuf,
}

/// How `kotonoha` keeps itself up to date in the background.
///
/// On every `kotonoha serve` launch the binary can quietly check GitHub
/// for a newer release and — depending on this mode — install it in the
/// background (the running process keeps the old binary; the new version
/// applies on the next launch). Mirrors the auto-update modes used across
/// the yukimemi/* CLIs.
///
/// - `off` — do nothing.
/// - `notify` — only print a banner when a newer release exists; never
///   install.
/// - `install` (default) — silently download + swap the binary in the
///   background, then print a one-line "restart to apply" notice.
///
/// The `KOTONOHA_NO_AUTOUPDATE` env var (non-empty and not `"0"`/`"false"`)
/// overrides this to `off`, regardless of config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoUpdateMode {
    /// No background update activity at all.
    Off,
    /// Print a banner when a newer release exists, but never install.
    Notify,
    /// Silently install a newer release in the background (default).
    #[default]
    Install,
}

/// `[update]` section of `kotonoha.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateConfig {
    /// Background auto-update behaviour. Defaults to [`AutoUpdateMode::Install`].
    #[serde(default)]
    pub auto_update: AutoUpdateMode,
    /// Minimum interval between consecutive background update checks
    /// (humantime format: `"24h"`, `"6h"`, `"1d"`). Unset → `"24h"`;
    /// invalid values fall back to the default with a warning.
    #[serde(default)]
    pub update_check_interval: Option<String>,
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
    /// Provider key — `"google"` (Gemini), or any OpenAI-compatible
    /// Chat Completions provider: `"openrouter"`, `"openai"`,
    /// `"deepseek"`.  Future: `"anthropic"`.
    pub provider: String,
    /// Model identifier passed to the provider.  Examples:
    ///   - google:     `"gemini-2.5-flash"`, `"gemini-2.5-pro"`
    ///   - openrouter: `"deepseek/deepseek-v4-flash"` (any OpenRouter slug)
    ///   - openai:     `"gpt-4o-mini"`
    ///   - deepseek:   `"deepseek-chat"`
    pub model: String,
    /// Name of the env var holding the API key (e.g. `"GEMINI_API_KEY"`).
    /// Resolving at request time means a missing key only breaks API
    /// backends — CLI backends keep working zero-config.
    pub api_key_env: String,
    /// Optional per-call temperature (provider default if omitted).
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Optional API base URL override.  Mainly for OpenAI-compatible
    /// servers the provider key doesn't already know about (Ollama,
    /// LM Studio, a corporate proxy, ...).  Known providers fill in
    /// their default, so this is rarely needed.
    #[serde(default)]
    pub base_url: Option<String>,
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

    /// Resolve the effective background auto-update mode.
    ///
    /// The `KOTONOHA_NO_AUTOUPDATE` env kill-switch takes precedence over
    /// config and forces [`AutoUpdateMode::Off`]; otherwise the configured
    /// `[update] auto_update` value is used. Folding the kill-switch in
    /// here keeps the resolution in one place so call sites can simply use
    /// `config.update_mode()`.
    pub fn update_mode(&self) -> AutoUpdateMode {
        if auto_update_disabled_by_env() {
            AutoUpdateMode::Off
        } else {
            self.update.auto_update
        }
    }
}

/// Pure truthiness of the kill-switch value: disabled when the value is
/// present, non-empty (after trim), and not `"0"` / `"false"`
/// (case-insensitive).
///
/// Split out from [`auto_update_disabled_by_env`] so the decision logic
/// can be unit-tested **by value**, without mutating the global process
/// environment (which would race under the default parallel test runner).
fn env_value_disables(value: Option<&str>) -> bool {
    match value {
        Some(v) => {
            let v = v.trim();
            !v.is_empty() && !v.eq_ignore_ascii_case("0") && !v.eq_ignore_ascii_case("false")
        }
        None => false,
    }
}

/// True when `KOTONOHA_NO_AUTOUPDATE` is set to a "truthy" value
/// (non-empty and not `"0"` / `"false"`). Takes precedence over config.
///
/// Uses [`std::env::var_os`] so a non-Unicode value still counts as set
/// rather than being silently treated as absent.
fn auto_update_disabled_by_env() -> bool {
    let raw = std::env::var_os("KOTONOHA_NO_AUTOUPDATE");
    let value = raw.as_ref().map(|v| v.to_string_lossy());
    env_value_disables(value.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes access to `KOTONOHA_NO_AUTOUPDATE` across tests. Cargo
    /// runs tests in parallel by default; since `update_mode()` reads this
    /// env var, every test that touches it (setting it, or asserting the
    /// resolved mode) must hold this lock so the reads/writes don't race.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Minimal valid config with an optional trailing `[update]` block.
    fn config_with_update(update_section: &str) -> Config {
        let toml = format!(
            r#"
[server]
bind = "127.0.0.1:7400"
{update_section}
"#
        );
        toml::from_str(&toml).expect("config should parse")
    }

    #[test]
    fn auto_update_defaults_to_install_when_section_absent() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let cfg = config_with_update("");
        assert_eq!(cfg.update.auto_update, AutoUpdateMode::Install);
        assert_eq!(cfg.update_mode(), AutoUpdateMode::Install);
        assert_eq!(cfg.update.update_check_interval, None);
    }

    #[test]
    fn auto_update_defaults_to_install_when_section_present_but_field_absent() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let cfg = config_with_update("[update]\n");
        assert_eq!(cfg.update_mode(), AutoUpdateMode::Install);
    }

    #[test]
    fn auto_update_parses_off() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let cfg = config_with_update("[update]\nauto_update = \"off\"\n");
        assert_eq!(cfg.update_mode(), AutoUpdateMode::Off);
    }

    #[test]
    fn auto_update_parses_notify() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let cfg = config_with_update("[update]\nauto_update = \"notify\"\n");
        assert_eq!(cfg.update_mode(), AutoUpdateMode::Notify);
    }

    #[test]
    fn auto_update_parses_install() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let cfg = config_with_update("[update]\nauto_update = \"install\"\n");
        assert_eq!(cfg.update_mode(), AutoUpdateMode::Install);
    }

    #[test]
    fn auto_update_parses_check_interval() {
        let cfg = config_with_update("[update]\nupdate_check_interval = \"12h\"\n");
        assert_eq!(cfg.update.update_check_interval.as_deref(), Some("12h"));
    }

    #[test]
    fn auto_update_mode_default_is_install() {
        assert_eq!(AutoUpdateMode::default(), AutoUpdateMode::Install);
    }

    /// Pure by-value test of the kill-switch decision. Deliberately does
    /// NOT touch the process environment, so it is safe to run in parallel
    /// with every other test (no `ENV_MUTEX`, no data race).
    #[test]
    fn env_value_disables_truthy_and_falsy() {
        // Absent / unset → not disabled.
        assert!(!env_value_disables(None));

        // Truthy.
        assert!(env_value_disables(Some("1")));
        assert!(env_value_disables(Some("true")));
        assert!(env_value_disables(Some("  yes  "))); // arbitrary non-empty, trimmed

        // Falsy.
        for falsy in ["", "   ", "0", "false", "FALSE", " false "] {
            assert!(
                !env_value_disables(Some(falsy)),
                "{falsy:?} should not disable auto-update"
            );
        }
    }

    #[test]
    fn env_kill_switch_forces_off_in_update_mode() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let saved = std::env::var("KOTONOHA_NO_AUTOUPDATE").ok();

        // Config wants `install`, but the env kill-switch wins.
        let cfg = config_with_update("[update]\nauto_update = \"install\"\n");
        unsafe { std::env::set_var("KOTONOHA_NO_AUTOUPDATE", "1") };
        assert_eq!(cfg.update_mode(), AutoUpdateMode::Off);

        match saved {
            Some(v) => unsafe { std::env::set_var("KOTONOHA_NO_AUTOUPDATE", v) },
            None => unsafe { std::env::remove_var("KOTONOHA_NO_AUTOUPDATE") },
        }
    }
}
