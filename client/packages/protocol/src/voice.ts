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
 * mediasoup device sit behind `VoiceMedia` (`voiceMedia.ts`), so the flow runs and is tested
 * without them.
 */

import { DEFAULT_DEVICE, type DeviceChoice } from "./preferences";
import type { components } from "./generated/openapi";
import type { Grants, ServerMessage } from "./generated/voiceSignal";
import type { VoiceSessionEndReason } from "./generated/events";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError } from "./problem";
import {
  type FileSink,
  FileTransfers,
  type FilesState,
  NO_FILES,
  type TransferMode,
} from "./transfers";
import { Signal } from "./signalSocket";
import { type RankedCandidate, rankCandidates, signallingUrl } from "./voiceCandidates";
import type {
  ExternalAudio,
  ExternalShare,
  RtpTarget,
  ScreenCapture,
  TransportParams,
  VoiceDevice,
  VoiceMedia,
  VoiceTransport,
} from "./voiceMedia";

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
export /** The camera this client sends: its track, and the producer once the voice server has it. */
interface CameraSend {
  track: MediaStreamTrack;
  producer: {
    id: string;
    close(): void;
    replaceTrack(options: { track: MediaStreamTrack }): Promise<void>;
  } | null;
}

/**
 * A camera is sent as one layer allowed up to 4 Mbps at 30 frames a second, which a 1080p camera
 * fills well. The encoder starts at 10 Mbps (in kbps here), as a share does, so nothing of ours
 * holds it back; the browser still sizes the picture to its bandwidth estimate, which climbs over
 * the first seconds of a camera, and gives up resolution rather than frames where bandwidth runs
 * short, which suits a face.
 */
const CAMERA_ENCODING = { maxBitrate: 4_000_000, maxFramerate: 30 };
const CAMERA_VIDEO_CODEC = { videoGoogleStartBitrate: 10_000 };

/** The two sounds a participant sends, whose volumes are set apart: their voice, and their stream's. */
export type AudioSource = "microphone" | "screenAudio";

function isAudioSource(source: string): source is AudioSource {
  return source === "microphone" || source === "screenAudio";
}

/**
 * A shared screen is sent at the best quality the network carries: one layer allowed up to
 * 25 Mbps at 60 frames a second, the ceiling rather than a target, since the encoder spends only
 * what the picture needs and the connection's bandwidth estimate brings it down.
 */
const SCREEN_ENCODING = { maxBitrate: 25_000_000, maxFramerate: 60 };
/**
 * The encoder starts at 10 Mbps (in kbps here) rather than the browser's few hundred kbps, which
 * it climbs from slowly, so a share is sharp from its first seconds.
 */
const SCREEN_VIDEO_CODEC = { videoGoogleStartBitrate: 10_000 };

const SCREEN_AUDIO_CODEC = {
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
  /**
   * Whether the channel lets the user send their microphone (Speak) and share a screen
   * (Share screen), as the join offer said; without Speak they join to listen.
   */
  readonly canSpeak: boolean;
  readonly canShare: boolean;
  /** Whether the channel lets the user turn on a camera (Use camera). */
  readonly canCamera: boolean;
  /** Whether the channel lets the user offer files to the call (Transfer files). */
  readonly canTransfer: boolean;
  /** Files offered in the call, this user's transfers, and everyone's transfers under way. */
  files: FilesState;
  /** Whether this client is sending a screen, window, or game into the call. */
  sharingScreen: boolean;
  /** The video this client is sending, for a local preview; `null` when not sharing. */
  localScreen: MediaStreamTrack | null;
  /** The screens other participants are sharing, in the order they arrived. */
  screens: readonly RemoteScreen[];
  /** This client's camera while it is on, for its own tile; `null` when off. */
  localCamera: MediaStreamTrack | null;
  /** The cameras other participants have on, in the order they arrived. */
  cameras: readonly RemoteScreen[];
  /**
   * Why the camera last failed to turn on, until it turns on, the user dismisses it
   * (`clearCameraError`), or the call ends.
   */
  cameraError: CameraFailure | null;
  /**
   * What failed when `status` is `failed`: the microphone (permission refused, no device, or an
   * insecure page origin, which browsers refuse media on), every voice server, or the voice
   * server refusing one of the call's requests (`refused`), most often for coming too fast.
   */
  errorKind: "microphone" | "server" | "refused" | null;
  /**
   * When the voice server refused a request for coming too fast, how many seconds until it
   * would take it; `null` otherwise.
   */
  retryAfterSeconds: number | null;
  /** Why the last attempt failed, for the UI; cleared on the next join. */
  readonly error: string | null;
  /**
   * Set when the server ended the call for being idle, until `acknowledgeEnd`, so the UI can
   * tell the user why they were dropped.
   */
  readonly endedReason: VoiceSessionEndReason | "kicked" | "accessLost" | null;
}

