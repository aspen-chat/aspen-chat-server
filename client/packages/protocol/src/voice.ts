/**
 * A voice call from the client's side.
 *
 * Joining asks the API server for an offer (a token and the candidate voice servers), measures
 * the latency to each candidate, and tries them nearest first: a signalling socket is opened,
 * the token presented, and if the server does not answer `ready` in time the failure is
 * reported to the API server and the next candidate is tried. Once in, the mediasoup device is
 * loaded from the router's capabilities, a send and a receive transport are created, the
 * microphone is produced, and every consumer the server announces is played.
 *
 * A call ended because its server was lost or removed is rejoined on its own after a random
 * delay of up to a second, so the participants of a lost call do not all hit the API server at
 * once. A call ended for being idle, or by the user, is not. The browser's media APIs and the
 * mediasoup device sit behind `VoiceMedia`, so the flow runs and is tested without them.
 */

import { DEFAULT_DEVICE, type DeviceChoice } from "./preferences";
import type { components } from "./generated/openapi";
import type { ClientMessage, ServerMessage } from "./generated/voiceSignal";
import type { VoiceSessionEndReason } from "./generated/events";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError } from "./problem";

type VoiceJoinOffer = components["schemas"]["VoiceJoinOffer"];
type VoiceServerCandidate = components["schemas"]["VoiceServerCandidate"];

/** How long the media transport has to connect once the server has accepted the call. */
export const CONNECT_TIMEOUT_MS = 15_000;
/** How long a candidate has to answer a latency ping. */
export const PING_TIMEOUT_MS = 3_000;
/** How long a candidate has to answer `identify` with `ready`. */
export const READY_TIMEOUT_MS = 6_000;

/**
 * How a screen share's own sound is encoded: stereo (a browser decodes Opus as mono unless the
 * producer says otherwise), without discontinuous transmission, which cuts quiet passages of
 * music, and at 128 kbps, where cymbals and other dense sound stay clean.
 */
export const SCREEN_AUDIO_CODEC = {
  opusStereo: true,
  opusDtx: false,
  opusMaxAverageBitrate: 128_000,
} as const;
/** The most a lost call waits before rejoining. */
export const REJOIN_DELAY_MAX_MS = 1_000;

export type VoiceCallStatus = "idle" | "joining" | "connected" | "rejoining" | "failed";

export interface VoiceCallState {
  readonly status: VoiceCallStatus;
  /** The channel being joined or in, `null` when idle. */
  readonly channelId: string | null;
  readonly session: string | null;
  readonly muted: boolean;
  readonly deafened: boolean;
  /** Whether this client is sending a screen, window, or game into the call. */
  sharingScreen: boolean;
  /** The video this client is sending, for a local preview; `null` when not sharing. */
  localScreen: MediaStreamTrack | null;
  /** The screens other participants are sharing, in the order they arrived. */
  screens: readonly RemoteScreen[];
  /**
   * What failed when `status` is `failed`: the microphone (permission refused, no device, or an
   * insecure page origin, which browsers refuse media on), or every voice server.
   */
  errorKind: "microphone" | "server" | null;
  /** Why the last attempt failed, for the UI; cleared on the next join. */
  readonly error: string | null;
  /**
   * Set when the server ended the call for being idle, until `acknowledgeEnd`, so the UI can
   * tell the user why they were dropped.
   */
  readonly endedReason: VoiceSessionEndReason | "kicked" | null;
}

/** A transport as mediasoup-client models it, narrowed to what the call needs. */
/** A screen another participant is sharing, as a playable video track. */
export interface RemoteScreen {
  readonly user: string;
  readonly consumerId: string;
  readonly track: MediaStreamTrack;
}

/** Where and how an external sender delivers SRTP for a producer the voice server made for it. */
export interface RtpTarget {
  readonly ip: string;
  readonly port: number;
  readonly ssrc: number;
  readonly payloadType: number;
  readonly srtpCryptoSuite: string;
  readonly srtpKeyBase64: string;
}

/** Where an external share sends: the picture, and the sound when the share carries any. */
export interface ExternalTargets {
  readonly video: RtpTarget;
  readonly audio: RtpTarget | null;
}

/**
 * A share produced outside the browser, such as the desktop shell's game capture: the call
 * asks the voice server for an RTP producer per stream, then `start` sends to them until
 * `stop`. `audio` says whether the share brings sound of its own, which needs a producer too.
 */
