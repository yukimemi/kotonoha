// One-shot voice preview helper.
//
// Fired from the settings dropdowns when the user picks a Kokoro
// voice or a VOICEVOX speaker — fetches a short sample from
// `/api/tts` and plays it through Web Audio so the same analyser
// driving the avatar's mouth animation can hook in too.
//
// Cancels any in-flight preview before starting a new one so
// rapid clicks through the dropdown don't pile up overlapping
// clips.

const SAMPLE_EN = "Hi! Nice to meet you.";
const SAMPLE_JA = "こんにちは、はじめまして。";

let current: { cancel: () => void } | null = null;

type Opts = {
  /** Optional 0-1 mouth level callback while the sample plays. */
  onLevel?: (v: number) => void;
};

export function previewKokoroVoice(voice: string, opts: Opts = {}): Promise<void> {
  return play({ text: SAMPLE_EN, lang: "en", voice }, opts);
}

export function previewVoicevoxSpeaker(speakerId: number, opts: Opts = {}): Promise<void> {
  return play({ text: SAMPLE_JA, lang: "ja", speaker_id: speakerId }, opts);
}

type TtsBody = {
  text: string;
  lang: "en" | "ja";
  voice?: string;
  speaker_id?: number;
};

async function play(body: TtsBody, opts: Opts): Promise<void> {
  // Stop any prior preview the moment we start a new one — no
  // overlap, no leaked AudioContext.
  current?.cancel();

  const abort = new AbortController();
  const fetchP = fetch("/api/tts", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal: abort.signal,
  });

  const handle: { cancel: () => void } = {
    cancel: () => {
      abort.abort();
    },
  };
  current = handle;

  try {
    const res = await fetchP;
    if (!res.ok) throw new Error(`tts http ${res.status}: ${await res.text()}`);
    const blob = await res.blob();
    if (current !== handle) return; // a newer preview took over
    await playBlob(blob, opts.onLevel, handle);
  } catch (e) {
    if (current === handle) current = null;
    if ((e as Error).name === "AbortError") return; // cancelled
    console.warn("voice preview failed:", e);
  }
}

async function playBlob(
  blob: Blob,
  onLevel: ((v: number) => void) | undefined,
  handle: { cancel: () => void },
): Promise<void> {
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
    onLevel?.(Math.min(1, (sum / (hi - lo)) / 140));
    raf = requestAnimationFrame(loop);
  };

  return new Promise<void>((resolve) => {
    const finish = () => {
      stopped = true;
      cancelAnimationFrame(raf);
      onLevel?.(0);
      URL.revokeObjectURL(url);
      ctx.close().catch(() => {});
      if (current === handle) current = null;
      resolve();
    };
    // Replace cancel to also abort audio playback.
    handle.cancel = () => {
      audio.pause();
      finish();
    };
    audio.onended = finish;
    audio.onerror = finish;
    audio.play().then(loop).catch(() => finish());
  });
}
