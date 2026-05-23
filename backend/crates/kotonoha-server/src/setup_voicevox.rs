//! `kotonoha setup-voicevox` — download VOICEVOX core + ONNX runtime
//! and preload speaker models so the first JA request is fast.
//!
//! `voicevox-dyn` does the actual downloading — this subcommand
//! just calls `VoiceVox::load()` eagerly with the configured
//! speakers so it happens at install time instead of during the
//! first server request.
//!
//! License note: each VOICEVOX speaker (春日部つむぎ, 四国めたん,
//! ずんだもん, ...) has its own usage terms. Verify per character
//! before shipping to end users — kotonoha's UI must credit
//! "VOICEVOX:<character>" wherever a speaker is exposed.

use std::time::Duration;

use anyhow::anyhow;
use clap::Args;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use kotonoha_core::Config;
use kotonoha_tts::voicevox::{Tts as VoicevoxTts, TtsConfig as VoicevoxConfig};

#[derive(Debug, Args)]
pub struct SetupVoicevoxArgs {
    /// Override the speakers to preload. Defaults to
    /// `[voice.voicevox].preload_speakers` from the config.
    #[arg(long, value_name = "ID", num_args = 1.., value_delimiter = ',')]
    pub speakers: Option<Vec<u32>>,

    /// Suppress the progress bars (still prints the summary). Useful
    /// when scripting setup from CI where TTY width is unstable.
    #[arg(long)]
    pub no_progress: bool,

    /// Accept the VOICEVOX licenses non-interactively. Without this,
    /// the first run shows the license URLs and prompts y/N before
    /// starting the ~700 MB download.
    #[arg(long, short = 'y', alias = "yes")]
    pub accept_license: bool,
}

pub async fn run(config: &Config, args: SetupVoicevoxArgs) -> anyhow::Result<()> {
    let vv_cfg = config.voice.voicevox.as_ref().ok_or_else(|| {
        anyhow!(
            "no `[voice.voicevox]` section in configs/kotonoha.toml — \
             add one with `default_speaker_id` + `preload_speakers` \
             before running `kotonoha setup-voicevox`, or pass \
             `--speakers <ID,...>` to override."
        )
    })?;

    let speakers = args
        .speakers
        .unwrap_or_else(|| vv_cfg.preload_speakers.clone());

    // License gate. Skip if the assets are already on disk (we know
    // the user accepted previously, downloader won't run again).
    // Skip if the user opted in via --accept-license. Otherwise
    // print the URLs and ask interactively.
    let license_accepted = if assets_already_present()? || args.accept_license {
        true
    } else {
        prompt_license_acceptance()?
    };
    if !license_accepted {
        anyhow::bail!("VOICEVOX 利用規約に同意されなかったため終了します。");
    }

    // voicevox doesn't surface byte-level progress, so the download
    // bar is a spinner — accurate completion signal, no percent. The
    // per-speaker bar IS accurate (we count load_model calls
    // ourselves below). --no-progress falls back to plain eprintln
    // so log scrapers stay happy.
    if args.no_progress {
        eprintln!("downloading VOICEVOX core + ONNX runtime…");
        eprintln!("preloading speaker models: {speakers:?}");
        let tts_cfg = VoicevoxConfig {
            speaker_ids: speakers.clone(),
            on_event: None,
            license_accepted,
        };
        let _tts = VoicevoxTts::load(&tts_cfg).await?;
    } else {
        let mp = MultiProgress::new();

        let download = mp.add(ProgressBar::new_spinner());
        download.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        download.set_message("downloading VOICEVOX core + ONNX runtime + initializing engine");
        download.enable_steady_tick(Duration::from_millis(100));

        let preload = mp.add(ProgressBar::new(speakers.len() as u64));
        preload.set_style(
            ProgressStyle::with_template("{bar:32.green/blue} {pos}/{len} {msg}").unwrap(),
        );
        preload.set_message("(waiting for engine init)");

        let preload_for_cb = preload.clone();
        let download_for_cb = download.clone();
        let on_event =
            std::sync::Arc::new(move |evt: kotonoha_tts::voicevox::LoadEvent| match evt {
                kotonoha_tts::voicevox::LoadEvent::EngineReady => {
                    download_for_cb.finish_with_message("VOICEVOX engine ready");
                    preload_for_cb.set_message("loading speaker model…");
                }
                kotonoha_tts::voicevox::LoadEvent::SpeakerLoaded { id } => {
                    preload_for_cb.inc(1);
                    preload_for_cb.set_message(format!("speaker {id} loaded"));
                }
            })
                as std::sync::Arc<dyn Fn(kotonoha_tts::voicevox::LoadEvent) + Send + Sync>;

        let tts_cfg = VoicevoxConfig {
            speaker_ids: speakers.clone(),
            on_event: Some(on_event),
            license_accepted,
        };
        let _tts = VoicevoxTts::load(&tts_cfg).await?;
        preload.finish_with_message("done");
    }

    eprintln!();
    eprintln!("✅ Done.");
    eprintln!();
    eprintln!("License notice:");
    eprintln!("  Each VOICEVOX speaker has its own usage terms.");
    eprintln!("  Verify per-character before redistributing, and");
    eprintln!("  surface `VOICEVOX:<character>` in any UI that");
    eprintln!("  exposes a speaker. See https://voicevox.hiroshiba.jp");
    Ok(())
}

