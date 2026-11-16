import {
  type AudioSource,
  type Call,
  DEFAULT_DEVICE,
  type DeviceChoice,
  type ExternalAudio,
  type ExternalShare,
  type FileSink,
  IDLE_CALL_STATE,
  NO_FILES,
  type ScreenCapture,
  type TransferMode,
  type VoiceCallListener,
  type VoiceCallOptions,
  type VoiceCallState,
  type VoiceCallStatus,
  type VoiceSessionEndReason,
  compareIds,
} from "@aspen/protocol";
import { Capacitor, registerPlugin, type PluginListenerHandle } from "@capacitor/core";

/**
 * The call engine's state as the plugin writes it (`CallState` in `voice_client/src/state.rs`,
 * the fields the page reads of it): one whole snapshot, numbered by the engine, with the
 * deployment it is of, so each deployment's `NativeCall` takes its own.
 */
export interface CallSnapshot {
  readonly origin: string;
  readonly revision: number;
  readonly status: VoiceCallStatus;
  readonly channel: string | null;
  readonly session: string | null;
  readonly user: string | null;
  readonly muted: boolean;
  readonly deafened: boolean;
  readonly held: boolean;
  readonly serverMuted: boolean;
  readonly grants: {
    readonly speak: boolean;
    readonly shareScreen: boolean;
    readonly camera: boolean;
    readonly transferFiles: boolean;
  };
  readonly participants: readonly {
    readonly user: string;
    readonly muted: boolean;
    readonly deafened: boolean;
    readonly held: boolean;
    readonly speaking: boolean;
  }[];
  /**
   * The engine's kinds, one more than the web's: `offer` is the API server refusing the join
   * offer, which `VoiceCall` reports as `server`.
   */
  readonly errorKind: "offer" | "microphone" | "server" | "refused" | "device" | null;
  readonly error: string | null;
  readonly retryAfterSeconds: number | null;
  readonly stateRefused: {
    readonly detail: string;
    readonly retryAfterSeconds: number | null;
  } | null;
  readonly endedReason: "kicked" | "accessLost" | "idle" | null;
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

/**
 * The mobile apps' native side of a call (`AspenCallPlugin`, in each): the call engine for each
 * deployment the user is signed in to, which runs the call itself, its snapshots, and where the
 * call plays. Every method names the deployment by its `origin` (the `public_url`), except the
 * routes, which are the phone's. `join` asks the system for the microphone first, and joins
 * through the system's call (CallKit, Telecom); `state` is a fresh snapshot, for a page that
 * slept through the `state` events.
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
  state(options: { origin: string }): Promise<CallSnapshot>;
  routes(): Promise<PhoneRoutes>;
  setRoute(options: { kind: RouteKind }): Promise<void>;
  clearRoute(): Promise<void>;
  addListener(
    event: "state",
    listener: (snapshot: CallSnapshot) => void,
  ): Promise<PluginListenerHandle>;
  addListener(
    event: "routes",
    listener: (routes: PhoneRoutes) => void,
  ): Promise<PluginListenerHandle>;
}

export const AspenCall = registerPlugin<AspenCallPlugin>("AspenCall");

/** Whether this is a mobile app whose calls the native call engine carries. */
export function nativeCalls(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenCall");
}

/**
 * Something this build's native calls cannot do yet: they carry voice alone, and the camera,
 * screen sharing, file transfers, a microphone other than the phone's own or a gain on it come
 * in later stages (`docs/architecture/voice/native-calls.md`). The controls that would ask are
 * hidden by `canCamera`, `canShare`, and `canTransfer`, which a native call keeps false.
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

/** The plugin reported a value this build's types lack, which a page and its plugin shipped together never do. */
export class NativeCallProtocolError extends Error {
  constructor(field: string, value: unknown) {
    super(`the native call's ${field} is ${JSON.stringify(value)}, which this build does not know`);
    this.name = "NativeCallProtocolError";
  }
}

function statusOf(status: CallSnapshot["status"]): VoiceCallStatus {
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
function errorKindOf(kind: CallSnapshot["errorKind"]): VoiceCallState["errorKind"] {
  switch (kind) {
    case null:
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

function endedReasonOf(reason: CallSnapshot["endedReason"]): VoiceCallState["endedReason"] {
  switch (reason) {
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
 * The call's state as the app reads it, from the engine's snapshot. Voice alone in this stage:
 * no file, screen, or camera is ever offered or shown, so those fields are the idle state's.
 */
export function stateOf(snapshot: CallSnapshot): VoiceCallState {
  return {
    ...IDLE_CALL_STATE,
    status: statusOf(snapshot.status),
    channelId: snapshot.channel,
    session: snapshot.session,
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
    canCamera: false,
    canTransfer: false,
    files: NO_FILES,
    errorKind: errorKindOf(snapshot.errorKind),
    retryAfterSeconds: snapshot.retryAfterSeconds,
    error: snapshot.error,
    stateRefused:
      snapshot.stateRefused === null
        ? null
        : { retryAfterSeconds: snapshot.stateRefused.retryAfterSeconds },
    endedReason: endedReasonOf(snapshot.endedReason),
  };
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
 * snapshots in the order of their revisions, so one that arrives late changes nothing. The page
 * hears each snapshot as it is published and asks for the current one as it wakes, since a page
 * the system froze missed those meanwhile. The browser's media is never touched: the microphone
 * is the engine's, opened by the plugin as the user joins.
 *
 * This stage carries voice: the camera, screen sharing, and file transfers reject with
 * `NativeCallUnavailableError`, and the grants that would show their controls stay false.
 */
export class NativeCall implements Call {
  readonly #origin: string;
  readonly #userVolume: (userId: string, source: AudioSource) => number;
  readonly #nameOf: (channelId: string) => string | null;
  readonly #report: (error: unknown) => void;
  readonly #listeners = new Set<VoiceCallListener>();
  #state: VoiceCallState = IDLE_CALL_STATE;
  /** The revision of the snapshot shown; one at or before it is dropped. `null` before the first. */
  #revision: number | null = null;
  /** The gain the engine was last told for each `user source`, until the call ends or volumes refresh. */
  readonly #told = new Map<string, number>();
  /** Who else the latest snapshot shows in the call. */
  #present: readonly string[] = [];
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
    void AspenCall.addListener("state", (snapshot) => {
      this.#take(snapshot);
    });
    const read = () => {
      AspenCall.state({ origin: this.#origin }).then(
        (snapshot) => {
          this.#take(snapshot);
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

  /** Shows `snapshot` when it is this deployment's and newer than the one shown. */
  #take(snapshot: CallSnapshot): void {
    if (snapshot.origin !== this.#origin) {
      return;
    }
    if (this.#revision !== null && snapshot.revision <= this.#revision) {
      return;
    }
    let state: VoiceCallState;
    try {
      state = stateOf(snapshot);
    } catch (error) {
      this.#report(error);
      return;
    }
    this.#revision = snapshot.revision;
    this.#state = state;
    this.#present = snapshot.participants.map((participant) => participant.user);
    if (state.status === "idle") {
      this.#told.clear();
    } else {
      this.#applyVolumes();
    }
    this.#settleJoin(state);
    for (const listener of Array.from(this.#listeners)) {
      listener();
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
  #command(sent: Promise<void>): void {
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
   * Nothing is received to show in this stage: the engine watches no video, so no screen or
   * camera arrives whatever shows them. The function returned releases nothing.
   */
  showVideo(): () => void {
    return () => undefined;
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

  startCamera(): Promise<void> {
    return Promise.reject(new NativeCallUnavailableError("The camera"));
  }

  clearCameraError(): void {
    throw new NativeCallUnavailableError("The camera");
  }

  stopCamera(): void {
    throw new NativeCallUnavailableError("The camera");
  }

  /**
   * The phone's own microphone is the one sent and the route (`AspenCall.setRoute`, typed) is
   * where the call plays, so the system's default for each is what this call does; a device
   * chosen by id is a browser's choice, which the engine has no counterpart for yet.
   */
  setAudioDevices(devices: {
    input: DeviceChoice;
    output: DeviceChoice;
    camera?: DeviceChoice;
  }): Promise<void> {
    const chosen = [devices.input, devices.output, devices.camera ?? DEFAULT_DEVICE].some(
      (choice) => choice !== DEFAULT_DEVICE,
    );
    return chosen
      ? Promise.reject(new NativeCallUnavailableError("Choosing a device by id"))
      : Promise.resolve();
  }

  /**
   * The engine sends the microphone as captured, which is a gain of 1; the phone's audio device
   * offers no gain over it yet, so any other is reported rather than shown as applied.
   */
  setInputVolume(gain: number): void {
    if (gain !== 1) {
      this.#report(new NativeCallUnavailableError("A gain on the microphone"));
    }
  }

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
