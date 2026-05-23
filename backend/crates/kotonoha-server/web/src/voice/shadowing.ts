// Shadowing-lesson tag parser.
//
// The shadowing lesson system prompt instructs the LLM to wrap
// rendered structure in inline tags:
//
//   [target]Hello, how are you?[/target]    -- target sentence to mimic
//   [score 80/100]                          -- 0-100 attempt score
//   [diff target="..." heard="..."]         -- word-level diff data
//
// This module strips those tags out of the chat-bubble text and
// returns the structured parts so ChatPanel can render the target
// as a big highlighted card + a diff line below the feedback. We
// run this on the FINAL (post-streaming) text in `onDone` — tags
// can span chunk boundaries and we'd rather not deal with that
// complexity during streaming. The bubble briefly shows the raw
// `[target]...[/target]` mid-stream and then snaps to the parsed
// form when the turn completes.

export type ShadowingTags = {
  /** "Mimic this" sentence. Rendered as a large card. */
  target?: string;
  /** 0-N out of max-N. */
  score?: { value: number; max: number };
  /** Word-level diff input. Frontend computes the actual diff. */
  diff?: { target: string; heard: string };
};

// `[\s\S]*?` so newlines inside the target are allowed.
const TARGET_RE = /\[target\]([\s\S]*?)\[\/target\]/g;
const SCORE_RE = /\[score\s+(\d+)\s*\/\s*(\d+)\]/gi;
// double-quoted attributes; we don't try to support escaped quotes
// because the LLM is asked to keep target/heard strings plain.
const DIFF_RE = /\[diff\s+target="([^"]*)"\s+heard="([^"]*)"\]/gi;

/** Strip shadowing tags from `text` and return the cleaned text
 *  plus the extracted structured parts. First-wins — if the LLM
 *  emits multiple `[target]`s in one turn, only the first is kept. */
export function extractShadowing(text: string): {
  stripped: string;
  tags: ShadowingTags;
} {
  const tags: ShadowingTags = {};
  let stripped = text.replace(TARGET_RE, (_, t: string) => {
    if (!tags.target) tags.target = t.trim();
    return "";
  });
  stripped = stripped.replace(SCORE_RE, (_, v: string, m: string) => {
    if (!tags.score) {
      tags.score = { value: parseInt(v, 10), max: parseInt(m, 10) };
    }
    return "";
  });
  stripped = stripped.replace(DIFF_RE, (_, t: string, h: string) => {
    if (!tags.diff) tags.diff = { target: t, heard: h };
    return "";
  });
  // Collapse 3+ blank lines that the strip can leave behind.
  return { stripped: stripped.replace(/\n{3,}/g, "\n\n").trim(), tags };
}

/** Word-level diff between target + heard, suitable for inline
 *  rendering. Each entry says whether the target word was matched,
 *  missing entirely, or substituted with something else. Lower-cases
 *  + strips punctuation for the comparison so "Hello!" still matches
 *  "hello". */
export type DiffWord = {
  /** The original target word (case + punctuation preserved). */
  target: string;
  /** "ok" — matched; "missed" — not in heard; "wrong" — heard
   *  something different at this position. */
  status: "ok" | "missed" | "wrong";
  /** The student's word at this position (only set when status="wrong"). */
  heard?: string;
};

export function computeDiff(target: string, heard: string): DiffWord[] {
  const t = target.trim().split(/\s+/).filter((w) => w.length > 0);
  // Filter empties so an empty `heard` ("" or whitespace-only)
  // becomes `[]` instead of `[""]` — otherwise the first target
  // word would be reported as a "wrong" substitution with an
  // empty tooltip instead of being marked "missed".
  const h = heard.trim().toLowerCase().split(/\s+/).filter((w) => w.length > 0);
  // O(n²) is fine for short sentences (<20 words).
  const norm = (w: string) => w.toLowerCase().replace(/[^a-z0-9'’]/g, "");
  const heardUsed = new Array<boolean>(h.length).fill(false);
  const out: DiffWord[] = [];
  for (let i = 0; i < t.length; i++) {
    const tn = norm(t[i]);
    // Try same-index match first (preserves order).
    if (i < h.length && norm(h[i]) === tn && !heardUsed[i]) {
      heardUsed[i] = true;
      out.push({ target: t[i], status: "ok" });
      continue;
    }
    // Otherwise scan: maybe the word's there but shifted.
    let found = -1;
    for (let j = 0; j < h.length; j++) {
      if (!heardUsed[j] && norm(h[j]) === tn) {
        found = j;
        break;
      }
    }
    if (found >= 0) {
      heardUsed[found] = true;
      out.push({ target: t[i], status: "ok" });
    } else if (i < h.length && !heardUsed[i]) {
      // There's something at this position, just not the right
      // thing — mark `h[i]` as used so a *later* target word
      // can't also claim it via the shifted scan and end up
      // showing as a separate match.
      heardUsed[i] = true;
      out.push({ target: t[i], status: "wrong", heard: h[i] });
    } else {
      out.push({ target: t[i], status: "missed" });
    }
  }
  return out;
}
