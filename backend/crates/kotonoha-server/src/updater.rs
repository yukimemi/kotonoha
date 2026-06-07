//! Background self-update wiring, built on `kaishin` 0.5.0.
//!
//! By default `kotonoha` is **opt-out silent install**: on launch it
//! quietly checks GitHub for a newer release and, if found, downloads +
//! swaps its own binary in the background. The running process keeps the
//! old binary; the new version applies on the next launch. A single
//! stderr line is printed only when an install actually happened.
//!
//! The behaviour is controlled by the `[update] auto_update` config field
//! (`off` / `notify` / `install`, default `install`) and can be force-
//! disabled with the `KOTONOHA_NO_AUTOUPDATE` env var.
//!
//! The long-running `serve` path uses the fire-and-forget
//! [`kaishin::Checker::spawn_auto_update`] / banner spawn (the server may
//! never exit cleanly, so there is no "finalize at exit" hook). The
//! short-lived setup subcommands use [`maybe_spawn_auto_update_check`] +
//! [`finalize_auto_update_check`] with a short bounded wait so a fast
//! command never hangs on a slow network.

use std::path::PathBuf;
use std::time::Duration;

use kotonoha_core::AutoUpdateMode;

/// The application/binary name. The crate is published as
/// `kotonoha-server`, but the user-facing executable (and GitHub repo)
/// is `kotonoha`, which is what the running binary detection and GitHub
/// release asset names use.
const BIN_NAME: &str = "kotonoha";
const OWNER: &str = "yukimemi";
const REPO: &str = "kotonoha";
/// The crates.io package name (`kotonoha-server`) — differs from
/// [`BIN_NAME`], and kaishin's `cargo install` fallback needs the
/// package, not the binary. Resolved at compile time so it can never
/// drift from `Cargo.toml`.
const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Build the `KaishinOptions` describing this binary.
pub fn kaishin_opts() -> kaishin::KaishinOptions {
    kaishin::KaishinOptions::new(OWNER, REPO, BIN_NAME, env!("CARGO_PKG_VERSION"))
        .crate_name(CRATE_NAME)
}

/// Resolve the transient update-check state file path:
/// `<cache dir>/kotonoha/last_update_check.json`.
///
/// This is throttle/cache state that can be safely deleted and
/// re-created, so it lives under the OS CACHE directory
/// (`dirs::cache_dir()`), not the persistent data directory that
/// kaishin would otherwise pick via [`kaishin::default_state_path`].
/// Returns `None` if the cache dir can't be resolved (then the checker
/// falls back to kaishin's default — resilience).
fn state_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join(BIN_NAME).join("last_update_check.json"))
}

/// Construct a `Checker` honouring the configured throttle interval.
///
/// Invalid interval strings fall back to [`kaishin::default_interval`]
/// with a `tracing::warn`. The throttle state file is pinned under the
/// OS cache dir (see [`state_path`]) rather than kaishin's data-dir
/// default.
fn checker(interval: Option<&str>) -> kaishin::Checker {
    let interval = interval
        .and_then(|s| match kaishin::parse_interval(s) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!("invalid [update] update_check_interval {s:?}: {e}; using default");
                None
            }
        })
        .unwrap_or_else(kaishin::default_interval);
    let mut checker = kaishin::Checker::new(BIN_NAME, kaishin_opts()).interval(interval);
    if let Some(path) = state_path() {
        checker = checker.state_path(path);
    }
    checker
}

/// Run the interactive `kotonoha self-update` flow.
pub async fn run_self_update(
    yes: bool,
    check_only: bool,
    non_interactive: bool,
) -> anyhow::Result<()> {
    let opts = kaishin_opts();
    let upd_opts = kaishin::UpdateOptions::new()
        .yes(yes)
        .check_only(check_only)
        .non_interactive(non_interactive);
    kaishin::run_self_update(&opts, upd_opts).await
}

/// Kick off the background auto-update for the long-running `serve`
/// path. Fire-and-forget — the server process may never exit, so there
/// is nothing to finalize.
///
/// - `Off` → nothing.
/// - `Notify` → spawn a check and print a banner if a newer release
///   exists (never installs).
/// - `Install` → [`kaishin::Checker::spawn_auto_update`] (silent
///   background download + swap). The "restart to apply" notice can't be
///   reliably timed against a never-returning server, so the install
///   simply applies on next launch; the user sees it via `self-update`
///   / the next start.
pub fn spawn_serve_auto_update(mode: AutoUpdateMode, interval: Option<&str>) {
    match mode {
        AutoUpdateMode::Off => {}
        AutoUpdateMode::Notify => {
            let checker = checker(interval);
            tokio::spawn(async move {
                // Resilience: any network/lock failure stays silent.
                if let Ok(Some(latest)) = checker.check_and_save().await {
                    eprintln!("\n{}", checker.format_banner(&latest));
                }
            });
        }
        AutoUpdateMode::Install => {
            checker(interval).spawn_auto_update();
        }
    }
}

