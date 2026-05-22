import { useEffect, useState } from "react";
import { fetchInfo } from "./api";
import type { ServerInfo } from "./types";
import VrmViewer from "./avatar/VrmViewer";
import ChatPanel from "./chat/ChatPanel";
import { listVoices, setPreferredVoice, speak } from "./voice/speech";
import { VOICEVOX_SPEAKERS, speakerCharacter, speakerIcon } from "./voice/voicevox-speakers";
import { previewKokoroVoice, previewVoicevoxSpeaker } from "./voice/preview";
import { usePersistedState } from "./usePersistedState";

export default function App() {
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // All persisted via localStorage — survives reload + tab restore.
  const [backend, setBackend] = usePersistedState<string>("kotonoha:backend", "");
  const [lesson, setLesson] = usePersistedState<string>("kotonoha:lesson", "");
  const [avatar, setAvatar] = usePersistedState<string>("kotonoha:avatar", "");
  const [ttsMode, setTtsMode] = usePersistedState<"browser" | "kokoro">(
    "kotonoha:ttsMode",
    "browser",
  );
  const [browserVoice, setBrowserVoice] = usePersistedState<string>("kotonoha:browserVoice", "");
  const [kokoroVoice, setKokoroVoice] = usePersistedState<string>("kotonoha:kokoroVoice", "");
  // VOICEVOX speaker id (numeric) — only used for JA sentences;
  // English ones always go through Kokoro regardless of this pick.
  const [voicevoxSpeaker, setVoicevoxSpeaker] = usePersistedState<number>(
    "kotonoha:voicevoxSpeaker",
    8, // 春日部つむぎ (ノーマル) — child-safe default
  );
  const [browserVoices, setBrowserVoices] = useState<SpeechSynthesisVoice[]>([]);
  const [mouth, setMouth] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    const refreshVoices = () => {
      const list = listVoices();
      setBrowserVoices(list);
      // Only auto-fill if nothing is stored — otherwise keep the user's pick
      // (will be a no-op if their voice isn't available, surfaced as empty).
      if (!browserVoice && list.length > 0) setBrowserVoice(list[0].name);
    };
    refreshVoices();
    speechSynthesis.addEventListener("voiceschanged", refreshVoices);
    return () => speechSynthesis.removeEventListener("voiceschanged", refreshVoices);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (browserVoice) setPreferredVoice(browserVoice);
  }, [browserVoice]);

  useEffect(() => {
    fetchInfo()
      .then((i) => {
        setInfo(i);
        // Honor stored choice if the server still offers it, otherwise
        // fall back to the server's default (or first available).
        if (!backend || !i.backends.includes(backend)) {
          setBackend(i.defaults.backend || i.backends[0] || "");
        }
        if (!lesson || !i.lessons.includes(lesson)) {
          setLesson(i.defaults.lesson || i.lessons[0] || "");
        }
        if (!avatar || !i.avatars.includes(avatar)) {
          const pickedAvatar = i.avatars.includes(i.defaults.avatar)
            ? i.defaults.avatar
            : i.avatars[0] ?? "";
          setAvatar(pickedAvatar);
        }
        // TTS mode default is only applied on the very first visit (when
        // stored value is the constructor default "browser").  After that
        // we trust the user's choice.
        const kokoroAvail = (i.voice.kokoro_voices?.length ?? 0) > 0;
        if (ttsMode === "kokoro" && !kokoroAvail) setTtsMode("browser");
        if (kokoroAvail && !kokoroVoice) {
          setKokoroVoice(i.voice.kokoro_default || i.voice.kokoro_voices![0]);
        }
      })
      .catch((e) => setErr(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (err) return <div className="p-6 text-kotonoha-accent">起動に失敗: {err}</div>;
  if (!info) return <div className="p-6">読み込み中…</div>;

  const voicevoxAvailable = info.voice.voicevox_default != null;
  const selectorsProps = {
    info,
    backend, lesson, avatar, ttsMode, browserVoice, browserVoices, kokoroVoice,
    voicevoxSpeaker, voicevoxAvailable,
    setMouth,
    onBackend: setBackend,
    onLesson: setLesson,
    onAvatar: setAvatar,
    onTtsMode: setTtsMode,
    onBrowserVoice: setBrowserVoice,
    onKokoroVoice: setKokoroVoice,
    onVoicevoxSpeaker: setVoicevoxSpeaker,
  };

  return (
    <div className="flex h-[100dvh] min-h-0 flex-col md:flex-row">
      {/* Mobile header: title + ⚙ button. Settings panel slides down on tap. */}
      <header className="relative z-20 flex shrink-0 items-center justify-between border-b border-kotonoha-ink/10 bg-kotonoha-paper px-4 py-2 md:hidden">
        <h1 className="font-ja text-lg">
          ことのは <span className="text-xs text-kotonoha-ink/50">英会話の先生</span>
        </h1>
        <button
          type="button"
          aria-label="設定"
          onClick={() => setSettingsOpen((v) => !v)}
          className="rounded-full border border-kotonoha-ink/20 bg-white px-3 py-1 text-sm"
        >
          {settingsOpen ? "✕" : "⚙"}
        </button>
      </header>

      {/* Mobile settings panel — slides down when open, stacks selectors vertically. */}
      {settingsOpen && (
        <div className="z-10 shrink-0 border-b border-kotonoha-ink/10 bg-white/95 px-4 py-3 shadow-sm md:hidden">
          <Selectors {...selectorsProps} stacked />
        </div>
      )}

      <section className="relative shrink-0 overflow-hidden bg-gradient-to-b from-kotonoha-leaf/10 to-kotonoha-paper md:h-full md:w-1/2 md:shrink h-[38vh] min-h-[220px]">
        {avatar ? (
          <VrmViewer src={`/avatars/${avatar}`} mouth={mouth} />
        ) : (
          <div className="flex h-full items-center justify-center p-6 text-center text-sm text-kotonoha-ink/60">
            avatars/ フォルダに *.vrm を置いてリロードしてね。<br />
            (Booth や VRoid Hub の CC0 / フリーモデルが使えます)
          </div>
        )}
      </section>

      <section className="flex h-full min-h-0 flex-1 flex-col bg-kotonoha-paper">
        {/* Desktop header — only renders ≥ md. */}
        <div className="hidden border-b border-kotonoha-ink/10 px-4 py-3 md:flex md:items-center md:justify-between md:gap-3">
          <h1 className="shrink-0 font-ja text-2xl">
            ことのは <span className="text-sm text-kotonoha-ink/50">— 英会話の先生</span>
          </h1>
          <Selectors {...selectorsProps} />
        </div>
        {backend && lesson && (
          <ChatPanel
            backend={backend}
            lesson={lesson}
            ttsMode={ttsMode}
            kokoroVoice={kokoroVoice}
            voicevoxSpeaker={voicevoxSpeaker}
            setMouth={setMouth}
          />
        )}
      </section>
    </div>
  );
}

type SelectorsProps = {
  info: ServerInfo;
  backend: string; lesson: string; avatar: string;
  ttsMode: "browser" | "kokoro";
  browserVoice: string;
  browserVoices: SpeechSynthesisVoice[];
  kokoroVoice: string;
  voicevoxSpeaker: number;
  voicevoxAvailable: boolean;
  /** Lets the in-settings preview drive the avatar's mouth too. */
  setMouth: (v: number) => void;
  onBackend: (v: string) => void;
  onLesson: (v: string) => void;
  onAvatar: (v: string) => void;
  onTtsMode: (v: "browser" | "kokoro") => void;
  onBrowserVoice: (v: string) => void;
  onKokoroVoice: (v: string) => void;
  onVoicevoxSpeaker: (v: number) => void;
  /** Stack rows full-width with labels (mobile settings panel). */
  stacked?: boolean;
};

function Selectors(p: SelectorsProps) {
  const kokoroAvailable = (p.info.voice.kokoro_voices?.length ?? 0) > 0;

  // Centralize the "store + preview" pairing so the stacked (mobile)
  // and pill-row (desktop) layouts can't drift — both call the same
  // handler, including the mouth-level wiring for the avatar.
  const handleKokoroChange = (v: string) => {
    p.onKokoroVoice(v);
    previewKokoroVoice(v, { onLevel: p.setMouth });
  };
  const handleBrowserVoiceChange = (v: string) => {
    p.onBrowserVoice(v);
    speak("Hi! Nice to meet you.");
  };
  const handleVoicevoxChange = (raw: string) => {
    const id = parseInt(raw, 10);
    p.onVoicevoxSpeaker(id);
    previewVoicevoxSpeaker(id, { onLevel: p.setMouth });
  };

  if (p.stacked) {
    // Mobile: one row per setting, full-width, with a small label.
    return (
      <div className="grid grid-cols-[5rem_1fr] items-center gap-x-3 gap-y-2 text-sm">
        <Row label="バックエンド">
          <SelectBare value={p.backend} onChange={p.onBackend}>
            {p.info.backends.map((b) => <option key={b} value={b}>{b}</option>)}
          </SelectBare>
        </Row>
        <Row label="レッスン">
          <SelectBare value={p.lesson} onChange={p.onLesson}>
            {p.info.lessons.map((l) => <option key={l} value={l}>{l}</option>)}
          </SelectBare>
        </Row>
        <Row label="アバター">
          <SelectBare value={p.avatar} onChange={p.onAvatar}>
            {p.info.avatars.length === 0 ? <option value="">(no avatars)</option> :
              p.info.avatars.map((a) => <option key={a} value={a}>{a}</option>)}
          </SelectBare>
        </Row>
        <Row label="TTS">
          <SelectBare
            value={p.ttsMode}
            onChange={(v) => p.onTtsMode(v as "browser" | "kokoro")}
            disabled={!kokoroAvailable}
          >
            <option value="browser">browser TTS</option>
            {kokoroAvailable && <option value="kokoro">Kokoro</option>}
          </SelectBare>
        </Row>
        <Row label="ボイス">
          {p.ttsMode === "kokoro" ? (
            <SelectBare value={p.kokoroVoice} onChange={handleKokoroChange}>
              {(p.info.voice.kokoro_voices ?? []).map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </SelectBare>
          ) : (
            <SelectBare value={p.browserVoice} onChange={handleBrowserVoiceChange}>
              {p.browserVoices.length === 0 ? <option value="">(no voices)</option> :
                p.browserVoices.map((v) => <option key={v.name} value={v.name}>{v.name}</option>)}
            </SelectBare>
          )}
        </Row>
        {p.voicevoxAvailable && (
          <Row label="JA ボイス">
            <SelectBare
              value={String(p.voicevoxSpeaker)}
              onChange={handleVoicevoxChange}
            >
              {VOICEVOX_SPEAKERS.map((s) => (
                <option key={s.id} value={s.id} title={s.hint}>
                  {speakerIcon(s.id)} {s.character} ({s.style})
                </option>
              ))}
            </SelectBare>
          </Row>
        )}
        {p.voicevoxAvailable && (
          <VoicevoxCredit speakerId={p.voicevoxSpeaker} stacked />
        )}
      </div>
    );
  }

  // Desktop: pill-shaped horizontal row.
  const cls = "rounded-full border border-kotonoha-ink/20 bg-white px-3 py-1 font-en text-sm max-w-[14rem]";
  return (
    <div className="flex flex-wrap items-center gap-2">
      <select className={cls} value={p.backend} onChange={(e) => p.onBackend(e.target.value)}>
        {p.info.backends.map((b) => <option key={b} value={b}>{b}</option>)}
      </select>
      <select className={cls} value={p.lesson} onChange={(e) => p.onLesson(e.target.value)}>
        {p.info.lessons.map((l) => <option key={l} value={l}>{l}</option>)}
      </select>
      <select className={cls} value={p.avatar} onChange={(e) => p.onAvatar(e.target.value)}>
        {p.info.avatars.length === 0 ? <option value="">(no avatars)</option> :
          p.info.avatars.map((a) => <option key={a} value={a}>{a}</option>)}
      </select>
      <select
        className={cls}
        title="TTS engine"
        value={p.ttsMode}
        onChange={(e) => p.onTtsMode(e.target.value as "browser" | "kokoro")}
        disabled={!kokoroAvailable}
      >
        <option value="browser">browser TTS</option>
        {kokoroAvailable && <option value="kokoro">Kokoro</option>}
      </select>
      {p.ttsMode === "kokoro" ? (
        <select className={cls + " truncate"} value={p.kokoroVoice}
          onChange={(e) => handleKokoroChange(e.target.value)}>
          {(p.info.voice.kokoro_voices ?? []).map((v) => (
            <option key={v} value={v}>{v}</option>
          ))}
        </select>
      ) : (
        <select className={cls + " truncate"} value={p.browserVoice}
          onChange={(e) => handleBrowserVoiceChange(e.target.value)}>
          {p.browserVoices.length === 0 ? <option value="">(no voices)</option> :
            p.browserVoices.map((v) => <option key={v.name} value={v.name}>{v.name}</option>)}
        </select>
      )}
      {p.voicevoxAvailable && (
        <select
          className={cls + " truncate"}
          title="VOICEVOX speaker (for Japanese sentences)"
          value={String(p.voicevoxSpeaker)}
          onChange={(e) => handleVoicevoxChange(e.target.value)}
        >
          {VOICEVOX_SPEAKERS.map((s) => (
            <option key={s.id} value={s.id} title={s.hint}>
              {speakerIcon(s.id)} {s.character} ({s.style})
            </option>
          ))}
        </select>
      )}
      {p.voicevoxAvailable && <VoicevoxCredit speakerId={p.voicevoxSpeaker} />}
    </div>
  );
}

/** Required credit per VOICEVOX usage terms — shows
 *  "VOICEVOX:<character>" alongside the speaker selector so the
 *  attribution is visible whenever a VOICEVOX clip might play. */
function VoicevoxCredit({ speakerId, stacked }: { speakerId: number; stacked?: boolean }) {
  // Fall back to a generic "VOICEVOX" label when the speaker id
  // isn't in the curated list — the license requires attribution
  // whenever the engine is used, even for ids outside our table.
  const character = speakerCharacter(speakerId);
  const label = character ? `VOICEVOX:${character}` : "VOICEVOX";
  const icon = speakerIcon(speakerId);
  if (stacked) {
    return (
      <>
        <span className="font-ja text-xs text-kotonoha-ink/60">クレジット</span>
        <a
          href="https://voicevox.hiroshiba.jp/"
          target="_blank"
          rel="noreferrer"
          className="font-en text-xs text-kotonoha-ink/70 hover:underline"
        >
          <span aria-hidden="true">{icon}</span> {label}
        </a>
      </>
    );
  }
  return (
    <a
      href="https://voicevox.hiroshiba.jp/"
      target="_blank"
      rel="noreferrer"
      className="ml-1 self-center font-en text-xs text-kotonoha-ink/60 hover:underline"
      title="VOICEVOX 利用規約に基づく必須クレジット表示"
    >
      <span aria-hidden="true">{icon}</span> {label}
    </a>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <span className="font-ja text-xs text-kotonoha-ink/60">{label}</span>
      {children}
    </>
  );
}

function SelectBare({
  value, onChange, disabled, children,
}: {
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <select
      className="h-9 w-full rounded-xl border border-kotonoha-ink/20 bg-white px-3 font-en text-sm disabled:opacity-50"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled}
    >
      {children}
    </select>
  );
}
