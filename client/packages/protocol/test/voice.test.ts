import { describe, expect, it, vi } from "vitest";
import {
  AspenClient,
  MemorySessionStore,
  REJOIN_DELAY_MAX_MS,
  VoiceCall,
  rankCandidates,
  signallingUrl,
  type VoiceDevice,
  type VoiceMedia,
  type VoiceTransport,
} from "../src";
import type { ClientMessage } from "../src/generated/voiceSignal";

const baseUrl = "http://api.example.org";
const channel = "0190f0a0-0000-7000-8000-000000000020";
const me = "0190f0a0-0000-7000-8000-000000000001";
const session = "0190f0a0-0000-7000-8000-000000000900";
const servers = {
  near: {
    id: "0190f0a0-0000-7000-8000-00000000a001",
    name: "near",
    url: "http://near.example.org",
  },
  far: { id: "0190f0a0-0000-7000-8000-00000000a002", name: "far", url: "http://far.example.org" },
  dead: {
    id: "0190f0a0-0000-7000-8000-00000000a003",
    name: "dead",
    url: "http://dead.example.org",
  },
};

function liveSession() {
  return {
    sessionToken: "s",
    sessionTokenExpires: new Date(Date.now() + 3_600_000).toISOString(),
    refreshToken: "r",
    userId: me,
  };
}

/** A voice server's socket: answers the signalling flow, or refuses, per its host. */
class FakeSocket {
  static instances: FakeSocket[] = [];
  static behaviour = new Map<string, "ready" | "refuse" | "silent">();
  readonly sent: ClientMessage[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  readonly OPEN = 1;
  constructor(readonly url: string) {
    FakeSocket.instances.push(this);
    queueMicrotask(() => this.onopen?.());
  }
  get readyState() {
    return this.closed ? 3 : this.OPEN;
  }
  frame(frame: unknown) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
  send(data: string) {
    const frame = JSON.parse(data) as ClientMessage;
    this.sent.push(frame);
    const host = new URL(this.url).hostname.split(".")[0] ?? "";
    const mode = FakeSocket.behaviour.get(host) ?? "ready";
    queueMicrotask(() => {
      switch (frame.type) {
        case "identify":
          if (mode === "refuse") {
            this.frame({
              type: "error",
              detail: "the token's signature does not match",
              fatal: true,
            });
          } else if (mode === "ready") {
            this.frame({
              type: "ready",
              session,
              user: me,
              routerRtpCapabilities: { codecs: [] },
              participants: [],
            });
          }
          break;
        case "createTransport":
          this.frame({
            type: "transportCreated",
            direction: frame.direction,
            id: `t-${frame.direction}`,
            iceParameters: {},
            iceCandidates: [],
            dtlsParameters: {},
          });
          break;
        case "connectTransport":
          this.frame({ type: "transportConnected", transportId: frame.transportId });
          break;
        case "produce":
          this.frame({ type: "produced", producerId: `p-${frame.source}`, source: frame.source });
          break;
        case "produceRtp":
          this.frame({
            type: "rtpProduced",
            producerId: `rtp-${frame.source}`,
            source: frame.source,
            ip: "192.0.2.10",
            port: 40000,
            ssrc: 1234,
            payloadType: 96,
            srtpCryptoSuite: "AES_CM_128_HMAC_SHA1_80",
            srtpKeyBase64: "a2V5",
          });
          break;
        default:
          break;
      }
    });
  }
  close() {
    this.closed = true;
    this.onclose?.();
  }
}

class FakeTransport implements VoiceTransport {
  handlers = new Map<string, ((...args: never[]) => void)[]>();
  closed = false;
  produced: string[] = [];
  closedProducers: string[] = [];
  replaced: MediaStreamTrack[] = [];
  consumed: string[] = [];
  connectionState = "new";
  /** Whether ICE succeeds once the microphone is produced. */
  constructor(readonly mediaReachable = true) {}
  on(event: string, handler: (...args: never[]) => void) {
    this.handlers.set(event, [...(this.handlers.get(event) ?? []), handler]);
    return this;
  }
  #handler(event: string) {
    return this.handlers.get(event)?.[0];
  }
  setConnectionState(state: string) {
    this.connectionState = state;
    for (const handler of this.handlers.get("connectionstatechange") ?? []) {
      (handler as (s: string) => void)(state);
    }
  }
  readonly codecOptions: Record<string, unknown>[] = [];
  async produce(options: {
    track: MediaStreamTrack;
    appData: Record<string, unknown>;
    codecOptions?: Record<string, unknown>;
  }) {
    if (options.codecOptions !== undefined) {
      this.codecOptions.push({ source: options.appData.source, ...options.codecOptions });
    }
    const connect = this.#handler("connect") as
      | ((p: { dtlsParameters: unknown }, cb: () => void, eb: (e: Error) => void) => void)
      | undefined;
    await new Promise<void>((resolve, reject) =>
      connect?.({ dtlsParameters: {} }, resolve, reject),
    );
    const produce = this.#handler("produce") as
      | ((
          p: { kind: string; rtpParameters: unknown; appData: Record<string, unknown> },
          cb: (r: { id: string }) => void,
          eb: (e: Error) => void,
        ) => void)
      | undefined;
    const id = await new Promise<string>((resolve, reject) =>
      produce?.(
        { kind: options.track.kind, rtpParameters: {}, appData: options.appData },
        (r) => {
          resolve(r.id);
        },
        reject,
      ),
    );
    this.produced.push(id);
    queueMicrotask(() => {
      this.setConnectionState(this.mediaReachable ? "connected" : "failed");
    });
    return {
      id,
      close: () => {
        this.closedProducers.push(id);
      },
      replaceTrack: (replacement: { track: MediaStreamTrack }) => {
        this.replaced.push(replacement.track);
        return Promise.resolve();
      },
    };
  }
  consume(options: { id: string }) {
    this.consumed.push(options.id);
    return Promise.resolve({
      id: options.id,
      track: { stop: () => undefined } as unknown as MediaStreamTrack,
      close: () => undefined,
    });
  }
  close() {
    this.closed = true;
  }
}

