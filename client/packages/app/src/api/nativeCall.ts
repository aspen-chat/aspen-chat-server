import {
  type AudioSource,
  type Call,
  CameraError,
  type CameraFailure,
  DEFAULT_DEVICE,
  type DeviceChoice,
  type EngineCallState,
  type ExternalAudio,
  type ExternalShare,
  type FileSink,
  IDLE_CALL_STATE,
  NO_FILES,
  type RemoteScreen,
  type ScreenCapture,
  type TransferMode,
  type VideoFacing,
  type VideoSource,
  type VoiceCallListener,
  type VoiceCallOptions,
  type VoiceCallState,
  type VoiceCallStatus,
  type VoiceSessionEndReason,
  type WatchedVideo,
  compareIds,
} from "@aspen/protocol";
import { Capacitor, registerPlugin, type PluginListenerHandle } from "@capacitor/core";

/**
 * One snapshot of a deployment's engine as the plugin publishes it: the JSON the engine writes
 * of its `CallState` (`EngineCallState`, generated from `call_bridge_schema.json`), untouched,
 * with the deployment it is of, so each deployment's `NativeCall` takes its own.
 */
export interface PublishedState {
  readonly origin: string;
  readonly state: string;
}

/**
 * What a phone can play a call through, by kind: its receiver (held to the ear), its
 * loudspeaker, a Bluetooth headset or speaker, a wired headset, a car, or something else the
 * system routes to (AirPlay, a USB device, a kind this build does not know).
 */
export type RouteKind = "receiver" | "loudspeaker" | "bluetooth" | "wiredHeadset" | "car" | "other";

export interface PhoneRoute {
  readonly kind: RouteKind;
  /** The system's name for it, for display alone; `null` for the receiver and loudspeaker. */
  readonly name: string | null;
  readonly inUse: boolean;
}

/** The call's routes on this phone, as the plugin's `routes` answers and its `routes` event tells. */
export interface PhoneRoutes {
  readonly routes: readonly PhoneRoute[];
  /** Whether the device has a receiver at all: a phone has one, a tablet none. */
  readonly hasReceiver: boolean;
  /** The kind the app chose for this call, until `clearRoute` or a new device takes the call. */
  readonly chosen: RouteKind | null;
}

/** A camera the phone has, as the system lists it, with which way it faces as the system says. */
export interface PhoneCamera {
  /** The system's id for it (`AVCaptureDevice.uniqueID`, Camera2's id), by which it is chosen. */
  readonly id: string;
  /** The system's name for it, for display alone; `null` where the system has none (Android). */
  readonly label: string | null;
  readonly facing: VideoFacing | null;
}

/** A rectangle on the screen, in CSS pixels of the page. */
export interface TileRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/**
 * One tile's hole, as the page reports it (`nativeVideo.tsx`): which surface it shows, where,
 * whether it is on screen, and how the picture is drawn in it. `tile` is the page's id for the
 * tile, unique for the page's life; two tiles may show one surface.
 */
export interface TileReport {
  readonly tile: string;
  readonly surface: string;
  readonly rect: TileRect;
  readonly visible: boolean;
  readonly mirrored: boolean;
  readonly cover: boolean;
  readonly fullScreen: boolean;
  readonly pictureInPicture: boolean;
}

/**
 * The shape of a surface's picture as the native side draws it, upright: a phone's camera
 * held upright is taller than wide. `rotation` is the sender's, in degrees clockwise, already
 * applied to `width` and `height`.
 */
export interface PictureShape {
  readonly surface: string;
  readonly width: number;
  readonly height: number;
  readonly rotation: number;
}

/** The camera the plugin opened, and which way it faces. */
export interface OpenedCamera {
  readonly device: string;
  readonly facing: VideoFacing | null;
}

/**
 * The mobile apps' native side of a call (`AspenCallPlugin`, in each): the call engine for each
 * deployment the user is signed in to, which runs the call itself, its snapshots, where the
 * call plays, and the video it draws. Every method names the deployment by its `origin` (the
 * `public_url`), except the routes, the cameras, and the tiles, which are the phone's. `join`
 * asks the system for the microphone first, and joins through the system's call (CallKit,
 * Telecom); `state` is a fresh snapshot, for a page that slept through the `state` events.
 * `startCamera` asks the system for the camera and rejects with the code `denied`, `none`, or
 * `failed` (a `CameraFailure`) when it cannot open one; `setTiles` says where each picture goes
 * (`nativeVideo.tsx`), and `shape` and `pip` events come back from the pictures drawn.
 */
