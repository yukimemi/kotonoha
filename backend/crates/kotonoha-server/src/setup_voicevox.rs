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

use anyhow::anyhow;
use clap::Args;

use kotonoha_core::Config;
use kotonoha_tts::voicevox::{Tts as VoicevoxTts, TtsConfig as VoicevoxConfig};

#[derive(Debug, Args)]
pub struct SetupVoicevoxArgs {
    /// Override the speakers to preload. Defaults to
    /// `[voice.voicevox].preload_speakers` from the config.
    #[arg(long, value_name = "ID", num_args = 1.., value_delimiter = ',')]
    pub speakers: Option<Vec<u32>>,
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
    eprintln!("downloading VOICEVOX core + ONNX runtime…");
    eprintln!("preloading speaker models: {speakers:?}");

    let tts_cfg = VoicevoxConfig {
        speaker_ids: speakers.clone(),
    };
    let _tts = VoicevoxTts::load(&tts_cfg).await?;

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
