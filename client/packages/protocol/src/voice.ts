/**
 * A voice call from the client's side.
 *
 * Joining asks the API server for an offer (a token and the candidate voice servers), measures
 * the latency to each candidate, and tries them nearest first: a signalling socket is opened,
 * the token presented, and if the server does not answer `ready` in time the failure is
 * reported to the API server and the next candidate is tried. Once in, the voice server offers
 * the call's one peer connection and the call answers every offer it makes: the server is the
 * only side that offers. Whatever the call wants changed (a source to send, the video to watch,
 * an ICE restart) it asks for, and the server offers the change. The connection receives an
 * audio slot for each other microphone, up to `ready`'s `audioSlots`, which the server fills
 * with whoever is loudest and says whom each carries (`slots`), and a section for each video it
 * watches (`showVideo`).
 *
 * A signalling socket that closes on its own is resumed with `ready`'s secret, the peer
 * connection and its media going on meanwhile; a call whose socket cannot be resumed, or ended
 * because its server was lost or removed, is rejoined on its own after a random delay of up to
 * a second, so the participants of a lost call do not all hit the API server at once. A call
 * ended for being idle, or by the user, is not. The browser's media APIs sit behind
 * `VoiceMedia` (`voiceMedia.ts`), so the flow runs and is tested without them.
 */

import { DEFAULT_DEVICE, type DeviceChoice } from "./preferences";
import type { components } from "./generated/openapi";
import type {
  ClientMessage,
  Grants,
  MediaSource,
  Section,
  ServerMessage,
  WatchedVideo,
} from "./generated/voiceSignal";
import type { VoiceSessionEndReason } from "./generated/events";
import { type AspenClient, problemOf } from "./http";
import { compareIds } from "./order";
import { ApiProblemError } from "./problem";
import {
  type FileSink,
  FileTransfers,
  type FilesState,
  NO_FILES,
  type TransferMode,
} from "./transfers";
import { Signal, VoiceServerError } from "./signalSocket";

export { VoiceServerError };
import { InsecureAddressError, securelyReached } from "./urls";
import { type RankedCandidate, rankCandidates, signallingUrl } from "./voiceCandidates";
import type {
  CallConnection,
  CallTransceiver,
  ExternalAudio,
  ExternalSender,
  ExternalShare,
  ScreenCapture,
  VoiceMedia,
} from "./voiceMedia";

type VoiceJoinOffer = components["schemas"]["VoiceJoinOffer"];
type VoiceServerCandidate = components["schemas"]["VoiceServerCandidate"];
type Offer = Extract<ServerMessage, { type: "offer" }>;
type Ready = Extract<ServerMessage, { type: "ready" }>;

/** How long the peer connection has to connect once the server has accepted the call. */
export const CONNECT_TIMEOUT_MS = 15_000;
/**
 * How often a connection lost mid-call asks for an ICE restart, and how long it may stay lost
 * before the call is rejoined. Eight restarts in half a minute stay within the voice server's
 * limit on `restartIce`.
 */
export const ICE_RESTART_INTERVAL_MS = 4_000;
export const ICE_RECOVERY_LIMIT_MS = 30_000;
/** How long a candidate has to answer a latency ping. */
export const PING_TIMEOUT_MS = 3_000;
/** How long a candidate has to answer `identify` or `resume` with `ready`. */
export const READY_TIMEOUT_MS = 6_000;
/**
 * How many times a signalling socket that closed on its own is opened again to resume, before
 * the call is rejoined instead. Each waits a random time of up to `RESUME_DELAY_MAX_MS` more
 * than the one before; all of them end well inside the server's twenty seconds of grace.
 */
export const RESUME_ATTEMPTS = 3;
export const RESUME_DELAY_MAX_MS = 1_000;

/** The two sounds a participant sends, whose volumes are set apart: their voice, and their stream's. */
export type AudioSource = "microphone" | "screenAudio";

/**
 * How a sent video is encoded, set on its sender once its section is answered: a ceiling the
 * encoder may spend up to, never a target, since it spends only what the picture needs and the
 * connection's bandwidth estimate brings it down. The voice server's offers say what the
 * encoder starts at (`x-google-start-bitrate`) and, for a share's sound, stereo without
 * discontinuous transmission at 128 kbps.
 */
interface Encoding {
  readonly maxBitrate: number;
  readonly maxFramerate: number;
}

/**
 * A camera is sent as one layer allowed up to 4 Mbps at 30 frames a second, which a 1080p
 * camera fills well; the browser sizes the picture to its bandwidth estimate, which climbs over
 * the first seconds of a camera, and gives up resolution rather than frames where bandwidth
 * runs short, which suits a face.
 */
const CAMERA_ENCODING: Encoding = { maxBitrate: 4_000_000, maxFramerate: 30 };

/** A shared screen is allowed up to 25 Mbps at 60 frames a second. */
const SCREEN_ENCODING: Encoding = { maxBitrate: 25_000_000, maxFramerate: 60 };

/** The most a lost call waits before rejoining. */
export const REJOIN_DELAY_MAX_MS = 1_000;
/**
 * How long a call whose signalling socket closed, and whose rejoin could not reach the server
 * the API server still offered, waits before asking for a join offer once more. It mirrors the
 * server's `offer_silence_seconds`: a voice server that has not reported for that long is no
 * longer offered, and a call bound to it is ended when someone next asks to join its channel,
 * so the second offer names a living server.
 */
export const DEAD_SERVER_RETRY_MS = 60_000;

export type VoiceCallStatus = "idle" | "joining" | "connected" | "rejoining" | "failed";

/** What the user asked of their own state in the call. */
interface Asked {
  readonly muted: boolean;
  readonly deafened: boolean;
  readonly held: boolean;
}

