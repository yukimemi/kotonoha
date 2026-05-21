// Curated VOICEVOX speaker catalog.
//
// The full speaker list (~50 entries across ~30 characters x styles)
// is fetchable at runtime from voicevox-core, but for kotonoha's
// "English teacher" persona we only surface a child-safe, female-
// friendly subset by default. Users who want more can extend this
// table; the backend will load any speaker_id on demand.
//
// **License**: each VOICEVOX speaker carries its own usage terms.
// Every entry here is from the publicly-listed VOICEVOX character
// set with "child / educational" usage typically permitted under
// the standard "VOICEVOX:<character>" credit. Verify against
// https://voicevox.hiroshiba.jp/term/ before redistribution.

export type VoicevoxSpeaker = {
  id: number;
  /** Character name (used for the on-screen credit). */
  character: string;
  /** Style name attached to this id (e.g. ノーマル, あまあま). */
  style: string;
  /** Short note shown next to the dropdown entry. */
  hint?: string;
};

export const VOICEVOX_SPEAKERS: VoicevoxSpeaker[] = [
  { id: 8,  character: "春日部つむぎ",       style: "ノーマル", hint: "中学生・落ち着いた先生役 (default)" },
  { id: 2,  character: "四国めたん",         style: "ノーマル", hint: "明るくはっきり" },
  { id: 0,  character: "四国めたん",         style: "あまあま", hint: "やさしい・幼児向け" },
  { id: 3,  character: "ずんだもん",         style: "ノーマル", hint: "親しみやすいマスコット系" },
  { id: 1,  character: "ずんだもん",         style: "あまあま", hint: "やさしい" },
  { id: 10, character: "雨晴はう",           style: "ノーマル", hint: "穏やか・落ち着いた" },
  { id: 14, character: "冥鳴ひまり",         style: "ノーマル", hint: "明るく元気" },
  { id: 11, character: "玄野武宏",           style: "ノーマル", hint: "男性・先生役" },
];

/** Lookup helper — falls back to the default-id label when an id
 *  isn't in the curated list (e.g. user typed one via API). */
export function speakerLabel(id: number): string {
  const sp = VOICEVOX_SPEAKERS.find((s) => s.id === id);
  if (!sp) return `speaker ${id}`;
  return `${sp.character} (${sp.style})`;
}

export function speakerCharacter(id: number): string | undefined {
  return VOICEVOX_SPEAKERS.find((s) => s.id === id)?.character;
}