/// A pending background auto-update check for the short-lived
/// subcommands, consumed by [`finalize_auto_update_check`].
pub enum AutoUpdateHandle {
    /// Throttle window not elapsed, but a cached result already shows a
    /// newer release — print the banner without a fetch.
    CachedAvailable {
        checker: kaishin::Checker,
        latest: kaishin::LatestRelease,
    },
    /// A background `notify` fetch is in flight; banner on completion.
    Pending {
        checker: kaishin::Checker,
        handle: tokio::task::JoinHandle<anyhow::Result<Option<kaishin::LatestRelease>>>,
        cached_latest: Option<kaishin::LatestRelease>,
    },
    /// A background silent install is in flight; a one-line notice is
    /// printed iff an install actually happened.
    Installing(tokio::task::JoinHandle<anyhow::Result<Option<kaishin::LatestRelease>>>),
}

/// Spawn the appropriate background check for a short-lived subcommand.
///
/// Returns `None` when nothing should be done (mode `Off`, or the
/// throttle window hasn't elapsed and there is no cached newer release).
/// All failures are swallowed (resilience).
pub fn maybe_spawn_auto_update_check(
    mode: AutoUpdateMode,
    interval: Option<&str>,
) -> Option<AutoUpdateHandle> {
    match mode {
        AutoUpdateMode::Off => None,
        AutoUpdateMode::Notify => {
            let checker = checker(interval);
            if !checker.should_check() {
                return checker
                    .cached_update()
                    .map(|latest| AutoUpdateHandle::CachedAvailable { checker, latest });
            }
            let cached_latest = checker.cached_update();
            let checker_clone = checker.clone();
            let handle = tokio::spawn(async move { checker_clone.check_and_save().await });
            Some(AutoUpdateHandle::Pending {
                checker,
                handle,
                cached_latest,
            })
        }
        AutoUpdateMode::Install => {
            let checker = checker(interval);
            if !checker.should_check() {
                return None;
            }
            let handle = tokio::spawn(async move { checker.auto_update().await });
            Some(AutoUpdateHandle::Installing(handle))
        }
    }
}

/// Consume a handle from [`maybe_spawn_auto_update_check`]. Uses a SHORT
/// bounded wait so a fast subcommand never hangs on a slow network. All
/// timeout / network / lock failures stay silent.
pub async fn finalize_auto_update_check(handle: AutoUpdateHandle) {
    // Keep this short — these are fast setup subcommands, not the server.
    const WAIT: Duration = Duration::from_secs(5);
    match handle {
        AutoUpdateHandle::CachedAvailable { checker, latest } => {
            eprintln!("\n{}", checker.format_banner(&latest));
        }
        AutoUpdateHandle::Pending {
            checker,
            handle,
            cached_latest,
        } => match tokio::time::timeout(WAIT, handle).await {
            Ok(Ok(Ok(Some(latest)))) => {
                eprintln!("\n{}", checker.format_banner(&latest));
            }
            Ok(Ok(Ok(None))) => {}
            _ => {
                // Timeout or fetch error: fall back to any cached result.
                if let Some(latest) = cached_latest {
                    eprintln!("\n{}", checker.format_banner(&latest));
                }
            }
        },
        AutoUpdateHandle::Installing(handle) => {
            // Only print when an install actually happened
            // (`auto_update` returns `Ok(Some(_))` iff it installed).
            if let Ok(Ok(Ok(Some(latest)))) = tokio::time::timeout(WAIT, handle).await {
                let version = latest.tag_name.trim_start_matches('v');
                eprintln!(
                    "\u{2713} {BIN_NAME} {version} installed in the background — restart to apply."
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kaishin_opts_use_kotonoha_names() {
        let opts = kaishin_opts();
        assert_eq!(opts.owner, "yukimemi");
        assert_eq!(opts.repo, "kotonoha");
        assert_eq!(opts.bin_name, "kotonoha");
        assert_eq!(opts.current_version, env!("CARGO_PKG_VERSION"));
        // The cargo-install fallback needs the crates.io package name,
        // which differs from the binary name.
        assert_eq!(opts.crate_name.as_deref(), Some("kotonoha-server"));
    }

    #[test]
    fn state_path_is_under_cache_dir() {
        // The transient throttle file must live under the OS cache dir,
        // not the persistent data dir. Skip if the cache dir can't be
        // resolved in this environment.
        if let (Some(path), Some(cache)) = (state_path(), dirs::cache_dir()) {
            assert!(
                path.starts_with(&cache),
                "{path:?} should be under the cache dir {cache:?}"
            );
            assert!(path.ends_with("last_update_check.json"));
        }
    }
}