export interface AspenCallPlugin {
  join(options: { origin: string; channel: string; name: string | null }): Promise<void>;
  leave(options: { origin: string }): Promise<void>;
  setMuted(options: { origin: string; muted: boolean }): Promise<void>;
  setDeafened(options: { origin: string; deafened: boolean }): Promise<void>;
  setHeld(options: { origin: string; held: boolean }): Promise<void>;
  setServerMuted(options: { origin: string; serverMuted: boolean }): Promise<void>;
  setUserVolume(options: {
    origin: string;
    user: string;
    source: AudioSource;
    gain: number;
  }): Promise<void>;
  sessionEnded(options: {
    origin: string;
    session: string;
    channel: string;
    reason: VoiceSessionEndReason;
  }): Promise<void>;
  acknowledgeEnd(options: { origin: string }): Promise<void>;
  clearStateRefusal(options: { origin: string }): Promise<void>;
  state(options: { origin: string }): Promise<PublishedState>;
  routes(): Promise<PhoneRoutes>;
  setRoute(options: { kind: RouteKind }): Promise<void>;
  clearRoute(): Promise<void>;
  cameras(): Promise<{ cameras: readonly PhoneCamera[] }>;
  startCamera(options: { origin: string; device: string | null }): Promise<OpenedCamera>;
  switchCamera(options: { origin: string; device: string | null }): Promise<OpenedCamera>;
  stopCamera(options: { origin: string }): Promise<void>;
  clearCameraError(options: { origin: string }): Promise<void>;
  setWatched(options: { origin: string; video: readonly WatchedVideo[] }): Promise<void>;
  setTiles(options: { tiles: readonly TileReport[] }): Promise<void>;
  addListener(
    event: "state",
    listener: (published: PublishedState) => void,
  ): Promise<PluginListenerHandle>;
  addListener(
    event: "routes",
    listener: (routes: PhoneRoutes) => void,
  ): Promise<PluginListenerHandle>;
  addListener(
    event: "shape",
    listener: (shape: PictureShape) => void,
  ): Promise<PluginListenerHandle>;
  addListener(
    event: "pip",
    listener: (pip: { active: boolean }) => void,
  ): Promise<PluginListenerHandle>;
}

export const AspenCall = registerPlugin<AspenCallPlugin>("AspenCall");

/** Whether this is a mobile app whose calls the native call engine carries. */
export function nativeCalls(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenCall");
}

/**
 * Something a native call cannot do: screen sharing from the phone and file transfers, which
 * come in later stages (`docs/architecture/voice/native-calls.md`) and whose controls are hidden
 * by `canShare` and `canTransfer`, which a native call keeps false; a microphone or speaker
 * chosen by id, which the routes stand in for, so `hasAudioDeviceChoice` is false and the
 * selects that would ask are not shown; and a gain on the microphone, which neither phone's
 * audio device module applies, so `hasInputVolume` is false and the slider that would ask is
 * not shown.
 */
export class NativeCallUnavailableError extends Error {
  constructor(what: string) {
    super(`${what} is not part of this build's native calls yet`);
    this.name = "NativeCallUnavailableError";
  }
}

/** The engine could not join or keep the call; `kind` is the engine's `errorKind`, as the state shows. */
export class NativeCallFailedError extends Error {
  constructor(
    readonly kind: VoiceCallState["errorKind"],
    detail: string | null,
  ) {
    super(detail ?? "the call failed");
    this.name = "NativeCallFailedError";
  }
}

/**
 * The engine's snapshot holds a value this build's types lack, which a page and its engine
 * shipped together never do; or is not the JSON the engine writes.
 */
export class NativeCallProtocolError extends Error {
  constructor(field: string, value: unknown) {
    super(`the native call's ${field} is ${JSON.stringify(value)}, which this build does not know`);
    this.name = "NativeCallProtocolError";
  }
}