export interface ExternalShare {
  readonly audio: boolean;
  start(targets: ExternalTargets): Promise<void>;
  stop(): void;
}

/**
 * Sound for a browser screen share that comes from outside the browser, such as one
 * application's audio captured by the desktop shell: the call asks the voice server for an RTP
 * producer, then `start` sends to it until `stop`. It stands in for any sound the browser
 * captured with the picture.
 */
export interface ExternalAudio {
  start(target: RtpTarget): Promise<void>;
  stop(): void;
}

/** What `getDisplayMedia` gave: the picture, and the sound that came with it when the browser offered any. */
export interface ScreenCapture {
  readonly video: MediaStreamTrack;
  readonly audio: MediaStreamTrack | null;
}

export interface VoiceTransport {
  /** The ICE and DTLS state: `new`, `connecting`, `connected`, `disconnected`, `failed`, or `closed`. */
  readonly connectionState: string;
  on(event: "connectionstatechange", handler: (state: string) => void): unknown;
  on(
    event: "connect",
    handler: (
      params: { dtlsParameters: unknown },
      callback: () => void,
      errback: (error: Error) => void,
    ) => void,
  ): unknown;
  on(
    event: "produce",
    handler: (
      params: { kind: string; rtpParameters: unknown; appData: Record<string, unknown> },
      callback: (result: { id: string }) => void,
      errback: (error: Error) => void,
    ) => void,
  ): unknown;
  produce(options: {
    track: MediaStreamTrack;
    appData: Record<string, unknown>;
    codecOptions?: { opusStereo?: boolean; opusDtx?: boolean; opusMaxAverageBitrate?: number };
  }): Promise<{
    id: string;
    close(): void;
    replaceTrack(options: { track: MediaStreamTrack }): Promise<void>;
  }>;
  consume(options: {
    id: string;
    producerId: string;
    kind: "audio" | "video";
    rtpParameters: unknown;
  }): Promise<{ id: string; track: MediaStreamTrack; close(): void }>;
  close(): void;
}

/** A mediasoup-client device, narrowed to what the call needs. */
export interface VoiceDevice {
  load(options: { routerRtpCapabilities: unknown }): Promise<void>;
  readonly rtpCapabilities: unknown;
  createSendTransport(params: TransportParams): VoiceTransport;
  createRecvTransport(params: TransportParams): VoiceTransport;
}

export interface TransportParams {
  id: string;
  iceParameters: unknown;
  iceCandidates: unknown;
  dtlsParameters: unknown;
}

/** What the call needs from the browser: media capture, the mediasoup device, and playback. */
export interface VoiceMedia {
  createDevice(): Promise<VoiceDevice>;
  /** Opens the microphone the choice names, or the system's default. */
  getMicrophone(choice: DeviceChoice): Promise<MediaStreamTrack>;
  /** Routes everything played to the speaker the choice names, or the system's default. */
  setOutput(choice: DeviceChoice): Promise<void>;
  /** Asks the user for a screen, window, or tab to share; rejects when they decline. */
  getScreen(): Promise<ScreenCapture>;
  /** Plays a remote track; called once per consumer. */
  play(consumerId: string, track: MediaStreamTrack): void;
  stop(consumerId: string): void;
  /** Scales what a playing consumer is heard at: 1 is as sent, 0 silent, up to 2. */
  setVolume(consumerId: string, gain: number): void;
}

export interface VoiceCallOptions {
  client: AspenClient;
  media: VoiceMedia;
  WebSocket?: typeof globalThis.WebSocket;
  fetch?: typeof globalThis.fetch;
  setTimeout?: typeof globalThis.setTimeout;
  now?: () => number;
  /** Uniform in [0, 1); seeds the rejoin delay. */
  random?: () => number;
  /** How loud each other user should be to this one; consulted as their audio arrives. */
  userVolume?: (userId: string) => number;
}

export type VoiceCallListener = () => void;

/** A candidate with the round trip to it, `Infinity` when it did not answer. */
export interface RankedCandidate {
  candidate: VoiceServerCandidate;
  latencyMs: number;
}

/** Nearest first; candidates that did not answer come last, in their offered order. */
export function rankCandidates(ranked: readonly RankedCandidate[]): RankedCandidate[] {
  return [...ranked].sort((a, b) => a.latencyMs - b.latencyMs);
}

