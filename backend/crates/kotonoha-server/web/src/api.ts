import type { ClientMsg, ServerInfo, ServerMsg } from "./types";

export async function fetchInfo(): Promise<ServerInfo> {
  const r = await fetch("/api/info");
  if (!r.ok) throw new Error(`info failed: ${r.status}`);
  return r.json();
}

export type WsHandlers = {
  onReady?: (m: { backend: string; lesson: string }) => void;
  onDelta?: (text: string) => void;
  onDone?: () => void;
  onError?: (message: string) => void;
  onClose?: () => void;
};

export class ChatSocket {
  private ws: WebSocket | null = null;
  private queue: ClientMsg[] = [];
  private closed = false;

  constructor(private handlers: WsHandlers) {}

  connect() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}/ws/chat`);
    this.ws = ws;
    ws.onopen = () => {
      // Flush anything queued before the socket finished handshaking.
      for (const msg of this.queue) ws.send(JSON.stringify(msg));
      this.queue = [];
    };
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data) as ServerMsg;
        switch (msg.type) {
          case "ready":   this.handlers.onReady?.({ backend: msg.backend, lesson: msg.lesson }); break;
          case "delta":   this.handlers.onDelta?.(msg.text); break;
          case "done":    this.handlers.onDone?.(); break;
          case "error":   this.handlers.onError?.(msg.message); break;
        }
      } catch {
        // ignore non-JSON frames
      }
    };
    ws.onclose = () => {
      if (!this.closed) this.handlers.onClose?.();
    };
  }

  /** Safe to call before `open` — the message is queued and flushed on connect. */
  send(msg: ClientMsg) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    } else {
      this.queue.push(msg);
    }
  }

  close() {
    this.closed = true;
    if (this.ws) {
      // close() during CONNECTING is allowed; the browser cancels the handshake silently.
      try { this.ws.close(); } catch { /* noop */ }
      this.ws = null;
    }
  }
}
