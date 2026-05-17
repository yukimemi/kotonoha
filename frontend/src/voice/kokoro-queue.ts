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
  voice: string;
  speed?: number;
  /** 0-1 mouth-open level, driven by the currently-playing audio. */
  onLevel: (v: number) => void;
};

export class KokoroQueue {
  private chain: Promise<void> = Promise.resolve();
  private cancelled = false;
  private aborts: Set<AbortController> = new Set();
  private currentAudio: HTMLAudioElement | null = null;
  private currentCtx: AudioContext | null = null;

  constructor(private opts: Opts) {}

  /** Enqueue a sentence — fetch starts now, playback after prior clips. */
  enqueue(text: string) {
    const sentence = text.trim();
    if (!sentence || this.cancelled) return;

    const abort = new AbortController();
    this.aborts.add(abort);
    const blobP = fetch("/api/tts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text: sentence, voice: this.opts.voice, speed: this.opts.speed ?? 1.0 }),
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
        if (!this.cancelled) console.warn("kokoro queue clip failed:", e);
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
