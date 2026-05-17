// Thin wrappers around the Web Speech API.
//
// STT: webkitSpeechRecognition is the only widely deployed implementation
//      on Chrome/Edge (desktop+mobile) and iOS Safari 16.4+. We fall back
//      to a no-op recognizer if unavailable so the UI keeps loading.
// TTS: speechSynthesis is universally available; we pick the best English
//      voice we can find at startup.

type RecHandlers = {
  onText: (text: string, isFinal: boolean) => void;
  onError?: (err: string) => void;
  onEnd?: () => void;
};

// `webkitSpeechRecognition` isn't in TS lib.dom by default.
type AnyWindow = typeof window & {
  webkitSpeechRecognition?: new () => SpeechRecognitionLike;
  SpeechRecognition?: new () => SpeechRecognitionLike;
};

interface SpeechRecognitionLike {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  start(): void;
  stop(): void;
  onresult: ((ev: SpeechRecognitionEventLike) => void) | null;
  onerror: ((ev: { error: string }) => void) | null;
  onend: (() => void) | null;
}
interface SpeechRecognitionEventLike {
  results: ArrayLike<ArrayLike<{ transcript: string }> & { isFinal: boolean }>;
}

export function createRecognizer(handlers: RecHandlers) {
  const w = window as AnyWindow;
  const Ctor = w.SpeechRecognition || w.webkitSpeechRecognition;
  if (!Ctor) {
    return {
      start: () => handlers.onError?.("SpeechRecognition not supported in this browser"),
      stop: () => {},
      supported: false as const,
    };
  }
  const rec = new Ctor();
  rec.lang = "en-US";
  rec.continuous = false;
  rec.interimResults = true;
  rec.onresult = (ev) => {
    let text = "";
    let isFinal = false;
    for (let i = 0; i < ev.results.length; i++) {
      const r = ev.results[i];
      text += r[0].transcript;
      if (r.isFinal) isFinal = true;
    }
    handlers.onText(text, isFinal);
  };
  rec.onerror = (ev) => handlers.onError?.(ev.error);
  rec.onend = () => handlers.onEnd?.();
  return {
    start: () => rec.start(),
    stop: () => rec.stop(),
    supported: true as const,
  };
}

/** Score voices so we pick the cutest-sounding English voice available.
 *  Higher score wins. */
function scoreVoice(v: SpeechSynthesisVoice): number {
  if (!v.lang.toLowerCase().startsWith("en")) return -100;
  const name = v.name.toLowerCase();
  let s = 0;
  // Microsoft Edge "Natural" voices sound the most human + cute.
  if (name.includes("natural"))     s += 50;
  if (name.includes("online"))      s += 5;
  // Specific female voices that sound bright / cute.
  if (name.includes("aria"))        s += 40;
  if (name.includes("jenny"))       s += 35;
  if (name.includes("ana"))         s += 30; // Microsoft Ana — child-ish
  if (name.includes("samantha"))    s += 25;
  if (name.includes("zira"))        s += 20;
  if (name.includes("google") && name.includes("us")) s += 15;
  if (name.includes("female"))      s += 10;
  // Penalize obviously male voices.
  if (/(david|mark|guy|brian|ryan|tony|christopher)/i.test(name)) s -= 50;
  // US English voices generally sound clearer than UK.
  if (v.lang.toLowerCase().includes("us")) s += 3;
  return s;
}

function pickVoice(): SpeechSynthesisVoice | null {
  const voices = speechSynthesis.getVoices();
  if (voices.length === 0) return null;
  let best = voices[0];
  let bestScore = scoreVoice(best);
  for (const v of voices.slice(1)) {
    const s = scoreVoice(v);
    if (s > bestScore) {
      best = v;
      bestScore = s;
    }
  }
  return best;
}

let cachedVoice: SpeechSynthesisVoice | null = null;
function getVoice(): SpeechSynthesisVoice | null {
  if (cachedVoice) return cachedVoice;
  cachedVoice = pickVoice();
  return cachedVoice;
}

/** Expose available English voices so the UI can offer a picker. */
export function listVoices(): SpeechSynthesisVoice[] {
  if (typeof speechSynthesis === "undefined") return [];
  return speechSynthesis.getVoices().filter((v) => v.lang.toLowerCase().startsWith("en"));
}

export function setPreferredVoice(name: string) {
  const v = speechSynthesis.getVoices().find((vv) => vv.name === name);
  if (v) cachedVoice = v;
}

// Voices load asynchronously on Chrome/Edge — refresh once they arrive.
if (typeof speechSynthesis !== "undefined") {
  speechSynthesis.onvoiceschanged = () => {
    cachedVoice = pickVoice();
  };
}

export type SpeakHandle = {
  cancel: () => void;
  done: Promise<void>;
};

export function speak(
  text: string,
  opts?: { rate?: number; pitch?: number; onBoundary?: () => void },
): SpeakHandle {
  if (typeof speechSynthesis === "undefined") {
    return { cancel: () => {}, done: Promise.resolve() };
  }
  speechSynthesis.cancel();
  const u = new SpeechSynthesisUtterance(text);
  const v = getVoice();
  if (v) u.voice = v;
  u.lang = v?.lang || "en-US";
  // Slightly slow + higher pitch → reads as a cheerful young teacher.
  u.rate = opts?.rate ?? 0.92;
  u.pitch = opts?.pitch ?? 1.35;
  if (opts?.onBoundary) u.onboundary = opts.onBoundary;
  const done = new Promise<void>((resolve) => {
    u.onend = () => resolve();
    u.onerror = () => resolve();
  });
  speechSynthesis.speak(u);
  return { cancel: () => speechSynthesis.cancel(), done };
}
