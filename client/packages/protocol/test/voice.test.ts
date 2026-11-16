import { describe, expect, it, vi } from "vitest";
import {
  AspenClient,
  CallSetupError,
  CameraError,
  DEAD_SERVER_RETRY_MS,
  ICE_RECOVERY_LIMIT_MS,
  ICE_RESTART_INTERVAL_MS,
  InsecureAddressError,
  MemorySessionStore,
  READY_TIMEOUT_MS,
  REJOIN_DELAY_MAX_MS,
  RESUME_ATTEMPTS,
  VoiceCall,
  rankCandidates,
  signallingUrl,
  type CallConnection,
  type CallTransceiver,
  type CameraFailure,
  type CaptureOffer,
  type VoiceMedia,
  trackSource,
} from "../src";
import type { ClientMessage, MediaSource, Section } from "../src/generated/voiceSignal";

const baseUrl = "https://api.example.org";
const channel = "0190f0a0-0000-7000-8000-000000000020";
const otherChannel = "0190f0a0-0000-7000-8000-000000000021";
const me = "0190f0a0-0000-7000-8000-000000000001";
const session = "0190f0a0-0000-7000-8000-000000000900";
const servers = {
  near: {
    id: "0190f0a0-0000-7000-8000-00000000a001",
    name: "near",
    url: "https://near.example.org",
  },
  far: { id: "0190f0a0-0000-7000-8000-00000000a002", name: "far", url: "https://far.example.org" },
  dead: {
    id: "0190f0a0-0000-7000-8000-00000000a003",
    name: "dead",
    url: "https://dead.example.org",
  },
  plain: {
    id: "0190f0a0-0000-7000-8000-00000000a004",
    name: "plain",
    url: "http://plain.example.org",
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

/** Lets every frame and step queued so far run: each server frame is a task of its own. */
async function settle(): Promise<void> {
  for (let i = 0; i < 12; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

/**
 * A fake offer's SDP: the mids of its sections, which `FakeConnection` makes a transceiver for
 * each of, as a browser does for a real offer's.
 */
function sdpOf(sections: readonly Section[]): string {
  return `mids:${sections.map((s) => s.mid).join(",")}`;
}

/**
 * A voice server's socket: answers the signalling flow as the server does, or refuses, per its
 * host. It offers the call connection after `ready`, and again after each `send`, `stopSending`,
 * and `watch`, each offer naming every section.
 */
class FakeSocket {
  static instances: FakeSocket[] = [];
  /**
   * Per host: answers everything (`ready`), refuses the token or the resume secret (`refuse`),
   * never answers `identify` (`silent`), turns `identify` away for coming too fast
   * (`throttledIdentify`), or refuses every `send` for coming too fast (`throttledSend`).
   */
  static behaviour = new Map<
    string,
    "ready" | "refuse" | "silent" | "throttledIdentify" | "throttledSend"
  >();
  /** Hosts whose sockets fail to open, as a server gone or a network away does. */
  static unreachable = new Set<string>();
  static secrets = 0;
  /** Who `ready` says is in the call. */
  static readyParticipants: unknown[] = [];
  readonly sent: ClientMessage[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  readonly OPEN = 1;
  revisions = { call: 0, capture: 0 };
  /** The call connection's sections, those the client sends and those it watches apart. */
  sending: Section[] = [{ mid: "0", kind: "audio", carries: "audioSlot", slot: 0 }];
  watched: Section[] = [];
  capture: Section[] = [];
  #mids = 1;
  constructor(readonly url: string) {
    FakeSocket.instances.push(this);
    setTimeout(() => {
      if (FakeSocket.unreachable.has(this.#host())) {
        this.onerror?.();
      } else {
        this.onopen?.();
      }
    }, 0);
  }
  #host(): string {
    return new URL(this.url).hostname.split(".")[0] ?? "";
  }
  get readyState() {
    return this.closed ? 3 : this.OPEN;
  }
  /** Delivers a frame from the server, in a task of its own as a browser's socket does. */
  frame(frame: { type: string } & Record<string, unknown>) {
    // Written as it is sent, as a server's frame is: what changes afterwards is not in it.
    const data = JSON.stringify(frame);
    setTimeout(() => {
      if (!this.closed) {
        this.onmessage?.({ data });
      }
    }, 0);
  }
  /** The call connection's sections as its next offer names them. */
  sections(): Section[] {
    return [...this.sending, ...this.watched];
  }
  offer(connection: "call" | "capture" = "call") {
    this.revisions[connection] += 1;
    const sections = connection === "call" ? this.sections() : this.capture;
    this.frame({
      type: "offer",
      connection,
      revision: this.revisions[connection],
      sdp: sdpOf(sections),
      sections,
    });
  }
  #ready() {
    FakeSocket.secrets += 1;
    this.frame({
      type: "ready",
      session,
      user: me,
      participants: FakeSocket.readyParticipants,
      audioSlots: 16,
      maxWatchedVideo: 25,
      resume: `secret-${String(FakeSocket.secrets)}`,
    });
  }
  #mid(): string {
    const mid = String(this.#mids);
    this.#mids += 1;
    return mid;
  }
  send(data: string) {
    const frame = JSON.parse(data) as ClientMessage;
    this.sent.push(frame);
    const mode = FakeSocket.behaviour.get(this.#host()) ?? "ready";
    switch (frame.type) {
      case "identify":
        if (mode === "refuse") {
          this.frame({
            type: "error",
            detail: "the token's signature does not match",
            fatal: true,
          });
        } else if (mode === "throttledIdentify") {
          this.frame({
            type: "error",
            detail: "too many identify frames; try again in 7s",
            fatal: true,
            retryAfterSeconds: 7,
          });
        } else if (mode !== "silent") {
          this.#ready();
          this.offer();
        }
        break;
      case "resume": {
        if (mode === "refuse" || mode === "silent") {
          this.frame({ type: "error", detail: "no such call to resume", fatal: true });
          break;
        }
        // The call's connection goes on at the server: its sections and revisions carry over,
        // and its last offer is restated, as one whose answer was lost with the socket is.
        const previous = FakeSocket.instances
          .filter((s) => s !== this && s.url === this.url)
          .at(-1);
        if (previous !== undefined) {
          this.revisions = { ...previous.revisions };
          this.sending = previous.sending;
          this.watched = previous.watched;
          this.capture = previous.capture;
          this.#mids = previous.#mids;
        }
        this.#ready();
        const sections = this.sections();
        this.frame({
          type: "offer",
          connection: "call",
          revision: this.revisions.call,
          sdp: "restated",
          sections,
        });
        break;
      }
      case "send": {
        if (mode === "throttledSend") {
          this.frame({
            type: "error",
            detail: "too many send frames; try again in 5s",
            fatal: false,
            retryAfterSeconds: 5,
            refused: "send",
          });
          break;
        }
        const kind = frame.source === "screen" || frame.source === "camera" ? "video" : "audio";
        const section: Section = {
          mid: this.#mid(),
          kind,
          carries: "sending",
          source: frame.source,
        };
        // Sent on once its media arrives, which a browser's here does at once, and a sender
        // outside the browser's never does.
        if (frame.external === true) {
          this.capture.push(section);
          this.offer("capture");
        } else {
          this.sending.push(section);
          this.offer();
          this.frame({ type: "sending", source: frame.source });
        }
        break;
      }
      case "stopSending": {
        const other = (s: Section) => !(s.carries === "sending" && s.source === frame.source);
        this.sending = this.sending.filter(other);
        this.capture = this.capture.filter(other);
        this.offer();
        break;
      }
      case "watch":
        this.watched = frame.video.map(({ user, source }) => ({
          mid: this.#mid(),
          kind: "video",
          carries: "video",
          user,
          source,
        }));
        this.offer();
        break;
      default:
        break;
    }
  }
  close() {
    this.closed = true;
    this.onclose?.();
  }
  /** The socket closes from the server's side, as a network change or a proxy does. */
  drop() {
    this.closed = true;
    this.onclose?.();
  }
  /** The answers the client sent, by connection. */
  answers(connection: "call" | "capture" = "call") {
    return this.sent.filter(
      (f): f is Extract<ClientMessage, { type: "answer" }> =>
        f.type === "answer" && f.connection === connection,
    );
  }
}

/** A media track with the things the call uses: stopping it, enabling it, and hearing it end. */
class FakeTrack {
  stopped = false;
  enabled = true;
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

class FakeTransceiver implements CallTransceiver {
  direction: RTCRtpTransceiverDirection = "recvonly";
  /** Every track the sender was given, in order, `null` for none. */
  readonly replaced: (MediaStreamTrack | null)[] = [];
  readonly encodings: RTCRtpEncodingParameters[] = [{}];
  readonly receiver = { track: new FakeTrack("audio") as unknown as MediaStreamTrack };
  readonly sender = {
    track: null as MediaStreamTrack | null,
    replaceTrack: (track: MediaStreamTrack | null) => {
      this.sender.track = track;
      this.replaced.push(track);
      return Promise.resolve();
    },
    getParameters: () =>
      ({ encodings: this.encodings.map((e) => ({ ...e })) }) as unknown as RTCRtpSendParameters,
    setParameters: (parameters: RTCRtpSendParameters) => {
      this.encodings.splice(0, this.encodings.length, ...parameters.encodings);
      return Promise.resolve();
    },
  };
  constructor(readonly mid: string) {}
}

/**
 * A peer connection that makes a transceiver for each mid an offer names, answers, and
 * connects once it has answered, unless it was made unreachable.
 */
class FakeConnection implements CallConnection {
  connectionState: RTCPeerConnectionState = "new";
  localDescription: { sdp: string } | null = null;
  readonly transceivers: FakeTransceiver[] = [];
  readonly offers: string[] = [];
  closed = false;
  /** What `setRemoteDescription` fails with, as a browser that cannot apply an offer does. */
  failure: Error | null = null;
  #listeners: (() => void)[] = [];
  #remote = "";
  constructor(readonly reachable: boolean) {}
  setRemoteDescription(description: RTCSessionDescriptionInit) {
    if (this.failure !== null) {
      return Promise.reject(this.failure);
    }
    const sdp = description.sdp ?? "";
    this.offers.push(sdp);
    this.#remote = sdp;
    for (const mid of sdp.replace("mids:", "").split(",").filter(Boolean)) {
      if (!this.transceivers.some((t) => t.mid === mid)) {
        this.transceivers.push(new FakeTransceiver(mid));
      }
    }
    return Promise.resolve();
  }
  createAnswer() {
    return Promise.resolve({ type: "answer" as const, sdp: `answer to ${this.#remote}` });
  }
  setLocalDescription(description: RTCSessionDescriptionInit) {
    this.localDescription = { sdp: description.sdp ?? "" };
    if (this.connectionState === "new") {
      setTimeout(() => {
        this.setState(this.reachable ? "connected" : "failed");
      }, 0);
    }
    return Promise.resolve();
  }
  getTransceivers() {
    return this.transceivers;
  }
  transceiver(mid: string) {
    return this.transceivers.find((t) => t.mid === mid);
  }
  addEventListener(_type: "connectionstatechange", listener: () => void) {
    this.#listeners.push(listener);
  }
  setState(state: RTCPeerConnectionState) {
    this.connectionState = state;
    for (const listener of this.#listeners) {
      listener();
    }
  }
  close() {
    this.closed = true;
    this.connectionState = "closed";
  }
}

/** An external sender that answers each capture offer, or fails to. */
function fakeSender(audio: boolean | null, failure: Error | null = null) {
  const offers: CaptureOffer[] = [];
  let stopped = 0;
  const sender = {
    answer: (offer: CaptureOffer) => {
      offers.push(offer);
      return failure === null
        ? Promise.resolve(`capture answer ${String(offer.revision)}`)
        : Promise.reject(failure);
    },
    stop: () => {
      stopped += 1;
    },
    ...(audio === null ? {} : { audio }),
  };
  return {
    sender,
    offers,
    stopped: () => stopped,
  };
}

function fakeMedia(unreachableConnections = 0, microphone: "ok" | "denied" = "ok") {
  const connections: FakeConnection[] = [];
  const screens: { video: FakeTrack; audio: FakeTrack }[] = [];
  const microphones: (string | null)[] = [];
  const microphoneTracks: FakeTrack[] = [];
  /** What making a connection fails with, when the browser cannot do WebRTC. */
  const setup: { failure: Error | null } = { failure: null };
  const outputs: (string | null)[] = [];
  const volumes: string[] = [];
  /** The gain each microphone opened at, then each change `setInputVolume` made. */
  const inputVolumes: string[] = [];
  const cameras: (string | null)[] = [];
  /** Each camera opened, and whether another was still open as it opened. */
  const cameraTracks: { track: FakeTrack; besideOpen: boolean }[] = [];
  /** Failures the next camera openings meet, in order; an empty queue opens one. */
  const cameraFailures: CameraFailure[] = [];
  const played: string[] = [];
  const stopped: string[] = [];
  const media: VoiceMedia = {
    hasScreenCapture: true,
    createConnection: () => {
      if (setup.failure !== null) {
        return Promise.reject(setup.failure);
      }
      const connection = new FakeConnection(connections.length >= unreachableConnections);
      connections.push(connection);
      return Promise.resolve(connection);
    },
    getMicrophone: (choice, gain) => {
      microphones.push(choice === "default" ? null : choice.id);
      inputVolumes.push(`open=${String(gain)}`);
      if (microphone !== "ok") {
        return Promise.reject(new Error("Permission denied"));
      }
      const track = new FakeTrack("audio");
      microphoneTracks.push(track);
      return Promise.resolve(track as unknown as MediaStreamTrack);
    },
    getCamera: (choice) => {
      cameras.push(choice === "default" ? null : choice.id);
      const failure = cameraFailures.shift();
      if (failure !== undefined) {
        return Promise.reject(new CameraError(failure));
      }
      const track = new FakeTrack("video");
      cameraTracks.push({ track, besideOpen: cameraTracks.some((c) => !c.track.stopped) });
      return Promise.resolve(track as unknown as MediaStreamTrack);
    },
    setOutput: (choice) => {
      outputs.push(choice === "default" ? null : choice.id);
      return Promise.resolve();
    },
    setVolume: (id, gain) => {
      volumes.push(`${id}=${String(gain)}`);
    },
    setInputVolume: (gain) => {
      inputVolumes.push(`set=${String(gain)}`);
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
    stop: (id) => {
      stopped.push(id);
    },
  };
  return {
    media,
    connections,
    played,
    stopped,
    screens,
    microphones,
    microphoneTracks,
    setup,
    outputs,
    volumes,
    inputVolumes,
    cameras,
    cameraTracks,
    cameraFailures,
  };
}

function makeCall(options: {
  candidates: (keyof typeof servers)[];
  latency: Record<string, number>;
  existing?: boolean;
  unreachableConnections?: number;
  microphone?: "ok" | "denied";
  userVolume?: (userId: string, source: "microphone" | "screenAudio") => number;
  /** The clock the call reads; the real one by default. */
  now?: () => number;
  /** What the join offer grants; all of them by default. */
  speak?: boolean;
  shareScreen?: boolean;
  useCamera?: boolean;
}) {
  FakeSocket.instances = [];
  FakeSocket.unreachable = new Set();
  FakeSocket.secrets = 0;
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
          transferFiles: false,
          useCamera: options.useCamera ?? true,
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
  const fake = fakeMedia(options.unreachableConnections ?? 0, options.microphone ?? "ok");
  const call = new VoiceCall({
    client,
    media: fake.media,
    fetch,
    WebSocket: FakeSocket as unknown as typeof WebSocket,
    setTimeout: ((fn: () => void, delay: number) => {
      timers.push({ fn, delay });
      return 0;
    }) as unknown as typeof setTimeout,
    random: () => 0.5,
    ...(options.now === undefined ? {} : { now: options.now }),
    ...(options.userVolume === undefined ? {} : { userVolume: options.userVolume }),
  });
  /** The connection of the call now, and the transceiver sending `source` on it. */
  const connection = () => fake.connections.at(-1);
  const sendingOn = (source: MediaSource, socket = FakeSocket.instances.at(-1)) => {
    const section = socket?.sections().find((s) => s.carries === "sending" && s.source === source);
    return section === undefined ? undefined : connection()?.transceiver(section.mid);
  };
  return { call, calls, timers, connection, sendingOn, ...fake };
}

/** The types of the frames a socket sent, leaving out answers, which follow every offer. */
function requests(socket: FakeSocket | undefined): string[] {
  return (socket?.sent ?? []).filter((f) => f.type !== "answer").map((f) => f.type);
}

describe("VoiceCall", () => {
  it("orders candidates by latency, joins the nearest, and sends the microphone", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, connection, sendingOn, microphoneTracks } = makeCall({
      candidates: ["far", "near"],
      latency: { far: 80, near: 5 },
    });
    const states: string[] = [];
    call.subscribe(() => states.push(call.state.status));
    await call.join(channel);
    await settle();
    expect(call.state).toMatchObject({ status: "connected", channelId: channel, session });
    expect(calls.filter((c) => c.includes("/health")).length).toBe(2);
    expect(FakeSocket.instances.map((s) => new URL(s.url).host)).toEqual(["near.example.org"]);
    const socket = FakeSocket.instances[0];
    expect(requests(socket)).toEqual(["identify", "send", "setState"]);
    expect(socket?.sent.find((f) => f.type === "send")).toEqual({
      type: "send",
      source: "microphone",
    });
    // Each offer is answered with its own revision, in order.
    expect(socket?.answers().map((a) => a.revision)).toEqual([1, 2]);
    expect(connection()?.offers).toEqual(["mids:0", "mids:0,1"]);
    // The section the server offers for the microphone sends it.
    const microphone = sendingOn("microphone");
    expect(microphone?.direction).toBe("sendonly");
    expect(microphone?.sender.track).toBe(microphoneTracks[0]);
    expect(states).toContain("joining");
    expect(calls.some((c) => c.includes("/failures"))).toBe(false);
  });

  it("joins to listen without opening the microphone when the channel does not allow speaking", async () => {
    FakeSocket.behaviour = new Map();
    const { call } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      speak: false,
      shareScreen: false,
      microphone: "denied",
    });
    await call.join(channel);
    expect(call.state).toMatchObject({ status: "connected", canSpeak: false, canShare: false });
    expect(FakeSocket.instances[0]?.sent.some((f) => f.type === "send")).toBe(false);
    const prepared = { video: new FakeTrack("video"), audio: null };
    await call.startScreenShare({
      prepared: prepared as unknown as { video: MediaStreamTrack; audio: null },
    });
    expect(prepared.video.stopped).toBe(true);
    expect(call.state.sharingScreen).toBe(false);
  });

  it("reports a server that refuses and falls through to the next", async () => {
    FakeSocket.behaviour = new Map([["dead", "refuse"]]);
    const { call, calls, connections } = makeCall({
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
    expect(connections[0]?.closed).toBe(true);
  });

  it("treats media that never connects as the server's failure and tries the next", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, connections } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 30 },
      unreachableConnections: 1,
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
    expect(connections[0]?.closed).toBe(true);
    expect(connections[1]?.connectionState).toBe("connected");
  });

  it("asks for ICE restarts while media is lost mid-call, rejoining when it does not come back", async () => {
    FakeSocket.behaviour = new Map();
    let time = 0;
    const { call, connection, timers } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      now: () => time,
    });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const restarts = () => socket?.sent.filter((f) => f.type === "restartIce").length;
    // Failed: a restart at once, and another each interval while it stays lost.
    connection()?.setState("failed");
    expect(socket?.sent.at(-1)).toEqual({ type: "restartIce", connection: "call" });
    expect(call.state.status).toBe("connected");
    connection()?.setState("failed");
    expect(restarts()).toBe(1);
    expect(timers.at(-1)?.delay).toBe(ICE_RESTART_INTERVAL_MS);
    time += ICE_RESTART_INTERVAL_MS;
    timers.at(-1)?.fn();
    expect(restarts()).toBe(2);
    // Connected again, the next interval asks for nothing.
    connection()?.setState("connected");
    time += ICE_RESTART_INTERVAL_MS;
    timers.at(-1)?.fn();
    expect(restarts()).toBe(2);
    // Disconnected first waits an interval, in which it may come back alone.
    connection()?.setState("disconnected");
    expect(restarts()).toBe(2);
    time += ICE_RESTART_INTERVAL_MS;
    timers.at(-1)?.fn();
    expect(restarts()).toBe(3);
    // Lost for the whole limit, the call is rejoined from scratch.
    time += ICE_RECOVERY_LIMIT_MS;
    timers.at(-1)?.fn();
    expect(call.state.status).toBe("rejoining");
    expect(timers.at(-1)?.delay).toBe(REJOIN_DELAY_MAX_MS / 2);
  });

  it("watches what is already shown after rejoining", async () => {
    FakeSocket.behaviour = new Map();
    const { call, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    call.showVideo();
    call.onSessionEnded({ id: session, channel, reason: "serverLost" });
    timers.at(-1)?.fn();
    await settle();
    expect(call.state.status).toBe("connected");
    // The rejoined call's `ready` names a camera that is already on, and no `sources` follows.
    const again = FakeSocket.instances[1];
    expect(again?.sent.filter((f) => f.type === "watch")).toEqual([]);
    call.leave();
    const camera = [{ source: "camera", paused: false }];
    FakeSocket.readyParticipants = [
      { user: "u2", muted: false, deafened: false, speaking: false, sources: camera },
    ];
    await call.join(channel);
    call.showVideo();
    await settle();
    call.onSessionEnded({ id: session, channel, reason: "serverLost" });
    timers.at(-1)?.fn();
    await settle();
    expect(FakeSocket.instances.at(-1)?.sent.filter((f) => f.type === "watch")).toEqual([
      { type: "watch", video: [{ user: "u2", source: "camera" }] },
    ]);
    expect(call.state.cameras.map((c) => c.user)).toEqual(["u2"]);
    FakeSocket.readyParticipants = [];
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

  it("fails a join this device cannot set up, blaming and trying no server for it", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, setup } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 30 },
    });
    setup.failure = new Error("WebRTC is not supported");
    await expect(call.join(channel)).rejects.toBeInstanceOf(CallSetupError);
    expect(call.state).toMatchObject({
      status: "failed",
      channelId: channel,
      errorKind: "device",
      error: "WebRTC is not supported",
    });
    // The browser would do no better for another server.
    expect(FakeSocket.instances).toHaveLength(0);
    expect(calls.some((c) => c.includes("/failures"))).toBe(false);
  });

  it("fails a join whose offer this device cannot apply, blaming and trying no server for it", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, media } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 30 },
    });
    const create = media.createConnection.bind(media);
    media.createConnection = async () => {
      const connection = (await create()) as FakeConnection;
      connection.failure = new Error("no codec in common");
      return connection;
    };
    await expect(call.join(channel)).rejects.toBeInstanceOf(CallSetupError);
    expect(call.state).toMatchObject({ errorKind: "device", error: "no codec in common" });
    expect(FakeSocket.instances).toHaveLength(1);
    expect(calls.some((c) => c.includes("/failures"))).toBe(false);
  });

  it("treats being turned away at identify for coming too fast as a refusal, blaming no server", async () => {
    FakeSocket.behaviour = new Map([["near", "throttledIdentify"]]);
    const { call, calls } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 30 },
    });
    await expect(call.join(channel)).rejects.toThrow("too many identify frames");
    expect(call.state).toMatchObject({ errorKind: "refused", retryAfterSeconds: 7 });
    // Another server would refuse the user no differently, and this one did nothing wrong.
    expect(FakeSocket.instances).toHaveLength(1);
    expect(calls.some((c) => c.includes("/failures"))).toBe(false);
  });

  it("gives up on a server that never answers identify and tries the next", async () => {
    FakeSocket.behaviour = new Map([["near", "silent"]]);
    const { call, calls, timers } = makeCall({
      candidates: ["near", "far"],
      latency: { near: 1, far: 30 },
    });
    const joined = call.join(channel);
    await vi.waitFor(() => {
      expect(FakeSocket.instances[0]?.sent.some((f) => f.type === "identify")).toBe(true);
    });
    for (const timer of timers.filter((t) => t.delay === READY_TIMEOUT_MS)) {
      timer.fn();
    }
    await joined;
    expect(call.state.status).toBe("connected");
    expect(FakeSocket.instances.map((s) => new URL(s.url).host)).toEqual([
      "near.example.org",
      "far.example.org",
    ]);
    expect(calls.filter((c) => c.includes("/failures"))).toEqual([
      `POST api.example.org/api/v1/voice-servers/${servers.near.id}/failures`,
    ]);
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

  it("never tries or blames a server not reached over HTTPS, and fails when no other is", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls } = makeCall({
      candidates: ["plain", "near"],
      latency: { plain: 1, near: 2 },
    });
    await call.join(channel);
    expect(call.state.status).toBe("connected");
    expect(FakeSocket.instances.map((s) => new URL(s.url).host)).toEqual(["near.example.org"]);
    expect(calls.filter((c) => c.includes("plain.example.org") || c.includes("/failures"))).toEqual(
      [],
    );
    call.leave();
    const { call: alone } = makeCall({ candidates: ["plain"], latency: { plain: 1 } });
    await expect(alone.join(channel)).rejects.toThrow(InsecureAddressError);
    expect(FakeSocket.instances).toEqual([]);
  });

  it("plays each audio slot and share's sound, attributing slots as the server says", async () => {
    FakeSocket.behaviour = new Map();
    const { call, played, stopped, volumes, connection } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      existing: true,
      userVolume: (user, source) => (user === "u2" && source === "microphone" ? 1.5 : 1),
    });
    await call.join(channel);
    await settle();
    const socket = FakeSocket.instances[0];
    // The slot every connection starts with plays from the first, silent until it carries someone.
    expect(played).toEqual(["0"]);
    expect(volumes).toEqual(["0=1"]);
    socket?.frame({ type: "slots", assignments: [{ slot: 0, user: "u2" }] });
    await settle();
    expect(volumes.at(-1)).toBe("0=1.5");
    // A second slot and a share's sound, offered as the call grows.
    socket?.sending.push({ mid: "s1", kind: "audio", carries: "audioSlot", slot: 1 });
    socket?.watched.push({ mid: "s2", kind: "audio", carries: "shareAudio", user: "u3" });
    socket?.offer();
    await settle();
    expect(played).toEqual(["0", "s1", "s2"]);
    expect(connection()?.transceiver("s2")).toBeDefined();
    // The share's sound goes when its section does.
    socket?.watched.splice(0);
    socket?.offer();
    await settle();
    expect(stopped).toEqual(["s2"]);
    call.setMuted(true);
    expect(socket?.sent.at(-1)).toEqual({
      type: "setState",
      muted: true,
      deafened: false,
      held: false,
    });
    call.leave();
    expect(call.state.status).toBe("idle");
    expect(socket?.closed).toBe(true);
    expect(connection()?.closed).toBe(true);
    expect(stopped).toEqual(["s2", "0", "s1"]);
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
    await settle();
    expect(call.state.status).toBe("connected");
    expect(FakeSocket.instances).toHaveLength(2);
    call.onSessionEnded({ id: session, channel, reason: "idle" });
    expect(call.state.status).toBe("idle");
    expect(call.state.endedReason).toBe("idle");
    call.acknowledgeEnd();
    expect(call.state.endedReason).toBeNull();
    expect(REJOIN_DELAY_MAX_MS).toBe(1000);
  });

  it("sends the camera when turned on, watches others' video only while it is shown, and stops", async () => {
    FakeSocket.behaviour = new Map();
    const { call, cameras, cameraTracks, sendingOn } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    expect(call.state.canCamera).toBe(true);
    await call.startCamera();
    await settle();
    expect(cameras).toEqual([null]);
    expect(call.state.localCamera).not.toBeNull();
    expect(socket?.sent.filter((f) => f.type === "send").map((f) => f.source)).toEqual([
      "microphone",
      "camera",
    ]);
    // The camera's sender is bounded once its section is answered.
    const camera = sendingOn("camera");
    expect(camera?.sender.track).toBe(cameraTracks[0]?.track);
    expect(camera?.encodings).toEqual([{ maxBitrate: 4_000_000, maxFramerate: 30 }]);
    // A second start while it is on opens nothing.
    await call.startCamera();
    expect(cameras).toHaveLength(1);
    // Another participant's camera is watched only while something shows video.
    socket?.frame({ type: "sources", user: "u2", sources: [{ source: "camera", paused: false }] });
    await settle();
    expect(socket?.sent.some((f) => f.type === "watch")).toBe(false);
    const hide = call.showVideo();
    await settle();
    expect(socket?.sent.filter((f) => f.type === "watch").at(-1)).toEqual({
      type: "watch",
      video: [{ user: "u2", source: "camera" }],
    });
    // It is theirs to show, and never a shared screen.
    expect(call.state.cameras.map((c) => c.user)).toEqual(["u2"]);
    expect(call.state.screens).toEqual([]);
    hide();
    await settle();
    expect(socket?.sent.filter((f) => f.type === "watch").at(-1)).toEqual({
      type: "watch",
      video: [],
    });
    expect(call.state.cameras).toEqual([]);
    call.stopCamera();
    expect(call.state.localCamera).toBeNull();
    expect(socket?.sent.at(-1)).toEqual({ type: "stopSending", source: "camera" });
    expect(camera?.replaced.at(-1)).toBeNull();
  });

  it("keeps the camera on as it moves to another, letting the one in use go first", async () => {
    FakeSocket.behaviour = new Map();
    const { call, cameras, cameraTracks, cameraFailures, sendingOn } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    await call.join(channel);
    await call.startCamera();
    await settle();
    const front = cameraTracks[0]?.track;
    await call.setAudioDevices({
      input: "default",
      output: "default",
      camera: { id: "back", label: "Back Camera" },
    });
    expect(cameras).toEqual([null, "back"]);
    const back = cameraTracks[1]?.track;
    expect(front?.stopped).toBe(true);
    expect(cameraTracks.map((c) => c.besideOpen)).toEqual([false, false]);
    expect(call.state.localCamera).toEqual({ kind: "track", track: back });
    // The same section sends the new camera, with nothing renegotiated.
    expect(sendingOn("camera")?.sender.track).toBe(back);
    expect(FakeSocket.instances[0]?.sent.filter((f) => f.type === "stopSending")).toEqual([]);
    // The camera it moved from ending, as iOS ends it, leaves the camera on; the one in use
    // ending turns it off.
    front?.end();
    expect(call.state.localCamera).toEqual({ kind: "track", track: back });
    back?.end();
    expect(call.state.localCamera).toBeNull();
    // With no camera to move to, the camera turns off and says why.
    await call.startCamera();
    cameraFailures.push("failed");
    await call.setAudioDevices({ input: "default", output: "default", camera: "default" });
    expect(call.state.localCamera).toBeNull();
    expect(call.state.cameraError).toBe("failed");
  });

  it("moves the camera to the latest choice when moves overlap", async () => {
    FakeSocket.behaviour = new Map();
    const { call, cameraTracks } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    await call.startCamera();
    const devices = { input: "default", output: "default" } as const;
    await Promise.all([
      call.setAudioDevices({ ...devices, camera: { id: "back", label: "Back Camera" } }),
      call.setAudioDevices({ ...devices, camera: { id: "front", label: "Front Camera" } }),
    ]);
    const latest = cameraTracks.at(-1)?.track;
    expect(call.state.localCamera).toEqual({ kind: "track", track: latest });
    expect(latest?.stopped).toBe(false);
    expect(cameraTracks.filter((c) => !c.track.stopped)).toHaveLength(1);
  });

  it("keeps why the camera failed until it turns on, is dismissed, or the call ends", async () => {
    FakeSocket.behaviour = new Map();
    const { call, cameraFailures } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    cameraFailures.push("none");
    await expect(call.startCamera()).rejects.toMatchObject({ failure: "none" });
    expect(call.state.cameraError).toBe("none");
    expect(call.state.localCamera).toBeNull();
    call.clearCameraError();
    expect(call.state.cameraError).toBeNull();
    cameraFailures.push("denied");
    await expect(call.startCamera()).rejects.toBeInstanceOf(CameraError);
    expect(call.state.cameraError).toBe("denied");
    await call.startCamera();
    expect(call.state.cameraError).toBeNull();
    expect(call.state.localCamera).not.toBeNull();
    call.stopCamera();
    cameraFailures.push("failed");
    await expect(call.startCamera()).rejects.toMatchObject({ failure: "failed" });
    call.leave();
    expect(call.state.cameraError).toBeNull();
  });

  it("turns the camera off as unsent when the voice server refuses to receive it", async () => {
    FakeSocket.behaviour = new Map();
    const { call, cameraTracks } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    await settle();
    FakeSocket.behaviour.set("near", "throttledSend");
    await call.startCamera();
    await settle();
    expect(call.state).toMatchObject({ localCamera: null, cameraError: "unsent" });
    expect(cameraTracks[0]?.track.stopped).toBe(true);
  });

  it("offers no camera where the channel does not allow one", async () => {
    FakeSocket.behaviour = new Map();
    const { call, cameras } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      useCamera: false,
    });
    await call.join(channel);
    expect(call.state.canCamera).toBe(false);
    await call.startCamera();
    expect(cameras).toEqual([]);
  });

  it("shares a screen with its sound, shows others' screens, and stops when the capture ends", async () => {
    FakeSocket.behaviour = new Map();
    const { call, screens, played, sendingOn } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    await call.startScreenShare();
    await settle();
    expect(call.state.sharingScreen).toBe(true);
    expect(call.state.localScreen).toEqual({ kind: "track", track: screens[0]?.video });
    expect(socket?.sent.filter((f) => f.type === "send")).toEqual([
      { type: "send", source: "microphone" },
      { type: "send", source: "screen" },
      { type: "send", source: "screenAudio" },
    ]);
    // The picture may spend up to its ceiling; the server's offer says the rest (a high start,
    // the sound in stereo at a music bitrate), and the sound's sender is left as it is.
    expect(sendingOn("screen")?.encodings).toEqual([{ maxBitrate: 25_000_000, maxFramerate: 60 }]);
    expect(sendingOn("screen")?.sender.track).toBe(screens[0]?.video);
    expect(sendingOn("screenAudio")?.encodings).toEqual([{}]);
    expect(sendingOn("screenAudio")?.sender.track).toBe(screens[0]?.audio);
    expect(screens[0]?.video.contentHint).toBe("");
    // a second start while sharing is a no-op
    await call.startScreenShare();
    expect(screens).toHaveLength(1);
    // another participant's screen, watched while shown, is shown rather than played
    call.showVideo();
    socket?.frame({ type: "sources", user: "u2", sources: [{ source: "screen", paused: false }] });
    await settle();
    expect(call.state.screens.map((s) => s.user)).toEqual(["u2"]);
    expect(played).toEqual(["0"]);
    socket?.frame({ type: "participantLeft", user: "u2" });
    await settle();
    expect(call.state.screens).toEqual([]);
    // the user stops the capture from the browser's own control
    screens[0]?.video.end();
    expect(call.state.sharingScreen).toBe(false);
    expect(call.state.localScreen).toBeNull();
    expect(socket?.sent.filter((f) => f.type === "stopSending").map((f) => f.source)).toEqual([
      "screen",
      "screenAudio",
    ]);
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
    expect(call.state.localScreen).toEqual({ kind: "track", track: prepared.video });
    const sends = FakeSocket.instances[0]?.sent.filter((f) => f.type === "send");
    expect(sends?.map((f) => f.source)).toEqual(["microphone", "screen"]);
    call.stopScreenShare();
    expect(prepared.video.stopped).toBe(true);
  });

  it("shares the browser's picture with sound from outside the browser in its place", async () => {
    FakeSocket.behaviour = new Map();
    const { call, screens } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const audio = fakeSender(null);
    await call.startScreenShare({ audio: audio.sender, contentHint: "motion" });
    expect(screens[0]?.video.contentHint).toBe("motion");
    // the picture is the browser's; its own sound is dropped for the external one, which holds
    // the capture connection and answers its offer
    expect(socket?.sent.filter((f) => f.type === "send")).toEqual([
      { type: "send", source: "microphone" },
      { type: "send", source: "screen" },
      { type: "send", source: "screenAudio", external: true },
    ]);
    expect(screens[0]?.audio.stopped).toBe(true);
    expect(audio.offers.map((o) => o.sections.map((s) => s.carries))).toEqual([["sending"]]);
    expect(socket?.answers("capture")).toEqual([
      { type: "answer", connection: "capture", revision: 1, sdp: "capture answer 1" },
    ]);
    expect(call.state).toMatchObject({
      sharingScreen: true,
      localScreen: { kind: "track", track: screens[0]?.video },
    });
    // an external share cannot start alongside it
    const external = fakeSender(false);
    await call.startExternalScreenShare(external.sender as never);
    expect(external.offers).toEqual([]);
    // stopping ends both the capture and the external sound
    call.stopScreenShare();
    expect(audio.stopped()).toBe(1);
    expect(socket?.sent.filter((f) => f.type === "stopSending").map((f) => f.source)).toEqual([
      "screen",
      "screenAudio",
    ]);
    expect(call.state).toMatchObject({ sharingScreen: false, localScreen: null });
    // sound that cannot start ends the share it was part of
    const failing = fakeSender(null, new Error("no route"));
    await expect(call.startScreenShare({ audio: failing.sender })).rejects.toThrow("no route");
    expect(call.state.sharingScreen).toBe(false);
    expect(screens[1]?.video.stopped).toBe(true);
  });

  it("shares an external sender on the capture connection and previews it from the server", async () => {
    FakeSocket.behaviour = new Map();
    const { call, played } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const share = fakeSender(true);
    call.showVideo();
    await call.startExternalScreenShare(share.sender as never);
    await settle();
    expect(socket?.sent.filter((f) => f.type === "send").slice(-2)).toEqual([
      { type: "send", source: "screen", external: true },
      { type: "send", source: "screenAudio", external: true },
    ]);
    // the sender answers the capture connection's offers, its first naming the picture, its
    // second the sound too
    expect(
      share.offers.map((o) => o.sections.map((s) => s.carries === "sending" && s.source)),
    ).toEqual([["screen"], ["screen", "screenAudio"]]);
    expect(socket?.answers("capture").map((a) => a.revision)).toEqual([1, 2]);
    // the preview is the call's own share, watched back from the server, and is no one's screen
    expect(socket?.sent.filter((f) => f.type === "watch").at(-1)).toEqual({
      type: "watch",
      video: [{ user: me, source: "screen" }],
    });
    expect(call.state.sharingScreen).toBe(true);
    expect(call.state.localScreen).not.toBeNull();
    expect(call.state.screens).toEqual([]);
    expect(played).toEqual(["0"]);
    // a second external share while one runs is a no-op
    await call.startExternalScreenShare(share.sender as never);
    expect(share.offers).toHaveLength(2);
    // the server ending the share (its capture connection failed) ends it here too
    socket?.frame({ type: "sources", user: me, sources: [{ source: "screen", paused: false }] });
    socket?.frame({ type: "sources", user: me, sources: [] });
    await settle();
    expect(share.stopped()).toBe(1);
    expect(call.state).toMatchObject({ sharingScreen: false, localScreen: null });
    expect(socket?.sent.filter((f) => f.type === "watch").at(-1)).toEqual({
      type: "watch",
      video: [],
    });
    // a share whose sender cannot start is stopped, and its start rejects
    const failing = fakeSender(false, new Error("no route"));
    await expect(call.startExternalScreenShare(failing.sender as never)).rejects.toThrow(
      "no route",
    );
    expect(call.state.sharingScreen).toBe(false);
    expect(failing.stopped()).toBe(1);
    expect(socket?.sent.slice(-2)).toEqual([
      { type: "stopSending", source: "screen" },
      { type: "watch", video: [] },
    ]);
    // a share whose capture connection fails before its media arrives ends when the server
    // says it did not take what was asked
    const unreached = fakeSender(false);
    await call.startExternalScreenShare(unreached.sender as never);
    expect(call.state.sharingScreen).toBe(true);
    socket?.frame({
      type: "error",
      detail: "the game capture's connection to the voice server failed",
      fatal: false,
      refused: "send",
    });
    await settle();
    expect(call.state.sharingScreen).toBe(false);
    expect(unreached.stopped()).toBe(1);
  });

  it("takes a moderator's mute as its own state and a kick as a reason to tell the user", async () => {
    FakeSocket.behaviour = new Map();
    const { call, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    socket?.frame({ type: "participantState", user: me, muted: true, deafened: false });
    await settle();
    expect(call.state.muted).toBe(true);
    // The community's mute, as its event tells, until a moderator lifts it.
    expect(call.state.serverMuted).toBe(false);
    call.setServerMuted(true);
    expect(call.state.serverMuted).toBe(true);
    call.setServerMuted(false);
    expect(call.state.serverMuted).toBe(false);
    socket?.frame({ type: "participantState", user: "someone-else", muted: false, deafened: true });
    await settle();
    expect(call.state).toMatchObject({ muted: true, deafened: false });
    socket?.frame({ type: "kicked", reason: "replaced" });
    await settle();
    expect(call.state).toMatchObject({ status: "idle", endedReason: null });
    await call.join(channel);
    FakeSocket.instances[1]?.frame({ type: "kicked", reason: "kicked" });
    await settle();
    expect(call.state).toMatchObject({ status: "idle", endedReason: "kicked" });
    call.acknowledgeEnd();
    // A server going down, or one that never had this client's media, is rejoined.
    for (const reason of ["serverStopping", "connectionFailed"]) {
      await call.join(channel);
      FakeSocket.instances.at(-1)?.frame({ type: "kicked", reason });
      await settle();
      expect(call.state.status).toBe("rejoining");
      expect(timers.at(-1)?.delay).toBe(REJOIN_DELAY_MAX_MS / 2);
      call.leave();
    }
  });

  it("holds at once beside the user's mute, and shows who else is on hold", async () => {
    FakeSocket.behaviour = new Map();
    FakeSocket.readyParticipants = [
      { user: "on-hold", muted: false, deafened: false, held: true, speaking: false, sources: [] },
      { user: "talking", muted: false, deafened: false, speaking: false, sources: [] },
    ];
    const { call, microphoneTracks } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    try {
      await call.join(channel);
      const socket = FakeSocket.instances[0];
      const track = microphoneTracks[0];
      expect(call.state.onHold).toEqual(["on-hold"]);
      call.setMuted(true);
      call.setHeld(true);
      expect(call.state).toMatchObject({ muted: true, held: true });
      expect(track?.enabled).toBe(false);
      expect(socket?.sent.at(-1)).toEqual({
        type: "setState",
        muted: true,
        deafened: false,
        held: true,
      });
      // The hold ends once the server says so, and the user's own mute stays.
      call.setHeld(false);
      expect(call.state.held).toBe(true);
      socket?.frame({
        type: "participantState",
        user: me,
        muted: true,
        deafened: false,
        held: false,
      });
      await settle();
      expect(call.state).toMatchObject({ muted: true, held: false });
      expect(track?.enabled).toBe(false);
      // Others put on hold and taken off it, and one leaving.
      socket?.frame({
        type: "participantState",
        user: "talking",
        muted: false,
        deafened: false,
        held: true,
      });
      socket?.frame({
        type: "participantState",
        user: "on-hold",
        muted: false,
        deafened: false,
        held: false,
      });
      await settle();
      expect(call.state.onHold).toEqual(["talking"]);
      socket?.frame({ type: "participantLeft", user: "talking" });
      await settle();
      expect(call.state.onHold).toEqual([]);
    } finally {
      FakeSocket.readyParticipants = [];
    }
  });

  it("mutes at once but shows itself unmuted only once the server says so", async () => {
    FakeSocket.behaviour = new Map();
    const { call, microphoneTracks } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    const socket = FakeSocket.instances[0];
    const track = microphoneTracks[0];
    expect(track?.enabled).toBe(true);
    call.setMuted(true);
    expect(call.state.muted).toBe(true);
    expect(track?.enabled).toBe(false);
    expect(socket?.sent.at(-1)).toEqual({
      type: "setState",
      muted: true,
      deafened: false,
      held: false,
    });
    // Unmuting waits for the server's word, and a refusal leaves the user muted and told why.
    call.setMuted(false);
    expect(socket?.sent.at(-1)).toEqual({
      type: "setState",
      muted: false,
      deafened: false,
      held: false,
    });
    expect(call.state.muted).toBe(true);
    expect(track?.enabled).toBe(false);
    socket?.frame({
      type: "error",
      detail: "too many setState frames; try again in 4s",
      fatal: false,
      retryAfterSeconds: 4,
      refused: "setState",
    });
    await settle();
    expect(call.state).toMatchObject({ muted: true, stateRefused: { retryAfterSeconds: 4 } });
    // Deafening meanwhile does not ask again for the refused unmute.
    call.setDeafened(true);
    expect(socket?.sent.at(-1)).toEqual({
      type: "setState",
      muted: true,
      deafened: true,
      held: false,
    });
    expect(call.state).toMatchObject({ deafened: true, stateRefused: null });
    call.setMuted(false);
    // The answer to the deafen, sent before the unmute, keeps the user muted.
    socket?.frame({ type: "participantState", user: me, muted: true, deafened: true });
    await settle();
    expect(call.state.muted).toBe(true);
    socket?.frame({ type: "participantState", user: me, muted: false, deafened: true });
    await settle();
    expect(call.state).toMatchObject({ muted: false, deafened: true });
    expect(track?.enabled).toBe(true);
    // A late answer to an earlier unmute never shows the user audible after they muted.
    call.setMuted(true);
    socket?.frame({ type: "participantState", user: me, muted: false, deafened: true });
    await settle();
    expect(call.state.muted).toBe(true);
    expect(track?.enabled).toBe(false);
  });

  it("captures the preferred microphone, swaps it mid-call, and moves playback to the preferred speaker", async () => {
    FakeSocket.behaviour = new Map();
    const { call, microphones, microphoneTracks, outputs, sendingOn } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    const micA = { id: "mic-a", label: "A" };
    const micB = { id: "mic-b", label: "B" };
    const spkA = { id: "spk-a", label: "S" };
    await call.setAudioDevices({ input: micA, output: spkA });
    expect(outputs).toEqual(["spk-a"]);
    await call.join(channel);
    await settle();
    expect(microphones).toEqual(["mic-a"]);
    await call.setAudioDevices({ input: micB, output: spkA });
    expect(microphones).toEqual(["mic-a", "mic-b"]);
    // The section the microphone is sent on takes the new one, the old one closed.
    expect(sendingOn("microphone")?.sender.track).toBe(microphoneTracks[1]);
    expect(microphoneTracks[0]?.stopped).toBe(true);
    expect(outputs).toEqual(["spk-a"]);
    await call.setAudioDevices({ input: micB, output: "default" });
    expect(outputs).toEqual(["spk-a", null]);
    expect(microphones).toHaveLength(2);
    call.leave();
    await call.setAudioDevices({ input: { id: "mic-c", label: "C" }, output: "default" });
    expect(microphones).toEqual(["mic-a", "mic-b"]);
  });

  it("opens the microphone at the input volume and changes it on the open one", async () => {
    FakeSocket.behaviour = new Map();
    const { call, inputVolumes } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    expect(call.hasInputVolume).toBe(true);
    expect(call.hasAudioDeviceChoice).toBe(true);
    expect(call.playsOwnSounds).toBe(false);
    call.setInputVolume(1.5);
    await call.join(channel);
    call.setInputVolume(0.5);
    await call.setAudioDevices({ input: { id: "mic-b", label: "B" }, output: "default" });
    expect(inputVolumes).toEqual(["set=1.5", "open=1.5", "set=0.5", "open=0.5"]);
  });

  it("moves to another call keeping the microphone", async () => {
    FakeSocket.behaviour = new Map();
    const { call, connections, microphones, microphoneTracks, sendingOn } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    await call.join(channel);
    await call.join(otherChannel);
    await settle();
    expect(call.state).toMatchObject({ status: "connected", channelId: otherChannel });
    const [first] = FakeSocket.instances;
    expect(first?.sent.at(-1)?.type).toBe("leave");
    expect(first?.closed).toBe(true);
    expect(connections[0]?.closed).toBe(true);
    // One microphone, opened once and still open, now sent into the second call.
    expect(microphones).toHaveLength(1);
    expect(microphoneTracks[0]?.stopped).toBe(false);
    expect(sendingOn("microphone")?.sender.track).toBe(microphoneTracks[0]);
    call.leave();
    expect(microphoneTracks[0]?.stopped).toBe(true);
  });

  it("closes a microphone kept from the last call when the next one only lets them listen", async () => {
    FakeSocket.behaviour = new Map();
    const options = { candidates: ["near" as const], latency: { near: 1 }, speak: true };
    const { call, microphoneTracks } = makeCall(options);
    await call.join(channel);
    options.speak = false;
    await call.join(otherChannel);
    expect(call.state).toMatchObject({ status: "connected", canSpeak: false });
    expect(microphoneTracks[0]?.stopped).toBe(true);
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

  it("applies each user's volume to the slot that carries them, as it changes hands", async () => {
    FakeSocket.behaviour = new Map();
    const { call, volumes } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (userId) => (userId === "quiet" ? 1.5 : 1),
    });
    await call.join(channel);
    await settle();
    const socket = FakeSocket.instances[0];
    socket?.frame({ type: "slots", assignments: [{ slot: 0, user: "quiet" }] });
    await settle();
    expect(volumes).toEqual(["0=1", "0=1.5"]);
    call.setUserVolume("quiet", 0.25);
    expect(volumes.at(-1)).toBe("0=0.25");
    // The slot moving to someone else takes their volume, and the first's no longer reaches it.
    socket?.frame({ type: "slots", assignments: [{ slot: 0, user: "loud" }] });
    await settle();
    expect(volumes.at(-1)).toBe("0=1");
    const before = volumes.length;
    call.setUserVolume("quiet", 2);
    call.setUserVolume("nobody", 2);
    expect(volumes).toHaveLength(before);
  });

  it("sets a person's stream apart from their voice", async () => {
    FakeSocket.behaviour = new Map();
    const { call, volumes } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (_user, source) => (source === "screenAudio" ? 0.5 : 1.5),
    });
    await call.join(channel);
    await settle();
    const socket = FakeSocket.instances[0];
    socket?.frame({ type: "slots", assignments: [{ slot: 0, user: "sharer" }] });
    socket?.watched.push({ mid: "s9", kind: "audio", carries: "shareAudio", user: "sharer" });
    socket?.offer();
    await settle();
    expect(volumes).toEqual(["0=1", "0=1.5", "s9=0.5"]);
    // Their stream's volume leaves their voice alone, and theirs leaves the stream alone.
    call.setUserVolume("sharer", 0, "screenAudio");
    call.setUserVolume("sharer", 1.25);
    expect(volumes.slice(3)).toEqual(["s9=0", "0=1.25"]);
  });

  it("sets every received sound's gain again when who is silenced changes", async () => {
    FakeSocket.behaviour = new Map();
    let silenced = new Set<string>();
    const { call, volumes } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (userId) => (silenced.has(userId) ? 0 : 1),
    });
    await call.join(channel);
    await settle();
    const socket = FakeSocket.instances[0];
    socket?.sending.push({ mid: "s1", kind: "audio", carries: "audioSlot", slot: 1 });
    socket?.offer();
    socket?.frame({
      type: "slots",
      assignments: [
        { slot: 0, user: "blocked-elsewhere" },
        { slot: 1, user: "friend" },
      ],
    });
    await settle();
    silenced = new Set(["blocked-elsewhere"]);
    volumes.splice(0);
    call.refreshVolumes();
    expect(volumes).toEqual(["0=0", "s1=1"]);
  });

  it("tells the server whom it plays at nothing, as people come and volumes change", async () => {
    FakeSocket.behaviour = new Map();
    let blocked = new Set(["blocked"]);
    FakeSocket.readyParticipants = [
      { user: "blocked", muted: false, deafened: false, speaking: false, sources: [] },
      { user: "friend", muted: false, deafened: false, speaking: false, sources: [] },
    ];
    const { call } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (user) => (blocked.has(user) ? 0 : 1),
    });
    await call.join(channel);
    await settle();
    const socket = FakeSocket.instances[0];
    const silences = () => socket?.sent.filter((f) => f.type === "silence") ?? [];
    expect(silences()).toEqual([{ type: "silence", voices: ["blocked"], shares: ["blocked"] }]);
    // Someone already blocked who joins is silenced from the start.
    blocked.add("late");
    socket?.frame({ type: "participantJoined", user: "late", muted: false, deafened: false });
    await settle();
    expect(silences().at(-1)).toEqual({
      type: "silence",
      voices: ["blocked", "late"],
      shares: ["blocked", "late"],
    });
    // Turned all the way down for their voice alone, then back up.
    call.setUserVolume("friend", 0);
    expect(silences().at(-1)).toEqual({
      type: "silence",
      voices: ["blocked", "friend", "late"],
      shares: ["blocked", "late"],
    });
    call.setUserVolume("friend", 0.5);
    expect(silences().at(-1)?.voices).toEqual(["blocked", "late"]);
    // Unblocked, and nothing sent again for what has not changed.
    blocked = new Set();
    call.refreshVolumes();
    expect(silences().at(-1)).toEqual({ type: "silence", voices: [], shares: [] });
    const sent = silences().length;
    call.refreshVolumes();
    expect(silences()).toHaveLength(sent);
    FakeSocket.readyParticipants = [];
  });

  it("follows a section the server hands to someone else", async () => {
    FakeSocket.behaviour = new Map();
    const { call, volumes, played, connection } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
      userVolume: (user, source) => (user === "cat" && source === "screenAudio" ? 0.25 : 1),
    });
    await call.join(channel);
    call.showVideo();
    await settle();
    const socket = FakeSocket.instances[0];
    socket?.watched.push(
      { mid: "v1", kind: "video", carries: "video", user: "bob", source: "camera" },
      { mid: "a1", kind: "audio", carries: "shareAudio", user: "bob" },
    );
    socket?.offer();
    await settle();
    expect(call.state.cameras.map((c) => [c.user, c.id])).toEqual([["bob", "v1"]]);
    // The same sections, now Cat's: her camera, her share's sound at her share's volume.
    socket?.watched.splice(
      0,
      2,
      { mid: "v1", kind: "video", carries: "video", user: "cat", source: "camera" },
      { mid: "a1", kind: "audio", carries: "shareAudio", user: "cat" },
    );
    socket?.offer();
    await settle();
    expect(call.state.cameras.map((c) => [c.user, c.id])).toEqual([["cat", "v1"]]);
    const received = connection()?.transceiver("v1")?.receiver.track;
    expect(received).toBeDefined();
    expect(call.state.cameras[0]?.video).toEqual(received && trackSource(received));
    expect(volumes.at(-1)).toBe("a1=0.25");
    expect(played.filter((id) => id === "a1")).toHaveLength(1);
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
    expect(signallingUrl(servers.near)).toBe("wss://near.example.org/ws");
    vi.restoreAllMocks();
  });
});

