/**
 * The signalling socket a voice call holds to one voice server, its frames typed by the
 * generated `voiceSignal.ts`.
 */

import type { ClientMessage, ServerMessage } from "./generated/voiceSignal";

/** One socket to a voice server with typed frames and waits for particular replies. */
export class Signal {
  readonly #socket: WebSocket;
  readonly #waiters: {
    pred: (f: ServerMessage) => boolean;
    resolve: (f: ServerMessage) => void;
  }[] = [];
  onFrame: (frame: ServerMessage) => void = () => undefined;
  onClose: () => void = () => undefined;
  #closedByUs = false;

  constructor(url: string, Socket: typeof globalThis.WebSocket) {
    this.#socket = new Socket(url);
    this.#socket.onmessage = (event: MessageEvent) => {
      let frame: ServerMessage;
      try {
        frame = JSON.parse(String(event.data)) as ServerMessage;
      } catch {
        return;
      }
      for (const waiter of [...this.#waiters]) {
        if (waiter.pred(frame)) {
          this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
          waiter.resolve(frame);
        }
      }
      this.onFrame(frame);
    };
    this.#socket.onclose = () => {
      if (!this.#closedByUs) {
        this.onClose();
      }
    };
  }

  open(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.#socket.onopen = () => {
        resolve();
      };
      this.#socket.onerror = () => {
        reject(new Error("the voice server did not accept the connection"));
      };
    });
  }

  /** Sends a frame; one addressed to a socket that is closing is dropped, as the server is gone. */
  send(frame: ClientMessage): void {
    if (this.#socket.readyState === this.#socket.OPEN) {
      this.#socket.send(JSON.stringify(frame));
    }
  }

  /** The next frame matching `pred`. */
  next(pred: (f: ServerMessage) => boolean): Promise<ServerMessage> {
    return new Promise((resolve) => {
      this.#waiters.push({ pred, resolve });
    });
  }

  close(): void {
    this.#closedByUs = true;
    this.#socket.close();
  }
}
