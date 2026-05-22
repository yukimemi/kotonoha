// Emotion-tag parser for teacher avatar facial expressions.
//
// The lesson system prompt instructs the LLM to start every turn
// with `[joy]` / `[anger]` / `[sad]` / `[relaxed]` / `[neutral]`,
// and optionally insert more mid-turn. This module pulls those
// tokens out of the streaming text, maps them to VRM expression
// names, and returns the stripped text + sequence of emotions seen
// so callers can update both the chat bubble and the avatar.

export type Emotion = "neutral" | "happy" | "sad" | "angry" | "relaxed" | "surprised";

// The LLM gets a small, opinionated vocabulary in the prompt
// (joy/anger/sad/relaxed/neutral). We also accept the raw VRM
// expression names (happy/angry/surprised) so a model that ignores
// the prompt and picks "happy" anyway still works.
const TAG_TO_EMOTION: Record<string, Emotion> = {
  joy: "happy",
  happy: "happy",
  anger: "angry",
  angry: "angry",
  sad: "sad",
  relaxed: "relaxed",
  neutral: "neutral",
  surprised: "surprised",
};

// Tag form: `[emotion]` optionally followed by one whitespace.
// The trailing-space eat is important — without it, `[joy] Hi`
// strips to ` Hi` (leading space stays). `gi` so we walk every
// occurrence in the chunk.
const TAG_RE = /\[(joy|anger|sad|relaxed|neutral|happy|surprised)\]\s?/gi;

/** Strip emotion tags from `text` and return the cleaned text
 *  plus the ordered sequence of emotions encountered. Multiple
 *  tags in the same input are returned in source order so the
 *  caller can replay them against the avatar in turn. */
export function extractEmotions(text: string): { stripped: string; emotions: Emotion[] } {
  const emotions: Emotion[] = [];
  const stripped = text.replace(TAG_RE, (_full, tag: string) => {
    const e = TAG_TO_EMOTION[tag.toLowerCase()];
    if (e) emotions.push(e);
    return "";
  });
  return { stripped, emotions };
}

/** Streaming-safe split: extract emotions from the portion of
 *  `raw` that's guaranteed to be tag-complete, and return whatever
 *  trails an unclosed `[` so the caller can re-feed it next chunk.
 *
 *  Without this, a chunk boundary inside a tag (`[j` arrives,
 *  `oy]` arrives later) would render the bracketed prefix to the
 *  user before we could strip it. The split point is the last `[`
 *  that has no `]` after it — everything before is safe. */
export function feedEmotionStream(raw: string): {
  safeStripped: string;
  emotions: Emotion[];
  rest: string;
} {
  const lastBracket = raw.lastIndexOf("[");
  let safe = raw;
  let rest = "";
  if (lastBracket >= 0 && !raw.slice(lastBracket).includes("]")) {
    safe = raw.slice(0, lastBracket);
    rest = raw.slice(lastBracket);
  }
  const { stripped, emotions } = extractEmotions(safe);
  return { safeStripped: stripped, emotions, rest };
}