function statusOf(status: EngineCallState["status"]): VoiceCallStatus {
  switch (status) {
    case "idle":
    case "joining":
    case "connected":
    case "rejoining":
    case "failed":
      return status;
    default:
      throw new NativeCallProtocolError("status", status satisfies never);
  }
}

/**
 * The engine's kind as the web shows it: the API server refusing the join offer (`offer`) is
 * what `VoiceCall` classes as `server`, the Problem's text carried as `error` either way, so
 * the call bar reads the same on a phone as in a browser.
 */
function errorKindOf(kind: EngineCallState["errorKind"]): VoiceCallState["errorKind"] {
  switch (kind) {
    case null:
    case undefined:
      return null;
    case "offer":
    case "server":
      return "server";
    case "microphone":
    case "refused":
    case "device":
      return kind;
    default:
      throw new NativeCallProtocolError("errorKind", kind satisfies never);
  }
}

function endedReasonOf(reason: EngineCallState["endedReason"]): VoiceCallState["endedReason"] {
  switch (reason) {
    case undefined:
      return null;
    case null:
    case "kicked":
    case "accessLost":
    case "idle":
      return reason;
    default:
      throw new NativeCallProtocolError("endedReason", reason satisfies never);
  }
}

/**
 * What the page knows of the call's video that the engine does not: which way the camera it
 * opened faces (the plugin says as it opens it; the engine carries no facing), and why the
 * camera last failed to open on this phone (`denied`, `none`, `failed`, from the plugin's
 * refusal), until it opens, the user dismisses it, or the call ends. The voice server refusing
 * the picture is the engine's to say (`sourceRefused`).
 */
export interface LocalVideo {
  readonly facing: VideoFacing | null;
  readonly cameraFailure: CameraFailure | null;
}

export const NO_LOCAL_VIDEO: LocalVideo = { facing: null, cameraFailure: null };

/** The surface the phone draws `track` on, as the page's picture. */
function nativeSource(track: string, facing: VideoFacing | null): VideoSource {
  return { kind: "native", surface: track, facing };
}

/**
 * The videos the engine receives of `source` from others, as the page's screens or cameras:
 * each a surface named by its track, with the track as its id, in the engine's order, which
 * is the order of their sections. The user's own share, watched for its preview, is not
 * among the screens; it is `localScreen`.
 */
function remoteVideos(snapshot: EngineCallState, source: "screen" | "camera"): RemoteScreen[] {
  return snapshot.videos
    .filter((video) => video.source === source && video.user !== snapshot.user)
    .map((video) => ({
      user: video.user,
      id: video.track,
      video: nativeSource(video.track, null),
    }));
}

/** The track the engine sends as `source`, if it sends one. */
function sent(snapshot: EngineCallState, source: "screen" | "camera"): string | null {
  return snapshot.sources.find((local) => local.source === source)?.track ?? null;
}

/**
 * The call's state as the app reads it, from the engine's snapshot and what the page knows of
 * its video (`LocalVideo`). The camera's picture, the others' cameras and screens, and the
 * preview of a share are the surfaces the phone draws; the grant to use the camera is the
 * engine's; `cameraError` is the phone's refusal to open one, or the voice server's to take it
 * (`unsent`). Files and screen sharing from the phone are later stages, so those fields are the
 * idle state's.
 */
export function stateOf(
  snapshot: EngineCallState,
  local: LocalVideo = NO_LOCAL_VIDEO,
): VoiceCallState {
  const camera = sent(snapshot, "camera");
  const screen = sent(snapshot, "screen");
  const serverRefused = snapshot.sourceRefused?.source === "camera";
  return {
    ...IDLE_CALL_STATE,
    status: statusOf(snapshot.status),
    channelId: snapshot.channel ?? null,
    session: snapshot.session ?? null,
    muted: snapshot.muted,
    deafened: snapshot.deafened,
    held: snapshot.held,
    onHold: snapshot.participants
      .filter((participant) => participant.held)
      .map((participant) => participant.user)
      .sort(compareIds),
    serverMuted: snapshot.serverMuted,
    canSpeak: snapshot.grants.speak,
    canShare: false,
    canCamera: snapshot.grants.camera,
    canTransfer: false,
    files: NO_FILES,
    sharingScreen: screen !== null,
    localScreen: screen === null ? null : nativeSource(screen, null),
    screens: remoteVideos(snapshot, "screen"),
    localCamera: camera === null ? null : nativeSource(camera, local.facing),
    cameras: remoteVideos(snapshot, "camera"),
    cameraError: camera !== null ? null : serverRefused ? "unsent" : local.cameraFailure,
    errorKind: errorKindOf(snapshot.errorKind),
    retryAfterSeconds: snapshot.retryAfterSeconds ?? null,
    error: snapshot.error ?? null,
    stateRefused:
      snapshot.stateRefused == null
        ? null
        : { retryAfterSeconds: snapshot.stateRefused.retryAfterSeconds ?? null },
    endedReason: endedReasonOf(snapshot.endedReason),
  };
}