describe("VoiceCall resuming", () => {
  it("takes its place back on a socket that closed, keeping its connection and media", async () => {
    FakeSocket.behaviour = new Map();
    const { call, connections, timers, played } = makeCall({
      candidates: ["near"],
      latency: { near: 1 },
    });
    await call.join(channel);
    await settle();
    const first = FakeSocket.instances[0];
    call.showVideo();
    call.setMuted(true);
    await settle();
    first?.drop();
    expect(call.state.status).toBe("connected");
    // Muting while away reaches the server once the call is back.
    call.setDeafened(true);
    timers.at(-1)?.fn();
    await settle();
    const second = FakeSocket.instances[1];
    expect(second?.url).toBe(first?.url);
    expect(second?.sent[0]).toEqual({ type: "resume", secret: "secret-1" });
    expect(requests(second)).toEqual(["resume", "watch", "setState"]);
    expect(second?.sent.filter((f) => f.type === "setState")).toEqual([
      { type: "setState", muted: true, deafened: true, held: false },
    ]);
    // The server restating an offer already answered is answered again, never applied twice.
    expect(connections).toHaveLength(1);
    expect(connections[0]?.offers).not.toContain("restated");
    expect(second?.answers()[0]?.revision).toBe(first?.answers().at(-1)?.revision);
    expect(played).toEqual(["0"]);
    expect(call.state.status).toBe("connected");
    // A second drop resumes with the secret the resume gave.
    second?.drop();
    timers.at(-1)?.fn();
    await settle();
    expect(FakeSocket.instances[2]?.sent[0]).toEqual({ type: "resume", secret: "secret-2" });
  });

  it("rejoins once the server refuses the secret", async () => {
    FakeSocket.behaviour = new Map();
    const { call, timers, calls } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    FakeSocket.behaviour.set("near", "refuse");
    FakeSocket.instances[0]?.drop();
    timers.at(-1)?.fn();
    await settle();
    expect(FakeSocket.instances).toHaveLength(2);
    expect(call.state.status).toBe("rejoining");
    FakeSocket.behaviour.set("near", "ready");
    timers.at(-1)?.fn();
    await settle();
    expect(call.state.status).toBe("connected");
    expect(calls.filter((c) => c.endsWith("/voice/join"))).toHaveLength(2);
  });

  it("tries again while the server cannot be reached, then rejoins", async () => {
    FakeSocket.behaviour = new Map();
    const { call, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    FakeSocket.unreachable.add("near");
    FakeSocket.instances[0]?.drop();
    for (let attempt = 0; attempt < RESUME_ATTEMPTS; attempt += 1) {
      timers.at(-1)?.fn();
      await settle();
      expect(call.state.status).toBe(attempt < RESUME_ATTEMPTS - 1 ? "connected" : "rejoining");
    }
    expect(FakeSocket.instances).toHaveLength(1 + RESUME_ATTEMPTS);
    expect(FakeSocket.instances.slice(1).every((s) => s.sent.length === 0)).toBe(true);
  });
});

describe("VoiceCall after its server goes silent", () => {
  const joins = (calls: string[]) => calls.filter((c) => c.endsWith("/voice/join")).length;

  it("asks for an offer once more after a minute when the offered server cannot be reached", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    expect(joins(calls)).toBe(1);
    // The socket closes without a word and will not resume; the API server, not yet knowing,
    // offers the same server.
    FakeSocket.behaviour.set("near", "refuse");
    FakeSocket.instances[0]?.drop();
    timers.at(-1)?.fn();
    await settle();
    expect(call.state.status).toBe("rejoining");
    timers.at(-1)?.fn();
    await settle();
    expect(joins(calls)).toBe(2);
    expect(call.state.status).toBe("rejoining");
    const wait = timers.at(-1);
    expect(wait?.delay).toBe(DEAD_SERVER_RETRY_MS);
    // By then the server has gone unreported for as long and the offer names a living one.
    FakeSocket.behaviour.set("near", "ready");
    wait?.fn();
    await settle();
    expect(joins(calls)).toBe(3);
    expect(call.state.status).toBe("connected");
    expect(DEAD_SERVER_RETRY_MS).toBe(60_000);
  });

  it("gives up at once when the second offer's server cannot be reached either, or the API server ended the call", async () => {
    FakeSocket.behaviour = new Map();
    const { call, calls, timers } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    FakeSocket.behaviour.set("near", "refuse");
    FakeSocket.instances[0]?.drop();
    timers.at(-1)?.fn();
    await settle();
    timers.at(-1)?.fn();
    await settle();
    expect(call.state.status).toBe("rejoining");
    timers.at(-1)?.fn();
    await settle();
    expect(call.state.status).toBe("failed");
    expect(call.state.errorKind).toBe("server");
    expect(joins(calls)).toBe(3);
    // A call the API server ended for a lost server is offered another at once; failing to
    // reach it is a failure.
    FakeSocket.behaviour.set("near", "ready");
    await call.join(channel);
    FakeSocket.behaviour.set("near", "refuse");
    call.onSessionEnded({ id: session, channel, reason: "serverLost" });
    timers.at(-1)?.fn();
    await settle();
    expect(call.state.status).toBe("failed");
  });

  it("ignores the end of a session that is not the call's own", async () => {
    FakeSocket.behaviour = new Map();
    const { call } = makeCall({ candidates: ["near"], latency: { near: 1 } });
    await call.join(channel);
    expect(call.state.session).toBe(session);
    call.onSessionEnded({
      id: "0190f0a0-0000-7000-8000-000000000901",
      channel,
      reason: "serverLost",
    });
    expect(call.state.status).toBe("connected");
    call.onSessionEnded({ id: session, channel, reason: "serverLost" });
    expect(call.state.status).toBe("rejoining");
  });
});
