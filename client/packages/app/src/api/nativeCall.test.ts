import type { AspenClient, EngineCallState, VoiceCallOptions, VoiceMedia } from "@aspen/protocol";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CallAccount, PublishedState } from "./nativeCall";

/** What the native side was asked, in order. */
const calls: { method: string; options: Record<string, unknown> | undefined }[] = [];
/** What `state` answers next, by `accountKey`. */
const current = new Map<string, PublishedState>();
/** What `state` rejects with, when set. */
let stateFailure: Error | null = null;
/** What `join` rejects with, when set. */
let joinFailure: Error | null = null;
/** What each method named here rejects with. */
const refusals = new Map<string, Error>();
/** What each method named here resolves with; `undefined` for the rest. */
const answers = new Map<string, unknown>();
/** Every listener added, by event: each `NativeCall` adds its own. */
const listeners = new Map<string, ((event: never) => void)[]>();

/** The plugin tells every listener of `event`. */
function emit(event: string, data: unknown): void {
  for (const listener of listeners.get(event) ?? []) {
    listener(data as never);
  }
}

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    isPluginAvailable: (name: string) => name === "AspenCall",
  },
  registerPlugin: () =>
    new Proxy(
      {},
      {
        get: (_, method: string) => {
          if (method === "addListener") {
            return (event: string, listener: (event: never) => void) => {
              listeners.set(event, [...(listeners.get(event) ?? []), listener]);
              return Promise.resolve({ remove: () => Promise.resolve() });
            };
          }
          return (options: Record<string, unknown> | undefined) => {
            calls.push({ method, options });
            if (method === "state") {
              // An engine not yet made answers its idle state, revision 0, as the plugin does.
              return stateFailure === null
                ? Promise.resolve(
                    current.get(accountKey(options as unknown as CallAccount)) ??
                      snapshot({ origin: options?.origin, userId: options?.userId, revision: 0 }),
                  )
                : Promise.reject(stateFailure);
            }
            if (method === "join" && joinFailure !== null) {
              return Promise.reject(joinFailure);
            }
            const refusal = refusals.get(method);
            return refusal === undefined
              ? Promise.resolve(answers.get(method))
              : Promise.reject(refusal);
          };
        },
      },
    ),
}));

vi.mock("@capacitor/app", () => ({
  App: { addListener: () => Promise.resolve({ remove: () => Promise.resolve() }) },
}));

const {
  NativeCall,
  NativeCallFailedError,
  NativeCallProtocolError,
  NativeCallUnavailableError,
  nativeCalls,
  parseState,
  stateOf,
  watchedOf,
} = await import("./nativeCall");

const { CameraError, DEFAULT_DEVICE, IDLE_CALL_STATE, NO_FILES } = await import("@aspen/protocol");
const { filesOf } = await import("./nativeCall");

const ORIGIN = "https://chat.example";
const CHANNEL = "0190f0a0-0000-7000-8000-000000000011";
const ANN = "0190f0a0-0000-7000-8000-000000000001";
const BOB = "0190f0a0-0000-7000-8000-000000000002";
const CAROL = "0190f0a0-0000-7000-8000-000000000003";

/** An idle snapshot of `ORIGIN`'s engine, as the engine writes it, with `patch` laid over it. */
function engineState(patch: Record<string, unknown> = {}): EngineCallState {
  return {
    revision: 1,
    status: "idle",
    channel: null,
    session: null,
    user: null,
    muted: false,
    deafened: false,
    held: false,
    serverMuted: false,
    grants: { speak: false, shareScreen: false, camera: false, transferFiles: false },
    participants: [],
    microphone: null,
    sources: [],
    audioSlots: 0,
    maxWatchedVideo: 0,
    slots: [],
    watching: [],
    videos: [],
    shareAudio: [],
    errorKind: null,
    error: null,
    retryAfterSeconds: null,
    stateRefused: null,
    sourceRefused: null,
    endedReason: null,
    files: { offers: [], transfers: [], links: [], relayMbps: null, failure: null },
    ...patch,
  };
}

const OFFER = "0190f0a0-0000-7000-8000-0000000000f1";

/** A transfer as the engine's snapshot carries one, with `patch` laid over it. */
function engineTransfer(
  patch: Record<string, unknown> = {},
): EngineCallState["files"]["transfers"][number] {
  return {
    offer: OFFER,
    peer: BOB,
    role: "sender",
    mode: "directPreferred",
    name: "notes.bin",
    size: 10,
    bytes: 4,
    status: "moving",
    route: "direct",
    endedBy: null,
    failure: null,
    ...patch,
  };
}

/** `engineState(patch)` as the plugin publishes it: the engine's JSON beside its account. */
function snapshot(patch: Record<string, unknown> = {}): PublishedState {
  const { origin, userId, ...rest } = patch;
  return {
    origin: typeof origin === "string" ? origin : ORIGIN,
    userId: typeof userId === "string" ? userId : ANN,
    state: JSON.stringify(engineState(rest)),
  };
}

/** How `current` is keyed: an account's deployment and user together. */
function accountKey(account: CallAccount): string {
  return `${account.origin} ${account.userId}`;
}

function participant(user: string, held = false, sources: { source: string }[] = []) {
  return { user, muted: false, deafened: false, held, speaking: false, sources };
}

/** A connected snapshot of `CHANNEL` as `ANN`, with `patch` laid over it. */
function connected(patch: Record<string, unknown> = {}) {
  return {
    status: "connected",
    channel: CHANNEL,
    session: "s1",
    user: ANN,
    grants: { speak: true, shareScreen: false, camera: true, transferFiles: false },
    maxWatchedVideo: 25,
    ...patch,
  };
}

