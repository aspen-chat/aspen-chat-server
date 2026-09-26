import { describe, expect, it } from "vitest";
import { EventStream, compileValidator, reconnectDelayMs } from "../src";

const uuid = "0190f0a0-0000-7000-8000-000000000001";

describe("reconnectDelayMs", () => {
  it("retries immediately, then backs off exponentially to a 5s ceiling", () => {
    expect([0, 1, 2, 3, 4, 5, 6, 40].map(reconnectDelayMs)).toEqual([
      0, 500, 1000, 2000, 4000, 5000, 5000, 5000,
    ]);
  });
});

describe("ServerMessage validator", () => {
  const validate = compileValidator();

  it("accepts ready, event, and error frames", () => {
    expect(validate({ type: "ready", userId: uuid, resumed: false })).toBe(true);
    expect(
      validate({
        type: "event",
        sequence: 7,
        event: { serverEvent: "react", type: "create", messageId: uuid, emoji: "😁", userId: uuid },
      }),
    ).toBe(true);
    expect(
      validate({
        type: "event",
        sequence: 8,
        event: { serverEvent: "community", type: "update", id: uuid, icon: null },
      }),
    ).toBe(true);
    expect(validate({ type: "error", code: "unauthorized", detail: "nope" })).toBe(true);
  });

  it("rejects frames that are not part of the protocol", () => {
    expect(validate({ type: "event", sequence: 1, event: { serverEvent: "nope" } })).toBe(false);
    expect(validate({ serverEvent: "react", type: "create" })).toBe(false);
    expect(validate({ type: "ready" })).toBe(false);
  });
});

/** Minimal scripted WebSocket. Tests drive `open`, `message`, and `close` explicitly. */
class FakeSocket {
  static instances: FakeSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: unknown }) => void) | null = null;
  onclose: ((e: { code: number; reason: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  readonly sent: unknown[] = [];

  constructor(readonly url: string) {
    FakeSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }

  close(): void {
    this.closed = true;
  }

  /** Simulates the server accepting `identify`. */
  ready(resumed: boolean): void {
    this.onmessage?.({ data: JSON.stringify({ type: "ready", userId: uuid, resumed }) });
  }

  event(sequence: number): void {
    this.onmessage?.({
      data: JSON.stringify({
        type: "event",
        sequence,
        event: { serverEvent: "message", type: "delete", id: uuid },
      }),
    });
  }
}

/** Manual clock so backoff timers fire only when a test says so. */
class FakeTimers {
  #pending: { at: number; fn: () => void; id: number }[] = [];
  #nextId = 1;
  now = 0;

  setTimeout = ((fn: () => void, ms: number) => {
    const id = this.#nextId++;
    this.#pending.push({ at: this.now + ms, fn, id });
    return id as unknown as ReturnType<typeof setTimeout>;
  }) as typeof globalThis.setTimeout;

  clearTimeout = ((id: ReturnType<typeof setTimeout>) => {
    this.#pending = this.#pending.filter((p) => p.id !== (id as unknown as number));
  }) as typeof globalThis.clearTimeout;

  advance(ms: number): void {
    this.now += ms;
    const due = this.#pending.filter((p) => p.at <= this.now).sort((a, b) => a.at - b.at);
    this.#pending = this.#pending.filter((p) => p.at > this.now);
    for (const p of due) {
      p.fn();
    }
  }
}

/** Lets the async `authenticate` step inside the stream settle. */
async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) {
    await Promise.resolve();
  }
}

function harness(tokens: (string | null)[] = ["token-1", "token-2", "token-3"]) {
  FakeSocket.instances = [];
  const timers = new FakeTimers();
  const log: string[] = [];
  const authCalls: boolean[] = [];
  const stream = new EventStream({
    url: "wss://aspen.test/api/v1/events",
    validate: true,
    authenticate: ({ forceRefresh }) => {
      authCalls.push(forceRefresh);
      return Promise.resolve(tokens.shift() ?? null);
    },
    WebSocket: FakeSocket as unknown as typeof WebSocket,
    setTimeout: timers.setTimeout,
    clearTimeout: timers.clearTimeout,
    onReady: (info) => log.push(`ready:${String(info.resumed)}`),
    onConnectionLost: (reason) => log.push(`lost:${reason}`),
    onResyncRequired: () => log.push("resync"),
    onEvent: (e) => log.push(`event:${e.serverEvent}`),
    onInvalidEvent: () => log.push("invalid"),
  });
  return { stream, timers, log, authCalls, sockets: FakeSocket.instances };
}

