<!-- kata:agents:base:begin -->
## Shared conventions

This file is the agent-agnostic source of truth (per the
[agents.md](https://agents.md) convention). The matching
`CLAUDE.md` and `GEMINI.md` files are thin shims that point back
here so each tool's auto-load behaviour still finds something.
**Edit AGENTS.md, not the shims.**

### Git workflow

- **No direct push to `main`.** Open a PR.
  - Exception: trivial typo / whitespace / docs wording fixes.
- Branch names: `feat/...`, `fix/...`, `chore/...`.
- **PR titles + bodies in English. Commit messages in English.**
- **Releases are PR-driven, tagging is automatic.** Bump
  `[workspace.package].version` (workspace) or `[package].version`
  (single crate) in a `chore/release-vX.Y.Z` PR. On merge to `main`,
  `.github/workflows/auto-tag.yml` (kata-managed) detects the bump,
  pushes the `vX.Y.Z` tag, and that tag fires `release.yml` for
  binary builds + crates.io publish. **Do not run `git tag` by
  hand** — the bot tag will collide and the manual push fails.

### PR review cycle

- Every PR runs reviews from **Claude Code**
  (`.github/workflows/claude-review.yml`, kata-managed) and
  **CodeRabbit**. Wait for both bots to post, address their
  comments (push fixes to the PR branch), and merge only after
  feedback is resolved. The claude-review workflow skips
  review-exempt PRs by itself (its job-level `if:` excludes
  `chore/release-*`, `kata-apply/auto`, `apm-bump/auto`, and
  Renovate / Dependabot authors) — a missing Claude review on
  those PRs is expected, not a failure.
- **Any PR that touches the Claude workflow files goes
  unreviewed.** `claude-code-action` requires the workflow file to
  already exist on the default branch **with identical content** —
  otherwise a PR could rewrite the workflow to exfiltrate the
  token. When the content differs it logs "Skipping action due to
  workflow validation" and exits 0 without reviewing: a green
  check with no review attached. This covers two cases, and the
  second is the one that keeps surprising people:
  - the PR that first adopts these templates (the workflow does
    not exist on the default branch yet), and
  - any later PR that **edits** `claude-review.yml` / `claude.yml`,
    e.g. hand-pulling an upstream template fix.

  Not fixable from this side — it is the mechanism that makes the
  token safe to hand to the action at all. Expected: merge on CI +
  owner approval; reviews resume on the next PR that leaves the
  workflows alone. The `kata-apply/auto` branch is already excluded
  by the job-level `if:`, so the daily template-refresh PRs do not
  add noise here.
- **A missing credential fails loudly instead.** If the repo has
  neither `CLAUDE_CODE_OAUTH_TOKEN` nor `ANTHROPIC_API_KEY` set,
  the guard step fails the job — set one and re-run (subscription
  path: `claude setup-token` → `gh secret set`; pay-as-you-go:
  store `ANTHROPIC_API_KEY` and swap the action input to
  `anthropic_api_key`). Distinguishing the two: **red** means no
  credential, **green with no review** means workflow validation.
- **The Claude full review fires once, at PR open** (plus
  `ready_for_review` / `reopened`) — fix pushes do **not** re-trigger
  it (`synchronize` is deliberately off the trigger list; a full
  re-review per push doubled up with the mention-driven re-check
  below and burned tokens for no extra signal). Verification of
  fixes rides the `@claude` thread replies. After a large rework
  that changes the PR's shape, request a fresh full pass
  explicitly: `@claude please re-review the full PR`. CodeRabbit
  still reviews pushes on its own cadence (its app config, not
  this workflow).
- **After opening a PR, immediately enter the review-monitoring
  loop — do not ask the user whether to start it.** Drive the
  cadence with `/loop` — fixed-interval mode (e.g.
  `/loop 60s …`) schedules ticks via `CronCreate`; dynamic mode
  (no interval, `/loop …`) self-paces via `ScheduleWakeup`. The
  agent actively pulls fresh state each tick with
  `gh pr view <N> --json state,reviews,comments,statusCheckRollup`
  and `gh api repos/<owner>/<repo>/pulls/<N>/comments` (the
  latter covers inline review comments, which `gh pr view`
  does not surface) and reacts to new bot feedback. Passive
  watchers (background `gh` polls, file watchers, hooks) cannot
  trigger active follow-up, so they are not a substitute —
  without an active wake-up the agent never re-reads the PR.
- **Default polling interval: 60s.** Claude Code review /
  CodeRabbit typically reply within ~1–5 minutes of a push or
  thread reply, so a 60s tick catches them on the next wake-up
  without burning cache: 60s sits well inside the 5-minute
  prompt-cache TTL, so the conversation context stays cached
  across ticks. Do **not** stretch the interval to 300s — that
  is the worst-of-both window (you pay the cache miss without
  amortizing it). If the PR is idle but a bot re-review is still
  expected (e.g. a CodeRabbit rate-limit refill window), step
  **up** to 1200–1800s instead.
- **Stop the loop entirely when only owner approval is missing.**
  Once review bots are quiet (or quiet-by-exception — version-bump
  skip, Renovate/Dependabot skip), CI is green, and there is no
  other expected follow-up, the *only* remaining action is human
  approval. GitHub already notifies the owner; the agent
  re-entering on every cron tick to find the same "still waiting
  on owner" state burns cache and adds no value. Stop scheduling
  further wake-ups (`CronDelete` in fixed-interval mode; simply
  omit the next `ScheduleWakeup` in dynamic mode) and report the
  wait state to the user. The owner restarts the loop after their
  next push if a fresh bot pass is wanted, or merges directly.
  (A CodeRabbit rate-limit window doesn't qualify on its own — a
  re-review is still expected once the quota refills, so step up
  to 1200–1800s instead and let it ride. Stopping is only correct
  when the owner has explicitly chosen to skip the bot pass per
  the rate-limit exception below.)
- **Reply to reviewers after pushing a fix — in each thread, not
  at the top level.** Every finding lives in its own inline review
  thread; answer *each* one as an in-thread reply, carrying an
  **@-mention** (`@claude` / `@coderabbitai`). Use the review-
  comment *replies* endpoint — `gh api repos/<owner>/<repo>/pulls/<N>/comments/<comment_id>/replies -f body=…`
  (or `-F in_reply_to=<comment_id> -f body=…` on the comments
  endpoint — `body` is required there too) — and
  get each comment's `<comment_id>` from
  `gh api repos/<owner>/<repo>/pulls/<N>/comments`. A single
  top-level `gh pr comment` does **not** count: it leaves every
  inline thread unresolved, the bot can't tie your response to the
  finding it raised, and the per-finding audit trail is lost.
  Reply in-thread even when you're **declining** a suggestion —
  say why; a silent skip reads as overlooked. Note `@claude` also
  triggers the interactive responder
  (`.github/workflows/claude.yml`, kata-managed) — it will
  re-check the fix and reply on the thread. Since fix pushes no
  longer re-trigger the full review, this mention-driven re-check
  is the **only** Claude-side verification of a fix — don't skip
  it for substantive fixes; do skip it for pure FYI notes that
  need no verification.
- A review thread is **settled** the moment the latest bot reply
  is ack-only ("Thank you" / "Understood" / a re-review summary
  with no new findings) or 30 minutes elapse with no actionable
  comment.
- **Merge gate**: review bots quiet AND owner explicit approval.
- Bot-authored PRs (Renovate / Dependabot) skip the bot-review
  gate; CI green + owner approval is enough.
- **Version-bump-only PRs** (a single `chore/release-vX.Y.Z`
  branch whose entire diff is `[workspace.package].version` /
  `[package].version` + the matching inter-crate refs +
  `Cargo.lock`) **also skip the bot-review gate.** There is
  nothing for the bots to find in a version bump, and the
  release pipeline downstream of merge (auto-tag → release.yml)
  is time-sensitive. CI green + owner approval is enough.
- **Treat CodeRabbit rate-limit notices as "quiet" for the
  merge gate.** If CodeRabbit only posts a "Review limit
  reached" quota-exhaustion message (no findings, no inline
  comments), it has produced no review content — there is
  nothing to address. Re-trigger with `@coderabbitai review`
  once the quota refills if you want a real pass; for small or
  time-sensitive PRs, merge on owner approval without waiting.

### Worktree workflow

> **Before your FIRST edit to any file, run `renri add` — NEVER edit the
> main checkout.** Read-only inspection (Read / Grep / Glob) stays on the
> main checkout; the instant you intend to *change* a file, you must
> already be in a worktree. The trap that keeps catching agents: diving
> into a fix the moment the diagnosis lands and editing in place. A
> concurrent agent shares the main checkout — your in-place edits will
> clobber theirs or be clobbered, and in a jj-colocated repo a stray
> working-copy commit entangles unrelated WIP into your branch. If you
> slip and edit in the main checkout, capture the diff first (jj already
> snapshotted it into the working-copy commit, so `jj diff > patch`; for
> git, `git stash` or save a patch — if you got as far as committing on a
> branch, just push it). Then reset the main checkout to pristine main
> (`jj new main@origin`, or `git switch -`), `renri add` a worktree, and
> re-apply the captured diff there.

Use [`renri`](https://github.com/yukimemi/renri) for any
commit-bound change. From the main checkout:

```sh
renri add <branch-name> --from main@origin            # create a worktree (jj-first), off latest upstream main
renri --vcs git add <branch-name> --from origin/main  # force a git worktree, off latest upstream main
renri remove <branch-name> -y --non-interactive  # cleanup after merge (agent-safe; see note)
renri prune                        # GC stale worktrees
```

Read-only inspection can stay on the main checkout.

**Always pass `--from <upstream main>`** (`main@origin` for jj,
`origin/main` for git). Without it, `renri add` forks off the *cwd
worktree's current HEAD* — in a long-lived main checkout that often
lags upstream, so the PR later shows up CONFLICTING against a `main`
that had already moved (e.g. a refactor merged upstream before the
branch was cut), forcing a manual re-port of the whole change.
`renri add` does fetch first, but fetching only updates `main@origin`
— it never moves the checkout's HEAD, so an explicit `--from` is what
guarantees a fresh base.

**Agents / non-interactive shells:** `renri remove` prints a details
panel and waits for a confirmation prompt — without `-y` it **hangs**,
and `--non-interactive` *alone* errors asking for `-y`. Always pass
`-y`, and add `--non-interactive` so a mistyped/omitted name fails
instead of opening a fuzzy picker (the same picker-fallback applies to
`remove` / `cd` / `exec` with no name). Use `-f`/`--force` to remove a
worktree that still has uncommitted changes or conflicts. To sweep
every merged-PR worktree in one shot: `renri remove --merged -y`.

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

This repo follows the shared Rust toolchain conventions. The
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
fix to `yukimemi/pj-rust` so every Rust project using these templates picks
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

### Releasing: version bump PR + auto-tag

Releases are triggered from `main` by a Cargo.toml version
change. `.github/workflows/auto-tag.yml` is kata-managed (source:
`yukimemi/pj-rust/.github/workflows/auto-tag.yml.tera`). It
watches `main` and, whenever a commit lands that changes the
top-level `version = "..."` in `Cargo.toml`, it pushes a matching
`vX.Y.Z` tag — no manual `git tag` step is needed. The tag push
then fires `release.yml`; see `kata:agents:rust-lib:*` or
`kata:agents:rust-cli:*` for what release.yml does in each
crate shape.

Cut a release via a small PR — never `git push` the bump
straight to `main`, even though the base block lists version
bumps as an exception to "no direct push". `auto-tag.yml` only
fires on `main`-branch pushes, so the bump must land via a merge
either way; using a PR also gives CI a chance to gate the
release. Enable automerge so CI green = release start:

```sh
git switch -c chore/release-vX.Y.Z
# Edit `package.version` in Cargo.toml, then:
cargo build                     # let Cargo.lock follow
git commit -am "chore: release vX.Y.Z"
git push -u origin chore/release-vX.Y.Z
gh pr create --fill
gh pr merge --auto --squash --delete-branch
```

Once CI is green the PR auto-merges. `auto-tag.yml` then pushes
`vX.Y.Z`, which fires `release.yml`.

**In a workspace, the version is in more than one place.** A member
that is published and depended on by another member is declared
with both a `path` and a `version` — crates.io needs a
requirement it can resolve for somebody who is not building from
the checkout, so a bare `path` will not do:

```toml
my-core = { path = "crates/my-core", version = "0.4.2" }
```

That literal does not follow `[workspace.package] version`.
Nothing in Cargo makes it, and the release above will not either.

**It fails late and quietly.** `version = "0.4.2"` means `^0.4.2`,
so a stale pin keeps resolving through every *patch* release and
stops only at the first bump that crosses the minor — where
`cargo build` refuses with `candidate versions found which didn't
match`, in the middle of cutting the release. Two repos on these
templates hit exactly this, one of them three releases after its
pins were last correct, and the other had already written the
hazard down in prose and drifted anyway.

So bump the pins in the same commit, keep them in
`[workspace.dependencies]` rather than in each member, and assert
it rather than remembering it. A test is the cheapest place —
`cargo test` already runs in CI, and it needs no toolchain a Rust
workspace does not have. [pj-rust-workspace's
README](https://github.com/yukimemi/pj-rust-workspace#the-internal-version-pin-and-the-check-for-it)
carries one to copy into any member's
`tests/check_versions.rs`: `internal_pins_match_the_workspace_version`
fails when a pin and the workspace version disagree, and
`members_inherit_the_workspace_version` fails when a member writes
its own version or reaches for a sibling by path.

**Repo settings to set once:** enable
`delete_branch_on_merge=true` (Settings → General →
"Automatically delete head branches"). The `--delete-branch`
flag on `gh pr merge --auto` is effectively a no-op — gh
returns as soon as automerge is enabled, so the deletion has to
happen server-side, which requires the repo setting.

**Why `KATA_APPLY_TOKEN`:** GitHub refuses to fire downstream
workflows from tags pushed by the default `GITHUB_TOKEN`, so
`auto-tag.yml` pushes with `KATA_APPLY_TOKEN` (the same PAT
`kata-apply.yml` already uses). Each consumer repo needs a
`KATA_APPLY_TOKEN` secret set; if a version-bump merge silently
doesn't fire `release.yml`, the missing PAT is the first thing
to check.
<!-- kata:agents:rust:end -->

## kotonoha — project specifics

A web app where Japanese elementary / junior-high students
practice English conversation with a cute VTuber-style 3D
teacher. The backend offers two flavours of LLM routing:

- **CLI**: spawn `claude` / `gemini` / `codex` per turn — zero
  config, but pays a 2-5s Node.js cold start every turn.
- **API**: hit the provider's HTTP API directly via
  `kotonoha-llm` (Gemini, plus any OpenAI-compatible Chat
  Completions server: OpenRouter / OpenAI / DeepSeek by provider
  key, others via `base_url`; Anthropic in scope) — streaming, no
  cold start, needs an API key in env.

Voice in is browser Web Speech API. Voice out is either the
browser's `speechSynthesis` or local Kokoro 82M ONNX through
`kotonoha-tts`.

### Repo layout

```
kotonoha/
├── backend/crates/
│   ├── kotonoha-core/      # config + lesson loader + Backend trait + CliBackend
│   ├── kotonoha-llm/       # HTTP API providers (Gemini + OpenAI-compatible)
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

### Frontend = bun, lives inside kotonoha-server

`backend/crates/kotonoha-server/web/` holds the React SPA — same
layout as `kanade-backend/web/`. Root `package.json` declares
that subdir as the workspace member so `bun run dev` from the
repo root delegates correctly.

The SPA is **baked into the `kotonoha` binary at compile time**
via `rust-embed` (`src/web.rs`). `cargo install kotonoha-server`
gives you a self-contained binary that serves both the API and
the UI from port 7400 — no separate frontend process needed.

`cargo make web-build` rebuilds `web/dist/`. The CI release
workflow runs it on every OS runner before `cargo build` so the
shipped binary always carries the bundle from the tagged commit.

**`build.rs` placeholder:** if `web/dist/` is empty (fresh clone,
no bun build yet), `build.rs` seeds an `index.html` that
explains how to build the real SPA. Keeps `cargo check` /
kata-managed CI working without bun.

**Windows gotcha:** `bun run vite` on Windows swallows vite's
stdout. Each script in `web/package.json` invokes node
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