/// Walk the four sibling asset directories voicevox_downloader
/// produces. If they're all present we know a previous run already
/// completed (and therefore that the user already accepted the
/// licenses); skip the interactive prompt.
fn assets_already_present() -> anyhow::Result<bool> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("exe has no parent dir"))?
        .to_path_buf();
    Ok(["c_api", "onnxruntime", "models", "dict"]
        .iter()
        .all(|d| exe_dir.join(d).is_dir()))
}

/// Print the VOICEVOX license URLs and read a y/N from stdin.
/// Returns true only on an explicit "y" / "yes" (case-insensitive).
fn prompt_license_acceptance() -> anyhow::Result<bool> {
    use std::io::{BufRead, Write};
    let mut stderr = std::io::stderr();
    writeln!(stderr)?;
    writeln!(
        stderr,
        "─────────────────────────────────────────────────────────"
    )?;
    writeln!(
        stderr,
        "VOICEVOX のセットアップには以下の規約への同意が必要です。"
    )?;
    writeln!(stderr)?;
    writeln!(
        stderr,
        "  公式ページ (規約 + 各キャラクター個別ページへのリンク):"
    )?;
    writeln!(stderr, "    https://voicevox.hiroshiba.jp/term/")?;
    writeln!(stderr)?;
    writeln!(stderr, "  代表的なキャラクターの利用規約:")?;
    writeln!(
        stderr,
        "    春日部つむぎ:   https://tsumugi-official.studio.site/rule"
    )?;
    writeln!(stderr, "    四国めたん / ずんだもん 等:")?;
    writeln!(
        stderr,
        "                    https://zunko.jp/con_ongen_kiyaku.html"
    )?;
    writeln!(
        stderr,
        "    冥鳴ひまり:     https://meimeihimari.wixsite.com/himari/terms-of-use"
    )?;
    writeln!(stderr)?;
    writeln!(stderr, "  ・商用 / 非商用ともに利用可、ただし生成音声には")?;
    writeln!(
        stderr,
        "    「VOICEVOX:<キャラクター名>」のクレジット表記が必須。"
    )?;
    writeln!(
        stderr,
        "  ・約 700 MB のモデル + ランタイムを kotonoha.exe と同じ"
    )?;
    writeln!(stderr, "    ディレクトリにダウンロードします。")?;
    writeln!(
        stderr,
        "─────────────────────────────────────────────────────────"
    )?;
    write!(stderr, "上記の規約に同意して setup を続行しますか? [y/N]: ")?;
    stderr.flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
