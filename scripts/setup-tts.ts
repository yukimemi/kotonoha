#!/usr/bin/env bun
//
// Download Kokoro 82M ONNX model + a curated voice set into `models/kokoro/`.
// Run from the project root:
//
//   bun run setup:tts                # default voices
//   bun run setup:tts -- --full      # full-precision model (325MB)
//   bun run setup:tts -- --all-voices
//
// Idempotent: skips files that already exist.

import { existsSync, mkdirSync, statSync, createWriteStream } from "node:fs";
import { join, dirname } from "node:path";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const HF_REPO = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main";

const args = new Set(process.argv.slice(2));
const useFull = args.has("--full");
const allVoices = args.has("--all-voices");

/** Smallest model that still sounds OK; full-precision is 3.5x bigger.
 *  q4f16 ≈ 92 MB,  q8f16 ≈ 160 MB,  full ≈ 325 MB. */
const MODEL_FILE = useFull ? "onnx/model.onnx" : "onnx/model_q4f16.onnx";

/** Curated set of voices. Bias towards "cute female teacher" energy.  */
const DEFAULT_VOICES = [
  "af_heart",   // bright + warm American female (default)
  "af_bella",   // softer American female
  "af_nicole",  // ASMR-soft American female
  "af_sky",     // young / cheerful American female
  "jf_alpha",   // Japanese female (for bilingual moments)
  "bf_emma",    // British female alternative
];

/** Full v1.0 voice list — handy to have when --all-voices is passed. */
const ALL_VOICES = [
  "af_alloy", "af_aoede", "af_bella", "af_heart", "af_jessica",
  "af_kore", "af_nicole", "af_nova", "af_river", "af_sarah", "af_sky",
  "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam",
  "am_michael", "am_onyx", "am_puck", "am_santa",
  "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
  "bm_daniel", "bm_fable", "bm_george", "bm_lewis",
  "ef_dora", "em_alex", "em_santa",
  "ff_siwis",
  "hf_alpha", "hf_beta", "hm_omega", "hm_psi",
  "if_sara", "im_nicola",
  "jf_alpha", "jf_gongitsune", "jf_nezumi", "jf_tebukuro", "jm_kumo",
  "pf_dora", "pm_alex", "pm_santa",
  "zf_xiaobei", "zf_xiaoni", "zf_xiaoxiao", "zf_xiaoyi",
  "zm_yunjian", "zm_yunxi", "zm_yunxia", "zm_yunyang",
];

const voiceList = allVoices ? ALL_VOICES : DEFAULT_VOICES;

const projectRoot = process.cwd();
const modelDir    = join(projectRoot, "models", "kokoro");
const voicesDir   = join(modelDir, "voices");
const modelOut    = join(modelDir, "model.onnx");

mkdirSync(voicesDir, { recursive: true });

async function downloadIfMissing(url: string, dest: string) {
  if (existsSync(dest) && statSync(dest).size > 1000) {
    console.log(`✓ already have ${dest.replace(projectRoot, ".")} (${(statSync(dest).size / 1024 / 1024).toFixed(1)} MB)`);
    return;
  }
  console.log(`↓ ${url}`);
  const res = await fetch(url);
  if (!res.ok || !res.body) {
    throw new Error(`HTTP ${res.status} for ${url}`);
  }
  mkdirSync(dirname(dest), { recursive: true });
  await pipeline(Readable.fromWeb(res.body as never), createWriteStream(dest));
  const size = (statSync(dest).size / 1024 / 1024).toFixed(1);
  console.log(`  saved ${dest.replace(projectRoot, ".")} (${size} MB)`);
}

async function main() {
  console.log(`models -> ${modelDir}`);
  console.log(`voices -> ${voicesDir} (${voiceList.length} files)`);

  await downloadIfMissing(`${HF_REPO}/${MODEL_FILE}`, modelOut);

  for (const v of voiceList) {
    await downloadIfMissing(`${HF_REPO}/voices/${v}.bin`, join(voicesDir, `${v}.bin`));
  }

  console.log("\nDone. Edit configs/kotonoha.toml [voice] section to enable kokoro:");
  console.log(`
[voice]
tts = "kokoro"

[voice.kokoro]
model_path = "./models/kokoro/model.onnx"
voices_dir = "./models/kokoro/voices"
default_voice = "af_heart"
`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
