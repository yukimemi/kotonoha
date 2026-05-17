use std::pin::Pin;
use std::process::Stdio;

use anyhow::Context as _;
use async_stream::try_stream;
use futures::Stream;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::config::CliBackendConfig;
use crate::session::Turn;

/// A streaming text source — yields chunks of the teacher's reply.
pub type ReplyStream = Pin<Box<dyn Stream<Item = anyhow::Result<String>> + Send>>;

/// A single "talk to the teacher" request, in a backend-neutral shape.
/// CLI backends will flatten this to a single stdin prompt; HTTP API
/// backends will translate `turns` into the provider's message array.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub system_prompt: String,
    /// Conversation so far, ending with the latest `Student` turn that
    /// the backend should respond to.
    pub turns: Vec<Turn>,
}

#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> anyhow::Result<ReplyStream>;
}

/// Generic CLI backend — spawns `cmd args...`, writes a flattened prompt
/// to its stdin, and streams stdout chunks back. Used unchanged for
/// claude / gemini / codex; the difference is just `cmd` + `args`.
#[derive(Debug, Clone)]
pub struct CliBackend {
    pub cmd: String,
    pub args: Vec<String>,
}

impl From<&CliBackendConfig> for CliBackend {
    fn from(cfg: &CliBackendConfig) -> Self {
        Self {
            cmd: cfg.cmd.clone(),
            args: cfg.args.clone(),
        }
    }
}

#[async_trait::async_trait]
impl Backend for CliBackend {
    async fn complete(&self, req: CompletionRequest) -> anyhow::Result<ReplyStream> {
        let prompt = render_cli_prompt(&req.system_prompt, &req.turns);
        let (cmd, args) = resolve_cmd(&self.cmd, &self.args);

        let mut child = Command::new(&cmd)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn {cmd}"))?;

        let mut stdin = child.stdin.take().context("take stdin")?;
        let stdout = child.stdout.take().context("take stdout")?;
        let stderr = child.stderr.take().context("take stderr")?;

        // Write the prompt and close stdin so the CLI sees EOF and
        // starts responding.
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });

        let cmd_for_log = cmd.clone();
        tokio::spawn(async move {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            if reader.read_to_string(&mut buf).await.is_ok() && !buf.trim().is_empty() {
                tracing::debug!(target: "kotonoha::backend", cmd = %cmd_for_log, "stderr: {buf}");
            }
        });

        let stream = try_stream! {
            let mut reader = BufReader::new(stdout);
            let mut buf = [0u8; 4096];
            loop {
                let n = reader.read(&mut buf).await?;
                if n == 0 { break; }
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                yield chunk;
            }
            let status = child.wait().await?;
            if !status.success() {
                Err(anyhow::anyhow!("{cmd} exited with status {status}"))?;
            }
            let _ = writer.await;
        };

        Ok(Box::pin(stream))
    }
}

/// Flatten a `CompletionRequest` to the `system / conversation / task`
/// format the CLI backends were already trained on (this is what
/// `Session::render_prompt` used to do). Public so other crates that
/// want CLI-style prompts (tests, debug tools) can reuse it.
pub fn render_cli_prompt(system_prompt: &str, turns: &[Turn]) -> String {
    let mut buf = String::with_capacity(1024 + turns.len() * 64);
    buf.push_str("[SYSTEM]\n");
    buf.push_str(system_prompt.trim());
    buf.push_str("\n\n[CONVERSATION]\n");
    if turns.is_empty() {
        buf.push_str("(this is the first turn — greet the student warmly in English and ask them a simple opening question.)\n");
    } else {
        for turn in turns {
            match turn {
                Turn::Student(t) => {
                    buf.push_str("Student: ");
                    buf.push_str(t.trim());
                    buf.push('\n');
                }
                Turn::Teacher(t) => {
                    buf.push_str("Teacher: ");
                    buf.push_str(t.trim());
                    buf.push('\n');
                }
            }
        }
    }
    buf.push_str("\n[TASK]\nReply as Kotonoha-sensei (Teacher). Output only the teacher's next utterance — no labels, no quotes, no stage directions.\n");
    buf
}

/// Resolve `cmd` to something `tokio::process::Command` can actually run.
///
/// On Windows, `Command::spawn` calls `CreateProcess`, which can NOT
/// directly execute PowerShell scripts (`.ps1`) or batch files
/// (`.cmd` / `.bat`).  We resolve via `which::which` (which honors
/// PATHEXT) and wrap each form appropriately:
///   - `.ps1` → `powershell -NoProfile -ExecutionPolicy Bypass -File <path>`
///   - `.cmd` / `.bat` → `cmd.exe /C <path>`
///   - real `.exe` → use the resolved full path directly
fn resolve_cmd(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    match which::which(cmd) {
        Ok(resolved) => {
            tracing::info!(
                target: "kotonoha::backend",
                "resolved cmd `{}` -> {}",
                cmd,
                resolved.display()
            );
            let ext = resolved
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase());
            let resolved_str = resolved.to_string_lossy().into_owned();
            match ext.as_deref() {
                Some("ps1") => {
                    let mut full = vec![
                        "-NoProfile".into(),
                        "-ExecutionPolicy".into(),
                        "Bypass".into(),
                        "-File".into(),
                        resolved_str,
                    ];
                    full.extend(args.iter().cloned());
                    tracing::info!(target: "kotonoha::backend", "→ powershell -File wrap");
                    ("powershell".into(), full)
                }
                Some("cmd") | Some("bat") => {
                    let mut full = vec!["/C".into(), resolved_str];
                    full.extend(args.iter().cloned());
                    tracing::info!(target: "kotonoha::backend", "→ cmd.exe /C wrap");
                    ("cmd.exe".into(), full)
                }
                _ => (resolved_str, args.to_vec()),
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "kotonoha::backend",
                "`which({cmd})` failed: {e}. Looked in PATH = {:?}",
                std::env::var("PATH").unwrap_or_default()
            );
            (cmd.to_string(), args.to_vec())
        }
    }
}