export interface VoiceCallState {
  readonly status: VoiceCallStatus;
  /** The channel being joined or in, `null` when idle. */
  readonly channelId: string | null;
  readonly session: string | null;
  readonly muted: boolean;
  readonly deafened: boolean;
  /**
   * Whether the call is on hold (`setHeld`), as a phone puts it while a phone call has its
   * audio: the voice server sends none of the user's media and none to them, and their mute and
   * deafen stay as they are for when it ends.
   */
  readonly held: boolean;
  /** The others in the call the voice server says are on hold, in id order. */
  readonly onHold: readonly string[];
  /**
   * Whether a moderator's mute of the user stands in the call's community, which keeps them
   * muted in every call there, whatever they ask, until a moderator lifts it.
   */
  readonly serverMuted: boolean;
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
  /**
   * The video this client is sending, for a local preview; `null` when not sharing. A share
   * sent from outside the browser is previewed from the voice server, while video is shown
   * (`showVideo`).
   */
  localScreen: MediaStreamTrack | null;
  /** The screens other participants are sharing, while video is shown, in the order they arrived. */
  screens: readonly RemoteScreen[];
  /** This client's camera while it is on, for its own tile; `null` when off. */
  localCamera: MediaStreamTrack | null;
  /** The cameras other participants have on, while video is shown, in the order they arrived. */
  cameras: readonly RemoteScreen[];
  /**
   * Why the camera last failed to turn on, until it turns on, the user dismisses it
   * (`clearCameraError`), or the call ends.
   */
  cameraError: CameraFailure | null;
  /**
   * What failed when `status` is `failed`: the microphone (permission refused, no device, or an
   * insecure page origin, which browsers refuse media on), every voice server, the voice
   * server refusing one of the call's requests (`refused`), most often for coming too fast, or
   * this device failing to set the call up (`device`: a browser without WebRTC or a codec the
   * call needs), which no voice server is blamed for.
   */
  errorKind: "microphone" | "server" | "refused" | "device" | null;
  /**
   * When the voice server refused a request for coming too fast, how many seconds until it
   * would take it; `null` otherwise.
   */
  retryAfterSeconds: number | null;
  /** Why the last attempt failed, for the UI; cleared on the next join. */
  readonly error: string | null;
  /**
   * Set when the voice server refused to unmute or undeafen the user (most often for doing it
   * too often), which leaves them muted or deafened as `muted` and `deafened` say, until they
   * try again, dismiss it (`clearStateRefusal`), or the call ends. `retryAfterSeconds` is how
   * long until the server would take it, when it said.
   */
  readonly stateRefused: { readonly retryAfterSeconds: number | null } | null;
  /**
   * Set when the server ended the call for being idle, until `acknowledgeEnd`, so the UI can
   * tell the user why they were dropped.
   */
  readonly endedReason: VoiceSessionEndReason | "kicked" | "accessLost" | null;
}

/** A screen another participant is sharing, or their camera, as a playable video track. */
export interface RemoteScreen {
  readonly user: string;
  /** The media section carrying it, unique in the call for as long as it is shown. */
  readonly id: string;
  readonly track: MediaStreamTrack;
}

export interface VoiceCallOptions {
  client: AspenClient;
  media: VoiceMedia;
  WebSocket?: typeof globalThis.WebSocket;
  fetch?: typeof globalThis.fetch;
  setTimeout?: typeof globalThis.setTimeout;
  now?: () => number;
  /** Uniform in [0, 1); seeds the rejoin and resume delays. */
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
  held: false,
  onHold: [],
  serverMuted: false,
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
  stateRefused: null,
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

/**
 * This device could not set the call up: the browser lacks WebRTC or a codec the voice server
 * offers, or could not apply an offer. `cause` is what it threw. No voice server is at fault,
 * so none is reported and no other is tried.
 */
export class CallSetupError extends Error {
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : String(cause), { cause });
    this.name = "CallSetupError";
  }
}

/**
 * Whether `error` is the voice servers offered failing to take the call (none reached, or the
 * token refused), rather than the join offer itself refused, the microphone, this device
 * failing to set the call up, a request turned away, or a server not reached over HTTPS, none
 * of which another offer would change.
 */
function isServerFailure(error: unknown): boolean {
  return (
    !(error instanceof ApiProblemError) &&
    !(error instanceof InsecureAddressError) &&
    failure(error).errorKind === "server"
  );
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
          : error instanceof CallSetupError
            ? "device"
            : "server",
    retryAfterSeconds: error instanceof VoiceRequestRefused ? error.retryAfterSeconds : null,
    error: error instanceof Error ? error.message : String(error),
  };
}

/** A source this client asked to send from the browser: its track and how it is encoded. */
interface Sent {
  track: MediaStreamTrack | null;
  readonly encoding: Encoding | null;
}

/** Received audio playing: an audio slot, or the sound of someone's share. */
type Heard =
  | { readonly carries: "audioSlot"; readonly slot: number }
  | {
      readonly carries: "shareAudio";
      readonly user: string;
    };

/** A `silence` naming no one, as the voice server starts each place in a call. */
const NO_SILENCE = JSON.stringify([[], []]);

function sameHeard(a: Heard, b: Heard): boolean {
  return a.carries === "audioSlot"
    ? b.carries === "audioSlot" && a.slot === b.slot
    : b.carries === "shareAudio" && a.user === b.user;
}

/** The answer sent to one revision of a connection's offers, sent again if it is offered again. */
interface Answered {
  readonly revision: number;
  readonly sdp: string;
}