/** A screen another participant is sharing, or their camera, as a playable video track. */
export interface RemoteScreen {
  readonly user: string;
  readonly consumerId: string;
  readonly track: MediaStreamTrack;
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
  /**
   * How loud each other user should be to this one, their voice (`microphone`) and the sound of
   * what they share (`screenAudio`) apart; consulted as their audio arrives.
   */
  userVolume?: (userId: string, source: AudioSource) => number;
  /** Makes file transfers' peer connections; the browser's own by default. */
  createPeerConnection?: (configuration: RTCConfiguration) => RTCPeerConnection;
}

export type VoiceCallListener = () => void;

const IDLE: VoiceCallState = {
  status: "idle",
  channelId: null,
  session: null,
  muted: false,
  deafened: false,
  canSpeak: false,
  canShare: false,
  canCamera: false,
  canTransfer: false,
  files: NO_FILES,
  sharingScreen: false,
  localScreen: null,
  screens: [],
  localCamera: null,
  cameraError: null,
  cameras: [],
  errorKind: null,
  retryAfterSeconds: null,
  error: null,
  endedReason: null,
};

function sameChoice(a: DeviceChoice, b: DeviceChoice): boolean {
  if (a === DEFAULT_DEVICE || b === DEFAULT_DEVICE) {
    return a === b;
  }
  return a.id === b.id && a.label === b.label;
}

/**
 * Why the camera did not turn on: no camera is connected (`none`), the browser or the system
 * refused access (`denied`), every camera there is failed to start (`failed`), or the voice
 * server did not take the picture (`unsent`).
 */
export type CameraFailure = "none" | "denied" | "failed" | "unsent";

/** The camera could not be turned on, for the reason `failure` names; `cause` is the underlying error. */
export class CameraError extends Error {
  constructor(
    readonly failure: CameraFailure,
    cause?: unknown,
  ) {
    super(cause instanceof Error ? cause.message : failure, { cause });
    this.name = "CameraError";
  }
}

/** The microphone could not be opened; `cause` is the browser's error. */
export class MicrophoneError extends Error {
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : String(cause), { cause });
    this.name = "MicrophoneError";
  }
}

/**
 * The voice server refused one of the call's requests (an `error` frame); `retryAfterSeconds` is
 * set when the request came too fast. The refusal is about the request, not the server, so it
 * is not reported as the server's failure and no other server is tried.
 */
export class VoiceRequestRefused extends Error {
  constructor(
    detail: string,
    readonly retryAfterSeconds: number | null,
  ) {
    super(detail);
    this.name = "VoiceRequestRefused";
  }
}

