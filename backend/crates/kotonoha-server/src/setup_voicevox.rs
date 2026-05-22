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

    // voicevox-dyn doesn't surface byte-level progress, so the
    // download bar is a spinner — accurate completion signal, no
    // percent. The per-speaker bar IS accurate (we count
    // load_model calls ourselves below). --no-progress falls back
    // to plain eprintln so log scrapers stay happy.
    if args.no_progress {
        eprintln!("downloading VOICEVOX core + ONNX runtime…");
        eprintln!("preloading speaker models: {speakers:?}");
        let tts_cfg = VoicevoxConfig {
            speaker_ids: speakers.clone(),
            on_event: None,
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