/** The two video sources a participant shows others. */
function isVideo(source: MediaSource): source is "screen" | "camera" {
  return source === "screen" || source === "camera";
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
  /** The signalling URL of the server the call is on, which a resume opens again. */
  #signalUrl: string | null = null;
  #lastReady: Ready | null = null;
  #connection: CallConnection | null = null;
  /**
   * Offers are applied one at a time, in the order they came: each waits for the one before to
   * be answered, as the server waits for each answer before its next offer.
   */
  #negotiation: Promise<void> = Promise.resolve();
  #answered: { call: Answered | null; capture: Answered | null } = { call: null, capture: null };
  /** What each section of the call connection carries, by mid, as its latest offer said. */
  #sections = new Map<string, Section>();
  /** The received audio playing, by its section's mid, which is also its id in `VoiceMedia`. */
  readonly #heard = new Map<string, Heard>();
  /** Whom each audio slot carries, as `slots` said; `null` leaves it silent. */
  readonly #slots = new Map<number, string | null>();
  /** The sources this client asked to send from the browser. */
  readonly #sent = new Map<MediaSource, Sent>();
  /** Sources asked for that the server has not yet said are sent on (`sending`). */
  readonly #pending = new Set<MediaSource>();
  /** What each participant sends, this client included, as `ready` and `sources` say. */
  readonly #sources = new Map<string, Set<MediaSource>>();
  /** How many places show video now (`showVideo`); none, and none is watched. */
  #videoShown = 0;
  /** The last `watch` sent, as JSON, so an unchanged one is not sent again. */
  #watching = "[]";
  /** Who is in the call besides this client, as `ready` and `participantJoined` say. */
  readonly #participants = new Set<string>();
  /**
   * The volumes `setUserVolume` set, by user and source, standing over `userVolume` until
   * `refreshVolumes` reads it again.
   */
  readonly #volumes = new Map<string, number>();
  /** The last `silence` sent, as JSON, so an unchanged one is not sent again. */
  #silencing = NO_SILENCE;
  /** Whether the connection has failed and an ICE restart was asked for, until it connects. */
  #restarting = false;
  /** Rejects the join in progress when an offer cannot be applied. */
  #setupFailed: ((error: unknown) => void) | null = null;
  #microphone: MediaStreamTrack | null = null;
  /**
   * The mute, deafen, and hold the user last asked for. In a call, `muted`, `deafened`, and
   * `held` show the voice server's word, except that they show the user silenced the moment
   * they ask to be: muting, deafening, and holding take effect at once (the microphone's track
   * is disabled here too, and the server never refuses them), while lifting them shows only
   * once the server says it did, so a refused one never shows the user audible when they are
   * not. Outside a call it is the state itself.
   */
  #asked: Asked = { muted: false, deafened: false, held: false };
  #screen: ScreenCapture | null = null;
  /** A share sent from outside the browser. */
  #externalShare: ExternalShare | null = null;
  /**
   * The sender holding the capture connection: the external share, or the outside sound of a
   * browser screen share.
   */
  #captureSender: ExternalSender | null = null;
  /** Settles the start of the capture sender once it answers its first offer, or fails to. */
  #captureStarted: { resolve: () => void; reject: (error: unknown) => void } | null = null;
  #devices: { input: DeviceChoice; output: DeviceChoice; camera: DeviceChoice } = {
    input: DEFAULT_DEVICE,
    output: DEFAULT_DEVICE,
    camera: DEFAULT_DEVICE,
  };
  /** The gain the microphone is sent at, from `setInputVolume`. */
  #inputVolume = 1;
  #camera: { track: MediaStreamTrack } | null = null;
  /** Counts moves of the camera to another, so a move overtaken by a later one gives way. */
  #cameraSwaps = 0;
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
        this.#send(frame);
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
    if (this.#state.status !== "connected") {
      this.#asked = this.#shown();
    }
    this.#holdMicrophone();
    for (const listener of Array.from(this.#listeners)) {
      listener();
    }
  }

  #send(frame: ClientMessage): void {
    this.#signal?.send(frame);
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
   * Shows other participants' video, and the preview of a share sent from outside the browser,
   * until the function returned is called: the call watches every screen and camera in it, as
   * many as the voice server gives sections for, while anything shows them. Nothing showing
   * them, none is received.
   */
  showVideo(): () => void {
    this.#videoShown += 1;
    this.#updateWatch();
    let shown = true;
    return () => {
      if (shown) {
        shown = false;
        this.#videoShown -= 1;
        this.#updateWatch();
      }
    };
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
    this.#ask({ ...this.#asked, muted });
  }

  setDeafened(deafened: boolean): void {
    this.#ask({ ...this.#asked, deafened });
  }

  setHeld(held: boolean): void {
    this.#ask({ ...this.#asked, held });
  }

  /** The mute, deafen, and hold the state shows now. */
  #shown(): Asked {
    return { muted: this.#state.muted, deafened: this.#state.deafened, held: this.#state.held };
  }

  /**
   * Follows a moderator's mute of the user in the call's community being made or lifted, as
   * its event says; the voice server applies it to the call itself.
   */
  setServerMuted(serverMuted: boolean): void {
    if (this.#state.status !== "idle" && this.#state.serverMuted !== serverMuted) {
      this.#set({ serverMuted });
    }
  }

  /** Forgets a refusal to unmute or undeafen once the user has seen it. */
  clearStateRefusal(): void {
    if (this.#state.stateRefused !== null) {
      this.#set({ stateRefused: null });
    }
  }

  /**
   * Asks for `asked` as the user's mute, deafen, and hold. Outside a call it simply is the
   * state; in one, whatever it silences shows at once and whatever it lifts waits for the voice
   * server.
   */
  #ask(asked: Asked): void {
    if (this.#state.status !== "connected") {
      this.#set({ ...asked, stateRefused: null });
      return;
    }
    this.#asked = asked;
    this.#set({
      muted: this.#state.muted || asked.muted,
      deafened: this.#state.deafened || asked.deafened,
      held: this.#state.held || asked.held,
      stateRefused: null,
    });
    this.#sendState();
  }

  /** Keeps the microphone's track silent while the user is shown muted or on hold. */
  #holdMicrophone(): void {
    const enabled = !this.#state.muted && !this.#state.held;
    if (this.#microphone !== null && this.#microphone.enabled !== enabled) {
      this.#microphone.enabled = enabled;
    }
  }

  /**
   * Shares the screen: asks the browser for a capture and sends the picture as `screen` and any
   * audio the browser captured with it as `screenAudio`. Ends on its own when the user stops
   * the capture from the browser's own control. `prepared` shares a capture already made
   * instead of asking the browser. `audio` replaces the browser's sound with sound sent from
   * outside it, as `screenAudio`. `contentHint` tells the encoder what the picture is: `motion`
   * (a game or video) keeps the frame rate and gives up resolution when bandwidth runs short,
   * where the browser's default for a screen keeps the resolution and drops frames.
   */
  async startScreenShare(
    options: {
      prepared?: ScreenCapture;
      audio?: ExternalAudio;
      contentHint?: "motion" | "detail" | "text";
    } = {},
  ): Promise<void> {
    const { prepared, audio, contentHint } = options;
    const signal = this.#signal;
    // Nothing is shared outside a call, into a call that does not allow it, or twice.
    if (
      this.#state.status !== "connected" ||
      !this.#state.canShare ||
      signal === null ||
      this.#screen !== null ||
      this.#externalShare !== null
    ) {
      prepared?.video.stop();
      prepared?.audio?.stop();
      return;
    }
    const generation = this.#generation;
    const capture = prepared ?? (await this.#media.getScreen());
    // The picker took time; the call may have moved on or another share started meanwhile.
    if (generation !== this.#generation || this.#sharing()) {
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
    this.#startSending("screen", { track: capture.video, encoding: SCREEN_ENCODING });
    if (audio !== undefined) {
      const started = this.#startCapture(audio);
      this.#pending.add("screenAudio");
      this.#send({ type: "send", source: "screenAudio", external: true });
      await started;
    } else if (capture.audio !== null) {
      this.#startSending("screenAudio", { track: capture.audio, encoding: null });
    }
  }

  /**
   * Shares something sent from outside the browser: the voice server offers it a connection of
   * its own (the capture connection), whose offers `share` answers. Resolves once it has
   * answered the first, and rejects with what it threw when it could not, the share then
   * stopped. Its preview comes back from the server, so `localScreen` fills in once media
   * flows, while video is shown.
   */
  startExternalScreenShare(share: ExternalShare): Promise<void> {
    if (
      this.#state.status !== "connected" ||
      !this.#state.canShare ||
      this.#signal === null ||
      this.#screen !== null ||
      this.#externalShare !== null
    ) {
      return Promise.resolve();
    }
    this.#externalShare = share;
    const started = this.#startCapture(share);
    this.#set({ sharingScreen: true });
    this.#pending.add("screen");
    this.#send({ type: "send", source: "screen", external: true });
    if (share.audio) {
      this.#pending.add("screenAudio");
      this.#send({ type: "send", source: "screenAudio", external: true });
    }
    this.#updateWatch();
    return started;
  }

  /** Makes `sender` the capture connection's, settled as `#captureStarted` says. */
  #startCapture(sender: ExternalSender): Promise<void> {
    this.#captureSender = sender;
    return new Promise((resolve, reject) => {
      this.#captureStarted = { resolve, reject };
    });
  }

  /** Whether a browser capture is being shared; read after an await, where narrowing is stale. */
  #sharing(): boolean {
    return this.#screen !== null;
  }

  /** Stops sharing the screen, if sharing. Safe to call at any time. */
  stopScreenShare(): void {
    const sender = this.#captureSender;
    if (sender !== null) {
      this.#captureSender = null;
      this.#answered.capture = null;
      // A share stopped before its sender started has nothing to report.
      this.#captureStarted?.resolve();
      this.#captureStarted = null;
      sender.stop();
    }
    const external = this.#externalShare;
    if (external !== null) {
      this.#externalShare = null;
      this.#stopSending("screen");
      if (external.audio) {
        this.#stopSending("screenAudio");
      }
      this.#set({ sharingScreen: false, localScreen: null });
      this.#updateWatch();
    }
    const capture = this.#screen;
    if (capture === null) {
      return;
    }
    this.#screen = null;
    this.#stopSending("screen");
    this.#stopSending("screenAudio");
    capture.video.stop();
    capture.audio?.stop();
    this.#set({ sharingScreen: false, localScreen: null });
  }

  /**
   * Turns the camera on: opens the chosen camera and sends it as `camera`, at up to 1080p and
   * 30 frames a second (`CAMERA_ENCODING`). Ends on its own if the camera goes away. Nothing
   * happens outside a call, in one that does not allow it, or while it is on. A failure to open
   * it is kept as `cameraError` and thrown as a `CameraError`; the voice server refusing it sets
   * `cameraError` to `unsent`.
   */
  async startCamera(): Promise<void> {
    if (
      this.#state.status !== "connected" ||
      !this.#state.canCamera ||
      this.#signal === null ||
      this.#camera !== null
    ) {
      return;
    }
    const generation = this.#generation;
    let track: MediaStreamTrack;
    try {
      track = await this.#media.getCamera(this.#devices.camera);
    } catch (error) {
      const failure = error instanceof CameraError ? error : new CameraError("failed", error);
      if (generation === this.#generation) {
        this.#set({ cameraError: failure.failure });
      }
      throw failure;
    }
    // Opening the camera took time; the call may have moved on, or it was turned on meanwhile.
    if (generation !== this.#generation || this.#cameraOn()) {
      track.stop();
      return;
    }
    const camera = { track };
    this.#camera = camera;
    this.#set({ localCamera: track, cameraError: null });
    this.#endWithTrack(camera, track);
    this.#startSending("camera", { track, encoding: CAMERA_ENCODING });
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
    this.#stopSending("camera");
    camera.track.stop();
    this.#set({ localCamera: null });
  }

  /**
   * Turns the camera off when `track` ends on its own (the camera unplugged, or taken by the
   * system), while `track` is the one it sends; a track it has moved on from ends harmlessly.
   */
  #endWithTrack(camera: { track: MediaStreamTrack }, track: MediaStreamTrack): void {
    track.addEventListener("ended", () => {
      if (this.#camera === camera && camera.track === track) {
        this.stopCamera();
      }
    });
  }

  /**
   * Moves a camera that is on to the camera now chosen, keeping it on. The camera in use is let
   * go before the next opens: a phone opens one of its cameras at a time, refusing a second or,
   * on iOS, ending the first one's track, so the picture pauses until the next camera's
   * arrives. Only the latest of several moves at once takes effect. Where no camera opens, the
   * camera turns off and `cameraError` says why.
   */
  async #swapCamera(): Promise<void> {
    const camera = this.#camera;
    if (camera === null) {
      return;
    }
    this.#cameraSwaps += 1;
    const swap = this.#cameraSwaps;
    const current = () => this.#camera === camera && this.#cameraSwaps === swap;
    camera.track.stop();
    let track: MediaStreamTrack;
    try {
      track = await this.#media.getCamera(this.#devices.camera);
    } catch (error) {
      if (current()) {
        this.stopCamera();
        this.#set({
          cameraError: error instanceof CameraError ? error.failure : "failed",
        });
      }
      return;
    }
    if (!current()) {
      track.stop();
      return;
    }
    camera.track = track;
    this.#endWithTrack(camera, track);
    try {
      await this.#replaceSent("camera", track);
    } catch {
      if (this.#camera === camera) {
        this.stopCamera();
        this.#set({ cameraError: "unsent" });
      }
      return;
    }
    if (current()) {
      this.#set({ localCamera: track });
    }
  }

  /**
   * The devices voice chat uses, from the user's preferences. Takes effect at once, in a call
   * or not: the microphone sent swaps to the new input and playback moves to the new output.
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
    const sent = this.#sent.get("microphone");
    if (!sameChoice(devices.input, previous.input) && sent !== undefined) {
      const track = await this.#media.getMicrophone(devices.input, this.#inputVolume);
      // The call may have ended or been rejoined while the microphone opened.
      if (this.#sent.get("microphone") !== sent) {
        track.stop();
        return;
      }
      // Silent from its first sample when the user is muted.
      track.enabled = !this.#state.muted && !this.#state.held;
      await this.#replaceSent("microphone", track);
      this.#microphone?.stop();
      this.#microphone = track;
      this.#holdMicrophone();
    }
  }

  /**
   * How loud the user's microphone is sent, from their preference: a gain, 1 as captured.
   * Takes effect at once on a microphone open now, and on every one opened after.
   */
  setInputVolume(gain: number): void {
    this.#inputVolume = gain;
    this.#media.setInputVolume(gain);
  }

  /** Sets every received sound's gain again from `userVolume`, after what it answers has changed. */
  refreshVolumes(): void {
    this.#volumes.clear();
    for (const mid of this.#heard.keys()) {
      this.#applyVolume(mid);
    }
    this.#updateSilence();
  }

  /**
   * Sets how loud `userId`'s voice, or with `source` `screenAudio` the sound of what they share,
   * is heard right now; the preference behind it is the caller's to keep.
   */
  setUserVolume(userId: string, gain: number, source: AudioSource = "microphone"): void {
    this.#volumes.set(`${userId} ${source}`, gain);
    for (const [mid, heard] of this.#heard) {
      if (this.#heardUser(heard) === userId && this.#heardSource(heard) === source) {
        this.#media.setVolume(mid, gain);
      }
    }
    this.#updateSilence();
  }

  /** How loud `userId`'s `source` is heard: what `setUserVolume` set, else `userVolume`. */
  #volumeOf(userId: string, source: AudioSource): number {
    return this.#volumes.get(`${userId} ${source}`) ?? this.#userVolume(userId, source);
  }

  /**
   * Tells the voice server whom this client plays at nothing (blocked, or turned all the way
   * down), when that changed: it then sends none of their voice or their share's sound, so none
   * plays even for the moment before a slot's volume follows its new occupant.
   */
  #updateSilence(): void {
    if (this.#signal === null) {
      return;
    }
    const silenced = (source: AudioSource) =>
      [...this.#participants].filter((user) => this.#volumeOf(user, source) === 0).sort(compareIds);
    const voices = silenced("microphone");
    const shares = silenced("screenAudio");
    const key = JSON.stringify([voices, shares]);
    if (key !== this.#silencing) {
      this.#silencing = key;
      this.#send({ type: "silence", voices, shares });
    }
  }

  /** Shows `user` as on hold or not, as the voice server says they are. */
  #setOnHold(user: string, held: boolean): void {
    if (this.#state.onHold.includes(user) === held) {
      return;
    }
    const onHold = held
      ? [...this.#state.onHold, user].sort(compareIds)
      : this.#state.onHold.filter((other) => other !== user);
    this.#set({ onHold });
  }

  /** Whose sound a received section carries now; `null` for a silent slot. */
  #heardUser(heard: Heard): string | null {
    return heard.carries === "audioSlot" ? (this.#slots.get(heard.slot) ?? null) : heard.user;
  }

  #heardSource(heard: Heard): AudioSource {
    return heard.carries === "audioSlot" ? "microphone" : "screenAudio";
  }

  /** Sets a received sound's gain from `userVolume`, for whoever it carries now. */
  #applyVolume(mid: string): void {
    const heard = this.#heard.get(mid);
    if (heard === undefined) {
      return;
    }
    const user = this.#heardUser(heard);
    this.#media.setVolume(mid, user === null ? 1 : this.#volumeOf(user, this.#heardSource(heard)));
  }

  #sendState(): void {
    if (this.#state.status === "connected") {
      this.#send({
        type: "setState",
        muted: this.#asked.muted,
        deafened: this.#asked.deafened,
        held: this.#asked.held,
      });
    }
  }

  /** Asks the voice server to receive `source` from the browser, sent as `sent` says. */
  #startSending(source: MediaSource, sent: Sent): void {
    this.#sent.set(source, sent);
    this.#pending.add(source);
    this.#send({ type: "send", source });
  }

  /** Ends sending `source`, if it is sent; the server takes its section away in its next offer. */
  #stopSending(source: MediaSource): void {
    const browser = this.#sent.get(source);
    this.#sent.delete(source);
    const transceiver = this.#sendingTransceiver(source);
    if (browser !== undefined && transceiver !== null) {
      void transceiver.sender.replaceTrack(null).catch(() => undefined);
    }
    this.#pending.delete(source);
    this.#send({ type: "stopSending", source });
  }

  /** Sends `track` as `source` from now on, without renegotiating. */
  async #replaceSent(source: MediaSource, track: MediaStreamTrack): Promise<void> {
    const sent = this.#sent.get(source);
    if (sent === undefined) {
      return;
    }
    sent.track = track;
    await this.#sendingTransceiver(source)?.sender.replaceTrack(track);
  }

  /** The transceiver of the call connection's section that `source` is sent on, once offered. */
  #sendingTransceiver(source: MediaSource): CallTransceiver | null {
    for (const [mid, section] of this.#sections) {
      if (section.carries === "sending" && section.source === source) {
        return this.#transceiver(mid);
      }
    }
    return null;
  }

  #transceiver(mid: string): CallTransceiver | null {
    return this.#connection?.getTransceivers().find((t) => t.mid === mid) ?? null;
  }

  /**
   * Called by the sync layer with every `voiceSessionEnded` event. The call's own end by a lost
   * or removed server triggers a rejoin; an idle end is surfaced to the user. The end of any
   * other session of the channel, as of the dead one a rejoin has already left for the session
   * the new offer made, changes nothing.
   */
  onSessionEnded(event: { id: string; channel: string; reason: VoiceSessionEndReason }): void {
    if (
      event.channel !== this.#state.channelId ||
      event.id !== this.#state.session ||
      this.#state.status === "idle"
    ) {
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

  /**
   * Rejoins the current channel after a random pause of at most `REJOIN_DELAY_MAX_MS`. With
   * `serverSilent`, the server went away without a word (its socket closed and would not
   * resume), so the API server may still offer it, not yet knowing it is gone: a rejoin that
   * fails to reach the offered servers then waits `DEAD_SERVER_RETRY_MS` and asks for an offer
   * once more before giving up.
   */
  #rejoin({ serverSilent = false }: { serverSilent?: boolean } = {}): void {
    const channelId = this.#state.channelId;
    if (channelId === null || this.#state.status === "rejoining") {
      return;
    }
    this.#teardown({ keepMicrophone: true });
    this.#set({ status: "rejoining", session: null });
    const generation = ++this.#generation;
    const fail = (error: unknown) => {
      this.#teardown();
      this.#set({ ...IDLE, status: "failed", channelId, ...failure(error) });
    };
    const attempt = (onFailure: (error: unknown) => void) => {
      this.#connect(channelId, generation).catch((error: unknown) => {
        if (generation === this.#generation) {
          onFailure(error);
        }
      });
    };
    const delay = Math.floor(this.#random() * REJOIN_DELAY_MAX_MS);
    this.#setTimeout(() => {
      if (generation !== this.#generation) {
        return;
      }
      attempt((error) => {
        if (!serverSilent || !isServerFailure(error)) {
          fail(error);
          return;
        }
        this.#teardown({ keepMicrophone: true });
        this.#setTimeout(() => {
          if (generation === this.#generation) {
            attempt(fail);
          }
        }, DEAD_SERVER_RETRY_MS);
      });
    }, delay);
  }

  /**
   * Takes the call's place back after its signalling socket closed on its own: the voice server
   * keeps it, media flowing, for twenty seconds, and a socket presenting `ready`'s secret takes
   * it back. The server then restates every connection's offer, and what this client asked for
   * while it was away (mute, video watched) is asked for again. A server that cannot be
   * reached again, or refuses the secret, leaves the call to be rejoined.
   */
  #resume(): void {
    const url = this.#signalUrl;
    const secret = this.#lastReady?.resume;
    const generation = this.#generation;
    if (url === null || secret === undefined) {
      this.#rejoin({ serverSilent: true });
      return;
    }
    const attempt = (left: number, delayMs: number) => {
      this.#setTimeout(() => {
        if (generation !== this.#generation) {
          return;
        }
        this.#identify(url, { type: "resume", secret }, generation)
          .then(() => {
            this.#watching = "";
            this.#updateWatch();
            this.#sendState();
          })
          .catch((error: unknown) => {
            if (generation !== this.#generation) {
              return;
            }
            // A socket that opened and was refused will not take the call back; one that did
            // not open may be a network still coming back.
            if (left > 1 && !(error instanceof VoiceRequestRefused) && !this.#refused) {
              attempt(left - 1, delayMs + Math.floor(this.#random() * RESUME_DELAY_MAX_MS));
            } else {
              this.#rejoin({ serverSilent: true });
            }
          });
      }, delayMs);
    };
    attempt(RESUME_ATTEMPTS, 0);
  }

  /** Whether the last `identify` or `resume` was answered with a fatal `error`. */
  #refused = false;

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
    for (const mid of this.#heard.keys()) {
      this.#media.stop(mid);
    }
    this.#heard.clear();
    this.#slots.clear();
    this.#sections.clear();
    this.#sent.clear();
    this.#pending.clear();
    this.#sources.clear();
    this.#watching = "[]";
    this.#participants.clear();
    this.#silencing = NO_SILENCE;
    this.#restarting = false;
    this.#answered = { call: null, capture: null };
    this.#negotiation = Promise.resolve();
    this.#setupFailed = null;
    this.#connection?.close();
    this.#connection = null;
    if (!keepMicrophone) {
      this.#microphone?.stop();
      this.#microphone = null;
    }
    if (this.#signal !== null) {
      this.#signal.send({ type: "leave" });
      this.#signal.close();
      this.#signal = null;
    }
    this.#signalUrl = null;
    this.#lastReady = null;
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
      serverMuted: offer.serverMuted ?? false,
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
        microphone = await this.#media.getMicrophone(this.#devices.input, this.#inputVolume);
      } catch (error) {
        throw new MicrophoneError(error);
      }
      // A join that took over meanwhile opens its own.
      if (generation !== this.#generation) {
        microphone.stop();
        return;
      }
      this.#microphone = microphone;
      this.#holdMicrophone();
    }
    // A server not reached over HTTPS is never tried, nor reported as failing: it is not at fault.
    const reachable = offer.candidates.filter((candidate) => securelyReached(candidate.url));
    const refused = offer.candidates.find((candidate) => !securelyReached(candidate.url));
    if (reachable.length === 0 && refused !== undefined) {
      throw new InsecureAddressError(refused.url);
    }
    const ranked = await this.#rank(reachable);
    let lastError: Error | null = null;
    for (const { candidate } of ranked) {
      if (generation !== this.#generation) {
        return;
      }
      try {
        await this.#establish(candidate, offer.token, channelId, generation);
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
        // Whatever is not known to be the server's own (the browser without WebRTC or a codec
        // the server offers, an offer it could not apply) is this device's, and would meet
        // every server alike: none is reported and no other is tried.
        if (!(error instanceof VoiceServerError)) {
          throw error instanceof CallSetupError ? error : new CallSetupError(error);
        }
        // Whatever the attempt set up (socket, connection) is released before the next
        // candidate builds its own; the microphone is kept for it.
        this.#teardown({ keepMicrophone: true });
        await this.#reportFailure(candidate.id);
      }
    }
    throw lastError ?? new VoiceServerError("no voice server could be reached");
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

  /**
   * Makes the call's peer connection, joins through the candidate, and resolves once the
   * connection is up and the microphone asked for.
   */
  async #establish(
    candidate: VoiceServerCandidate,
    token: string,
    channelId: string,
    generation: number,
  ): Promise<void> {
    // Made before identifying, so the offer that follows `ready` has a connection to answer.
    let connection: CallConnection;
    try {
      connection = await this.#media.createConnection();
    } catch (error) {
      throw new CallSetupError(error);
    }
    if (generation !== this.#generation) {
      connection.close();
      throw new Error("superseded");
    }
    this.#connection = connection;
    connection.addEventListener("connectionstatechange", () => {
      this.#connectionChanged(connection);
    });
    const setupFailed = new Promise<never>((_, reject) => {
      this.#setupFailed = reject;
    });
    // Nothing waits on it once the join is over; a failure then rejoins (`#negotiationFailed`).
    setupFailed.catch(() => undefined);
    await this.#identify(signallingUrl(candidate), { type: "identify", token }, generation);
    this.#set({ session: this.#lastReady?.session ?? null, channelId });
    if (this.#state.canSpeak) {
      if (this.#microphone === null) {
        throw new Error("microphone missing");
      }
      this.#startSending("microphone", { track: this.#microphone, encoding: null });
    }
    // The server accepting the call says nothing about media: ICE runs after the signalling,
    // and fails when the server announces an address this browser cannot reach.
    await Promise.race([this.#awaitConnected(connection), setupFailed]);
    if (generation !== this.#generation) {
      throw new Error("superseded");
    }
    this.#setupFailed = null;
    this.#set({ status: "connected", error: null });
    this.#sendState();
  }

  /**
   * Opens a socket to `url` and presents `first` (`identify` with a token, or `resume` with a
   * secret); resolves once `ready` arrives, with the socket the call's.
   */
  async #identify(url: string, first: ClientMessage, generation: number): Promise<void> {
    this.#refused = false;
    const signal = new Signal(url, this.#Socket);
    const ready = signal.next((f) => f.type === "ready" || (f.type === "error" && f.fatal));
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = this.#setTimeout(() => {
        reject(new VoiceServerError("the voice server did not answer in time"));
      }, READY_TIMEOUT_MS);
    });
    try {
      await signal.open();
      signal.send(first);
      const frame = await Promise.race([ready, timeout]);
      if (frame.type !== "ready") {
        this.#refused = true;
        // Turned away for joining too fast is a refusal to wait out, not the server failing.
        if (frame.type === "error" && frame.retryAfterSeconds != null) {
          throw new VoiceRequestRefused(frame.detail, frame.retryAfterSeconds);
        }
        throw new VoiceServerError(frame.type === "error" ? frame.detail : "unexpected reply");
      }
      if (generation !== this.#generation) {
        signal.close();
        throw new Error("superseded");
      }
      this.#signal = signal;
      this.#signalUrl = url;
      this.#lastReady = frame;
      this.#files.reset(frame);
      this.#sources.clear();
      this.#participants.clear();
      for (const participant of frame.participants) {
        this.#participants.add(participant.user);
        this.#sources.set(participant.user, new Set(participant.sources.map((s) => s.source)));
      }
      this.#set({
        onHold: frame.participants
          .filter((participant) => participant.held === true)
          .map((participant) => participant.user)
          .sort(compareIds),
      });
      signal.onClose = () => {
        // The socket closed without a word: the server keeps the call a while for a resume.
        if (this.#signal === signal && this.#state.status === "connected") {
          this.#signal = null;
          this.#resume();
        }
      };
      signal.onFrame = (f) => {
        this.#onFrame(f);
      };
      // What is shown already (a call rejoined, or another joined from the call screen) is
      // watched from what `ready` says is sent, and whom the user silenced is said: a new place
      // in a call silences no one, and a resumed one keeps what was said before.
      this.#updateWatch();
      if (first.type === "identify") {
        this.#silencing = NO_SILENCE;
      }
      this.#updateSilence();
    } catch (error) {
      signal.close();
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }

  /** Resolves once the connection's ICE and DTLS are up; rejects when they fail or time out. */
  #awaitConnected(connection: CallConnection): Promise<void> {
    if (connection.connectionState === "connected") {
      return Promise.resolve();
    }
    return new Promise((resolve, reject) => {
      const timer = this.#setTimeout(() => {
        reject(new VoiceServerError("media could not reach the voice server in time"));
      }, CONNECT_TIMEOUT_MS);
      connection.addEventListener("connectionstatechange", () => {
        const state = connection.connectionState;
        if (state === "connected") {
          clearTimeout(timer);
          resolve();
        } else if (state === "failed" || state === "closed") {
          clearTimeout(timer);
          reject(new VoiceServerError("media could not reach the voice server"));
        }
      });
    });
  }

  /**
   * Follows the connection mid-call. One that is disconnected or has failed asks for new ICE
   * credentials every `ICE_RESTART_INTERVAL_MS` until it connects: a restart tried while the
   * network is still away fails in its turn, and a browser never checks a failed generation
   * again, so the one asked for after the network returns is what reconnects. One that fails at
   * once asks at once; one disconnected first waits an interval, since a network that returns
   * quickly brings the connection back without one. One still not connected
   * `ICE_RECOVERY_LIMIT_MS` after it went is rejoined from scratch rather than left silent.
   */
  #connectionChanged(connection: CallConnection): void {
    if (connection !== this.#connection || this.#state.status !== "connected") {
      return;
    }
    const state = connection.connectionState;
    if (state === "connected") {
      this.#restarting = false;
      return;
    }
    if ((state !== "failed" && state !== "disconnected") || this.#restarting) {
      return;
    }
    this.#restarting = true;
    const generation = this.#generation;
    const lost = this.#now();
    const unchanged = () =>
      generation === this.#generation &&
      this.#connection === connection &&
      connection.connectionState !== "connected";
    const retry = () => {
      if (!unchanged()) {
        return;
      }
      if (this.#now() - lost >= ICE_RECOVERY_LIMIT_MS) {
        this.#rejoin();
        return;
      }
      this.#send({ type: "restartIce", connection: "call" });
      this.#setTimeout(retry, ICE_RESTART_INTERVAL_MS);
    };
    if (state === "failed") {
      retry();
    } else {
      this.#setTimeout(retry, ICE_RESTART_INTERVAL_MS);
    }
  }

  /** Answers an offer of the call connection, after every offer before it. */
  #applyOffer(frame: Offer): void {
    const connection = this.#connection;
    const signal = this.#signal;
    if (connection === null || signal === null) {
      return;
    }
    this.#negotiation = this.#negotiation.then(async () => {
      if (connection !== this.#connection) {
        return;
      }
      const answered = this.#answered.call;
      // An offer restated on resuming, already answered before the socket closed.
      if (answered?.revision === frame.revision) {
        signal.send({ type: "answer", connection: "call", ...answered });
        return;
      }
      try {
        await connection.setRemoteDescription({ type: "offer", sdp: frame.sdp });
        this.#sections = new Map(frame.sections.map((section) => [section.mid, section]));
        const encodings: [CallTransceiver, Encoding][] = [];
        for (const transceiver of connection.getTransceivers()) {
          const section =
            transceiver.mid === null ? undefined : this.#sections.get(transceiver.mid);
          if (section?.carries !== "sending") {
            continue;
          }
          // The server receives on a section it offers for what this client sends.
          transceiver.direction = "sendonly";
          const sent = this.#sent.get(section.source);
          const track = sent?.track ?? null;
          if (transceiver.sender.track !== track) {
            await transceiver.sender.replaceTrack(track);
          }
          if (sent?.encoding != null) {
            encodings.push([transceiver, sent.encoding]);
          }
        }
        await connection.setLocalDescription(await connection.createAnswer());
        const sdp = connection.localDescription?.sdp;
        if (sdp === undefined) {
          throw new Error("no answer");
        }
        // An encoding is set once the answer is applied, which makes the sender's.
        for (const [transceiver, encoding] of encodings) {
          await this.#encode(transceiver, encoding);
        }
        if (connection !== this.#connection) {
          return;
        }
        this.#answered.call = { revision: frame.revision, sdp };
        this.#signal?.send({ type: "answer", connection: "call", revision: frame.revision, sdp });
        this.#received();
      } catch (error) {
        this.#negotiationFailed(error);
      }
    });
  }

  /** Bounds what a sender's encoder may spend, as `encoding` says. */
  async #encode(transceiver: CallTransceiver, encoding: Encoding): Promise<void> {
    const parameters = transceiver.sender.getParameters();
    const [first, ...rest] = parameters.encodings;
    if (
      first === undefined ||
      (first.maxBitrate === encoding.maxBitrate && first.maxFramerate === encoding.maxFramerate)
    ) {
      return;
    }
    await transceiver.sender.setParameters({
      ...parameters,
      encodings: [
        { ...first, maxBitrate: encoding.maxBitrate, maxFramerate: encoding.maxFramerate },
        ...rest,
      ],
    });
  }

  /**
   * An offer this device could not apply. While joining, the join fails as this device's
   * (`CallSetupError`); in a call, the call is rejoined, since nothing more can be negotiated
   * on this connection.
   */
  #negotiationFailed(error: unknown): void {
    const setupFailed = this.#setupFailed;
    if (setupFailed !== null) {
      this.#setupFailed = null;
      setupFailed(new CallSetupError(error));
    } else if (this.#state.status === "connected") {
      this.#rejoin();
    }
  }

  /** Answers an offer of the capture connection through the sender outside the browser. */
  #applyCaptureOffer(frame: Offer): void {
    const sender = this.#captureSender;
    if (sender === null) {
      return;
    }
    const answered = this.#answered.capture;
    if (answered?.revision === frame.revision) {
      this.#send({ type: "answer", connection: "capture", ...answered });
      return;
    }
    sender
      .answer({ revision: frame.revision, sdp: frame.sdp, sections: frame.sections })
      .then((sdp) => {
        if (this.#captureSender === sender) {
          this.#answered.capture = { revision: frame.revision, sdp };
          this.#send({ type: "answer", connection: "capture", revision: frame.revision, sdp });
          this.#captureStarted?.resolve();
          this.#captureStarted = null;
        }
      })
      .catch((error: unknown) => {
        if (this.#captureSender === sender) {
          const started = this.#captureStarted;
          this.#captureStarted = null;
          this.stopScreenShare();
          started?.reject(error);
        }
      });
  }

  /**
   * Follows what the call connection receives, as its latest offer said: plays each audio slot
   * and share's sound it newly carries and stops each it no longer does, and shows the video it
   * carries, keeping each screen and camera already shown in its place.
   */
  #received(): void {
    const connection = this.#connection;
    if (connection === null) {
      return;
    }
    const self = this.#lastReady?.user;
    for (const [mid, section] of this.#sections) {
      let heard: Heard;
      if (section.carries === "audioSlot") {
        heard = { carries: "audioSlot", slot: section.slot };
      } else if (section.carries === "shareAudio") {
        heard = { carries: "shareAudio", user: section.user };
      } else {
        continue;
      }
      const before = this.#heard.get(mid);
      if (before !== undefined) {
        // A section the server took over for other sound plays on, at the new sound's volume.
        if (!sameHeard(before, heard)) {
          this.#heard.set(mid, heard);
          this.#applyVolume(mid);
        }
        continue;
      }
      const transceiver = this.#transceiver(mid);
      if (transceiver === null) {
        continue;
      }
      this.#heard.set(mid, heard);
      this.#media.play(mid, transceiver.receiver.track);
      this.#applyVolume(mid);
    }
    for (const mid of [...this.#heard.keys()]) {
      if (!this.#sections.has(mid)) {
        this.#heard.delete(mid);
        this.#media.stop(mid);
      }
    }
    const shown = (source: "screen" | "camera", held: readonly RemoteScreen[]) => {
      // A section the server took over for someone else's video is theirs from now on.
      const kept = held.filter((video) => {
        const section = this.#sections.get(video.id);
        return (
          section?.carries === "video" && section.source === source && section.user === video.user
        );
      });
      for (const [mid, section] of this.#sections) {
        if (
          section.carries !== "video" ||
          section.source !== source ||
          section.user === self ||
          kept.some((video) => video.id === mid)
        ) {
          continue;
        }
        const transceiver = this.#transceiver(mid);
        if (transceiver !== null) {
          kept.push({ user: section.user, id: mid, track: transceiver.receiver.track });
        }
      }
      return kept;
    };
    const preview =
      this.#externalShare === null
        ? null
        : [...this.#sections].find(
            ([, section]) =>
              section.carries === "video" && section.source === "screen" && section.user === self,
          )?.[0];
    this.#set({
      screens: shown("screen", this.#state.screens),
      cameras: shown("camera", this.#state.cameras),
      ...(this.#externalShare === null
        ? {}
        : {
            localScreen:
              preview === undefined || preview === null
                ? null
                : (this.#transceiver(preview)?.receiver.track ?? null),
          }),
    });
  }

  /**
   * Asks for the video to watch, when it changed: while anything shows video, every screen and
   * camera another participant sends, up to `ready`'s `maxWatchedVideo`, and the preview of a
   * share sent from outside the browser; otherwise none.
   */
  #updateWatch(): void {
    const self = this.#lastReady?.user;
    const video: WatchedVideo[] = [];
    if (this.#videoShown > 0 && self !== undefined) {
      if (this.#externalShare !== null) {
        video.push({ user: self, source: "screen" });
      }
      for (const [user, sources] of this.#sources) {
        if (user === self) {
          continue;
        }
        for (const source of sources) {
          if (isVideo(source)) {
            video.push({ user, source });
          }
        }
      }
    }
    const limited = video.slice(0, this.#lastReady?.maxWatchedVideo ?? 0);
    const key = JSON.stringify(limited);
    if (key !== this.#watching && this.#signal !== null) {
      this.#watching = key;
      this.#send({ type: "watch", video: limited });
    }
  }

  #onFrame(frame: ServerMessage): void {
    if (this.#files.handle(frame)) {
      return;
    }
    switch (frame.type) {
      case "offer":
        if (frame.connection === "call") {
          this.#applyOffer(frame);
        } else {
          this.#applyCaptureOffer(frame);
        }
        break;
      case "sections":
        // Restated on resuming where no offer was outstanding: nothing to answer.
        if (frame.connection === "call") {
          this.#sections = new Map(frame.sections.map((section) => [section.mid, section]));
          this.#received();
        }
        break;
      case "slots":
        for (const { slot, user } of frame.assignments) {
          this.#slots.set(slot, user ?? null);
        }
        this.refreshVolumes();
        break;
      case "sending":
        this.#pending.delete(frame.source);
        break;
      case "sources":
        this.#sourcesChanged(frame.user, new Set(frame.sources.map((s) => s.source)));
        break;
      case "participantLeft":
        this.#participants.delete(frame.user);
        this.#setOnHold(frame.user, false);
        this.#sourcesChanged(frame.user, new Set());
        this.#updateSilence();
        break;
      case "participantJoined":
        // Someone already silenced (blocked, say) is silenced from their first word.
        this.#participants.add(frame.user);
        this.#updateSilence();
        break;
      case "kicked":
        if (frame.reason === "serverStopping" || frame.reason === "connectionFailed") {
          // The server is going down, or this client's media never reached it or was lost:
          // the offer names a server to rejoin.
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
        // The server's word on this client's own state, a moderator's mute included. One
        // answering an earlier request never shows the user less silenced than they have
        // since asked to be.
        if (frame.user === this.#lastReady?.user) {
          this.#set({
            muted: frame.muted || this.#asked.muted,
            deafened: frame.deafened || this.#asked.deafened,
            held: (frame.held ?? false) || this.#asked.held,
          });
        } else {
          this.#setOnHold(frame.user, frame.held ?? false);
        }
        break;
      case "error":
        if (frame.fatal) {
          this.#generation += 1;
          this.#teardown();
          this.#set({ ...IDLE, status: "failed", error: frame.detail });
        } else if (frame.refused === "setState") {
          // Only lifting a mute, deafen, or hold is ever refused: the user stays as they are
          // shown.
          this.#asked = this.#shown();
          this.#set({ stateRefused: { retryAfterSeconds: frame.retryAfterSeconds ?? null } });
        } else if (frame.refused === "send") {
          this.#sendRefused();
        }
        break;
      case "ready":
      case "speaking":
      case "fileOffered":
      case "fileWithdrawn":
      case "transferStarting":
      case "transferSignal":
      case "transferEnded":
      case "transferLinkChanged":
        // The participants and their state reach the app through the API server's events; file
        // frames are `FileTransfers`'.
        break;
      default:
        frame satisfies never;
    }
  }

  /**
   * What a participant sends changed. The video watched follows it; this client's own share
   * sent from outside the browser, gone from what it sends once it was sent on, ended at the
   * server (its capture connection failed), so it ends here too.
   */
  #sourcesChanged(user: string, sources: Set<MediaSource>): void {
    const before = this.#sources.get(user);
    if (sources.size === 0) {
      this.#sources.delete(user);
    } else {
      this.#sources.set(user, sources);
    }
    if (
      user === this.#lastReady?.user &&
      this.#externalShare !== null &&
      before?.has("screen") === true &&
      !sources.has("screen")
    ) {
      this.stopScreenShare();
    }
    this.#updateWatch();
  }

  /**
   * The voice server refused to receive a source asked for; its `error` names the request, not
   * the source, so whatever is still waiting to be sent on stops: the camera, saying so in
   * `cameraError`, or the share.
   */
  #sendRefused(): void {
    const pending = [...this.#pending];
    this.#pending.clear();
    if (pending.includes("camera")) {
      this.stopCamera();
      this.#set({ cameraError: "unsent" });
    }
    if (pending.includes("screen") || pending.includes("screenAudio")) {
      this.stopScreenShare();
    }
    if (pending.includes("microphone")) {
      this.#sent.delete("microphone");
      this.#set({
        error: new MicrophoneError("the voice server did not take the microphone").message,
      });
    }
  }

  /**
   * Follows what the client may now do in the call, as the voice server says once the
   * permissions behind it change: what it may no longer send stops (the server has already
   * stopped receiving it), and a microphone it may now send starts.
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
      this.#stopSending("microphone");
      this.#microphone?.stop();
      this.#microphone = null;
      return;
    }
    if (this.#sent.has("microphone") || this.#signal === null) {
      return;
    }
    const generation = this.#generation;
    try {
      const microphone =
        this.#microphone ??
        (await this.#media.getMicrophone(this.#devices.input, this.#inputVolume));
      if (generation !== this.#generation || this.#sent.has("microphone")) {
        if (microphone !== this.#microphone) {
          microphone.stop();
        }
        return;
      }
      this.#microphone = microphone;
      this.#holdMicrophone();
      this.#startSending("microphone", { track: microphone, encoding: null });
      this.#sendState();
    } catch (error) {
      this.#set({ error: new MicrophoneError(error).message });
    }
  }
}