describe("EventStream", () => {
  it("identifies on open, then delivers validated events", async () => {
    const { stream, log, sockets, authCalls } = harness();
    stream.start();
    await flush();
    const socket = sockets[0];
    expect(socket?.url).toBe("wss://aspen.test/api/v1/events");
    expect(authCalls).toEqual([false]);
    socket?.onopen?.();
    expect(socket?.sent).toEqual([{ type: "identify", sessionToken: "token-1" }]);
    expect(stream.status).toBe("connecting");
    socket?.ready(false);
    expect(stream.status).toBe("open");
    socket?.event(41);
    socket?.onmessage?.({ data: "not json" });
    socket?.onmessage?.({ data: JSON.stringify({ type: "event", sequence: 42, event: {} }) });
    expect(log).toEqual(["ready:false", "event:message", "invalid", "invalid"]);
    expect(stream.lastSequence).toBe(41);
  });

  it("resumes from the last sequence and needs no resync when the server honours it", async () => {
    const { stream, timers, log, sockets } = harness();
    stream.start();
    await flush();
    sockets[0]?.onopen?.();
    sockets[0]?.ready(false);
    sockets[0]?.event(10);
    sockets[0]?.onclose?.({ code: 1006, reason: "" });
    expect(stream.status).toBe("reconnecting");
    // Attempt 1 is immediate.
    timers.advance(0);
    await flush();
    expect(sockets).toHaveLength(2);
    sockets[1]?.onopen?.();
    expect(sockets[1]?.sent).toEqual([
      { type: "identify", sessionToken: "token-2", resumeAfter: 10 },
    ]);
    sockets[1]?.ready(true);
    expect(log).toEqual([
      "ready:false",
      "event:message",
      "lost:connection closed (1006)",
      "ready:true",
    ]);
  });

  it("demands a resync when the server could not replay the gap", async () => {
    const { stream, timers, log, sockets } = harness();
    stream.start();
    await flush();
    sockets[0]?.onopen?.();
    sockets[0]?.ready(false);
    sockets[0]?.event(10);
    sockets[0]?.onclose?.({ code: 1006, reason: "server restart" });
    timers.advance(0);
    await flush();
    sockets[1]?.onopen?.();
    sockets[1]?.ready(false);
    expect(log).toEqual([
      "ready:false",
      "event:message",
      "lost:server restart",
      "resync",
      "ready:false",
    ]);
    // After a resync the next identify starts fresh rather than asking to resume a stale point.
    expect(stream.lastSequence).toBeNull();
  });

  it("backs off between failed attempts and reports one outage", async () => {
    const { stream, timers, log, sockets } = harness();
    stream.start();
    await flush();
    sockets[0]?.onclose?.({ code: 1006, reason: "" });
    timers.advance(0);
    await flush();
    expect(sockets).toHaveLength(2);
    sockets[1]?.onclose?.({ code: 1006, reason: "" });
    timers.advance(499);
    await flush();
    expect(sockets).toHaveLength(2);
    timers.advance(1);
    await flush();
    expect(sockets).toHaveLength(3);
    expect(log).toEqual(["lost:connection closed (1006)"]);
  });

  it("asks for a fresh token after the server rejects the current one", async () => {
    const { stream, timers, sockets, authCalls } = harness();
    stream.start();
    await flush();
    sockets[0]?.onopen?.();
    sockets[0]?.onmessage?.({
      data: JSON.stringify({ type: "error", code: "unauthorized", detail: "Invalid auth token." }),
    });
    sockets[0]?.onclose?.({ code: 4401, reason: "unauthorized" });
    timers.advance(0);
    await flush();
    expect(authCalls).toEqual([false, true]);
    sockets[1]?.onopen?.();
    expect(sockets[1]?.sent).toEqual([{ type: "identify", sessionToken: "token-2" }]);
  });

  it("stops when there is no session to identify with", async () => {
    const { stream, log, sockets } = harness([null]);
    stream.start();
    await flush();
    expect(sockets).toHaveLength(0);
    expect(stream.status).toBe("closed");
    expect(log).toEqual(["lost:no session"]);
  });

  it("stop() closes the socket and cancels pending reconnects", async () => {
    const { stream, timers, sockets } = harness();
    stream.start();
    await flush();
    sockets[0]?.onclose?.({ code: 1006, reason: "" });
    stream.stop();
    timers.advance(10_000);
    await flush();
    expect(sockets).toHaveLength(1);
    expect(stream.status).toBe("closed");
  });
});
