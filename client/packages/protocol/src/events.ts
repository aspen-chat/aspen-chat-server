import { Ajv2020, type ValidateFunction } from "ajv/dist/2020.js";
import addFormats from "ajv-formats";
import eventSchema from "./generated/event_schema.json";
import type { ClientMessage, ServerEvent, ServerMessage } from "./generated/events";

export type { ClientMessage, ServerEvent, ServerMessage };

/** Reconnect backoff: immediate, then 0.5s, 1s, 2s, 4s, then 5s forever. */
export function reconnectDelayMs(attempt: number): number {
  if (attempt <= 0) {
    return 0;
  }
  return Math.min(500 * 2 ** (attempt - 1), 5_000);
}

/** WebSocket close code the server uses when the `identify` token is rejected. */
const CLOSE_UNAUTHORIZED = 4401;

export type EventStreamStatus = "connecting" | "open" | "reconnecting" | "closed";

export interface ReadyInfo {
  userId: string;
  /** Whether the server replayed everything since the last sequence this client had seen. */
  resumed: boolean;
}

export interface EventStreamHandlers {
  /** One decoded server event, after the connection is live. */
  onEvent?: (event: ServerEvent) => void;
  /** The server accepted `identify`. Fires on the first connect and after every reconnect. */
  onReady?: (info: ReadyInfo) => void;
  /** The connection dropped or could not be made; reconnection is being attempted. */
  onConnectionLost?: (reason: string) => void;
  /**
   * Fires just before `onReady` when the server could not resume from the last sequence this
   * client processed: the cached state has a gap. Clear it and re-bootstrap from REST before
   * trusting subsequent events.
   */
  onResyncRequired?: () => void;
  /** A frame failed schema validation and was dropped. Only fires when `validate` is on. */
  onInvalidEvent?: (raw: unknown, errors: string) => void;
}

export interface EventStreamOptions extends EventStreamHandlers {
  /** `wss://host/api/v1/events`; see `eventStreamUrl`. */
  url: string;
  /**
   * Supplies the session token for the `identify` frame. Called before every connection
   * attempt; `forceRefresh` is set when the server rejected the previous token, so the
   * implementation should obtain a new one rather than hand back the same value. Returning
   * `null` means there is no session and the stream stops.
   */
  authenticate: (options: { forceRefresh: boolean }) => Promise<string | null>;
  /**
   * Validate every incoming frame against `event_schema.json`. Costs some CPU per event; on by
   * default in development builds, off in production.
   */
  validate?: boolean;
  /** WebSocket constructor override for tests and non-browser shells. */
  WebSocket?: typeof globalThis.WebSocket;
  /** Timer overrides for tests. */
  setTimeout?: typeof globalThis.setTimeout;
  clearTimeout?: typeof globalThis.clearTimeout;
}

/**
 * Consumes the server's event stream with automatic reconnection and exact resumption.
 *
 * On every connection the first frame sent is `identify`, carrying the session token and, on
 * reconnects, the sequence of the last event processed. The server replies `ready`, after
 * which `event` frames flow. If the server could not resume from that sequence it says so in
 * `ready`, and `onResyncRequired` fires so the caller can rebuild from REST.
 *
 * A dropped connection enters an outage: `onConnectionLost` fires once, reconnection follows
 * `reconnectDelayMs`, and `onReady` fires again when the handshake succeeds. A `4401` close
 * makes the next `authenticate` call ask for a fresh token.
 */
export class EventStream {
  #status: EventStreamStatus = "closed";
  #socket: WebSocket | null = null;
  #attempt = 0;
  #inOutage = false;
  #tokenRejected = false;
  /** Sequence of the last event handed to `onEvent`; `null` until the first one. */
  #lastSequence: number | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** Increments on every start/stop so a stale async connect attempt can notice and bail. */
  #generation = 0;
  readonly #options: EventStreamOptions;
  readonly #validator: ValidateFunction<ServerMessage> | null;

  constructor(options: EventStreamOptions) {
    this.#options = options;
    this.#validator = options.validate === true ? compileValidator() : null;
  }

  get status(): EventStreamStatus {
    return this.#status;
  }

  /** Sequence to resume from on the next connection, for callers that persist it. */
  get lastSequence(): number | null {
    return this.#lastSequence;
  }

  start(): void {
    if (this.#status !== "closed") {
      return;
    }
    this.#status = "connecting";
    this.#attempt = 0;
    this.#inOutage = false;
    this.#generation += 1;
    void this.#connect(this.#generation);
  }

