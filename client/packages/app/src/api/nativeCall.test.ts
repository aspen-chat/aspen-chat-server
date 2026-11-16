import type { AspenClient, EngineCallState, VoiceCallOptions, VoiceMedia } from "@aspen/protocol";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PublishedState } from "./nativeCall";

/** What the native side was asked, in order. */
const calls: { method: string; options: Record<string, unknown> | undefined }[] = [];
/** What `state` answers next, by origin. */
const current = new Map<string, { origin: string; state: string }>();
/** What `state` rejects with, when set. */
let stateFailure: Error | null = null;
/** What `join` rejects with, when set. */
let joinFailure: Error | null = null;
/** What each method named here rejects with. */
const refusals = new Map<string, Error>();
const listeners = new Map<string, (event: PublishedState) => void>();

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
            return (event: string, listener: (event: PublishedState) => void) => {
              listeners.set(event, listener);
              return Promise.resolve({ remove: () => Promise.resolve() });
            };
          }
          return (options: Record<string, unknown> | undefined) => {
            calls.push({ method, options });
            if (method === "state") {
              // An engine not yet made answers its idle state, revision 0, as the plugin does.
              return stateFailure === null
                ? Promise.resolve(
                    current.get(options?.origin as string) ??
                      snapshot({ origin: options?.origin, revision: 0 }),
                  )
                : Promise.reject(stateFailure);
            }
            if (method === "join" && joinFailure !== null) {
              return Promise.reject(joinFailure);
            }
            const refusal = refusals.get(method);
            return refusal === undefined ? Promise.resolve(undefined) : Promise.reject(refusal);
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
} = await import("./nativeCall");

const { DEFAULT_DEVICE, IDLE_CALL_STATE, NO_FILES } = await import("@aspen/protocol");

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
    ...patch,
  };
}

/** `engineState(patch)` as the plugin publishes it: the engine's JSON beside the deployment. */
function snapshot(patch: Record<string, unknown> = {}): PublishedState {
  const { origin, ...rest } = patch;
  return {
    origin: typeof origin === "string" ? origin : ORIGIN,
    state: JSON.stringify(engineState(rest)),
  };
}

function participant(user: string, held = false) {
  return { user, muted: false, deafened: false, held, speaking: false };
}

