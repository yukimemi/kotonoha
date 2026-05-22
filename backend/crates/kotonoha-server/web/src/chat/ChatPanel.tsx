import { useEffect, useRef, useState } from "react";
import { ChatSocket } from "../api";
import { createRecognizer, speak } from "../voice/speech";
import { KokoroQueue, extractSentences } from "../voice/kokoro-queue";
import { type Emotion, feedEmotionStream, extractEmotions } from "../voice/emotion";

type Turn = { role: "student" | "teacher"; text: string };

function explainSpeechError(code: string): string {
  switch (code) {
    case "not-allowed":         return "マイクへのアクセスが拒否されました。ブラウザの設定を確認してください。";
    case "service-not-allowed": return "音声認識サービスが利用できません (HTTPS 必須かも)。";
    case "network":             return "ネットワークエラー。HTTPS 接続か、回線状況を確認してください。";
    case "no-speech":           return "声が検出できませんでした。";
    case "audio-capture":       return "マイクが見つかりません。";
    case "aborted":             return "音声入力が中断されました。";
    default:                    return code;
  }
}

type Props = {
  backend: string;
  lesson: string;
  ttsMode: "browser" | "kokoro";
  kokoroVoice: string;
  voicevoxSpeaker: number;
  /** Drive the avatar's mouth-open value while TTS is speaking. */
  setMouth: (v: number) => void;
  /** Drive the avatar's facial expression. Updated whenever the LLM
   *  emits an `[emotion]` tag mid-stream. */
  setEmotion: (e: Emotion) => void;
};