/**
 * The video the page asks the engine to watch when `snapshot` is the call: every screen and
 * camera another participant sends, in the participants' order, as `VoiceCall` asks for them;
 * the engine cuts it to `maxWatchedVideo`.
 */
export function watchedOf(snapshot: EngineCallState): WatchedVideo[] {
  const video: WatchedVideo[] = [];
  for (const participant of snapshot.participants) {
    if (participant.user === snapshot.user) {
      continue;
    }
    for (const { source } of participant.sources) {
      if (source === "screen" || source === "camera") {
        video.push({ user: participant.user, source });
      }
    }
  }
  return video;
}

/** The engine's snapshot from the JSON it wrote; what is not that is a protocol error. */
export function parseState(json: string): EngineCallState {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    throw new NativeCallProtocolError("state", json);
  }
  if (parsed === null || typeof parsed !== "object" || !("revision" in parsed)) {
    throw new NativeCallProtocolError("state", parsed);
  }
  return parsed as EngineCallState;
}

/**
 * Why the plugin could not open the camera, from the code it rejected with, each a
 * `CameraFailure` by name; a refusal without one is the camera failing to start.
 */
export function cameraFailureOf(error: unknown): CameraFailure {
  const code =
    error !== null && typeof error === "object" && "code" in error ? error.code : undefined;
  switch (code) {
    case "denied":
    case "none":
    case "failed":
      return code;
    default:
      return "failed";
  }
}

/** What besides the options every call takes a `NativeCall` is made with. */
export interface NativeCallOptions {
  /**
   * What the system shows the call as (CallKit's caller name, Telecom's display name) for a
   * channel: its name, or a DM's other people; `null` leaves it to the platform.
   */
  nameOf?: (channelId: string) => string | null;
  /** Hears a command the plugin refused, or a snapshot that could not be read; `console.error` by default. */
  report?: (error: unknown) => void;
  /**
   * Calls `listener` each time the page wakes, which is when it asks the plugin for the current
   * snapshot; by default on `visibilitychange` to visible and on Capacitor's `resume`.
   */
  onWake?: (listener: () => void) => void;
}

/** Calls `listener` when the page comes back into view or the app resumes. */
export function onPageWake(listener: () => void): void {
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      listener();
    }
  });
  void import("@capacitor/app").then(({ App }) => App.addListener("resume", listener));
}

/**
 * The user's call in a mobile app, carried by the phone's call engine: every command goes to the
 * `AspenCall` plugin for this deployment, and the state is the engine's, mirrored from whole
 * snapshots (the JSON the engine writes of its `CallState`, typed from `call_bridge_schema.json`
 * as `EngineCallState`) in the order of their revisions, so one that arrives late changes
 * nothing, with what the page alone knows of the video laid over it (`LocalVideo`). The page
 * hears each snapshot as it is published and asks for the current one as it wakes, since a page
 * the system froze missed those meanwhile. The browser's media is never touched: the microphone
 * and the camera are the engine's, opened by the plugin, and every picture is a surface the
 * phone draws (`VideoSource` of the `native` kind) behind the page (`nativeVideo.tsx`).
 *
 * What the page shows is what is watched: `showVideo` counts the places showing video, and
 * while there are any the engine is asked to watch every screen and camera the others send
 * (`watchedOf`), again whenever that changes, and none when nothing shows them.
 *
 * Screen sharing from the phone and file transfers reject with `NativeCallUnavailableError`,
 * and the grants that would show their controls stay false. The call's sounds are the engine's
 * (`playsOwnSounds`), and the microphone and speaker are the phone's and its routes'
 * (`hasAudioDeviceChoice` is false).
 */