/** A media track with the two things the call uses: stopping it, and hearing it end. */
class FakeTrack {
  stopped = false;
  contentHint = "";
  #listeners: (() => void)[] = [];
  constructor(readonly kind: "audio" | "video") {}
  stop() {
    this.stopped = true;
  }
  addEventListener(_event: "ended", listener: () => void) {
    this.#listeners.push(listener);
  }
  end() {
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

function fakeMedia(unreachableSendTransports = 0, microphone: "ok" | "denied" = "ok") {
  const transports: FakeTransport[] = [];
  const screens: { video: FakeTrack; audio: FakeTrack }[] = [];
  const microphones: (string | null)[] = [];
  const outputs: (string | null)[] = [];
  const volumes: string[] = [];
  let sendTransports = 0;
  const played: string[] = [];
  const device: VoiceDevice = {
    load: () => Promise.resolve(),
    rtpCapabilities: { codecs: [] },
    createSendTransport: () => {
      sendTransports += 1;
      const t = new FakeTransport(sendTransports > unreachableSendTransports);
      transports.push(t);
      return t;
    },
    createRecvTransport: () => {
      const t = new FakeTransport();
      transports.push(t);
      return t;
    },
  };
  const media: VoiceMedia = {
    createDevice: () => Promise.resolve(device),
    getMicrophone: (choice) => {
      microphones.push(choice === "default" ? null : choice.id);
      return microphone === "ok"
        ? Promise.resolve(new FakeTrack("audio") as unknown as MediaStreamTrack)
        : Promise.reject(new Error("Permission denied"));
    },
    setOutput: (choice) => {
      outputs.push(choice === "default" ? null : choice.id);
      return Promise.resolve();
    },
    setVolume: (consumerId, gain) => {
      volumes.push(`${consumerId}=${String(gain)}`);
    },
    getScreen: () => {
      const capture = { video: new FakeTrack("video"), audio: new FakeTrack("audio") };
      screens.push(capture);
      return Promise.resolve(
        capture as unknown as { video: MediaStreamTrack; audio: MediaStreamTrack },
      );
    },
    play: (id) => {
      played.push(id);
    },
    stop: () => undefined,
  };
  return { media, transports, played, screens, microphones, outputs, volumes };
}

function makeCall(options: {
  candidates: (keyof typeof servers)[];
  latency: Record<string, number>;
  existing?: boolean;
  unreachableSendTransports?: number;
  microphone?: "ok" | "denied";
  userVolume?: (userId: string) => number;
  /** What the join offer grants; both by default. */
  speak?: boolean;
  shareScreen?: boolean;
}) {
  FakeSocket.instances = [];
  const calls: string[] = [];
  const timers: { fn: () => void; delay: number }[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init);
    const url = new URL(request.url);
    calls.push(`${request.method} ${url.host}${url.pathname}`);
    if (url.pathname.endsWith("/voice/join")) {
      return new Response(
        JSON.stringify({
          channelId: channel,
          ...(options.existing
            ? {
                session: {
                  id: session,
                  channel,
                  voiceServer: servers.near.id,
                  createdAt: "2026-09-26T00:00:00Z",
                },
              }
            : {}),
          candidates: options.candidates.map((k) => servers[k]),
          token: "tok",
          expiresAt: "2030-01-01T00:00:00Z",
          speak: options.speak ?? true,
          shareScreen: options.shareScreen ?? true,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (url.pathname === "/health") {
      const ms = options.latency[url.hostname.split(".")[0] ?? ""] ?? Infinity;
      if (ms === Infinity) {
        throw new TypeError("unreachable");
      }
      await new Promise((resolve) => setTimeout(resolve, ms));
      return new Response(null, { status: 204 });
    }
    if (url.pathname.includes("/failures")) {
      return new Response(JSON.stringify({ failures: 1, disabled: false }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    throw new Error(`unexpected ${url.toString()}`);
  };
  const store = new MemorySessionStore();
  store.save(liveSession());
  const client = new AspenClient({ baseUrl, sessionStore: store, fetch });
  const { media, transports, played, screens, microphones, outputs, volumes } = fakeMedia(
    options.unreachableSendTransports ?? 0,
    options.microphone ?? "ok",
  );
  const call = new VoiceCall({
    client,
    media,
    fetch,
    WebSocket: FakeSocket as unknown as typeof WebSocket,
    setTimeout: ((fn: () => void, delay: number) => {
      timers.push({ fn, delay });
      return 0;
    }) as unknown as typeof setTimeout,
    random: () => 0.5,
    ...(options.userVolume === undefined ? {} : { userVolume: options.userVolume }),
  });
  return { call, calls, transports, played, screens, microphones, outputs, volumes, timers };
}

describe("VoiceCall", () => {
  it("orders candidates by latency and joins the nearest, producing the microphone", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, transports } = makeCall({
      candidates: ["far", "near"],
      latency: { far: 80, near: 5 },
    });
    const states: string[] = [];
    call.subscribe(() => states.push(call.state.status));
    await call.join(channel);
    expect(call.state).toMatchObject({ status: "connected", channelId: channel, session });
    expect(calls.filter((c) => c.includes("/health")).length).toBe(2);
    expect(FakeSocket.instances.map((s) => new URL(s.url).host)).toEqual(["near.example.org"]);
    expect(FakeSocket.instances[0]?.sent.map((f) => f.type)).toEqual([
      "identify",
      "setCapabilities",
      "createTransport",
      "createTransport",
      "connectTransport",
      "produce",
      "setState",
    ]);
    expect(transports[0]?.produced).toEqual(["p-microphone"]);
    expect(states).toContain("joining");
    expect(calls.some((c) => c.includes("/failures"))).toBe(false);
  });

  it("joins to listen without opening the microphone when the channel does not allow speaking", async () => {
    FakeSocket.behaviour = new Map();
    const { call, transports } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      speak: false,
      shareScreen: false,
      microphone: "denied",
    });
    await call.join(channel);
    expect(call.state).toMatchObject({ status: "connected", canSpeak: false, canShare: false });
    expect(transports[0]?.produced).toEqual([]);
    expect(FakeSocket.instances[0]?.sent.some((f) => f.type === "produce")).toBe(false);
    const prepared = { video: new FakeTrack("video"), audio: null };
    await call.startScreenShare({
      prepared: prepared as unknown as { video: MediaStreamTrack; audio: null },
    });
    expect(prepared.video.stopped).toBe(true);
    expect(call.state.sharingScreen).toBe(false);
  });

  it("reports a server that refuses and falls through to the next", async () => {
    FakeSocket.behaviour = new Map([["dead", "refuse"]]);
    const { call, calls } = makeCall({
      candidates: ["dead", "far"],
      latency: { dead: 1, far: 50 },
    });
    await call.join(channel);
    expect(call.state.status).toBe("connected");
    expect(FakeSocket.instances.map((s) => new URL(s.url).host)).toEqual([
      "dead.example.org",
      "far.example.org",
    ]);
    expect(calls.filter((c) => c.includes("/failures"))).toEqual([
      `POST api.example.org/api/v1/voice-servers/${servers.dead.id}/failures`,
    ]);
  });

  it("treats media that never connects as the server's failure and tries the next", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, transports } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 30 },
      unreachableSendTransports: 1,
    });
    await call.join(channel);
    expect(call.state.status).toBe("connected");
    expect(FakeSocket.instances.map((s) => new URL(s.url).host)).toEqual([
      "near.example.org",
      "far.example.org",
    ]);
    expect(calls.filter((c) => c.includes("/failures"))).toEqual([
      `POST api.example.org/api/v1/voice-servers/${servers.near.id}/failures`,
    ]);
    expect(transports[0]?.closed).toBe(true);
    expect(transports[2]?.connectionState).toBe("connected");
  });