export default function ChatPanel({ backend, lesson, ttsMode, kokoroVoice, voicevoxSpeaker, setMouth, setEmotion }: Props) {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [listening, setListening] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("接続中…");
  const wsRef = useRef<ChatSocket | null>(null);
  // pendingRef holds the rendered text shown to the user (post
  // emotion-tag stripping). rawBufRef holds the trailing portion
  // of the raw stream we couldn't safely render yet — typically an
  // unclosed `[` waiting for the next chunk to close it.
  const pendingRef = useRef<string>("");
  const rawBufRef = useRef<string>("");
  const recRef = useRef<ReturnType<typeof createRecognizer> | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const mouthTimer = useRef<number | null>(null);
  // Kokoro sentence-streaming pipeline. Reset per turn.
  const ttsQueueRef = useRef<KokoroQueue | null>(null);
  const ttsBufRef = useRef<string>("");

  useEffect(() => {
    const ws = new ChatSocket({
      onReady: ({ backend, lesson }) => setStatus(`${backend} / ${lesson}`),
      onDelta: (text) => {
        // Pull emotion tags out of the raw stream; everything that
        // makes it into pendingRef is already stripped, so chat
        // bubbles + TTS see clean text and the avatar gets its
        // expression updates as the tags appear.
        rawBufRef.current += text;
        const { safeStripped, emotions, rest } = feedEmotionStream(rawBufRef.current);
        rawBufRef.current = rest;
        for (const e of emotions) setEmotion(e);

        if (!safeStripped) return;
        pendingRef.current += safeStripped;
        const accumulated = pendingRef.current;
        setTurns((prev) => {
          const last = prev[prev.length - 1];
          if (last?.role === "teacher") {
            return [...prev.slice(0, -1), { role: "teacher", text: accumulated }];
          }
          return [...prev, { role: "teacher", text: accumulated }];
        });

        // Sentence-streaming Kokoro: pluck complete sentences out of the
        // running buffer and fire `/api/tts` for each — fetches run in
        // parallel, audio playback stays serialized inside KokoroQueue.
        if (ttsModeRef.current === "kokoro" && kokoroVoiceRef.current) {
          if (!ttsQueueRef.current) {
            ttsQueueRef.current = new KokoroQueue({
              voice: kokoroVoiceRef.current,
              voicevoxSpeakerId: voicevoxSpeakerRef.current,
              onLevel: setMouth,
            });
          }
          ttsBufRef.current += safeStripped;
          const { sentences, remainder } = extractSentences(ttsBufRef.current, false);
          ttsBufRef.current = remainder;
          for (const s of sentences) ttsQueueRef.current.enqueue(s);
        }
      },
      onDone: () => {
        // Flush any trailing raw — if it's an unclosed `[`, it's
        // just an LLM stutter; render literally rather than swallow.
        if (rawBufRef.current) {
          const { stripped, emotions } = extractEmotions(rawBufRef.current);
          for (const e of emotions) setEmotion(e);
          if (stripped) {
            pendingRef.current += stripped;
            if (ttsModeRef.current === "kokoro" && kokoroVoiceRef.current) {
              ttsBufRef.current += stripped;
            }
          }
          rawBufRef.current = "";
        }
        const text = pendingRef.current;
        pendingRef.current = "";
        busyRef.current = false;
        setBusy(false);
        if (!text.trim()) return;

        if (ttsModeRef.current === "kokoro" && kokoroVoiceRef.current) {
          // Flush whatever's left in the sentence buffer (trailing text
          // with no terminator, e.g. "Sure"). Then let the queue drain.
          const q = ttsQueueRef.current
            ?? new KokoroQueue({
              voice: kokoroVoiceRef.current,
              voicevoxSpeakerId: voicevoxSpeakerRef.current,
              onLevel: setMouth,
            });
          ttsQueueRef.current = q;
          const { sentences } = extractSentences(ttsBufRef.current, true);
          ttsBufRef.current = "";
          for (const s of sentences) q.enqueue(s);
          // Detach the queue ref — next turn gets a fresh one.
          ttsQueueRef.current = null;
        } else {
          // Browser TTS path: fire whole-text speak with sin-wave mouth.
          startMouthAnimation();
          speak(text).done.then(() => stopMouthAnimation());
        }
      },
      onError: (m) => {
        setStatus(`error: ${m}`);
        busyRef.current = false;
        setBusy(false);
      },
      onClose: () => setStatus("切断されました"),
    });
    ws.connect();
    wsRef.current = ws;
    return () => {
      ws.close();
      ttsQueueRef.current?.cancel();
      ttsQueueRef.current = null;
      ttsBufRef.current = "";
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const busyRef = useRef(false);
  useEffect(() => { busyRef.current = busy; }, [busy]);
  const ttsModeRef = useRef(ttsMode);
  const kokoroVoiceRef = useRef(kokoroVoice);
  const voicevoxSpeakerRef = useRef(voicevoxSpeaker);
  useEffect(() => { ttsModeRef.current = ttsMode; }, [ttsMode]);
  useEffect(() => { kokoroVoiceRef.current = kokoroVoice; }, [kokoroVoice]);
  useEffect(() => { voicevoxSpeakerRef.current = voicevoxSpeaker; }, [voicevoxSpeaker]);

  useEffect(() => {
    wsRef.current?.send({ type: "configure", backend, lesson });
    setTurns([]);
    // Reset per-conversation TTS + emotion state.
    ttsQueueRef.current?.cancel();
    ttsQueueRef.current = null;
    ttsBufRef.current = "";
    rawBufRef.current = "";
    setEmotion("neutral");
  }, [backend, lesson]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [turns]);

  const startMouthAnimation = () => {
    let t = 0;
    if (mouthTimer.current) window.clearInterval(mouthTimer.current);
    mouthTimer.current = window.setInterval(() => {
      t += 0.18;
      setMouth(0.35 + 0.35 * Math.abs(Math.sin(t * 6)) + Math.random() * 0.1);
    }, 60) as unknown as number;
  };
  const stopMouthAnimation = () => {
    if (mouthTimer.current) window.clearInterval(mouthTimer.current);
    mouthTimer.current = null;
    setMouth(0);
  };

  const sendUser = (text: string) => {
    if (!text.trim() || busy) return;
    // Mid-reply interrupt: kill any pending TTS + emotion buffers
    // from the previous turn.
    ttsQueueRef.current?.cancel();
    ttsQueueRef.current = null;
    ttsBufRef.current = "";
    rawBufRef.current = "";
    setTurns((prev) => [...prev, { role: "student", text }]);
    pendingRef.current = "";
    busyRef.current = true;
    setBusy(true);
    wsRef.current?.send({ type: "user", text });
    setDraft("");
  };

  const toggleListen = () => {
    if (listening) {
      recRef.current?.stop();
      setListening(false);
      return;
    }
    // Browsers silently disable SpeechRecognition on insecure origins
    // (HTTP + non-localhost). On a phone connecting over LAN / Tailscale
    // this is the #1 reason the mic button "does nothing".
    if (!window.isSecureContext) {
      alert(
        "音声入力には HTTPS が必要です。\n\n" +
        "PC のローカル動作はそのまま使えますが、スマホから音声で\n" +
        "話しかけたい時は HTTPS 化が必要です。\n\n" +
        "おすすめ: Tailscale Funnel で簡単に HTTPS URL を生やせます。\n" +
        "(README の「スマホで使う」セクション参照)"
      );
      return;
    }
    const rec = createRecognizer({
      onText: (text, isFinal) => {
        setDraft(text);
        if (isFinal) {
          setListening(false);
          sendUser(text);
        }
      },
      onError: (e) => {
        const msg = explainSpeechError(e);
        setStatus(`音声認識: ${msg}`);
        setListening(false);
        // For the most actionable errors, also pop an alert so the user
        // on mobile (where the status bar is tiny) actually sees it.
        if (e === "not-allowed" || e === "service-not-allowed" || e === "network") {
          alert(`音声認識エラー: ${msg}`);
        }
      },
      onEnd: () => setListening(false),
    });
    recRef.current = rec;
    setListening(true);
    rec.start();
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="px-4 py-2 text-xs text-kotonoha-ink/60">{status}</div>
      <div ref={scrollRef} className="flex-1 min-h-0 overflow-y-auto px-4 pb-3 space-y-2">
        {turns.map((t, i) => (
          <div
            key={i}
            className={
              "max-w-[85%] rounded-2xl px-4 py-2 leading-relaxed " +
              (t.role === "teacher"
                ? "bg-white/80 text-kotonoha-ink font-en"
                : "ml-auto bg-kotonoha-leaf/20 text-kotonoha-ink font-ja")
            }
          >
            {t.text}
          </div>
        ))}
        {busy && pendingRef.current === "" && (
          <div className="text-xs text-kotonoha-ink/40">先生が考え中…</div>
        )}
      </div>
      <form
        className="flex items-center gap-2 border-t border-kotonoha-ink/10 bg-white/80 px-3 py-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]"
        onSubmit={(e) => {
          e.preventDefault();
          sendUser(draft);
        }}
      >
        <button
          type="button"
          onClick={toggleListen}
          aria-label={listening ? "録音停止" : "録音開始"}
          className={
            "h-10 w-10 shrink-0 rounded-full border-2 text-base transition " +
            (listening
              ? "border-kotonoha-accent bg-kotonoha-accent text-white"
              : "border-kotonoha-ink/30 bg-white text-kotonoha-ink hover:bg-kotonoha-leaf/10")
          }
        >
          {listening ? "■" : "🎙"}
        </button>
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Type in English or 日本語…"
          className="h-10 min-w-0 flex-1 rounded-full border border-kotonoha-ink/20 bg-white px-4 font-en text-base outline-none focus:border-kotonoha-leaf"
        />
        <button
          type="submit"
          disabled={busy || !draft.trim()}
          className="h-10 shrink-0 rounded-full bg-kotonoha-leaf px-4 font-en text-sm text-white disabled:opacity-50"
        >
          送る
        </button>
      </form>
    </div>
  );
}