export class NativeCall implements Call {
  readonly #origin: string;
  readonly #userVolume: (userId: string, source: AudioSource) => number;
  readonly #nameOf: (channelId: string) => string | null;
  readonly #report: (error: unknown) => void;
  readonly #listeners = new Set<VoiceCallListener>();
  #state: VoiceCallState = IDLE_CALL_STATE;
  /** The snapshot shown; `null` before the first. */
  #snapshot: EngineCallState | null = null;
  #local: LocalVideo = NO_LOCAL_VIDEO;
  /** The gain the engine was last told for each `user source`, until the call ends or volumes refresh. */
  readonly #told = new Map<string, number>();
  /** Who else the latest snapshot shows in the call. */
  #present: readonly string[] = [];
  /** How many places show video now (`showVideo`); none, and none is watched. */
  #videoShown = 0;
  /** The last `setWatched` sent, as JSON, so an unchanged one is not sent again. */
  #watching = "[]";
  /** The camera the user chose (`setAudioDevices`), by the system's id; `null` for the phone's default. */
  #cameraDevice: string | null = null;
  /** The join awaiting its outcome, settled by the snapshots that follow it. */
  #joining: {
    readonly channelId: string;
    readonly resolve: () => void;
    readonly reject: (error: unknown) => void;
  } | null = null;

  constructor(options: VoiceCallOptions, native: NativeCallOptions = {}) {
    this.#origin = options.client.baseUrl;
    this.#userVolume = options.userVolume ?? (() => 1);
    this.#nameOf = native.nameOf ?? (() => null);
    this.#report =
      native.report ??
      ((error) => {
        console.error(error);
      });
    void AspenCall.addListener("state", (published) => {
      this.#take(published);
    });
    const read = () => {
      AspenCall.state({ origin: this.#origin }).then(
        (published) => {
          this.#take(published);
        },
        (error: unknown) => {
          this.#report(error);
        },
      );
    };
    (native.onWake ?? onPageWake)(read);
    read();
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

  /** Shows the published snapshot when it is this deployment's and newer than the one shown. */
  #take(published: PublishedState): void {
    if (published.origin !== this.#origin) {
      return;
    }
    let snapshot: EngineCallState;
    try {
      snapshot = parseState(published.state);
      stateOf(snapshot, this.#local);
    } catch (error) {
      this.#report(error);
      return;
    }
    if (this.#snapshot !== null && snapshot.revision <= this.#snapshot.revision) {
      return;
    }
    this.#snapshot = snapshot;
    this.#present = snapshot.participants.map((participant) => participant.user);
    if (snapshot.status === "idle") {
      this.#told.clear();
      this.#watching = "[]";
      // A failure to open the camera is the call's to show; the next call starts clean.
      this.#local = { ...this.#local, cameraFailure: null };
    } else {
      this.#applyVolumes();
    }
    this.#show();
    this.#updateWatch();
  }

  /** Derives the state from the snapshot and the page's own knowledge, and tells the listeners. */
  #show(): void {
    if (this.#snapshot === null) {
      return;
    }
    this.#state = stateOf(this.#snapshot, this.#local);
    this.#settleJoin(this.#state);
    for (const listener of Array.from(this.#listeners)) {
      listener();
    }
  }

  #setLocal(local: Partial<LocalVideo>): void {
    this.#local = { ...this.#local, ...local };
    this.#show();
  }

  /**
   * Asks the engine for the video to watch, when that changed: while anything shows video,
   * every screen and camera another participant sends; otherwise none. Outside a call the
   * engine keeps nothing to watch, so nothing is asked.
   */
  #updateWatch(): void {
    const snapshot = this.#snapshot;
    if (snapshot?.status !== "connected") {
      return;
    }
    const video = this.#videoShown > 0 ? watchedOf(snapshot) : [];
    const key = JSON.stringify(video);
    if (key !== this.#watching) {
      this.#watching = key;
      this.#command(AspenCall.setWatched({ origin: this.#origin, video }));
    }
  }

  /** Tells the engine how loud each participant it has not yet been told of is to this user. */
  #applyVolumes(): void {
    for (const user of this.#present) {
      for (const source of ["microphone", "screenAudio"] as const) {
        if (!this.#told.has(`${user} ${source}`)) {
          this.#tell(user, source, this.#userVolume(user, source));
        }
      }
    }
  }

  #tell(userId: string, source: AudioSource, gain: number): void {
    this.#told.set(`${userId} ${source}`, gain);
    this.#command(AspenCall.setUserVolume({ origin: this.#origin, user: userId, source, gain }));
  }

  /**
   * Settles the join under way by what the engine shows of its channel: connected resolves,
   * failed rejects with the failure, and the call left or taken over by another join resolves,
   * as `VoiceCall.join` returns when a later join or a leave supersedes it.
   */
  #settleJoin(state: VoiceCallState): void {
    const joining = this.#joining;
    if (joining === null) {
      return;
    }
    const ours = state.channelId === joining.channelId;
    if (ours && (state.status === "joining" || state.status === "rejoining")) {
      return;
    }
    this.#joining = null;
    if (ours && state.status === "failed") {
      joining.reject(new NativeCallFailedError(state.errorKind, state.error));
    } else {
      joining.resolve();
    }
  }

  /** Runs a command whose outcome the state shows, reporting the plugin's refusal of it. */
  #command(sent: Promise<unknown>): void {
    sent.catch((error: unknown) => {
      this.#report(error);
    });
  }

  async join(channelId: string): Promise<void> {
    const name = this.#nameOf(channelId);
    this.#joining?.resolve();
    const settled = new Promise<void>((resolve, reject) => {
      this.#joining = { channelId, resolve, reject };
    });
    try {
      await AspenCall.join({ origin: this.#origin, channel: channelId, name });
    } catch (error) {
      if (this.#joining?.channelId === channelId) {
        this.#joining = null;
      }
      throw error;
    }
    return settled;
  }

  leave(): void {
    this.#command(AspenCall.leave({ origin: this.#origin }));
  }

  acknowledgeEnd(): void {
    this.#command(AspenCall.acknowledgeEnd({ origin: this.#origin }));
  }

  /**
   * Shows other participants' video until the function returned is called: while anything
   * shows it, the engine watches every screen and camera in the call, as many as the voice
   * server gives sections for. Nothing showing them, none is received.
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

  offerFile(_file: Blob, _name: string, _allowDirect: boolean, _validForSeconds: number): string {
    throw new NativeCallUnavailableError("Offering a file");
  }

  withdrawOffer(_offer: string): void {
    throw new NativeCallUnavailableError("Withdrawing a file offer");
  }

  acceptOffer(_offer: string, _mode: TransferMode, _sink?: FileSink): void {
    throw new NativeCallUnavailableError("Accepting a file offer");
  }

  cancelTransfer(_offer: string, _peer: string): void {
    throw new NativeCallUnavailableError("Cancelling a file transfer");
  }

  dismissTransfer(_offer: string, _peer: string): void {
    throw new NativeCallUnavailableError("Dismissing a file transfer");
  }

  setMuted(muted: boolean): void {
    this.#command(AspenCall.setMuted({ origin: this.#origin, muted }));
  }

  setDeafened(deafened: boolean): void {
    this.#command(AspenCall.setDeafened({ origin: this.#origin, deafened }));
  }

  setHeld(held: boolean): void {
    this.#command(AspenCall.setHeld({ origin: this.#origin, held }));
  }

  setServerMuted(serverMuted: boolean): void {
    this.#command(AspenCall.setServerMuted({ origin: this.#origin, serverMuted }));
  }

  clearStateRefusal(): void {
    this.#command(AspenCall.clearStateRefusal({ origin: this.#origin }));
  }

  startScreenShare(_options?: {
    prepared?: ScreenCapture;
    audio?: ExternalAudio;
    contentHint?: "motion" | "detail" | "text";
  }): Promise<void> {
    return Promise.reject(new NativeCallUnavailableError("Sharing the screen"));
  }

  startExternalScreenShare(_share: ExternalShare): Promise<void> {
    return Promise.reject(new NativeCallUnavailableError("Sharing from outside the app"));
  }

  stopScreenShare(): void {
    throw new NativeCallUnavailableError("Sharing the screen");
  }

  /**
   * Turns the camera on: the plugin asks the system for it, opens the camera chosen (or the
   * phone's default, its front camera), and gives the engine the track to send; the picture
   * shows as `localCamera` once the engine's snapshot carries the source, facing as the plugin
   * said. A refusal to open it is kept as `cameraError` and thrown as a `CameraError`; the
   * voice server refusing the picture shows as `unsent` from the engine's snapshot.
   */
  async startCamera(): Promise<void> {
    if (this.#state.status !== "connected" || !this.#state.canCamera) {
      return;
    }
    try {
      const opened = await AspenCall.startCamera({
        origin: this.#origin,
        device: this.#cameraDevice,
      });
      this.#setLocal({ facing: opened.facing, cameraFailure: null });
    } catch (error) {
      const failure = cameraFailureOf(error);
      this.#setLocal({ cameraFailure: failure });
      throw new CameraError(failure, error);
    }
  }

  /** Dismisses the reason the camera last failed, the phone's and the voice server's alike. */
  clearCameraError(): void {
    this.#setLocal({ cameraFailure: null });
    this.#command(AspenCall.clearCameraError({ origin: this.#origin }));
  }

  /** Turns the camera off, if it is on; the engine's snapshot then shows it gone. */
  stopCamera(): void {
    this.#command(AspenCall.stopCamera({ origin: this.#origin }));
  }

  /**
   * The phone's own microphone is the one sent and the route (`AspenCall.setRoute`, typed) is
   * where the call plays, so the system's default for each is what this call does; a device
   * chosen by id is a browser's choice, which the engine has no counterpart for yet. The camera
   * is chosen by the system's id, among those `AspenCall.cameras` lists: the choice is kept for
   * the next time the camera turns on, and moves a camera that is on now (`switchCamera`),
   * keeping it on; the picture pauses until the next camera's frames arrive.
   */
  async setAudioDevices(devices: {
    input: DeviceChoice;
    output: DeviceChoice;
    camera?: DeviceChoice;
  }): Promise<void> {
    const camera = devices.camera ?? DEFAULT_DEVICE;
    const device = camera === DEFAULT_DEVICE ? null : camera.id;
    if (device !== this.#cameraDevice) {
      this.#cameraDevice = device;
      if (this.#state.localCamera !== null) {
        const opened = await AspenCall.switchCamera({ origin: this.#origin, device });
        this.#setLocal({ facing: opened.facing });
      }
    }
    if ([devices.input, devices.output].some((choice) => choice !== DEFAULT_DEVICE)) {
      throw new NativeCallUnavailableError("Choosing a microphone or speaker by id");
    }
  }

  /**
   * The microphone is sent as the phone captures it: neither platform's audio device module
   * applies a gain over it, so the sync never asks, and the slider that would is not shown.
   */
  readonly hasInputVolume = false;

  setInputVolume(_gain: number): void {
    throw new NativeCallUnavailableError("A gain on the microphone");
  }

  /**
   * The engine plays the call's sounds itself (`voice_client::sounds`), through the call's own
   * audio session, so they follow the call's route and play in the background; the page plays
   * none for this call.
   */
  readonly playsOwnSounds = true;

  /**
   * The phone's one microphone is sent and the routes (`AspenCall.setRoute`) are where the call
   * plays, so there is no microphone or speaker to choose by id, and `setAudioDevices` rejects
   * one.
   */
  readonly hasAudioDeviceChoice = false;

  refreshVolumes(): void {
    this.#told.clear();
    this.#applyVolumes();
  }

  setUserVolume(userId: string, gain: number, source: AudioSource = "microphone"): void {
    this.#tell(userId, source, gain);
  }

  onSessionEnded(event: { id: string; channel: string; reason: VoiceSessionEndReason }): void {
    this.#command(
      AspenCall.sessionEnded({
        origin: this.#origin,
        session: event.id,
        channel: event.channel,
        reason: event.reason,
      }),
    );
  }
}