/** Lets the bridge's awaited calls run to the next one that waits on the native side. */
async function settle(): Promise<void> {
  for (let i = 0; i < 20; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

/** The engine publishes `snapshot` to the page. */
function publish(published: PublishedState): void {
  emit("state", published);
}

const media = {} as VoiceMedia;

/** A call with the plugin faked around it, and what it reported and heard. */
function call(options: Partial<VoiceCallOptions> = {}, origin = ORIGIN, userId = ANN) {
  const reported: unknown[] = [];
  const wakes: (() => void)[] = [];
  const changes: string[] = [];
  const made = new NativeCall(
    { client: { baseUrl: origin, session: { userId } } as AspenClient, media, ...options },
    {
      report: (error) => reported.push(error),
      onWake: (listener) => wakes.push(listener),
      nameOf: (channelId) => (channelId === CHANNEL ? "Lounge" : null),
    },
  );
  made.subscribe(() => {
    changes.push(made.state.status);
  });
  return {
    call: made,
    reported,
    wake: () => {
      wakes.forEach((wake) => {
        wake();
      });
    },
    changes,
  };
}

describe("nativeCall", () => {
  beforeEach(() => {
    calls.length = 0;
    current.clear();
    listeners.clear();
    stateFailure = null;
    joinFailure = null;
    refusals.clear();
    answers.clear();
  });

  it("is used in a mobile app that has the plugin", () => {
    expect(nativeCalls()).toBe(true);
  });

  describe("stateOf", () => {
    it("carries each status by name", () => {
      for (const status of ["idle", "joining", "connected", "rejoining", "failed"] as const) {
        expect(stateOf(engineState({ status })).status).toBe(status);
      }
    });

    it("refuses a status this build does not know", () => {
      expect(() => stateOf(engineState({ status: "dialling" }))).toThrow(NativeCallProtocolError);
    });

    it("maps the channel, the session, and the user's own state", () => {
      const state = stateOf(
        engineState({
          status: "connected",
          channel: CHANNEL,
          session: "s1",
          muted: true,
          deafened: true,
          held: true,
          serverMuted: true,
          grants: { speak: true, shareScreen: true, camera: true, transferFiles: true },
        }),
      );
      expect(state.channelId).toBe(CHANNEL);
      expect(state.session).toBe("s1");
      expect(state.muted).toBe(true);
      expect(state.deafened).toBe(true);
      expect(state.held).toBe(true);
      expect(state.serverMuted).toBe(true);
      expect(state.canSpeak).toBe(true);
    });

    it("shows the camera's, the share's, and the files' grants", () => {
      const state = stateOf(
        engineState({
          grants: { speak: true, shareScreen: true, camera: true, transferFiles: true },
        }),
      );
      expect(state.canCamera).toBe(true);
      expect(state.canShare).toBe(true);
      expect(state.canTransfer).toBe(true);
      expect(state.files).toEqual(NO_FILES);
      expect(stateOf(engineState()).canTransfer).toBe(false);
      expect(state.sharingScreen).toBe(false);
      expect(state.localScreen).toBeNull();
      expect(state.localCamera).toBeNull();
      expect(state.screens).toEqual([]);
      expect(state.cameras).toEqual([]);
      expect(state.cameraError).toBeNull();
      expect(state.shareError).toBeNull();
      expect(stateOf(engineState()).canCamera).toBe(false);
      expect(stateOf(engineState()).canShare).toBe(false);
    });

    it("shows the screen sent as the user's own share, a surface, and why none is", () => {
      const sharing = stateOf(
        engineState(
          connected({
            sources: [
              { source: "screen", track: "share-video", live: true, external: true },
              { source: "screenAudio", track: "share-audio", live: true, external: true },
            ],
          }),
        ),
        { facing: null, cameraFailure: null, shareFailure: "failed" },
      );
      expect(sharing.sharingScreen).toBe(true);
      expect(sharing.localScreen).toEqual({ kind: "native", surface: "share-video", facing: null });
      expect(sharing.shareError).toBeNull();
      expect(sharing.screens).toEqual([]);
      const refused = stateOf(
        engineState(
          connected({
            sourceRefused: {
              source: "screenAudio",
              refusal: { detail: "no", retryAfterSeconds: null },
            },
          }),
        ),
        { facing: null, cameraFailure: null, shareFailure: "denied" },
      );
      expect(refused.sharingScreen).toBe(false);
      expect(refused.shareError).toBe("unsent");
      expect(
        stateOf(engineState(connected()), {
          facing: null,
          cameraFailure: null,
          shareFailure: "denied",
        }).shareError,
      ).toBe("denied");
    });

    it("shows the videos received as the others' cameras and screens, each a surface", () => {
      const state = stateOf(
        engineState(
          connected({
            videos: [
              { user: BOB, source: "camera", track: "t-bob-cam" },
              { user: CAROL, source: "screen", track: "t-carol-screen" },
              { user: BOB, source: "screen", track: "t-bob-screen" },
              // The user's own share, watched for its preview, is not among the others'.
              { user: ANN, source: "screen", track: "t-ann-screen" },
            ],
          }),
        ),
      );
      expect(state.cameras).toEqual([
        {
          user: BOB,
          id: "t-bob-cam",
          video: { kind: "native", surface: "t-bob-cam", facing: null },
        },
      ]);
      expect(state.screens).toEqual([
        {
          user: CAROL,
          id: "t-carol-screen",
          video: { kind: "native", surface: "t-carol-screen", facing: null },
        },
        {
          user: BOB,
          id: "t-bob-screen",
          video: { kind: "native", surface: "t-bob-screen", facing: null },
        },
      ]);
    });

    it("shows the camera sent as the local picture, facing as the page was told", () => {
      const sent = engineState(
        connected({ sources: [{ source: "camera", track: "t-cam", live: true, external: false }] }),
      );
      expect(stateOf(sent).localCamera).toEqual({ kind: "native", surface: "t-cam", facing: null });
      expect(
        stateOf(sent, { facing: "back", cameraFailure: null, shareFailure: null }).localCamera,
      ).toEqual({
        kind: "native",
        surface: "t-cam",
        facing: "back",
      });
      expect(stateOf(sent).cameraError).toBeNull();
    });

    it("shows why the camera is off: the voice server's refusal, else the phone's", () => {
      const refused = engineState(
        connected({
          sourceRefused: { source: "camera", refusal: { detail: "no", retryAfterSeconds: null } },
        }),
      );
      expect(stateOf(refused).cameraError).toBe("unsent");
      expect(
        stateOf(refused, { facing: null, cameraFailure: "denied", shareFailure: null }).cameraError,
      ).toBe("unsent");
      expect(
        stateOf(engineState(connected()), {
          facing: null,
          cameraFailure: "denied",
          shareFailure: null,
        }).cameraError,
      ).toBe("denied");
      const other = engineState(
        connected({
          sourceRefused: { source: "screen", refusal: { detail: "no", retryAfterSeconds: null } },
        }),
      );
      expect(stateOf(other).cameraError).toBeNull();
    });

    it("lists who is on hold in id order", () => {
      const state = stateOf(
        engineState({
          participants: [participant(CAROL, true), participant(ANN), participant(BOB, true)],
        }),
      );
      expect(state.onHold).toEqual([BOB, CAROL]);
    });

    it("shows the join offer's refusal as the servers failing, as the web does", () => {
      expect(stateOf(engineState({ errorKind: "offer" })).errorKind).toBe("server");
      expect(stateOf(engineState({ errorKind: "server" })).errorKind).toBe("server");
      expect(stateOf(engineState({ errorKind: "microphone" })).errorKind).toBe("microphone");
      expect(stateOf(engineState({ errorKind: "refused" })).errorKind).toBe("refused");
      expect(stateOf(engineState({ errorKind: "device" })).errorKind).toBe("device");
      expect(stateOf(engineState({ errorKind: null })).errorKind).toBeNull();
      expect(() => stateOf(engineState({ errorKind: "network" }))).toThrow(NativeCallProtocolError);
    });

    it("carries the failure's words and how long to wait", () => {
      const state = stateOf(
        engineState({
          status: "failed",
          errorKind: "refused",
          error: "Too fast",
          retryAfterSeconds: 7,
        }),
      );
      expect(state.error).toBe("Too fast");
      expect(state.retryAfterSeconds).toBe(7);
    });

    it("carries a refused state change", () => {
      expect(stateOf(engineState()).stateRefused).toBeNull();
      expect(
        stateOf(engineState({ stateRefused: { detail: "no", retryAfterSeconds: 3 } })).stateRefused,
      ).toEqual({ retryAfterSeconds: 3 });
      expect(
        stateOf(engineState({ stateRefused: { detail: "no", retryAfterSeconds: null } }))
          .stateRefused,
      ).toEqual({ retryAfterSeconds: null });
    });

    it("carries each ended reason by name", () => {
      for (const endedReason of ["kicked", "accessLost", "idle", null] as const) {
        expect(stateOf(engineState({ endedReason })).endedReason).toBe(endedReason);
      }
      expect(() => stateOf(engineState({ endedReason: "bored" }))).toThrow(NativeCallProtocolError);
    });
  });

  describe("filesOf", () => {
    it("maps offers with their names as shown and their expiry on this device's clock", () => {
      const files = filesOf({
        offers: [
          {
            id: OFFER,
            from: BOB,
            name: "gpj.\u202eexe",
            size: 200,
            allowDirect: true,
            expiresAtMs: 1_700_000_000_000,
            own: false,
          },
        ],
        transfers: [],
        links: [{ sender: BOB, receiver: CAROL }],
        relayMbps: 50,
        failure: null,
      });
      expect(files.offers).toEqual([
        {
          id: OFFER,
          from: BOB,
          name: "gpj.exe",
          size: 200,
          allowDirect: true,
          expiresAt: 1_700_000_000_000,
          own: false,
        },
      ]);
      expect(files.links).toEqual([{ sender: BOB, receiver: CAROL }]);
      expect(files.relayMbps).toBe(50);
      expect(files.failure).toBeNull();
    });

    it("maps each transfer, written to disk on the receiving side and never held", () => {
      const files = filesOf({
        offers: [],
        transfers: [
          engineTransfer(),
          engineTransfer({
            role: "receiver",
            mode: "relayOnly",
            status: "completed",
            route: "relayed",
            endedBy: "self",
            bytes: 10,
          }),
          engineTransfer({ status: "cancelled", route: null, endedBy: "peer" }),
        ],
        links: [],
        relayMbps: null,
        failure: null,
      });
      expect(files.transfers).toEqual([
        {
          offer: OFFER,
          peer: BOB,
          role: "sender",
          mode: "directPreferred",
          name: "notes.bin",
          size: 10,
          bytes: 4,
          status: "moving",
          route: "direct",
          endedBy: null,
          file: null,
          toDisk: false,
        },
        {
          offer: OFFER,
          peer: BOB,
          role: "receiver",
          mode: "relayOnly",
          name: "notes.bin",
          size: 10,
          bytes: 10,
          status: "completed",
          route: "relayed",
          endedBy: "self",
          file: null,
          toDisk: true,
        },
        {
          offer: OFFER,
          peer: BOB,
          role: "sender",
          mode: "directPreferred",
          name: "notes.bin",
          size: 10,
          bytes: 4,
          status: "cancelled",
          route: null,
          endedBy: "peer",
          file: null,
          toDisk: false,
        },
      ]);
      expect(files.relayMbps).toBeNull();
    });

    it("carries each transfer status and ending by name", () => {
      for (const status of [
        "connecting",
        "moving",
        "completed",
        "cancelled",
        "failed",
        "left",
        "notPermitted",
      ] as const) {
        expect(
          filesOf({
            offers: [],
            transfers: [engineTransfer({ status })],
            links: [],
            relayMbps: null,
          }).transfers[0]?.status,
        ).toBe(status);
      }
    });

    it("carries the file command that failed, and refuses what it does not know", () => {
      const failed = filesOf({
        offers: [],
        transfers: [],
        links: [],
        relayMbps: null,
        failure: { command: "accept", offer: OFFER, name: "a\u0000.txt", detail: "no room" },
      });
      expect(failed.failure).toEqual({
        command: "accept",
        offer: OFFER,
        name: "a.txt",
        detail: "no room",
      });
      const bad = (patch: Record<string, unknown>) =>
        filesOf({
          offers: [],
          transfers: [engineTransfer(patch)],
          links: [],
          relayMbps: null,
        });
      expect(() => bad({ status: "teleporting" })).toThrow(NativeCallProtocolError);
      expect(() => bad({ route: "pigeon" })).toThrow(NativeCallProtocolError);
      expect(() => bad({ endedBy: "nobody" })).toThrow(NativeCallProtocolError);
      expect(() =>
        filesOf({
          offers: [],
          transfers: [],
          links: [],
          relayMbps: null,
          failure: { command: "shred", offer: OFFER, name: "a", detail: "b" },
        } as never),
      ).toThrow(NativeCallProtocolError);
    });
  });

  describe("parseState", () => {
    it("reads the engine's JSON, and refuses what is not it", () => {
      expect(parseState(JSON.stringify(engineState({ revision: 4 }))).revision).toBe(4);
      expect(() => parseState("not json")).toThrow(NativeCallProtocolError);
      expect(() => parseState("[]")).toThrow(NativeCallProtocolError);
      expect(() => parseState("null")).toThrow(NativeCallProtocolError);
    });
  });

  describe("watchedOf", () => {
    it("names every screen and camera the others send, in their order, and none of the user's", () => {
      const snapshot = engineState(
        connected({
          participants: [
            participant(BOB, false, [{ source: "microphone" }, { source: "camera" }]),
            participant(ANN, false, [{ source: "camera" }]),
            participant(CAROL, false, [
              { source: "screen" },
              { source: "screenAudio" },
              { source: "camera" },
            ]),
          ],
        }),
      );
      expect(watchedOf(snapshot)).toEqual([
        { user: BOB, source: "camera" },
        { user: CAROL, source: "screen" },
        { user: CAROL, source: "camera" },
      ]);
    });
  });

  describe("snapshots", () => {
    it("reads the current snapshot as it is made, and again as the page wakes", async () => {
      current.set(
        accountKey({ origin: ORIGIN, userId: ANN }),
        snapshot({ revision: 3, status: "connected", channel: CHANNEL }),
      );
      const { call: made, wake } = call();
      await settle();
      expect(calls.filter((c) => c.method === "state").map((c) => c.options)).toEqual([
        { origin: ORIGIN, userId: ANN },
      ]);
      expect(made.state.status).toBe("connected");
      current.set(
        accountKey({ origin: ORIGIN, userId: ANN }),
        snapshot({ revision: 4, status: "idle" }),
      );
      wake();
      await settle();
      expect(calls.filter((c) => c.method === "state")).toHaveLength(2);
      expect(made.state.status).toBe("idle");
    });

    it("reports a snapshot it could not read, and keeps its state", async () => {
      stateFailure = new Error("no engine");
      const { call: made, reported } = call();
      await settle();
      expect(reported).toEqual([stateFailure]);
      expect(made.state).toEqual(IDLE_CALL_STATE);
    });

    it("applies snapshots in the order of their revisions, dropping what arrives late", async () => {
      const { call: made, changes } = call();
      await settle();
      // The first read, the engine's idle state at revision 0, was itself a change heard.
      expect(changes).toEqual(["idle"]);
      changes.length = 0;
      publish(snapshot({ revision: 5, status: "connected", channel: CHANNEL }));
      publish(snapshot({ revision: 4, status: "joining", channel: CHANNEL }));
      publish(snapshot({ revision: 5, status: "connected", channel: CHANNEL }));
      expect(made.state.status).toBe("connected");
      expect(changes).toEqual(["connected"]);
      publish(snapshot({ revision: 6, status: "idle" }));
      expect(made.state.status).toBe("idle");
      expect(changes).toEqual(["connected", "idle"]);
    });

    it("takes only its own deployment's snapshots", async () => {
      const { call: made } = call();
      await settle();
      publish(snapshot({ origin: "https://other.example", revision: 9, status: "connected" }));
      expect(made.state.status).toBe("idle");
    });

    it("takes only its own account's snapshots and shares on a deployment holding two", async () => {
      const { call: ann } = call();
      const { call: bob } = call({}, ORIGIN, BOB);
      await settle();
      expect(calls.filter((c) => c.method === "state").map((c) => c.options)).toEqual([
        { origin: ORIGIN, userId: ANN },
        { origin: ORIGIN, userId: BOB },
      ]);
      publish(snapshot({ userId: BOB, revision: 9, status: "connected", channel: CHANNEL }));
      expect(bob.state.status).toBe("connected");
      expect(ann.state.status).toBe("idle");
      emit("share", { origin: ORIGIN, userId: BOB, failure: "failed" });
      expect(bob.state.shareError).toBe("failed");
      expect(ann.state.shareError).toBeNull();
    });

    it("refuses to be made for a client with no session", () => {
      expect(
        () => new NativeCall({ client: { baseUrl: ORIGIN, session: null } as AspenClient, media }),
      ).toThrow();
    });

    it("reports a snapshot it cannot map and shows nothing of it", async () => {
      const { call: made, reported } = call();
      await settle();
      publish(snapshot({ revision: 2, status: "connected", endedReason: "bored" }));
      publish({ origin: ORIGIN, userId: ANN, state: "{not json" });
      expect(reported).toHaveLength(2);
      expect(reported[0]).toBeInstanceOf(NativeCallProtocolError);
      expect(reported[1]).toBeInstanceOf(NativeCallProtocolError);
      expect(made.state.status).toBe("idle");
      publish(snapshot({ revision: 2, status: "connected", channel: CHANNEL }));
      expect(made.state.status).toBe("connected");
    });
  });

  describe("join", () => {
    it("asks the plugin to join, named for the system, and resolves once connected", async () => {
      const { call: made } = call();
      await settle();
      let settled = false;
      const joined = made.join(CHANNEL).then(() => {
        settled = true;
      });
      await settle();
      expect(calls.find((c) => c.method === "join")?.options).toEqual({
        origin: ORIGIN,
        userId: ANN,
        channel: CHANNEL,
        name: "Lounge",
      });
      expect(settled).toBe(false);
      publish(snapshot({ revision: 1, status: "joining", channel: CHANNEL }));
      await settle();
      expect(settled).toBe(false);
      publish(snapshot({ revision: 2, status: "connected", channel: CHANNEL }));
      await joined;
      expect(settled).toBe(true);
    });

    it("rejects with the failure the engine shows", async () => {
      const { call: made } = call();
      await settle();
      const joined = made.join(CHANNEL);
      await settle();
      publish(
        snapshot({
          revision: 1,
          status: "failed",
          channel: CHANNEL,
          errorKind: "offer",
          error: "You may not join this call.",
        }),
      );
      await expect(joined).rejects.toMatchObject({
        name: "NativeCallFailedError",
        kind: "server",
        message: "You may not join this call.",
      });
    });

    it("resolves when the call is left or another join takes over before it connects", async () => {
      const { call: made } = call();
      await settle();
      const first = made.join(CHANNEL);
      await settle();
      publish(snapshot({ revision: 1, status: "joining", channel: CHANNEL }));
      publish(snapshot({ revision: 2, status: "idle" }));
      await expect(first).resolves.toBeUndefined();
      const second = made.join(CHANNEL);
      await settle();
      const third = made.join("another");
      await expect(second).resolves.toBeUndefined();
      publish(snapshot({ revision: 3, status: "connected", channel: "another" }));
      await expect(third).resolves.toBeUndefined();
    });

    it("rejects as the plugin refuses, with no join left waiting", async () => {
      joinFailure = new Error("the microphone is in use");
      const { call: made } = call();
      await settle();
      await expect(made.join(CHANNEL)).rejects.toBe(joinFailure);
      joinFailure = null;
      const again = made.join(CHANNEL);
      await settle();
      publish(snapshot({ revision: 1, status: "connected", channel: CHANNEL }));
      await expect(again).resolves.toBeUndefined();
    });

    it("tells NativeCallFailedError apart from what the plugin refused", () => {
      expect(new NativeCallFailedError("microphone", null).message).toBe("the call failed");
    });
  });

  describe("files", () => {
    it("offers a picked file under an id of its own, and refuses a browser's Blob", async () => {
      const { call: made } = call();
      await settle();
      calls.length = 0;
      const picked = { reference: "picked-1", name: "notes.bin", size: 1234 };
      const offer = made.offerFile(picked, "notes.bin", true, 60);
      expect(offer).toMatch(/^[0-9a-f-]{36}$/);
      expect(calls).toEqual([
        {
          method: "offerFile",
          options: {
            origin: ORIGIN,
            userId: ANN,
            offer,
            reference: "picked-1",
            name: "notes.bin",
            allowDirect: true,
            validForSeconds: 60,
          },
        },
      ]);
      expect(made.offerFile(picked, "notes.bin", true, 60)).not.toBe(offer);
      expect(() => made.offerFile(new Blob(), "a", false, 60)).toThrow(NativeCallUnavailableError);
    });

    it("accepts into the place the sink names, and reports an acceptance with none", async () => {
      const { call: made, reported } = call();
      await settle();
      calls.length = 0;
      const sink = {
        destination: "doc-1",
        write: () => Promise.resolve(),
        close: () => Promise.resolve(),
        abort: () => Promise.resolve(),
      };
      made.acceptOffer(OFFER, "relayOnly", sink);
      expect(calls).toEqual([
        {
          method: "acceptOffer",
          options: {
            origin: ORIGIN,
            userId: ANN,
            offer: OFFER,
            mode: "relayOnly",
            destination: "doc-1",
          },
        },
      ]);
      made.acceptOffer(OFFER, "directPreferred");
      made.acceptOffer(OFFER, "directPreferred", {
        write: sink.write,
        close: sink.close,
        abort: sink.abort,
      });
      expect(calls).toHaveLength(1);
      expect(reported).toHaveLength(2);
      expect(reported[0]).toBeInstanceOf(NativeCallUnavailableError);
    });

    it("forwards withdrawing, cancelling, dismissing, and clearing a failure", async () => {
      const { call: made } = call();
      await settle();
      calls.length = 0;
      made.withdrawOffer(OFFER);
      made.cancelTransfer(OFFER, BOB);
      made.dismissTransfer(OFFER, BOB);
      made.dismissFileFailure();
      expect(calls).toEqual([
        { method: "withdrawOffer", options: { origin: ORIGIN, userId: ANN, offer: OFFER } },
        {
          method: "cancelTransfer",
          options: { origin: ORIGIN, userId: ANN, offer: OFFER, peer: BOB },
        },
        {
          method: "dismissTransfer",
          options: { origin: ORIGIN, userId: ANN, offer: OFFER, peer: BOB },
        },
        { method: "clearFileFailure", options: { origin: ORIGIN, userId: ANN } },
      ]);
    });

    it("shows the engine's files in the state", async () => {
      const { call: made } = call();
      await settle();
      publish(
        snapshot({
          revision: 1,
          status: "connected",
          channel: CHANNEL,
          grants: { speak: true, shareScreen: false, camera: false, transferFiles: true },
          files: {
            offers: [],
            transfers: [engineTransfer({ bytes: 7 })],
            links: [],
            relayMbps: 50,
            failure: null,
          },
        }),
      );
      expect(made.state.canTransfer).toBe(true);
      expect(made.state.files.transfers[0]?.bytes).toBe(7);
      expect(made.state.files.relayMbps).toBe(50);
    });
  });

  describe("commands", () => {
    it("forwards each command with the deployment", async () => {
      const { call: made } = call();
      await settle();
      calls.length = 0;
      made.leave();
      made.setMuted(true);
      made.setDeafened(true);
      made.setHeld(true);
      made.setServerMuted(true);
      made.clearStateRefusal();
      made.acknowledgeEnd();
      made.onSessionEnded({ id: "s1", channel: CHANNEL, reason: "serverLost" });
      expect(calls).toEqual([
        { method: "leave", options: { origin: ORIGIN, userId: ANN } },
        { method: "setMuted", options: { origin: ORIGIN, userId: ANN, muted: true } },
        { method: "setDeafened", options: { origin: ORIGIN, userId: ANN, deafened: true } },
        { method: "setHeld", options: { origin: ORIGIN, userId: ANN, held: true } },
        { method: "setServerMuted", options: { origin: ORIGIN, userId: ANN, serverMuted: true } },
        { method: "clearStateRefusal", options: { origin: ORIGIN, userId: ANN } },
        { method: "acknowledgeEnd", options: { origin: ORIGIN, userId: ANN } },
        {
          method: "sessionEnded",
          options: {
            origin: ORIGIN,
            userId: ANN,
            session: "s1",
            channel: CHANNEL,
            reason: "serverLost",
          },
        },
      ]);
    });

    it("reports a command the plugin refused", async () => {
      const { call: made, reported } = call();
      await settle();
      const refusal = new Error("no such engine");
      refusals.set("setMuted", refusal);
      made.setMuted(true);
      made.setDeafened(true);
      await settle();
      expect(reported).toEqual([refusal]);
    });
  });

  describe("volumes", () => {
    it("tells the engine each participant's volume as they appear, from userVolume", async () => {
      const gains = new Map([
        [`${BOB} microphone`, 0.5],
        [`${BOB} screenAudio`, 0],
      ]);
      const { call: made } = call({
        userVolume: (userId, source) => gains.get(`${userId} ${source}`) ?? 1,
      });
      await settle();
      publish(
        snapshot({
          revision: 1,
          status: "connected",
          channel: CHANNEL,
          participants: [participant(BOB)],
        }),
      );
      const volumes = () => calls.filter((c) => c.method === "setUserVolume").map((c) => c.options);
      expect(volumes()).toEqual([
        { origin: ORIGIN, userId: ANN, user: BOB, source: "microphone", gain: 0.5 },
        { origin: ORIGIN, userId: ANN, user: BOB, source: "screenAudio", gain: 0 },
      ]);
      // Told once: the next snapshot with the same people tells nothing again.
      publish(
        snapshot({
          revision: 2,
          status: "connected",
          channel: CHANNEL,
          participants: [participant(BOB), participant(CAROL)],
        }),
      );
      expect(volumes()).toHaveLength(4);
      expect(volumes()[2]).toEqual({
        origin: ORIGIN,
        userId: ANN,
        user: CAROL,
        source: "microphone",
        gain: 1,
      });
      // A refresh reads every present participant again.
      gains.set(`${CAROL} microphone`, 0);
      made.refreshVolumes();
      expect(volumes()).toHaveLength(8);
      expect(volumes()[6]).toEqual({
        origin: ORIGIN,
        userId: ANN,
        user: CAROL,
        source: "microphone",
        gain: 0,
      });
      // The call's end forgets whom it told, so a later call tells again.
      publish(snapshot({ revision: 3, status: "idle" }));
      publish(
        snapshot({
          revision: 4,
          status: "connected",
          channel: CHANNEL,
          participants: [participant(BOB)],
        }),
      );
      expect(volumes()).toHaveLength(10);
      expect(made.state.onHold).toEqual([]);
    });

    it("forwards a volume set by hand, for the voice by default", async () => {
      const { call: made } = call();
      await settle();
      made.setUserVolume(BOB, 1.5);
      made.setUserVolume(BOB, 0, "screenAudio");
      expect(calls.filter((c) => c.method === "setUserVolume").map((c) => c.options)).toEqual([
        { origin: ORIGIN, userId: ANN, user: BOB, source: "microphone", gain: 1.5 },
        { origin: ORIGIN, userId: ANN, user: BOB, source: "screenAudio", gain: 0 },
      ]);
    });
  });

  describe("video", () => {
    it("watches every screen and camera while video is shown, and none when it is not", async () => {
      const { call: made } = call();
      await settle();
      // The participants' volumes are told beside; only the watching is read here.
      const watched = () => calls.filter((c) => c.method === "setWatched");
      const release = made.showVideo();
      // Not in a call: the engine keeps nothing to watch, so nothing is asked.
      expect(watched()).toEqual([]);
      publish(
        snapshot({
          revision: 2,
          ...connected({
            participants: [participant(BOB, false, [{ source: "camera" }]), participant(ANN)],
          }),
        }),
      );
      expect(watched()).toEqual([
        {
          method: "setWatched",
          options: { origin: ORIGIN, userId: ANN, video: [{ user: BOB, source: "camera" }] },
        },
      ]);
      // A snapshot that changes nothing watched asks nothing again.
      publish(
        snapshot({
          revision: 3,
          ...connected({ participants: [participant(BOB, false, [{ source: "camera" }])] }),
        }),
      );
      expect(watched()).toHaveLength(1);
      publish(
        snapshot({
          revision: 4,
          ...connected({
            participants: [participant(BOB, false, [{ source: "camera" }, { source: "screen" }])],
          }),
        }),
      );
      expect(watched().at(-1)).toEqual({
        method: "setWatched",
        options: {
          origin: ORIGIN,
          userId: ANN,
          video: [
            { user: BOB, source: "camera" },
            { user: BOB, source: "screen" },
          ],
        },
      });
      const again = made.showVideo();
      release();
      expect(watched()).toHaveLength(2);
      again();
      expect(watched().at(-1)).toEqual({
        method: "setWatched",
        options: { origin: ORIGIN, userId: ANN, video: [] },
      });
      // Released twice releases once.
      again();
      expect(watched()).toHaveLength(3);
    });

    it("turns the camera on through the plugin, facing as it says, and off", async () => {
      const { call: made } = call();
      await settle();
      publish(snapshot({ revision: 2, ...connected() }));
      calls.length = 0;
      answers.set("startCamera", { device: "front-1", facing: "front" });
      await made.startCamera();
      expect(calls).toEqual([
        { method: "startCamera", options: { origin: ORIGIN, userId: ANN, device: null } },
      ]);
      // The picture shows once the engine's snapshot carries the source.
      expect(made.state.localCamera).toBeNull();
      publish(
        snapshot({
          revision: 3,
          ...connected({
            sources: [{ source: "camera", track: "t-cam", live: false, external: false }],
          }),
        }),
      );
      expect(made.state.localCamera).toEqual({ kind: "native", surface: "t-cam", facing: "front" });
      made.stopCamera();
      expect(calls.at(-1)).toEqual({
        method: "stopCamera",
        options: { origin: ORIGIN, userId: ANN },
      });
      publish(snapshot({ revision: 4, ...connected() }));
      expect(made.state.localCamera).toBeNull();
    });

    it("does not ask for the camera outside a call or where it is not granted", async () => {
      const { call: made } = call();
      await settle();
      calls.length = 0;
      await made.startCamera();
      publish(
        snapshot({
          revision: 2,
          ...connected({
            grants: { speak: true, shareScreen: false, camera: false, transferFiles: false },
          }),
        }),
      );
      await made.startCamera();
      expect(calls).toEqual([]);
    });

    it("keeps the plugin's refusal of the camera as cameraError, and dismisses it", async () => {
      const { call: made } = call();
      await settle();
      publish(snapshot({ revision: 2, ...connected() }));
      // The plugin's refusal, as Capacitor hands it to the page: an error with the code beside.
      refusals.set(
        "startCamera",
        Object.assign(new Error("the camera is not allowed"), { code: "denied" }),
      );
      await expect(made.startCamera()).rejects.toSatisfy(
        (error) => error instanceof CameraError && error.failure === "denied",
      );
      expect(made.state.cameraError).toBe("denied");
      refusals.set("startCamera", Object.assign(new Error("no camera"), { code: "none" }));
      await expect(made.startCamera()).rejects.toSatisfy(
        (error) => error instanceof CameraError && error.failure === "none",
      );
      expect(made.state.cameraError).toBe("none");
      // A refusal with no code the page knows is the camera failing to start.
      refusals.set("startCamera", new Error("boom"));
      await expect(made.startCamera()).rejects.toSatisfy(
        (error) => error instanceof CameraError && error.failure === "failed",
      );
      expect(made.state.cameraError).toBe("failed");
      calls.length = 0;
      made.clearCameraError();
      expect(made.state.cameraError).toBeNull();
      expect(calls).toEqual([
        { method: "clearCameraError", options: { origin: ORIGIN, userId: ANN } },
      ]);
      // The next call starts clean.
      refusals.set("startCamera", Object.assign(new Error("not allowed"), { code: "denied" }));
      await expect(made.startCamera()).rejects.toBeInstanceOf(CameraError);
      publish(snapshot({ revision: 3, status: "idle" }));
      expect(made.state.cameraError).toBeNull();
    });

    it("keeps the chosen camera for the next start, and moves a camera that is on", async () => {
      const { call: made } = call();
      await settle();
      publish(snapshot({ revision: 2, ...connected() }));
      calls.length = 0;
      await made.setAudioDevices({
        input: DEFAULT_DEVICE,
        output: DEFAULT_DEVICE,
        camera: { id: "back-1", label: "Back" },
      });
      // Off, the choice waits for the next start.
      expect(calls).toEqual([]);
      answers.set("startCamera", { device: "back-1", facing: "back" });
      await made.startCamera();
      expect(calls).toEqual([
        { method: "startCamera", options: { origin: ORIGIN, userId: ANN, device: "back-1" } },
      ]);
      publish(
        snapshot({
          revision: 3,
          ...connected({
            sources: [{ source: "camera", track: "t-cam", live: true, external: false }],
          }),
        }),
      );
      expect(made.state.localCamera).toEqual({ kind: "native", surface: "t-cam", facing: "back" });
      answers.set("switchCamera", { device: "front-1", facing: "front" });
      await made.setAudioDevices({
        input: DEFAULT_DEVICE,
        output: DEFAULT_DEVICE,
        camera: { id: "front-1", label: "Front" },
      });
      expect(calls.at(-1)).toEqual({
        method: "switchCamera",
        options: { origin: ORIGIN, userId: ANN, device: "front-1" },
      });
      expect(made.state.localCamera).toEqual({ kind: "native", surface: "t-cam", facing: "front" });
      // The same choice again moves nothing.
      await made.setAudioDevices({
        input: DEFAULT_DEVICE,
        output: DEFAULT_DEVICE,
        camera: { id: "front-1", label: "Front" },
      });
      expect(calls).toHaveLength(2);
    });
  });

  describe("sharing the screen", () => {
    it("captures the screen itself, so the share control is offered", () => {
      expect(call().call.hasScreenCapture).toBe(true);
    });

    it("asks the plugin for the screen with its sound, and shows the share from the snapshot", async () => {
      const { call: made } = call();
      await settle();
      publish(
        snapshot(
          connected({
            grants: { speak: true, shareScreen: true, camera: false, transferFiles: false },
          }),
        ),
      );
      calls.length = 0;
      await made.startScreenShare();
      expect(calls).toEqual([
        { method: "startScreenShare", options: { origin: ORIGIN, userId: ANN, audio: true } },
      ]);
      publish(
        snapshot(
          connected({
            revision: 2,
            grants: { speak: true, shareScreen: true, camera: false, transferFiles: false },
            sources: [
              { source: "screen", track: "share-video", live: true, external: true },
              { source: "screenAudio", track: "share-audio", live: true, external: true },
            ],
          }),
        ),
      );
      expect(made.state.sharingScreen).toBe(true);
      expect(made.state.localScreen).toEqual({
        kind: "native",
        surface: "share-video",
        facing: null,
      });
      // Sharing already, nothing more is asked.
      calls.length = 0;
      await made.startScreenShare();
      expect(calls).toEqual([]);
      made.stopScreenShare();
      expect(calls).toEqual([
        { method: "stopScreenShare", options: { origin: ORIGIN, userId: ANN } },
      ]);
    });

    it("does not ask for the screen outside a call or where sharing is not granted", async () => {
      const { call: made } = call();
      await settle();
      calls.length = 0;
      await made.startScreenShare();
      publish(snapshot(connected()));
      await made.startScreenShare();
      expect(calls.filter((c) => c.method === "startScreenShare")).toEqual([]);
    });

    it("keeps the plugin's refusal as shareError, thrown as a ScreenShareError, and dismisses it", async () => {
      const { call: made } = call();
      await settle();
      publish(
        snapshot(
          connected({
            grants: { speak: true, shareScreen: true, camera: false, transferFiles: false },
          }),
        ),
      );
      refusals.set("startScreenShare", Object.assign(new Error("declined"), { code: "denied" }));
      await expect(made.startScreenShare()).rejects.toMatchObject({
        name: "ScreenShareError",
        failure: "denied",
      });
      expect(made.state.shareError).toBe("denied");
      refusals.set("startScreenShare", new Error("no code"));
      await expect(made.startScreenShare()).rejects.toMatchObject({ failure: "failed" });
      expect(made.state.shareError).toBe("failed");
      refusals.delete("startScreenShare");
      calls.length = 0;
      made.clearShareError();
      expect(made.state.shareError).toBeNull();
      expect(calls).toEqual([
        { method: "clearShareError", options: { origin: ORIGIN, userId: ANN } },
      ]);
      // A share that ends after the plugin answered (iOS's picker dismissed) comes as an event.
      emit("share", { origin: ORIGIN, userId: ANN, failure: "unsupported" });
      expect(made.state.shareError).toBe("unsupported");
      emit("share", { origin: "https://other.example", failure: "failed" });
      expect(made.state.shareError).toBe("unsupported");
      emit("share", { origin: ORIGIN, userId: ANN, failure: null });
      expect(made.state.shareError).toBeNull();
      // The call's end starts the next one clean.
      emit("share", { origin: ORIGIN, userId: ANN, failure: "failed" });
      publish(snapshot({ revision: 9, status: "idle" }));
      expect(made.state.shareError).toBeNull();
    });
  });

  describe("what a native call lacks", () => {
    it("rejects a capture made elsewhere, naming it", async () => {
      const { call: made } = call();
      await settle();
      const video = { stop: vi.fn() };
      await expect(
        made.startScreenShare({ prepared: { video, audio: null } as never }),
      ).rejects.toBeInstanceOf(NativeCallUnavailableError);
      expect(video.stop).toHaveBeenCalled();
      await expect(
        made.startExternalScreenShare({ audio: false, stop: () => undefined } as never),
      ).rejects.toBeInstanceOf(NativeCallUnavailableError);
    });

    it("takes the system's default microphone and speaker and refuses one chosen by id", async () => {
      const { call: made } = call();
      await settle();
      await expect(
        made.setAudioDevices({ input: DEFAULT_DEVICE, output: DEFAULT_DEVICE }),
      ).resolves.toBeUndefined();
      await expect(
        made.setAudioDevices({
          input: DEFAULT_DEVICE,
          output: DEFAULT_DEVICE,
          camera: DEFAULT_DEVICE,
        }),
      ).resolves.toBeUndefined();
      await expect(
        made.setAudioDevices({ input: { id: "mic-2", label: "USB" }, output: DEFAULT_DEVICE }),
      ).rejects.toBeInstanceOf(NativeCallUnavailableError);
      expect(calls.filter((c) => c.method !== "state")).toEqual([]);
    });

    it("plays its own sounds, and chooses no microphone or speaker by id", async () => {
      const { call: made } = call();
      await settle();
      expect(made.playsOwnSounds).toBe(true);
      expect(made.hasAudioDeviceChoice).toBe(false);
    });

    it("has no input volume, and refuses a gain on the microphone", async () => {
      const { call: made } = call();
      await settle();
      expect(made.hasInputVolume).toBe(false);
      expect(() => {
        made.setInputVolume(1.5);
      }).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.setInputVolume(1);
      }).toThrow(NativeCallUnavailableError);
    });
  });
});