/** The signalling URL of a candidate: its `/ws` over the WebSocket scheme matching its own. */
export function signallingUrl(candidate: VoiceServerCandidate): string {
  const url = new URL(candidate.url);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = `${url.pathname.replace(/\/$/, "")}/ws`;
  return url.toString();
}

const IDLE: VoiceCallState = {
  status: "idle",
  channelId: null,
  session: null,
  muted: false,
  deafened: false,
  sharingScreen: false,
  localScreen: null,
  screens: [],
  errorKind: null,
  error: null,
  endedReason: null,
};

function sameChoice(a: DeviceChoice, b: DeviceChoice): boolean {
  if (a === DEFAULT_DEVICE || b === DEFAULT_DEVICE) {
    return a === b;
  }
  return a.id === b.id && a.label === b.label;
}

/** The microphone could not be opened; `cause` is the browser's error. */
export class MicrophoneError extends Error {
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : String(cause), { cause });
    this.name = "MicrophoneError";
  }
}

function failure(error: unknown): Pick<VoiceCallState, "errorKind" | "error"> {
  return {
    errorKind: error instanceof MicrophoneError ? "microphone" : "server",
    error: error instanceof Error ? error.message : String(error),
  };
}

/** One socket to a voice server with typed frames and waits for particular replies. */
class Signal {
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

export class VoiceCall {
  readonly #client: AspenClient;
  readonly #media: VoiceMedia;
  readonly #Socket: typeof globalThis.WebSocket;
  readonly #fetch: typeof globalThis.fetch;
  readonly #setTimeout: typeof globalThis.setTimeout;
  readonly #now: () => number;
  readonly #random: () => number;
  readonly #listeners = new Set<VoiceCallListener>();
  #state: VoiceCallState = IDLE;
  #signal: Signal | null = null;
  #sendTransport: VoiceTransport | null = null;
  #recvTransport: VoiceTransport | null = null;
  #microphone: MediaStreamTrack | null = null;
  #screen: ScreenCapture | null = null;
  #screenProducers: { id: string; close(): void }[] = [];
  /** The external sound of the browser screen share, and the RTP producer it feeds. */
  #screenAudio: { producerId: string; audio: ExternalAudio } | null = null;
  #microphoneProducer: {
    replaceTrack(options: { track: MediaStreamTrack }): Promise<void>;
  } | null = null;
  #devices: { input: DeviceChoice; output: DeviceChoice } = {
    input: DEFAULT_DEVICE,
    output: DEFAULT_DEVICE,
  };
  #external: { producerIds: string[]; share: ExternalShare } | null = null;
  /** The consumer carrying the call's own preview of an external share. */
  #previewConsumerId: string | null = null;
  readonly #consumers = new Map<string, { close(): void; user: string }>();
  readonly #userVolume: (userId: string) => number;
  /** Increments on every join and leave so a stale async step can notice and bail. */
  #generation = 0;

  constructor(options: VoiceCallOptions) {
    this.#client = options.client;
    this.#media = options.media;
    this.#Socket = options.WebSocket ?? globalThis.WebSocket;
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#setTimeout = options.setTimeout ?? globalThis.setTimeout.bind(globalThis);
    this.#userVolume = options.userVolume ?? (() => 1);
    this.#now = options.now ?? (() => Date.now());
    this.#random = options.random ?? Math.random;
  }

  get state(): VoiceCallState {
    return this.#state;
  }

  readonly subscribe = (listener: VoiceCallListener): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  #set(patch: Partial<VoiceCallState>): void {
    this.#state = { ...this.#state, ...patch };
    for (const listener of Array.from(this.#listeners)) {
      listener();
    }
  }

  /** Joins a channel's call, leaving any current one first. */
  async join(channelId: string): Promise<void> {
    // Already in, or on the way into, this very call: nothing to do.
    if (
      this.#state.channelId === channelId &&
      (this.#state.status === "connected" ||
        this.#state.status === "joining" ||
        this.#state.status === "rejoining")
    ) {
      return;
    }
    if (this.#state.channelId !== null) {
      this.#teardown();
    }
    const generation = ++this.#generation;
    this.#set({
      status: "joining",
      channelId,
      session: null,
      errorKind: null,
      error: null,
      endedReason: null,
    });
    try {
      await this.#connect(channelId, generation);
    } catch (error) {
      if (generation !== this.#generation) {
        return;
      }
      this.#teardown();
      this.#set({ ...IDLE, status: "failed", channelId, ...failure(error) });
      throw error;
    }
  }

  /** Leaves the call, if any. */
  leave(): void {
    if (this.#state.status === "idle") {
      return;
    }
    this.#generation += 1;
    this.#teardown();
    this.#set(IDLE);
  }

  /** Clears `endedReason` once the user has seen it. */
  acknowledgeEnd(): void {
    if (this.#state.endedReason !== null) {
      this.#set({ endedReason: null });
    }
  }

  setMuted(muted: boolean): void {
    this.#set({ muted });
    this.#sendState();
  }

  setDeafened(deafened: boolean): void {
    this.#set({ deafened });
    this.#sendState();
  }

  /**
   * Shares the screen: asks the browser for a capture and produces the picture as `screen` and
   * any audio the browser captured with it as `screenAudio`. Ends on its own when the user
   * stops the capture from the browser's own control. `prepared` shares a capture already
   * made instead of asking the browser. `audio` replaces the browser's sound with sound from
   * outside it, fed to an RTP producer of source `screenAudio`. `contentHint` tells the encoder
   * what the picture is: `motion` (a game or video) keeps the frame rate and gives up
   * resolution when bandwidth runs short, where the browser's default for a screen keeps the
   * resolution and drops frames.
   */
  async startScreenShare(
    options: {
      prepared?: ScreenCapture;
      audio?: ExternalAudio;
      contentHint?: "motion" | "detail" | "text";
    } = {},
  ): Promise<void> {
    const { prepared, audio, contentHint } = options;
    const transport = this.#sendTransport;
    const signal = this.#signal;
    if (
      this.#state.status !== "connected" ||
      transport === null ||
      signal === null ||
      this.#screen !== null ||
      this.#external !== null
    ) {
      prepared?.video.stop();
      prepared?.audio?.stop();
      return;
    }
    const capture = prepared ?? (await this.#media.getScreen());
    // The picker took time; the call may have moved on or another share started meanwhile.
    if (this.#sendTransport !== transport || this.#sharing()) {
      capture.video.stop();
      capture.audio?.stop();
      return;
    }
    // Sound from outside the browser stands in for the browser's own.
    if (audio !== undefined) {
      capture.audio?.stop();
    }
    if (contentHint !== undefined) {
      capture.video.contentHint = contentHint;
    }
    this.#screen = capture;
    this.#set({ sharingScreen: true, localScreen: capture.video });
    capture.video.addEventListener("ended", () => {
      if (this.#screen === capture) {
        this.stopScreenShare();
      }
    });
    try {
      this.#screenProducers.push(
        await transport.produce({ track: capture.video, appData: { source: "screen" } }),
      );
      if (audio !== undefined) {
        const produced = await this.#produceRtp(signal, "screenAudio");
        if (this.#screen !== capture) {
          signal.send({ type: "closeProducer", producerId: produced.producerId });
          return;
        }
        this.#screenAudio = { producerId: produced.producerId, audio };
        await audio.start(produced.target);
      } else if (capture.audio !== null) {
        this.#screenProducers.push(
          await transport.produce({
            track: capture.audio,
            appData: { source: "screenAudio" },
            codecOptions: SCREEN_AUDIO_CODEC,
          }),
        );
      }
    } catch (error) {
      this.stopScreenShare();
      throw error;
    }
  }

  /** Asks the voice server for a producer fed from outside the browser, and where to send it. */
  async #produceRtp(
    signal: Signal,
    source: "screen" | "screenAudio",
  ): Promise<{ producerId: string; target: RtpTarget }> {
    const produced = signal.next((f) => f.type === "rtpProduced" && f.source === source);
    signal.send({ type: "produceRtp", source });
    const timeout = new Promise<never>((_, reject) => {
      this.#setTimeout(() => {
        reject(new Error("the voice server did not answer in time"));
      }, READY_TIMEOUT_MS);
    });
    const frame = await Promise.race([produced, timeout]);
    if (frame.type !== "rtpProduced") {
      throw new Error("unexpected frame");
    }
    return {
      producerId: frame.producerId,
      target: {
        ip: frame.ip,
        port: frame.port,
        ssrc: frame.ssrc,
        payloadType: frame.payloadType,
        srtpCryptoSuite: frame.srtpCryptoSuite,
        srtpKeyBase64: frame.srtpKeyBase64,
      },
    };
  }

  /**
   * Shares something produced outside the browser: the voice server makes an RTP producer for
   * it and the share sends there. The preview comes back from the server as a consumer of the
   * call's own producer, so `localScreen` fills in once media flows.
   */
  async startExternalScreenShare(share: ExternalShare): Promise<void> {
    const signal = this.#signal;
    if (
      this.#state.status !== "connected" ||
      signal === null ||
      this.#screen !== null ||
      this.#external !== null
    ) {
      return;
    }
    const video = await this.#produceRtp(signal, "screen");
    const audio = share.audio ? await this.#produceRtp(signal, "screenAudio") : null;
    if (this.#signal !== signal) {
      return;
    }
    this.#external = {
      producerIds: [video.producerId, ...(audio === null ? [] : [audio.producerId])],
      share,
    };
    this.#set({ sharingScreen: true });
    try {
      await share.start({ video: video.target, audio: audio?.target ?? null });
    } catch (error) {
      this.stopScreenShare();
      throw error;
    }
  }

  /** Whether a browser capture is being shared; read after an await, where narrowing is stale. */
  #sharing(): boolean {
    return this.#screen !== null;
  }

  /** Stops sharing the screen, if sharing. Safe to call at any time. */
  stopScreenShare(): void {
    const external = this.#external;
    if (external !== null) {
      this.#external = null;
      external.share.stop();
      for (const producerId of external.producerIds) {
        this.#signal?.send({ type: "closeProducer", producerId });
      }
      this.#set({ sharingScreen: false, localScreen: null });
    }
    const capture = this.#screen;
    if (capture === null) {
      return;
    }
    this.#screen = null;
    const screenAudio = this.#screenAudio;
    if (screenAudio !== null) {
      this.#screenAudio = null;
      screenAudio.audio.stop();
      this.#signal?.send({ type: "closeProducer", producerId: screenAudio.producerId });
    }
    for (const producer of this.#screenProducers) {
      producer.close();
      this.#signal?.send({ type: "closeProducer", producerId: producer.id });
    }
    this.#screenProducers = [];
    capture.video.stop();
    capture.audio?.stop();
    this.#set({ sharingScreen: false, localScreen: null });
  }

  /**
   * The devices voice chat uses, from the user's preferences. Takes effect at once, in a call
   * or not: the microphone producer swaps to the new input and playback moves to the new
   * output.
   */
  async setAudioDevices(devices: { input: DeviceChoice; output: DeviceChoice }): Promise<void> {
    const previous = this.#devices;
    this.#devices = devices;
    if (!sameChoice(devices.output, previous.output)) {
      await this.#media.setOutput(devices.output);
    }
    if (!sameChoice(devices.input, previous.input) && this.#microphoneProducer !== null) {
      const producer = this.#microphoneProducer;
      const track = await this.#media.getMicrophone(devices.input);
      // The call may have ended or been rejoined while the microphone opened.
      const current: typeof producer | null = this.#microphoneProducer;
      if (current !== producer) {
        track.stop();
        return;
      }
      await producer.replaceTrack({ track });
      this.#microphone?.stop();
      this.#microphone = track;
    }
  }

  /** Sets how loud `userId` is heard right now; the preference behind it is the caller's to keep. */
  setUserVolume(userId: string, gain: number): void {
    for (const [consumerId, consumer] of this.#consumers) {
      if (consumer.user === userId) {
        this.#media.setVolume(consumerId, gain);
      }
    }
  }

  #sendState(): void {
    if (this.#signal !== null && this.#state.status === "connected") {
      this.#signal.send({
        type: "setState",
        muted: this.#state.muted,
        deafened: this.#state.deafened,
      });
    }
  }

  /**
   * Called by the sync layer with every `voiceSessionEnded` event. The call's own end by a lost
   * or removed server triggers a rejoin; an idle end is surfaced to the user.
   */
  onSessionEnded(event: { id: string; channel: string; reason: VoiceSessionEndReason }): void {
    if (event.channel !== this.#state.channelId || this.#state.status === "idle") {
      return;
    }
    switch (event.reason) {
      case "serverLost":
      case "serverRemoved":
        this.#rejoin();
        break;
      case "idle":
        this.#generation += 1;
        this.#teardown();
        this.#set({ ...IDLE, endedReason: "idle" });
        break;
      case "empty":
        break;
    }
  }

  /** Rejoins the current channel after a random pause of at most `REJOIN_DELAY_MAX_MS`. */
  #rejoin(): void {
    const channelId = this.#state.channelId;
    if (channelId === null || this.#state.status === "rejoining") {
      return;
    }
    this.#teardown();
    this.#set({ status: "rejoining", session: null });
    const generation = ++this.#generation;
    const delay = Math.floor(this.#random() * REJOIN_DELAY_MAX_MS);
    this.#setTimeout(() => {
      if (generation !== this.#generation) {
        return;
      }
      this.#connect(channelId, generation).catch((error: unknown) => {
        if (generation === this.#generation) {
          this.#teardown();
          this.#set({ ...IDLE, status: "failed", channelId, ...failure(error) });
        }
      });
    }, delay);
  }

  #teardown(): void {
    this.stopScreenShare();
    if (this.#state.screens.length > 0) {
      this.#set({ screens: [] });
    }
    for (const consumer of this.#consumers.values()) {
      consumer.close();
    }
    for (const id of this.#consumers.keys()) {
      this.#media.stop(id);
    }
    this.#consumers.clear();
    this.#sendTransport?.close();
    this.#recvTransport?.close();
    this.#sendTransport = null;
    this.#recvTransport = null;
    this.#microphone?.stop();
    this.#microphone = null;
    this.#microphoneProducer = null;
    if (this.#signal !== null) {
      this.#signal.send({ type: "leave" });
      this.#signal.close();
      this.#signal = null;
    }
  }

  async #connect(channelId: string, generation: number): Promise<void> {
    const offer = await this.#offer(channelId);
    // The microphone comes first: without it there is nothing to send, and its failure is
    // the browser's or the user's, never a voice server's, so no server is tried or reported.
    try {
      this.#microphone = await this.#media.getMicrophone(this.#devices.input);
    } catch (error) {
      throw new MicrophoneError(error);
    }
    if (generation !== this.#generation) {
      return;
    }
    const ranked = await this.#rank(offer.candidates);
    let lastError: Error | null = null;
    for (const { candidate } of ranked) {
      if (generation !== this.#generation) {
        return;
      }
      try {
        const signal = await this.#identify(candidate, offer.token, channelId, generation);
        await this.#establish(signal, generation);
        return;
      } catch (error) {
        lastError = error instanceof Error ? error : new Error(String(error));
        if (generation !== this.#generation) {
          return;
        }
        // Whatever the attempt set up (socket, transports) is released before the next
        // candidate builds its own; the microphone is kept for it.
        const microphone: MediaStreamTrack | null = this.#microphone;
        this.#microphone = null;
        this.#teardown();
        this.#microphone = microphone;
        await this.#reportFailure(candidate.id);
      }
    }
    throw lastError ?? new Error("no voice server could be reached");
  }

  async #offer(channelId: string): Promise<VoiceJoinOffer> {
    const result = await this.#client.api.POST("/api/v1/channels/{channel}/voice/join", {
      params: { path: { channel: channelId } },
    });
    if (result.data === undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
    return result.data;
  }

  /** Pings every candidate at once and orders them by round trip. */
  async #rank(candidates: readonly VoiceServerCandidate[]): Promise<RankedCandidate[]> {
    if (candidates.length <= 1) {
      return candidates.map((candidate) => ({ candidate, latencyMs: 0 }));
    }
    const ranked = await Promise.all(
      candidates.map(async (candidate) => {
        const started = this.#now();
        const controller = new AbortController();
        const timer = this.#setTimeout(() => {
          controller.abort();
        }, PING_TIMEOUT_MS);
        try {
          const response = await this.#fetch(`${candidate.url.replace(/\/$/, "")}/health`, {
            signal: controller.signal,
          });
          return { candidate, latencyMs: response.ok ? this.#now() - started : Infinity };
        } catch {
          return { candidate, latencyMs: Infinity };
        } finally {
          clearTimeout(timer);
        }
      }),
    );
    return rankCandidates(ranked);
  }

  async #reportFailure(serverId: string): Promise<void> {
    await this.#client.api
      .POST("/api/v1/voice-servers/{server}/failures", {
        params: { path: { server: serverId } },
      })
      .catch(() => undefined);
  }

  /** Opens a socket to the candidate and presents the token; resolves once `ready` arrives. */
  async #identify(
    candidate: VoiceServerCandidate,
    token: string,
    channelId: string,
    generation: number,
  ): Promise<Signal> {
    const signal = new Signal(signallingUrl(candidate), this.#Socket);
    const ready = signal.next((f) => f.type === "ready" || (f.type === "error" && f.fatal));
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = this.#setTimeout(() => {
        reject(new Error("the voice server did not answer in time"));
      }, READY_TIMEOUT_MS);
    });
    try {
      await signal.open();
      signal.send({ type: "identify", token });
      const frame = await Promise.race([ready, timeout]);
      if (frame.type !== "ready") {
        throw new Error(frame.type === "error" ? frame.detail : "unexpected reply");
      }
      if (generation !== this.#generation) {
        signal.close();
        throw new Error("superseded");
      }
      this.#signal = signal;
      this.#lastReady = frame;
      this.#set({ session: frame.session, channelId });
      signal.onClose = () => {
        // The server went away without a word: treat it as lost and rejoin.
        if (this.#signal === signal && this.#state.status === "connected") {
          this.#signal = null;
          this.#rejoin();
        }
      };
      signal.onFrame = (f) => {
        this.#onFrame(f);
      };
      return signal;
    } catch (error) {
      signal.close();
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }

  /** Loads the device, sets up both transports, and produces the microphone. */
  async #establish(signal: Signal, generation: number): Promise<void> {
    const ready = this.#lastReady;
    if (ready === null) {
      throw new Error("ready frame missing");
    }
    const device = await this.#media.createDevice();
    await device.load({ routerRtpCapabilities: ready.routerRtpCapabilities });
    signal.send({ type: "setCapabilities", rtpCapabilities: device.rtpCapabilities });
    const send = this.#awaitTransport(signal, "send");
    signal.send({ type: "createTransport", direction: "send" });
    const sendParams = await send;
    const recv = this.#awaitTransport(signal, "recv");
    signal.send({ type: "createTransport", direction: "recv" });
    const recvParams = await recv;
    if (generation !== this.#generation) {
      throw new Error("superseded");
    }
    this.#sendTransport = this.#wire(device.createSendTransport(sendParams), signal, sendParams.id);
    this.#recvTransport = this.#wire(device.createRecvTransport(recvParams), signal, recvParams.id);
    if (this.#microphone === null) {
      throw new Error("microphone missing");
    }
    this.#microphoneProducer = await this.#sendTransport.produce({
      track: this.#microphone,
      appData: { source: "microphone" },
    });
    // The server accepting the producer says nothing about media: ICE runs after the
    // signalling, and fails when the server announces an address this browser cannot reach.
    await this.#awaitConnected(this.#sendTransport);
    if (generation !== this.#generation) {
      throw new Error("superseded");
    }
    this.#set({ status: "connected", error: null });
    this.#sendState();
  }

  /** Resolves once the transport's ICE and DTLS are up; rejects when they fail or time out. */
  #awaitConnected(transport: VoiceTransport): Promise<void> {
    if (transport.connectionState === "connected") {
      return Promise.resolve();
    }
    if (transport.connectionState === "failed" || transport.connectionState === "closed") {
      return Promise.reject(new Error("media could not reach the voice server"));
    }
    return new Promise((resolve, reject) => {
      const timer = this.#setTimeout(() => {
        reject(new Error("media could not reach the voice server in time"));
      }, CONNECT_TIMEOUT_MS);
      transport.on("connectionstatechange", (state) => {
        if (state === "connected") {
          clearTimeout(timer);
          resolve();
        } else if (state === "failed" || state === "closed") {
          clearTimeout(timer);
          reject(new Error("media could not reach the voice server"));
        }
      });
    });
  }

  #lastReady: Extract<ServerMessage, { type: "ready" }> | null = null;

  #awaitTransport(signal: Signal, direction: "send" | "recv"): Promise<TransportParams> {
    return signal
      .next((f) => f.type === "transportCreated" && f.direction === direction)
      .then((f) => {
        if (f.type !== "transportCreated") {
          throw new Error("unexpected frame");
        }
        return {
          id: f.id,
          iceParameters: f.iceParameters,
          iceCandidates: f.iceCandidates,
          dtlsParameters: f.dtlsParameters,
        };
      });
  }

  /** Bridges a transport's connect and produce requests to the signalling socket. */
  #wire(transport: VoiceTransport, signal: Signal, transportId: string): VoiceTransport {
    transport.on("connectionstatechange", (state) => {
      // Media lost mid-call, with the socket still up: the call is rejoined from scratch
      // rather than left silent.
      if (
        state === "failed" &&
        this.#state.status === "connected" &&
        (transport === this.#sendTransport || transport === this.#recvTransport)
      ) {
        this.#rejoin();
      }
    });
    transport.on("connect", ({ dtlsParameters }, callback, errback) => {
      signal
        .next((f) => f.type === "transportConnected" && f.transportId === transportId)
        .then(() => {
          callback();
        })
        .catch(errback);
      signal.send({ type: "connectTransport", transportId, dtlsParameters });
    });
    transport.on("produce", ({ kind, rtpParameters, appData }, callback, errback) => {
      const source =
        appData.source === "screen" || appData.source === "screenAudio"
          ? appData.source
          : "microphone";
      signal
        .next((f) => f.type === "produced" && f.source === source)
        .then((f) => {
          if (f.type === "produced") {
            callback({ id: f.producerId });
          }
        })
        .catch(errback);
      signal.send({
        type: "produce",
        transportId,
        kind: kind === "video" ? "video" : "audio",
        source,
        rtpParameters,
      });
    });
    return transport;
  }

  #onFrame(frame: ServerMessage): void {
    switch (frame.type) {
      case "newConsumer":
        void this.#consume(frame);
        break;
      case "consumerClosed": {
        const consumer = this.#consumers.get(frame.consumerId);
        if (consumer !== undefined) {
          consumer.close();
          this.#consumers.delete(frame.consumerId);
          this.#media.stop(frame.consumerId);
        }
        if (this.#state.screens.some((screen) => screen.consumerId === frame.consumerId)) {
          this.#set({
            screens: this.#state.screens.filter((screen) => screen.consumerId !== frame.consumerId),
          });
        }
        if (this.#previewConsumerId === frame.consumerId) {
          this.#previewConsumerId = null;
          if (this.#external !== null) {
            this.#set({ localScreen: null });
          }
        }
        break;
      }
      case "kicked":
        if (frame.reason === "serverStopping") {
          // The server is going down; the API server ends its calls and the offer names another.
          this.#rejoin();
          break;
        }
        this.#generation += 1;
        this.#teardown();
        // Replaced means another of the user's own clients took the call over, which needs no
        // notice; removed by a moderator does.
        this.#set({ ...IDLE, endedReason: frame.reason === "kicked" ? "kicked" : null });
        break;
      case "participantState":
        // A moderator's mute arrives as the server's word on this client's own state.
        if (frame.user === this.#lastReady?.user) {
          this.#set({ muted: frame.muted, deafened: frame.deafened });
        }
        break;
      case "error":
        if (frame.fatal) {
          this.#generation += 1;
          this.#teardown();
          this.#set({ ...IDLE, status: "failed", error: frame.detail });
        }
        break;
      default:
        break;
    }
  }

  async #consume(frame: Extract<ServerMessage, { type: "newConsumer" }>): Promise<void> {
    const transport = this.#recvTransport;
    const signal = this.#signal;
    if (transport === null || signal === null) {
      return;
    }
    try {
      const consumer = await transport.consume({
        id: frame.consumerId,
        producerId: frame.producerId,
        kind: frame.kind,
        rtpParameters: frame.rtpParameters,
      });
      this.#consumers.set(frame.consumerId, {
        close: () => {
          consumer.close();
        },
        user: frame.user,
      });
      if (frame.kind === "video" && frame.user === this.#lastReady?.user) {
        // The call's own external share, back from the server as its preview.
        this.#previewConsumerId = frame.consumerId;
        this.#set({ localScreen: consumer.track });
      } else if (frame.kind === "video") {
        this.#set({
          screens: [
            ...this.#state.screens,
            { user: frame.user, consumerId: frame.consumerId, track: consumer.track },
          ],
        });
      } else {
        this.#media.play(frame.consumerId, consumer.track);
        const gain = this.#userVolume(frame.user);
        if (gain !== 1) {
          this.#media.setVolume(frame.consumerId, gain);
        }
      }
      signal.send({ type: "resumeConsumer", consumerId: frame.consumerId });
    } catch {
      // The transport is gone; the consumer will be re-announced after the rejoin.
    }
  }
}
