export type ServerInfo = {
  backends: string[];
  lessons: string[];
  avatars: string[];
  defaults: { backend: string; lesson: string; avatar: string };
  voice: {
    stt: string;
    tts: string;
    kokoro_voices?: string[];
    kokoro_default?: string;
    /** VOICEVOX speaker id the server preloaded as default. */
    voicevox_default?: number;
    /** Speaker ids preloaded (warm); UI may request any id and the
     *  server loads it on demand on the first call. */
    voicevox_speakers?: number[];
  };
};

export type ServerMsg =
  | { type: "ready"; backend: string; lesson: string }
  | { type: "delta"; text: string }
  | { type: "done" }
  | { type: "error"; message: string };

export type ClientMsg =
  | { type: "configure"; backend?: string; lesson?: string }
  | { type: "user"; text: string }
  | { type: "reset" };