function failure(
  error: unknown,
): Pick<VoiceCallState, "errorKind" | "error" | "retryAfterSeconds"> {
  return {
    errorKind:
      error instanceof MicrophoneError
        ? "microphone"
        : error instanceof VoiceRequestRefused
          ? "refused"
          : "server",
    retryAfterSeconds: error instanceof VoiceRequestRefused ? error.retryAfterSeconds : null,
    error: error instanceof Error ? error.message : String(error),
  };
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
    close(): void;
  } | null = null;
  #devices: { input: DeviceChoice; output: DeviceChoice; camera: DeviceChoice } = {
    input: DEFAULT_DEVICE,
    output: DEFAULT_DEVICE,
    camera: DEFAULT_DEVICE,
  };
  #camera: CameraSend | null = null;
  #external: { producerIds: string[]; share: ExternalShare } | null = null;
  /** The consumer carrying the call's own preview of an external share. */
  #previewConsumerId: string | null = null;
  readonly #consumers = new Map<string, { close(): void; user: string; source: string }>();
  readonly #userVolume: (userId: string, source: AudioSource) => number;
  /** Increments on every join and leave so a stale async step can notice and bail. */
  #generation = 0;
  readonly #files: FileTransfers;

  constructor(options: VoiceCallOptions) {
    this.#client = options.client;
    this.#media = options.media;
    this.#Socket = options.WebSocket ?? globalThis.WebSocket;
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#setTimeout = options.setTimeout ?? globalThis.setTimeout.bind(globalThis);
    this.#userVolume = options.userVolume ?? (() => 1);
    this.#now = options.now ?? (() => Date.now());
    this.#random = options.random ?? Math.random;
    this.#files = new FileTransfers({
      send: (frame) => {
        this.#signal?.send(frame);
      },
      onChange: () => {
        this.#set({ files: this.#files.state });
      },
      ...(options.createPeerConnection === undefined
        ? {}
        : { createPeerConnection: options.createPeerConnection }),
      now: this.#now,
    });
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
    // Moving from one call to another keeps the microphone open for the next one.
    if (this.#state.channelId !== null) {
      this.#teardown({ keepMicrophone: true });
    }
    const generation = ++this.#generation;
    this.#set({
      status: "joining",
      channelId,
      session: null,
      errorKind: null,
      retryAfterSeconds: null,
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

  /**
   * Offers a file to everyone else in the call for `validForSeconds`. Every acceptance then
   * starts sending on its own, whether or not the user is watching. `allowDirect` lets
   * receivers connect directly, which could expose this device's address to them.
   */
  offerFile(file: Blob, name: string, allowDirect: boolean, validForSeconds: number): string {
    return this.#files.offer(file, name, allowDirect, validForSeconds);
  }

  /** Takes back one of the user's offers; transfers already started go on. */
  withdrawOffer(offer: string): void {
    this.#files.withdraw(offer);
  }

  /**
   * Accepts an offer in the transfer mode the user acknowledged, writing it to `sink` as it
   * arrives when the user chose where to save it first.
   */
  acceptOffer(offer: string, mode: TransferMode, sink?: FileSink): void {
    this.#files.accept(offer, mode, sink);
  }

  /** Cancels one transfer, which ends at once on both sides. */
  cancelTransfer(offer: string, peer: string): void {
    this.#files.cancel(offer, peer);
  }

  /** Forgets a finished transfer, and any file it received. */
  dismissTransfer(offer: string, peer: string): void {
    this.#files.dismiss(offer, peer);
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
    // Nothing is shared outside a call, into a call that does not allow it, or twice.
    if (
      this.#state.status !== "connected" ||
      !this.#state.canShare ||
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
        await transport.produce({
          track: capture.video,
          appData: { source: "screen" },
          encodings: [SCREEN_ENCODING],
          codecOptions: SCREEN_VIDEO_CODEC,
        }),
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
    const produced = this.#reply(signal, "rtpProduced", (f) => f.source === source);
    signal.send({ type: "produceRtp", source });
    const frame = await produced;
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
      !this.#state.canShare ||
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
   * Turns the camera on: opens the chosen camera and produces it as `camera`, at up to 1080p and
   * 30 frames a second (`CAMERA_QUALITY`, `CAMERA_ENCODING`). Ends on its own if the camera goes
   * away. Nothing happens outside a call, in one that does not allow it, or while it is on. A
   * failure is kept as `cameraError` and thrown as a `CameraError`.
   */
  async startCamera(): Promise<void> {
    const transport = this.#sendTransport;
    if (
      this.#state.status !== "connected" ||
      !this.#state.canCamera ||
      transport === null ||
      this.#camera !== null
    ) {
      return;
    }
    let track: MediaStreamTrack;
    try {
      track = await this.#media.getCamera(this.#devices.camera);
    } catch (error) {
      const failure = error instanceof CameraError ? error : new CameraError("failed", error);
      if (this.#sendTransport === transport) {
        this.#set({ cameraError: failure.failure });
      }
      throw failure;
    }
    // Opening the camera took time; the call may have moved on, or it was turned on meanwhile.
    if (this.#sendTransport !== transport || this.#cameraOn()) {
      track.stop();
      return;
    }
    const camera: CameraSend = { track, producer: null };
    this.#camera = camera;
    this.#set({ localCamera: track, cameraError: null });
    track.addEventListener("ended", () => {
      if (this.#camera === camera) {
        this.stopCamera();
      }
    });
    try {
      camera.producer = await transport.produce({
        track,
        appData: { source: "camera" },
        encodings: [CAMERA_ENCODING],
        codecOptions: CAMERA_VIDEO_CODEC,
      });
      if (this.#camera !== camera) {
        camera.producer.close();
        this.#signal?.send({ type: "closeProducer", producerId: camera.producer.id });
      }
    } catch (error) {
      this.stopCamera();
      if (this.#sendTransport === transport) {
        this.#set({ cameraError: "unsent" });
      }
      throw new CameraError("unsent", error);
    }
  }

  /** Dismisses the reason the camera last failed. */
  clearCameraError(): void {
    this.#set({ cameraError: null });
  }

  /** Whether the camera is on, read afresh where an `await` may have changed it. */
  #cameraOn(): boolean {
    return this.#camera !== null;
  }

  /** Turns the camera off, if it is on. */
  stopCamera(): void {
    const camera = this.#camera;
    if (camera === null) {
      return;
    }
    this.#camera = null;
    if (camera.producer !== null) {
      camera.producer.close();
      this.#signal?.send({ type: "closeProducer", producerId: camera.producer.id });
    }
    camera.track.stop();
    this.#set({ localCamera: null });
  }

  /** Moves a camera that is on to the camera now chosen. */
  async #swapCamera(): Promise<void> {
    const camera = this.#camera;
    if (camera?.producer == null) {
      return;
    }
    const track = await this.#media.getCamera(this.#devices.camera);
    if (this.#camera !== camera) {
      track.stop();
      return;
    }
    await camera.producer.replaceTrack({ track });
    camera.track.stop();
    camera.track = track;
    this.#set({ localCamera: track });
  }

  /**
   * The devices voice chat uses, from the user's preferences. Takes effect at once, in a call
   * or not: the microphone producer swaps to the new input and playback moves to the new
   * output.
   */
  async setAudioDevices(devices: {
    input: DeviceChoice;
    output: DeviceChoice;
    camera?: DeviceChoice;
  }): Promise<void> {
    const previous = this.#devices;
    this.#devices = { ...devices, camera: devices.camera ?? previous.camera };
    if (!sameChoice(this.#devices.camera, previous.camera)) {
      await this.#swapCamera();
    }
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

  /** Sets every consumer's gain again from `userVolume`, after what it answers has changed. */
  refreshVolumes(): void {
    for (const [consumerId, consumer] of this.#consumers) {
      if (isAudioSource(consumer.source)) {
        this.#media.setVolume(consumerId, this.#userVolume(consumer.user, consumer.source));
      }
    }
  }

  /**
   * Sets how loud `userId`'s voice, or with `source` `screenAudio` the sound of what they share,
   * is heard right now; the preference behind it is the caller's to keep.
   */
  setUserVolume(userId: string, gain: number, source: AudioSource = "microphone"): void {
    for (const [consumerId, consumer] of this.#consumers) {
      if (consumer.user === userId && consumer.source === source) {
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
    this.#teardown({ keepMicrophone: true });
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

  /**
   * Releases everything the call holds. `keepMicrophone` leaves the microphone open for the
   * next attempt or call, which then does not wait on the browser to open it again.
   */
  #teardown({ keepMicrophone = false }: { keepMicrophone?: boolean } = {}): void {
    this.#files.closeAll();
    this.stopScreenShare();
    this.stopCamera();
    if (
      this.#state.screens.length > 0 ||
      this.#state.cameras.length > 0 ||
      this.#state.cameraError !== null
    ) {
      this.#set({ screens: [], cameras: [], cameraError: null });
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
    if (!keepMicrophone) {
      this.#microphone?.stop();
      this.#microphone = null;
    }
    this.#microphoneProducer = null;
    if (this.#signal !== null) {
      this.#signal.send({ type: "leave" });
      this.#signal.close();
      this.#signal = null;
    }
  }

  async #connect(channelId: string, generation: number): Promise<void> {
    const offer = await this.#offer(channelId);
    if (generation !== this.#generation) {
      return;
    }
    this.#set({
      canSpeak: offer.speak,
      canShare: offer.shareScreen,
      canCamera: offer.useCamera,
      canTransfer: offer.transferFiles,
    });
    // The microphone comes first: without it there is nothing to send, and its failure is
    // the browser's or the user's, never a voice server's, so no server is tried or reported.
    // Someone who may not speak joins to listen and never opens it, and one kept open from
    // a call they could speak in is closed.
    if (!offer.speak) {
      this.#microphone?.stop();
      this.#microphone = null;
    } else if (this.#microphone === null) {
      let microphone: MediaStreamTrack;
      try {
        microphone = await this.#media.getMicrophone(this.#devices.input);
      } catch (error) {
        throw new MicrophoneError(error);
      }
      // A join that took over meanwhile opens its own.
      if (generation !== this.#generation) {
        microphone.stop();
        return;
      }
      this.#microphone = microphone;
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
        // A refused request is the user's to wait out, not this server's failure, and another
        // server would refuse it no differently.
        if (error instanceof VoiceRequestRefused) {
          throw error;
        }
        // Whatever the attempt set up (socket, transports) is released before the next
        // candidate builds its own; the microphone is kept for it.
        this.#teardown({ keepMicrophone: true });
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
        // Turned away for joining too fast is a refusal to wait out, not the server failing.
        if (frame.type === "error" && frame.retryAfterSeconds != null) {
          throw new VoiceRequestRefused(frame.detail, frame.retryAfterSeconds);
        }
        throw new Error(frame.type === "error" ? frame.detail : "unexpected reply");
      }
      if (generation !== this.#generation) {
        signal.close();
        throw new Error("superseded");
      }
      this.#signal = signal;
      this.#lastReady = frame;
      this.#files.reset(frame);
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
    const device = await this.#loadedDevice(ready.routerRtpCapabilities);
    signal.send({ type: "setCapabilities", rtpCapabilities: device.rtpCapabilities });
    // Both transports are asked for at once; the server answers each by its direction.
    const send = this.#awaitTransport(signal, "send");
    const recv = this.#awaitTransport(signal, "recv");
    signal.send({ type: "createTransport", direction: "send" });
    signal.send({ type: "createTransport", direction: "recv" });
    const [sendParams, recvParams] = await Promise.all([send, recv]);
    if (generation !== this.#generation) {
      throw new Error("superseded");
    }
    this.#sendTransport = this.#wire(device.createSendTransport(sendParams), signal, sendParams.id);
    this.#recvTransport = this.#wire(device.createRecvTransport(recvParams), signal, recvParams.id);
    if (this.#state.canSpeak) {
      if (this.#microphone === null) {
        throw new Error("microphone missing");
      }
      // The call owns the microphone's track, which outlives the transport when the user
      // moves to another call.
      this.#microphoneProducer = await this.#sendTransport.produce({
        track: this.#microphone,
        appData: { source: "microphone" },
        stopTracks: false,
      });
      // The server accepting the producer says nothing about media: ICE runs after the
      // signalling, and fails when the server announces an address this browser cannot reach.
      // A listener sends nothing, so its transports connect with the first consumer, and a
      // server it cannot reach shows only then.
      await this.#awaitConnected(this.#sendTransport);
    }
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

  /**
   * The device loaded for the last router, kept by its capabilities. Loading probes what the
   * browser can send and receive, which takes a while, and every call on a voice server (and
   * every voice server of one version) answers the same capabilities, so moving between calls
   * loads it once.
   */
  #device: { capabilities: string; device: Promise<VoiceDevice> } | null = null;

  /** A device loaded with the router's capabilities, the one kept when they match. */
  #loadedDevice(routerRtpCapabilities: unknown): Promise<VoiceDevice> {
    const capabilities = JSON.stringify(routerRtpCapabilities);
    if (this.#device?.capabilities !== capabilities) {
      const device = this.#media.createDevice().then(async (created) => {
        await created.load({ routerRtpCapabilities });
        return created;
      });
      this.#device = { capabilities, device };
      // A device that failed to load is not kept for the next call.
      device.catch(() => {
        if (this.#device?.device === device) {
          this.#device = null;
        }
      });
    }
    return this.#device.device;
  }

  /**
   * The server's answer to a request: the next frame of `type` that `matches`. An `error` frame
   * arriving first refuses it (`VoiceRequestRefused`), since the server answers a request it
   * will not honour with one, and no answer within `READY_TIMEOUT_MS` is the server's failure.
   * Error frames do not say which request they answer, so one refuses every request waiting
   * for an answer when it comes.
   */
  #reply<T extends ServerMessage["type"]>(
    signal: Signal,
    type: T,
    matches: (frame: Extract<ServerMessage, { type: T }>) => boolean = () => true,
  ): Promise<Extract<ServerMessage, { type: T }>> {
    const isAnswer = (frame: ServerMessage): frame is Extract<ServerMessage, { type: T }> =>
      frame.type === type && matches(frame as Extract<ServerMessage, { type: T }>);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = this.#setTimeout(() => {
        reject(new Error("the voice server did not answer in time"));
      }, READY_TIMEOUT_MS);
    });
    const answer = signal
      .next((frame) => isAnswer(frame) || frame.type === "error")
      .then((frame) => {
        if (frame.type === "error") {
          throw new VoiceRequestRefused(frame.detail, frame.retryAfterSeconds ?? null);
        }
        if (!isAnswer(frame)) {
          throw new Error("unexpected frame");
        }
        return frame;
      });
    return Promise.race([answer, timeout]).finally(() => {
      clearTimeout(timer);
    });
  }

  async #awaitTransport(signal: Signal, direction: "send" | "recv"): Promise<TransportParams> {
    const frame = await this.#reply(signal, "transportCreated", (f) => f.direction === direction);
    return {
      id: frame.id,
      iceParameters: frame.iceParameters,
      iceCandidates: frame.iceCandidates,
      dtlsParameters: frame.dtlsParameters,
    };
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
      this.#reply(signal, "transportConnected", (f) => f.transportId === transportId)
        .then(() => {
          callback();
        })
        .catch(errback);
      signal.send({ type: "connectTransport", transportId, dtlsParameters });
    });
    transport.on("produce", ({ kind, rtpParameters, appData }, callback, errback) => {
      const source =
        appData.source === "screen" ||
        appData.source === "screenAudio" ||
        appData.source === "camera"
          ? appData.source
          : "microphone";
      this.#reply(signal, "produced", (f) => f.source === source)
        .then((f) => {
          callback({ id: f.producerId });
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
    if (this.#files.handle(frame)) {
      return;
    }
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
        if (this.#state.cameras.some((camera) => camera.consumerId === frame.consumerId)) {
          this.#set({
            cameras: this.#state.cameras.filter((camera) => camera.consumerId !== frame.consumerId),
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
        // notice; removed by a moderator, or for no longer being allowed in, does.
        this.#set({
          ...IDLE,
          endedReason:
            frame.reason === "kicked" || frame.reason === "accessLost" ? frame.reason : null,
        });
        break;
      case "grantsChanged":
        void this.#grantsChanged(frame.grants);
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

  /**
   * Follows what the client may now do in the call, as the voice server says once the
   * permissions behind it change: what it may no longer send stops (the server has already
   * closed it), and a microphone it may now send starts.
   */
  async #grantsChanged(grants: Grants): Promise<void> {
    this.#set({
      canSpeak: grants.speak,
      canShare: grants.shareScreen,
      canCamera: grants.camera,
      canTransfer: grants.transferFiles,
    });
    if (!grants.shareScreen) {
      this.stopScreenShare();
    }
    if (!grants.camera) {
      this.stopCamera();
    }
    if (!grants.speak) {
      this.#microphoneProducer?.close();
      this.#microphoneProducer = null;
      this.#microphone?.stop();
      this.#microphone = null;
      return;
    }
    const transport = this.#sendTransport;
    if (this.#microphoneProducer !== null || transport === null) {
      return;
    }
    const generation = this.#generation;
    try {
      const microphone = this.#microphone ?? (await this.#media.getMicrophone(this.#devices.input));
      if (generation !== this.#generation || this.#sendTransport !== transport) {
        if (microphone !== this.#microphone) {
          microphone.stop();
        }
        return;
      }
      this.#microphone = microphone;
      this.#microphoneProducer = await transport.produce({
        track: microphone,
        appData: { source: "microphone" },
        stopTracks: false,
      });
      this.#sendState();
    } catch (error) {
      this.#set({ error: new MicrophoneError(error).message });
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
        source: frame.source,
      });
      if (frame.kind === "video" && frame.source === "camera") {
        this.#set({
          cameras: [
            ...this.#state.cameras,
            { user: frame.user, consumerId: frame.consumerId, track: consumer.track },
          ],
        });
      } else if (frame.kind === "video" && frame.user === this.#lastReady?.user) {
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
        const gain = this.#userVolume(
          frame.user,
          frame.source === "screenAudio" ? "screenAudio" : "microphone",
        );
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