  stop(): void {
    this.#status = "closed";
    this.#generation += 1;
    if (this.#timer !== null) {
      (this.#options.clearTimeout ?? clearTimeout)(this.#timer);
      this.#timer = null;
    }
    const socket = this.#socket;
    this.#socket = null;
    if (socket !== null) {
      socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null;
      socket.close();
    }
  }

  async #connect(generation: number): Promise<void> {
    let token: string | null;
    try {
      token = await this.#options.authenticate({ forceRefresh: this.#tokenRejected });
    } catch (error) {
      if (generation === this.#generation) {
        this.#onDropped(error instanceof Error ? error.message : String(error));
      }
      return;
    }
    if (generation !== this.#generation) {
      return;
    }
    if (token === null) {
      // Nobody is logged in; there is nothing to stream and no point retrying.
      this.#status = "closed";
      this.#options.onConnectionLost?.("no session");
      return;
    }
    this.#tokenRejected = false;
    const WS = this.#options.WebSocket ?? globalThis.WebSocket;
    let socket: WebSocket;
    try {
      socket = new WS(this.#options.url);
    } catch (error) {
      this.#onDropped(error instanceof Error ? error.message : String(error));
      return;
    }
    this.#socket = socket;
    socket.onopen = () => {
      if (this.#socket !== socket) {
        return;
      }
      const identify: ClientMessage = { type: "identify", sessionToken: token };
      if (this.#lastSequence !== null) {
        identify.resumeAfter = this.#lastSequence;
      }
      socket.send(JSON.stringify(identify));
    };
    socket.onmessage = (message: MessageEvent) => {
      if (this.#socket !== socket) {
        return;
      }
      this.#dispatch(message.data);
    };
    socket.onclose = (event: CloseEvent) => {
      if (this.#socket !== socket) {
        return;
      }
      if (event.code === CLOSE_UNAUTHORIZED) {
        this.#tokenRejected = true;
      }
      this.#onDropped(
        event.reason.length > 0 ? event.reason : `connection closed (${String(event.code)})`,
      );
    };
    socket.onerror = () => {
      // The browser follows every error with a close event, which carries the reason.
    };
  }

  #onDropped(reason: string): void {
    if (this.#status === "closed") {
      return;
    }
    this.#socket = null;
    if (!this.#inOutage) {
      this.#inOutage = true;
      this.#options.onConnectionLost?.(reason);
    }
    this.#status = "reconnecting";
    const delay = reconnectDelayMs(this.#attempt);
    this.#attempt += 1;
    const generation = this.#generation;
    this.#timer = (this.#options.setTimeout ?? setTimeout)(() => {
      this.#timer = null;
      if (this.#status === "reconnecting" && generation === this.#generation) {
        void this.#connect(generation);
      }
    }, delay);
  }

  #dispatch(data: unknown): void {
    if (typeof data !== "string") {
      return;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(data);
    } catch {
      this.#options.onInvalidEvent?.(data, "frame is not JSON");
      return;
    }
    if (this.#validator !== null && !this.#validator(parsed)) {
      this.#options.onInvalidEvent?.(
        parsed,
        (this.#validator.errors ?? [])
          .map((e) => `${e.instancePath} ${e.message ?? ""}`)
          .join("; "),
      );
      return;
    }
    const frame = parsed as ServerMessage;
    switch (frame.type) {
      case "ready": {
        const hadCache = this.#lastSequence !== null;
        this.#status = "open";
        this.#attempt = 0;
        this.#inOutage = false;
        if (hadCache && !frame.resumed) {
          // The server could not replay the gap; whatever we hold is now unreliable.
          this.#lastSequence = null;
          this.#options.onResyncRequired?.();
        }
        this.#options.onReady?.({ userId: frame.userId, resumed: frame.resumed });
        break;
      }
      case "event":
        this.#lastSequence = frame.sequence;
        this.#options.onEvent?.(frame.event);
        break;
      case "error":
        // The server closes right after this; the close handler drives reconnection and, for
        // `unauthorized`, the token refresh.
        if (frame.code === "unauthorized") {
          this.#tokenRejected = true;
        }
        break;
    }
  }
}

let cachedValidator: ValidateFunction<ServerMessage> | null = null;

/** Compiles the `ServerMessage` schema once; ajv compilation is not free. */
export function compileValidator(): ValidateFunction<ServerMessage> {
  if (cachedValidator === null) {
    const ajv = new Ajv2020({ allErrors: false, strict: false });
    addFormats(ajv);
    // schemars emits these formats for Rust's unsigned integers (JetStream sequence numbers,
    // vote counts, player dimensions) and ajv-formats has no entry for them.
    const unsigned = {
      type: "number" as const,
      validate: (n: number) => Number.isInteger(n) && n >= 0,
    };
    ajv.addFormat("uint64", unsigned);
    ajv.addFormat("uint32", unsigned);
    ajv.addSchema(eventSchema, "protocol");
    const validator = ajv.getSchema<ServerMessage>("protocol#/$defs/ServerMessage");
    if (validator === undefined) {
      throw new Error("event_schema.json has no ServerMessage definition; rerun codegen");
    }
    cachedValidator = validator;
  }
  return cachedValidator;
}
