// Streaming Kokoro TTS queue.
//
// Design:
//   - `enqueue(sentence)` kicks off a `/api/tts` fetch immediately
//     (so multiple sentences are synthesizing in parallel).
//   - Playback is serialized via a promise chain so audio never overlaps.
//   - The currently-playing clip drives the mouth level via Web Audio's
//     `AnalyserNode`.
//   - `cancel()` aborts every in-flight fetch and stops the current
//     audio — used when the user sends a new message before the queue
//     finishes draining.

type Opts = {
  /** Kokoro voice id for English sentences (e.g. "jf_alpha"). */
  voice: string;
  /** VOICEVOX speaker id for Japanese sentences. Optional — if
   *  omitted the server falls back to the default from config. */
  voicevoxSpeakerId?: number;
  speed?: number;
  /** 0-1 mouth-open level, driven by the currently-playing audio. */
  onLevel: (v: number) => void;
  /** Fired when a sentence fetch fails. Without this the only
   *  signal was a console.warn — easy to miss when VOICEVOX isn't
   *  set up and every JP sentence silently 5xx's. `lang` lets the
   *  UI tailor the message ("run setup-voicevox" vs "run setup-tts"). */
  onError?: (err: { message: string; lang: "en" | "ja" }) => void;
};

/** Trim + strip leading non-language chars (emoji, decorative
 *  symbols, leading punctuation) so Open JTalk inside VOICEVOX
 *  doesn't emit
 *      WARNING: JPCommonLabel_insert_pause(): First mora should
 *      not be short pause.
 *  for every "🎧 これを真似してね" sentence. The synth still
 *  worked through the warning, but it floods server logs and
 *  obscures real diagnostics. Letters/digits (any script,
 *  including JP) and the ASCII apostrophe (so "I'll" stays
 *  intact) survive; the rest of the leading run is shaved off.
 *  Returns "" when the sentence is purely decorative — the
 *  caller skips empty enqueues. */