  it("rejoins when media fails mid-call", async () => {
    FakeSocket.behaviour = new Map();
    const { call, transports, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    transports[0]?.setConnectionState("failed");
    expect(call.state.status).toBe("rejoining");
    expect(timers.at(-1)?.delay).toBe(REJOIN_DELAY_MAX_MS / 2);
  });

  it("a refused microphone fails the join without touching or blaming any server", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 2 },
      microphone: "denied",
    });
    await expect(call.join(channel)).rejects.toThrow("Permission denied");
    expect(call.state).toMatchObject({
      status: "failed",
      channelId: channel,
      errorKind: "microphone",
      error: "Permission denied",
    });
    expect(FakeSocket.instances).toHaveLength(0);
    expect(calls.some((c) => c.includes("/failures") || c.includes("/health"))).toBe(false);
  });

  it("fails once every candidate is exhausted", async () => {
    FakeSocket.behaviour = new Map([
      ["dead", "refuse"],
      ["far", "refuse"],
    ]);
    const { call } = makeCall({ candidates: ["dead", "far"], latency: { dead: 1, far: 2 } });
    await expect(call.join(channel)).rejects.toThrow();
    expect(call.state).toMatchObject({ status: "failed", errorKind: "server" });
    expect(call.state.error).toContain("signature");
  });

  it("plays consumers the server announces and stops them when told", async () => {
    FakeSocket.behaviour = new Map();
    const { call, played, transports } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      existing: true,
    });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    socket?.frame({
      type: "newConsumer",
      consumerId: "c1",
      producerId: "p9",
      user: "u2",
      kind: "audio",
      source: "microphone",
      rtpParameters: {},
      producerPaused: false,
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(played).toEqual(["c1"]);
    expect(transports[1]?.consumed).toEqual(["c1"]);
    expect(socket?.sent.at(-1)).toEqual({ type: "resumeConsumer", consumerId: "c1" });
    socket?.frame({ type: "consumerClosed", consumerId: "c1" });
    call.setMuted(true);
    expect(socket?.sent.at(-1)).toEqual({ type: "setState", muted: true, deafened: false });
    call.leave();
    expect(call.state.status).toBe("idle");
    expect(socket?.closed).toBe(true);
  });

  it("rejoins after a random delay when the server is lost, but not when idle", async () => {
    FakeSocket.behaviour = new Map();
    const { call, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    call.onSessionEnded({ id: session, channel, reason: "serverLost" });
    expect(call.state.status).toBe("rejoining");
    // random() is 0.5, so the pause is half the maximum
    const pause = timers.at(-1);
    expect(pause?.delay).toBe(REJOIN_DELAY_MAX_MS / 2);
    pause?.fn();
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(call.state.status).toBe("connected");
    expect(FakeSocket.instances).toHaveLength(2);
    call.onSessionEnded({ id: session, channel, reason: "idle" });
    expect(call.state.status).toBe("idle");
    expect(call.state.endedReason).toBe("idle");
    call.acknowledgeEnd();
    expect(call.state.endedReason).toBeNull();
    expect(REJOIN_DELAY_MAX_MS).toBe(1000);
  });

  it("shares a screen with its sound, shows others' screens, and stops when the capture ends", async () => {
    FakeSocket.behaviour = new Map();
    const { call, transports, screens, played } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    await call.startScreenShare();
    expect(call.state.sharingScreen).toBe(true);
    expect(call.state.localScreen).toBe(screens[0]?.video);
    const produced = socket?.sent.filter((f) => f.type === "produce");
    expect(produced?.map((f) => `${f.kind}:${f.source}`)).toEqual([
      "audio:microphone",
      "video:screen",
      "audio:screenAudio",
    ]);
    // the picture starts at a high bitrate rather than climbing to it, the screen's sound goes
    // out in stereo at a music bitrate, and the microphone keeps the defaults
    expect(transports[0]?.codecOptions).toEqual([
      { source: "screen", videoGoogleStartBitrate: 10_000 },
      { source: "screenAudio", opusStereo: true, opusDtx: false, opusMaxAverageBitrate: 128_000 },
    ]);
    expect(screens[0]?.video.contentHint).toBe("");
    // a second start while sharing is a no-op
    await call.startScreenShare();
    expect(screens).toHaveLength(1);
    // another participant's screen arrives as a video consumer, shown rather than played
    socket?.frame({
      type: "newConsumer",
      consumerId: "v1",
      producerId: "p9",
      user: "u2",
      kind: "video",
      source: "screen",
      rtpParameters: {},
      producerPaused: false,
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(call.state.screens.map((s) => [s.user, s.consumerId])).toEqual([["u2", "v1"]]);
    expect(played).toEqual([]);
    expect(transports[1]?.consumed).toEqual(["v1"]);
    socket?.frame({ type: "consumerClosed", consumerId: "v1" });
    expect(call.state.screens).toEqual([]);
    // the user stops the capture from the browser's own control
    screens[0]?.video.end();
    expect(call.state.sharingScreen).toBe(false);
    expect(call.state.localScreen).toBeNull();
    expect(transports[0]?.closedProducers).toEqual(["p-screen", "p-screenAudio"]);
    expect(socket?.sent.filter((f) => f.type === "closeProducer").map((f) => f.producerId)).toEqual(
      ["p-screen", "p-screenAudio"],
    );
    expect(screens[0]?.video.stopped && screens[0].audio.stopped).toBe(true);
    // sharing again, then leaving, releases the capture
    await call.startScreenShare();
    expect(call.state.sharingScreen).toBe(true);
    call.leave();
    expect(call.state.sharingScreen).toBe(false);
    expect(screens[1]?.video.stopped).toBe(true);
  });

  it("shares a capture made elsewhere, and releases one it cannot use", async () => {
    FakeSocket.behaviour = new Map();
    const { call, screens } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    const idle = { video: new FakeTrack("video"), audio: null };
    await call.startScreenShare({
      prepared: idle as unknown as { video: MediaStreamTrack; audio: null },
    });
    expect(idle.video.stopped).toBe(true);
    await call.join(channel);
    const prepared = { video: new FakeTrack("video"), audio: null };
    await call.startScreenShare({
      prepared: prepared as unknown as { video: MediaStreamTrack; audio: null },
    });
    expect(screens).toHaveLength(0);
    expect(call.state.localScreen).toBe(prepared.video);
    const produced = FakeSocket.instances[0]?.sent.filter((f) => f.type === "produce");
    expect(produced?.map((f) => f.source)).toEqual(["microphone", "screen"]);
    call.stopScreenShare();
    expect(prepared.video.stopped).toBe(true);
  });

  it("shares the browser's picture with sound from outside the browser in its place", async () => {
    FakeSocket.behaviour = new Map();
    const { call, transports, screens } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const started: unknown[] = [];
    let stopped = 0;
    const audio = {
      start: (target: unknown) => {
        started.push(target);
        return Promise.resolve();
      },
      stop: () => {
        stopped += 1;
      },
    };
    await call.startScreenShare({ audio, contentHint: "motion" });
    expect(screens[0]?.video.contentHint).toBe("motion");
    // the picture is the browser's; its own sound is dropped for the external one
    expect(
      socket?.sent.filter((f) => f.type === "produce").map((f) => `${f.kind}:${f.source}`),
    ).toEqual(["audio:microphone", "video:screen"]);
    expect(screens[0]?.audio.stopped).toBe(true);
    expect(socket?.sent.filter((f) => f.type === "produceRtp")).toEqual([
      { type: "produceRtp", source: "screenAudio" },
    ]);
    expect(started).toEqual([
      {
        ip: "192.0.2.10",
        port: 40000,
        ssrc: 1234,
        payloadType: 96,
        srtpCryptoSuite: "AES_CM_128_HMAC_SHA1_80",
        srtpKeyBase64: "a2V5",
      },
    ]);
    expect(call.state).toMatchObject({ sharingScreen: true, localScreen: screens[0]?.video });
    // an external share cannot start alongside it
    const external = { audio: false, start: () => Promise.resolve(), stop: () => undefined };
    await call.startExternalScreenShare(external);
    expect(socket?.sent.filter((f) => f.type === "produceRtp")).toHaveLength(1);
    // stopping ends both the capture and the external sound
    call.stopScreenShare();
    expect(stopped).toBe(1);
    expect(transports[0]?.closedProducers).toEqual(["p-screen"]);
    expect(socket?.sent.filter((f) => f.type === "closeProducer").map((f) => f.producerId)).toEqual(
      ["rtp-screenAudio", "p-screen"],
    );
    expect(call.state).toMatchObject({ sharingScreen: false, localScreen: null });
    // sound that cannot start ends the share it was part of
    const failing = { start: () => Promise.reject(new Error("no route")), stop: () => undefined };
    await expect(call.startScreenShare({ audio: failing })).rejects.toThrow("no route");
    expect(call.state.sharingScreen).toBe(false);
    expect(screens[1]?.video.stopped).toBe(true);
  });

  it("shares an external sender through RTP producers and previews it from the server", async () => {
    FakeSocket.behaviour = new Map();
    const { call, played } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const started: unknown[] = [];
    let stopped = 0;
    const share = {
      audio: true,
      start: (targets: unknown) => {
        started.push(targets);
        return Promise.resolve();
      },
      stop: () => {
        stopped += 1;
      },
    };
    await call.startExternalScreenShare(share);
    expect(socket?.sent.slice(-2)).toEqual([
      { type: "produceRtp", source: "screen" },
      { type: "produceRtp", source: "screenAudio" },
    ]);
    const target = {
      ip: "192.0.2.10",
      port: 40000,
      ssrc: 1234,
      payloadType: 96,
      srtpCryptoSuite: "AES_CM_128_HMAC_SHA1_80",
      srtpKeyBase64: "a2V5",
    };
    expect(started).toEqual([{ video: target, audio: target }]);
    expect(call.state).toMatchObject({ sharingScreen: true, localScreen: null });
    // the preview is a consumer of the call's own producer
    socket?.frame({
      type: "newConsumer",
      consumerId: "self1",
      producerId: "rtp-screen",
      user: me,
      kind: "video",
      source: "screen",
      rtpParameters: {},
      producerPaused: false,
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(call.state.localScreen).not.toBeNull();
    expect(call.state.screens).toEqual([]);
    expect(played).toEqual([]);
    // a second external share while one runs is a no-op
    await call.startExternalScreenShare(share);
    expect(started).toHaveLength(1);
    call.stopScreenShare();
    expect(stopped).toBe(1);
    expect(socket?.sent.slice(-2)).toEqual([
      { type: "closeProducer", producerId: "rtp-screen" },
      { type: "closeProducer", producerId: "rtp-screenAudio" },
    ]);
    expect(call.state).toMatchObject({ sharingScreen: false, localScreen: null });
    // a share whose sender cannot start is closed again; one without audio asks for one producer
    const failing = {
      audio: false,
      start: () => Promise.reject(new Error("no route")),
      stop: () => undefined,
    };
    await expect(call.startExternalScreenShare(failing)).rejects.toThrow("no route");
    expect(call.state.sharingScreen).toBe(false);
    expect(socket?.sent.at(-1)).toEqual({ type: "closeProducer", producerId: "rtp-screen" });
    expect(socket?.sent.filter((f) => f.type === "produceRtp")).toHaveLength(3);
  });

  it("takes a moderator's mute as its own state and a kick as a reason to tell the user", async () => {
    FakeSocket.behaviour = new Map();
    const { call, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    socket?.frame({ type: "participantState", user: me, muted: true, deafened: false });
    expect(call.state.muted).toBe(true);
    socket?.frame({ type: "participantState", user: "someone-else", muted: false, deafened: true });
    expect(call.state).toMatchObject({ muted: true, deafened: false });
    socket?.frame({ type: "kicked", reason: "replaced" });
    expect(call.state).toMatchObject({ status: "idle", endedReason: null });
    await call.join(channel);
    socket?.frame({ type: "kicked", reason: "kicked" });
    FakeSocket.instances[1]?.frame({ type: "kicked", reason: "kicked" });
    expect(call.state).toMatchObject({ status: "idle", endedReason: "kicked" });
    call.acknowledgeEnd();
    await call.join(channel);
    FakeSocket.instances[2]?.frame({ type: "kicked", reason: "serverStopping" });
    expect(call.state.status).toBe("rejoining");
    expect(timers.at(-1)?.delay).toBe(REJOIN_DELAY_MAX_MS / 2);
  });

  it("captures the preferred microphone, swaps it mid-call, and moves playback to the preferred speaker", async () => {
    FakeSocket.behaviour = new Map();
    const { call, transports, microphones, outputs } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    const micA = { id: "mic-a", label: "A" };
    const micB = { id: "mic-b", label: "B" };
    const spkA = { id: "spk-a", label: "S" };
    await call.setAudioDevices({ input: micA, output: spkA });
    expect(outputs).toEqual(["spk-a"]);
    await call.join(channel);
    expect(microphones).toEqual(["mic-a"]);
    await call.setAudioDevices({ input: micB, output: spkA });
    expect(microphones).toEqual(["mic-a", "mic-b"]);
    expect(transports[0]?.replaced).toHaveLength(1);
    expect(outputs).toEqual(["spk-a"]);
    await call.setAudioDevices({ input: micB, output: "default" });
    expect(outputs).toEqual(["spk-a", null]);
    expect(transports[0]?.replaced).toHaveLength(1);
    call.leave();
    await call.setAudioDevices({ input: { id: "mic-c", label: "C" }, output: "default" });
    expect(microphones).toEqual(["mic-a", "mic-b"]);
  });

  it("leaves a call it is already in alone when asked to join it again", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const offers = () => calls.filter((c) => c.includes("/voice/join")).length;
    expect(offers()).toBe(1);
    await call.join(channel);
    expect(offers()).toBe(1);
    expect(FakeSocket.instances).toHaveLength(1);
    expect(call.state.status).toBe("connected");
  });

  it("applies each user's volume to their audio, present and future", async () => {
    FakeSocket.behaviour = new Map();
    const { call, volumes } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (userId) => (userId === "quiet" ? 1.5 : 1),
    });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const arrive = (consumerId: string, user: string) => {
      socket?.frame({
        type: "newConsumer",
        consumerId,
        producerId: `p-${consumerId}`,
        user,
        kind: "audio",
        source: "microphone",
        rtpParameters: {},
        producerPaused: false,
      });
    };
    arrive("a1", "quiet");
    arrive("a2", "loud");
    await new Promise((r) => setTimeout(r, 0));
    expect(volumes).toEqual(["a1=1.5"]);
    call.setUserVolume("loud", 0.25);
    expect(volumes).toEqual(["a1=1.5", "a2=0.25"]);
    call.setUserVolume("nobody", 2);
    expect(volumes).toHaveLength(2);
  });

  it("sets every consumer's gain again when who is silenced changes", async () => {
    FakeSocket.behaviour = new Map();
    let silenced = new Set<string>();
    const { call, volumes } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (userId) => (silenced.has(userId) ? 0 : 1),
    });
    await call.join(channel);
    for (const [consumerId, user] of [
      ["a1", "blocked-elsewhere"],
      ["a2", "friend"],
    ] as const) {
      FakeSocket.instances[0]?.frame({
        type: "newConsumer",
        consumerId,
        producerId: `p-${consumerId}`,
        user,
        kind: "audio",
        source: "microphone",
        rtpParameters: {},
        producerPaused: false,
      });
    }
    await new Promise((r) => setTimeout(r, 0));
    silenced = new Set(["blocked-elsewhere"]);
    call.refreshVolumes();
    expect(volumes).toEqual(["a1=0", "a2=1"]);
  });

  it("ranks unreachable servers last and builds signalling URLs", () => {
    const ranked = rankCandidates([
      { candidate: servers.far, latencyMs: Infinity },
      { candidate: servers.near, latencyMs: 3 },
    ]);
    expect(ranked.map((r) => r.candidate.name)).toEqual(["near", "far"]);
    expect(signallingUrl({ ...servers.near, url: "https://voice.example.org/" })).toBe(
      "wss://voice.example.org/ws",
    );
    expect(signallingUrl(servers.near)).toBe("ws://near.example.org/ws");
    vi.restoreAllMocks();
  });
});
