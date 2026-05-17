<!-- kata:agents:base:begin -->
## yukimemi/* shared conventions

This file is the agent-agnostic source of truth (per the
[agents.md](https://agents.md) convention). The matching
`CLAUDE.md` and `GEMINI.md` files are thin shims that point back
here so each tool's auto-load behaviour still finds something.
**Edit AGENTS.md, not the shims.**

### Git workflow

- **No direct push to `main`.** Open a PR.
  - Exception: trivial typo / whitespace / docs wording fixes.
  - Exception: standalone version bumps.
- Branch names: `feat/...`, `fix/...`, `chore/...`.
- **PR titles + bodies in English. Commit messages in English.**
- Tag-based releases: `git tag vX.Y.Z && git push origin vX.Y.Z`.

### PR review cycle

- Every PR runs reviews from **Gemini Code Assist** and
  **CodeRabbit**. Wait for both bots to post, address their
  comments (push fixes to the PR branch), and merge only after
  feedback is resolved.
- **Reply to reviewers after pushing a fix.** Reply on the
  corresponding review thread with an **@-mention**
  (`@gemini-code-assist` / `@coderabbitai`). Silent fixes are
  invisible to reviewers and cost the audit trail.
- A review thread is **settled** the moment the latest bot reply
  is ack-only ("Thank you" / "Understood" / a re-review summary
  with no new findings) or 30 minutes elapse with no actionable
  comment.
- **Merge gate**: review bots quiet AND owner explicit approval.
- Bot-authored PRs (Renovate / Dependabot) skip the bot-review
  gate; CI green + owner approval is enough.

### Worktree workflow

Use [`renri`](https://github.com/yukimemi/renri) for any
commit-bound change. From the main checkout:

```sh
renri add <branch-name>            # create a worktree (jj-first)
renri --vcs git add <branch-name>  # force a git worktree
renri remove <branch-name>         # cleanup after merge
renri prune                        # GC stale worktrees
```

Read-only inspection can stay on the main checkout.

### kata-managed sections

Several files in this repo are managed by `kata apply` from the
[`yukimemi/pj-presets`](https://github.com/yukimemi/pj-presets)
templates — the bytes between `<!-- kata:*:begin -->` and
`<!-- kata:*:end -->` markers, plus the overwrite-always files
listed in `.kata/applied.toml`. **Editing those bytes locally
won't survive the next `kata apply`** — push the change to the
upstream template repo (`yukimemi/pj-base` / `yukimemi/pj-rust` /
…) instead. The marker scopes are layered:

- `kata:agents:base:*` — language-agnostic conventions (this section).
- `kata:agents:rust:*` — added when `pj-rust` applies.
- `kata:agents:rust-cli:*` — added when `pj-rust-cli` applies.
<!-- kata:agents:base:end -->
<!-- kata:agents:rust:begin -->
### Rust workflow

This repo follows the yukimemi/* Rust toolchain conventions. The
language-agnostic conventions block above (`kata:agents:base:*`)
covers git workflow, PR review cycle, and worktree usage.

### Build / lint / test

```sh
cargo make check                    # fmt --check + clippy + test + lock-check (the pre-push gate)
cargo make setup                    # one-time hook install + apm install
cargo build                         # debug build
cargo build --release               # release build
cargo test                          # tests; add -- --nocapture for stdout
```

`cargo make check` is what `.github/workflows/ci.yml` runs and what
the local pre-push hook calls — anything that passes locally
should pass on CI and vice versa. Don't paper over a failing
clippy by sprinkling `#[allow(clippy::...)]`; fix the underlying
issue or push back on the lint with reasoning.

### Toolchain pin

The Rust toolchain is pinned via `rust-toolchain.toml` and the
project compiles with the `stable` channel. Don't introduce
nightly-only features without a real reason; if you do, document
the reason in the relevant module.

### Lint / format policy

`rustfmt.toml` and `clippy.toml` are kata-managed (sourced from
`yukimemi/pj-rust`). Edits to those files in this repo won't
survive the next `kata apply`; if a setting is wrong, push the
fix to `yukimemi/pj-rust` so every yukimemi/* Rust project picks
it up.

### CI workflow

`.github/workflows/ci.yml` is also kata-managed. The source lives
in `yukimemi/pj-rust/.github/workflows/ci.yml.template` (the
`.template` suffix keeps GitHub Actions from running the source
itself in pj-rust); each Rust project receives the rendered
`ci.yml` via `kata apply`. Action versions are bumped centrally
by Renovate at `yukimemi/pj-rust` and propagate down on the next
apply, so don't bump them locally — Renovate is configured
(via the kata-distributed `renovate.json`) to ignore
`.github/workflows/ci.yml` and `.github/workflows/release.yml`
in each PJ to avoid the bump→clobber loop.
<!-- kata:agents:rust:end -->

## kotonoha — project specifics

A web app where Japanese elementary / junior-high students
practice English conversation with a cute VTuber-style 3D
teacher. The backend offers two flavours of LLM routing:

- **CLI**: spawn `claude` / `gemini` / `codex` per turn — zero
  config, but pays a 2-5s Node.js cold start every turn.
- **API**: hit the provider's HTTP API directly via
  `kotonoha-llm` (currently Gemini; Anthropic / OpenAI in scope) —
  streaming, no cold start, needs an API key in env.

Voice in is browser Web Speech API. Voice out is either the
browser's `speechSynthesis` or local Kokoro 82M ONNX through
`kotonoha-tts`.

### Repo layout

```
kotonoha/
├── backend/crates/
│   ├── kotonoha-core/      # config + lesson loader + Backend trait + CliBackend
│   ├── kotonoha-llm/       # HTTP API providers (Gemini today)
│   ├── kotonoha-server/    # axum HTTP + WebSocket + /api/tts + /api/info
│   └── kotonoha-tts/       # Kokoro 82M ONNX wrapper (pure-Rust phonemizer)
├── frontend/               # Vite + React + TS + Tailwind + three-vrm (bun)
├── configs/
│   ├── kotonoha.toml       # backends, voice, defaults
│   └── lessons/*.toml      # per-grade system prompts (teravars)
├── scripts/setup-tts.ts    # downloads Kokoro model + voices
├── models/kokoro/          # gitignored, populated by `cargo make setup-tts`
└── avatars/                # *.vrm files served at /avatars/*
```

### Running locally

Two processes — the Rust server (which spawns CLIs and serves
`/avatars/*` + `/api/*` + `/ws/*`) and the Vite dev server (which
proxies those paths to the Rust server).

```sh
# Optional: API mode needs a key in env (see configs/kotonoha.toml)
$env:GEMINI_API_KEY = "..."

# Terminal A — backend (port 7400 by default)
cargo make server-dev

# Terminal B — frontend (5173, proxies /api + /ws to 7400)
cargo make frontend-dev
```

Open <http://localhost:5173>. From a phone use Tailscale Funnel
(`tailscale funnel --bg --https=443 5173`) — Web Speech API
silently fails on plain HTTP origins.

### Frontend = bun (not pnpm)

Same convention as `kanade-backend/web`. Root `package.json`
declares `frontend` as a workspace so `bun run dev` from the
repo root delegates correctly.

**Windows gotcha:** `bun run vite` on Windows swallows vite's
stdout. Each script in `frontend/package.json` invokes node
directly (`node node_modules/vite/bin/vite.js`) to bypass the
bun script-shim layer. See
`reference_bun_windows_vite_stdout` in agent memory.

### Backend configuration (untagged enum)

`[backend.*]` entries in `configs/kotonoha.toml` are untagged —
serde picks CLI vs API by which fields are present:

- **CLI** = `cmd` + `args`
- **API** = `provider` + `model` + `api_key_env`

The Backend trait in `kotonoha-core::backend` takes a
`CompletionRequest { system_prompt, turns }`. CLI backends flatten
it via `render_cli_prompt`; API backends translate `turns` to the
provider's message array.

#### Windows CLI dispatch

`CliBackend` resolves the binary via `which::which` so PATHEXT
(`.cmd` / `.ps1` / `.bat`) is honored. PowerShell scripts get
wrapped with `powershell.exe -File`; batch files with `cmd.exe /C`.
See `reference_rust_windows_ps1_spawn` in agent memory.

### Kokoro TTS (opt-in)

The `kotonoha-tts` crate wraps `kokoro-en` with the `misaki-lean`
feature so **no espeak-ng C++ build is needed** (the default
`g2p-espeak` feature triggers a multi-minute CMake build on
Windows that often fails outright). Trade-off: G2P is CMU-dict
based, so common English is great but rare proper nouns can be
mispronounced.

Model + voice files live under `./models/kokoro/` (gitignored).
Run `cargo make setup-tts` to populate. Switch between Kokoro and
browser TTS at runtime in the UI's TTS selector.

The server's `/api/tts` endpoint lazy-initializes the engine on
first call and reuses it afterward (`OnceCell`).

Sentence-streaming on the frontend: `voice/kokoro-queue.ts` fires
`/api/tts` for each completed sentence (boundary = `.!?。！？`)
in parallel while serializing audio playback. Cuts perceived
latency from "wait for full LLM reply, then synthesize" to
"audio starts on first sentence boundary."

### Adding a VRM avatar

Drop a `*.vrm` file into `avatars/` and reload — `/api/info`
re-scans every request. `.gitignore` excludes `avatars/*.vrm` so
licensed models don't accidentally get committed. Default avatar
filename is set via `[avatars].default` in `configs/kotonoha.toml`.

### Editing lessons / system prompts

`configs/lessons/*.toml` files are teravars-rendered (TOML +
Tera). `system_prompt` must come **before** `[vars]` (or any
table header) or it gets parsed as a vars-table key:

```toml
system_prompt = """
You are a teacher for {{ vars.grade }} students.
"""

[vars]
grade = "elementary-low"
```

Register the new lesson in `configs/kotonoha.toml` under
`[lesson.<name>] extends = "lessons/<file>.toml"`.

Verify all lessons parse: `cargo run -p kotonoha-core --example dump-lesson`.

### WebSocket protocol

`/ws/chat` carries JSON frames in both directions.

Client → server:
```json
{ "type": "configure", "backend": "gemini-flash", "lesson": "elementary-low" }
{ "type": "user", "text": "Hello!" }
{ "type": "reset" }
```

Server → client:
```json
{ "type": "ready", "backend": "...", "lesson": "..." }
{ "type": "delta", "text": "Hi!" }
{ "type": "done" }
{ "type": "error", "message": "..." }
```

Each user turn re-renders the full system + transcript and pipes
it to the chosen backend. Multi-turn state lives in the server's
per-WebSocket `Session` — no persistence yet.

### Privacy / child safety

The teacher prompts in `configs/lessons/*.toml` include explicit
"keep content appropriate for school-age students" directives.
When tweaking prompts, preserve those guardrails — this is the
only thing standing between a child user and whatever else the
underlying LLM would happily talk about.