export function sanitizeForSynth(text: string): string {
  return text.trim().replace(/^[^\p{L}\p{N}']+/u, "");
}

/** Detect if a sentence is "Japanese enough" to route to VOICEVOX.
 *  Threshold-based instead of any-match: an English sentence with a
 *  single Japanese parenthetical (e.g. "Are you `over the moon`
 *  (とても嬉しい) today?") used to flip the whole sentence to
 *  VOICEVOX and read the English part with a Japanese voice. Now we
 *  require the JP characters to be at least ~30% of the
 *  letter-equivalent character count before routing to ja.
 *
 *  Matches the backend's `detect_lang`. */
function detectLang(text: string): "ja" | "en" {
  // Single-pass: count Japanese chars + letter-equivalent denominator
  // in one walk. Skips whitespace and ASCII punctuation so the
  // brackets around "(とても嬉しい)" don't dilute their own contents.
  let jp = 0;
  let letters = 0;
  for (const char of text) {
    if (/[぀-ゟ゠-ヿ一-鿿]/.test(char)) {
      jp++;
      letters++;
    } else if (/[A-Za-z]/.test(char)) {
      letters++;
    }
  }
  if (jp === 0) return "en";
  // Integer arithmetic mirrors the backend's `detect_lang` so both
  // ends round the 30% threshold identically — no floating-point
  // edge cases between client + server.
  return jp * 10 >= letters * 3 ? "ja" : "en";
}

/** Carve a mixed-script sentence into same-language runs so each
 *  goes to the engine that pronounces it natively. Used when a
 *  shadowing feedback line is mostly Japanese but quotes English
 *  phrases the student should mimic:
 *
 *      "cheese sandwich" はバッチリでした！
 *
 *  Without this split the whole sentence routes to VOICEVOX
 *  (detectLang=ja) and `"cheese sandwich"` comes out in katakana —
 *  the very pronunciation the lesson is trying to teach away from.
 *  An EN run is one-or-more ASCII letters/digits with internal
 *  apostrophes, hyphens, or spaces (so `Let's try`, `ham and`,
 *  `I'll` stay together as one Kokoro clip). Single-letter runs
 *  (`a`, `d`) match too — they're rare outside intentional
 *  pronunciation callouts in feedback. */
export function splitByScript(sentence: string): Array<{ text: string; lang: "en" | "ja" }> {
  const out: Array<{ text: string; lang: "en" | "ja" }> = [];
  // Greedy multi-word run, with a fallback for the single-letter case.
  const enRe = /[A-Za-z][A-Za-z0-9'\- ]*[A-Za-z0-9]|[A-Za-z]/g;
  let lastEnd = 0;
  let m: RegExpExecArray | null;
  while ((m = enRe.exec(sentence)) !== null) {
    if (m.index > lastEnd) {
      out.push({ text: sentence.slice(lastEnd, m.index), lang: "ja" });
    }
    out.push({ text: m[0], lang: "en" });
    lastEnd = enRe.lastIndex;
  }
  if (lastEnd < sentence.length) {
    out.push({ text: sentence.slice(lastEnd), lang: "ja" });
  }
  return out;
}

export class KokoroQueue {
  private chain: Promise<void> = Promise.resolve();
  private cancelled = false;
  private aborts: Set<AbortController> = new Set();
  private currentAudio: HTMLAudioElement | null = null;
  private currentCtx: AudioContext | null = null;

  constructor(private opts: Opts) {}

  /** Enqueue a sentence — fetch starts now, playback after prior clips.
   *  Mixed-script sentences (JP + EN) are split into runs so each
   *  language is read by its native engine; pure JP / pure EN
   *  sentences stay whole so neither engine loses prosody on text
   *  it fully owns. */
  enqueue(text: string) {
    const sentence = sanitizeForSynth(text);
    if (!sentence || this.cancelled) return;
    const hasEn = /[A-Za-z]/.test(sentence);
    const hasJa = /[぀-ヿ一-鿿]/.test(sentence);
    if (!(hasEn && hasJa)) {
      this.enqueueChunk(sentence, detectLang(sentence));
      return;
    }
    for (const seg of splitByScript(sentence)) {
      const piece = sanitizeForSynth(seg.text);
      if (piece) this.enqueueChunk(piece, seg.lang);
    }
  }

  /** Fire the `/api/tts` fetch + chain the playback. The server
   *  picks the engine off `lang`: VOICEVOX for "ja", Kokoro for
   *  "en". Both engine-specific fields (`voice`, `speaker_id`) are
   *  always sent — server keeps whichever matches the resolved
   *  lang and ignores the other. */
  private enqueueChunk(sentence: string, lang: "en" | "ja") {
    const abort = new AbortController();
    this.aborts.add(abort);
    const blobP = fetch("/api/tts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        text: sentence,
        lang,
        voice: this.opts.voice,
        speaker_id: this.opts.voicevoxSpeakerId,
        speed: this.opts.speed ?? 1.0,
      }),
      signal: abort.signal,
    })
      .then(async (r) => {
        if (!r.ok) throw new Error(`tts http ${r.status}: ${await r.text()}`);
        return r.blob();
      })
      .finally(() => this.aborts.delete(abort));

    // Chain the playback so it starts only after previous sentence ends.
    this.chain = this.chain
      .then(async () => {
        if (this.cancelled) return;
        const blob = await blobP;
        if (this.cancelled) return;
        await this.play(blob);
      })
      .catch((e) => {
        if (this.cancelled) return;
        console.warn("kokoro queue clip failed:", e);
        this.opts.onError?.({ message: String(e?.message ?? e), lang });
      });
  }

  private async play(blob: Blob): Promise<void> {
    const url = URL.createObjectURL(blob);
    const ctx = new (window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext)();
    const audio = new Audio(url);

    const source = ctx.createMediaElementSource(audio);
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 256;
    analyser.smoothingTimeConstant = 0.5;
    source.connect(analyser);
    analyser.connect(ctx.destination);

    this.currentAudio = audio;
    this.currentCtx = ctx;

    const buf = new Uint8Array(analyser.frequencyBinCount);
    let raf = 0;
    let stopped = false;
    const loop = () => {
      if (stopped) return;
      analyser.getByteFrequencyData(buf);
      let sum = 0;
      const lo = 2;
      const hi = Math.min(20, buf.length);
      for (let i = lo; i < hi; i++) sum += buf[i];
      this.opts.onLevel(Math.min(1, (sum / (hi - lo)) / 140));
      raf = requestAnimationFrame(loop);
    };

    return new Promise<void>((resolve) => {
      const finish = () => {
        stopped = true;
        cancelAnimationFrame(raf);
        this.opts.onLevel(0);
        URL.revokeObjectURL(url);
        ctx.close().catch(() => {});
        if (this.currentAudio === audio) {
          this.currentAudio = null;
          this.currentCtx = null;
        }
        resolve();
      };
      audio.onended = finish;
      audio.onerror = finish;
      audio.play().then(loop).catch(() => finish());
    });
  }

  /** Stop everything immediately. Safe to call multiple times. */
  cancel() {
    this.cancelled = true;
    for (const a of this.aborts) a.abort();
    this.aborts.clear();
    if (this.currentAudio) {
      this.currentAudio.pause();
      this.currentAudio = null;
    }
    if (this.currentCtx) {
      this.currentCtx.close().catch(() => {});
      this.currentCtx = null;
    }
    this.opts.onLevel(0);
  }

  /** Resolves when every queued sentence has finished playing. */
  done(): Promise<void> {
    return this.chain;
  }
}

/** Pull complete sentences out of a streaming buffer.
 *
 *  `flush=true` returns whatever is left as a final sentence (call this
 *  on stream end). Otherwise the trailing text is kept in `remainder`
 *  for the next call.
 *
 *  Boundary = `.!?` (or 。!? for JA) optionally followed by closing
 *  quotes, then whitespace or end-of-string. */
export function extractSentences(
  buffer: string,
  flush: boolean,
): { sentences: string[]; remainder: string } {
  const sentences: string[] = [];
  // \p{Sentence_Terminator} would be ideal but we keep it explicit.
  const re = /[^.!?。！?]*[.!?。！?]+["')\]]*(?=\s|$)/g;
  let lastEnd = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(buffer)) !== null) {
    const piece = m[0].trim();
    if (piece) sentences.push(piece);
    lastEnd = re.lastIndex;
  }
  let remainder = buffer.slice(lastEnd);
  if (flush) {
    const tail = remainder.trim();
    if (tail) sentences.push(tail);
    remainder = "";
  }
  return { sentences, remainder };
}