/** Lets the bridge's awaited calls run to the next one that waits on the native side. */
async function settle(): Promise<void> {
  for (let i = 0; i < 20; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

/** The engine publishes `snapshot` to the page. */
function publish(published: PublishedState): void {
  listeners.get("state")?.(published);
}

const media = {} as VoiceMedia;

/** A call with the plugin faked around it, and what it reported and heard. */
function call(options: Partial<VoiceCallOptions> = {}, origin = ORIGIN) {
  const reported: unknown[] = [];
  const wakes: (() => void)[] = [];
  const changes: string[] = [];
  const made = new NativeCall(
    { client: { baseUrl: origin } as AspenClient, media, ...options },
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

    it("keeps the voice-only fields whatever the grants say", () => {
      const state = stateOf(
        engineState({
          grants: { speak: true, shareScreen: true, camera: true, transferFiles: true },
        }),
      );
      expect(state.canShare).toBe(false);
      expect(state.canCamera).toBe(false);
      expect(state.canTransfer).toBe(false);
      expect(state.files).toBe(NO_FILES);
      expect(state.sharingScreen).toBe(false);
      expect(state.localScreen).toBeNull();
      expect(state.localCamera).toBeNull();
      expect(state.screens).toEqual([]);
      expect(state.cameras).toEqual([]);
      expect(state.cameraError).toBeNull();
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

  describe("parseState", () => {
    it("reads the engine's JSON, and refuses what is not it", () => {
      expect(parseState(JSON.stringify(engineState({ revision: 4 }))).revision).toBe(4);
      expect(() => parseState("not json")).toThrow(NativeCallProtocolError);
      expect(() => parseState("[]")).toThrow(NativeCallProtocolError);
      expect(() => parseState("null")).toThrow(NativeCallProtocolError);
    });
  });

  describe("snapshots", () => {
    it("reads the current snapshot as it is made, and again as the page wakes", async () => {
      current.set(ORIGIN, snapshot({ revision: 3, status: "connected", channel: CHANNEL }));
      const { call: made, wake } = call();
      await settle();
      expect(calls.filter((c) => c.method === "state").map((c) => c.options)).toEqual([
        { origin: ORIGIN },
      ]);
      expect(made.state.status).toBe("connected");
      current.set(ORIGIN, snapshot({ revision: 4, status: "idle" }));
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

    it("reports a snapshot it cannot map and shows nothing of it", async () => {
      const { call: made, reported } = call();
      await settle();
      publish(snapshot({ revision: 2, status: "connected", endedReason: "bored" }));
      publish({ origin: ORIGIN, state: "{not json" });
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
        { method: "leave", options: { origin: ORIGIN } },
        { method: "setMuted", options: { origin: ORIGIN, muted: true } },
        { method: "setDeafened", options: { origin: ORIGIN, deafened: true } },
        { method: "setHeld", options: { origin: ORIGIN, held: true } },
        { method: "setServerMuted", options: { origin: ORIGIN, serverMuted: true } },
        { method: "clearStateRefusal", options: { origin: ORIGIN } },
        { method: "acknowledgeEnd", options: { origin: ORIGIN } },
        {
          method: "sessionEnded",
          options: { origin: ORIGIN, session: "s1", channel: CHANNEL, reason: "serverLost" },
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
        { origin: ORIGIN, user: BOB, source: "microphone", gain: 0.5 },
        { origin: ORIGIN, user: BOB, source: "screenAudio", gain: 0 },
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
      expect(volumes()[2]).toEqual({ origin: ORIGIN, user: CAROL, source: "microphone", gain: 1 });
      // A refresh reads every present participant again.
      gains.set(`${CAROL} microphone`, 0);
      made.refreshVolumes();
      expect(volumes()).toHaveLength(8);
      expect(volumes()[6]).toEqual({ origin: ORIGIN, user: CAROL, source: "microphone", gain: 0 });
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
        { origin: ORIGIN, user: BOB, source: "microphone", gain: 1.5 },
        { origin: ORIGIN, user: BOB, source: "screenAudio", gain: 0 },
      ]);
    });
  });

  describe("what this stage lacks", () => {
    it("rejects the camera, sharing, and file transfers, naming them", async () => {
      const { call: made } = call();
      await settle();
      await expect(made.startCamera()).rejects.toBeInstanceOf(NativeCallUnavailableError);
      await expect(made.startScreenShare()).rejects.toBeInstanceOf(NativeCallUnavailableError);
      await expect(
        made.startExternalScreenShare({ audio: false, stop: () => undefined } as never),
      ).rejects.toBeInstanceOf(NativeCallUnavailableError);
      expect(() => {
        made.stopCamera();
      }).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.clearCameraError();
      }).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.stopScreenShare();
      }).toThrow(NativeCallUnavailableError);
      expect(() => made.offerFile(new Blob(), "a", false, 60)).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.withdrawOffer("o");
      }).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.acceptOffer("o", "relayOnly");
      }).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.cancelTransfer("o", BOB);
      }).toThrow(NativeCallUnavailableError);
      expect(() => {
        made.dismissTransfer("o", BOB);
      }).toThrow(NativeCallUnavailableError);
      expect(() => made.offerFile(new Blob(), "a", false, 60)).toThrow(
        "Offering a file is not part of this build's native calls yet",
      );
    });

    it("shows video to no effect, and releases it", async () => {
      const { call: made } = call();
      await settle();
      const release = made.showVideo();
      expect(typeof release).toBe("function");
      release();
      expect(calls.filter((c) => c.method !== "state")).toEqual([]);
    });

    it("takes the system's default devices and refuses one chosen by id", async () => {
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

    it("sends the microphone as captured, and reports any other gain", async () => {
      const { call: made, reported } = call();
      await settle();
      made.setInputVolume(1);
      expect(reported).toEqual([]);
      made.setInputVolume(1.5);
      expect(reported).toHaveLength(1);
      expect(reported[0]).toBeInstanceOf(NativeCallUnavailableError);
    });
  });
});
