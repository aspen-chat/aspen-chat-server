import type { VoiceCallState } from "@aspen/protocol";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { PhoneRoutes, RouteKind } from "./nativeCall";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A phone with its receiver and loudspeaker, playing through the receiver. */
const phone: PhoneRoutes = {
  routes: [
    { kind: "receiver", name: null, inUse: true },
    { kind: "loudspeaker", name: null, inUse: false },
  ],
  hasReceiver: true,
  chosen: null,
};
let changed: ((routes: typeof phone) => void) | null = null;
/** Every route command the page sent the plugin, in order. */
const commands: string[] = [];

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    isPluginAvailable: (name: string) => name === "AspenCall",
  },
  registerPlugin: () => ({
    routes: () => Promise.resolve(phone),
    setRoute: ({ kind }: { kind: RouteKind }) => {
      commands.push(`set ${kind}`);
      return Promise.resolve();
    },
    clearRoute: () => {
      commands.push("clear");
      return Promise.resolve();
    },
    addListener: (_event: string, listener: (routes: typeof phone) => void) => {
      changed = listener;
      return Promise.resolve({ remove: () => Promise.resolve() });
    },
  }),
}));

/** The call the page is in, as `useCallSource` answers it: `null` outside one. */
let callStatus: VoiceCallState["status"] = "idle";
/** The `SPEAKERPHONE` preference as the page reads it. */
let speakerphone = false;
const preferenceListeners = new Set<() => void>();
const voiceListeners = new Set<() => void>();
vi.mock("./calls", () => ({
  useCallSource: () => ({
    sync: {
      voice: {
        get state() {
          return { status: callStatus, endedReason: null };
        },
        subscribe: (listener: () => void) => {
          voiceListeners.add(listener);
          return () => voiceListeners.delete(listener);
        },
      },
      preferences: {
        get: () => speakerphone,
        subscribe: (listener: () => void) => {
          preferenceListeners.add(listener);
          return () => preferenceListeners.delete(listener);
        },
      },
    },
  }),
}));

const { choosesSpeakerphone, outputsOf, useFollowSpeakerphone, usePhoneOutputs } =
  await import("./audioRoute");

const outputs = { receiver: true, speaker: true, other: false };
const headset: PhoneRoutes = {
  ...phone,
  routes: [...phone.routes, { kind: "bluetooth", name: "Buds", inUse: true }],
};

/** What `usePhoneOutputs` answers each time it draws. */
const seen: unknown[] = [];

function Outputs() {
  seen.push(usePhoneOutputs());
  return null;
}

/** Follows the speakerphone choice, drawn again whenever the call or the preference changes. */
function Follower({ version }: { version: number }) {
  useFollowSpeakerphone();
  return <span data-version={version} />;
}

describe("audioRoute", () => {
  it("reads the outputs off the typed routes", () => {
    expect(outputsOf(phone)).toEqual(outputs);
    expect(outputsOf(headset)).toEqual({ ...outputs, other: true });
    expect(outputsOf({ ...phone, hasReceiver: false })).toEqual({ ...outputs, receiver: false });
    // Outside a call Android lists no routes; the device still has its receiver and loudspeaker.
    expect(outputsOf({ routes: [], hasReceiver: true, chosen: null })).toEqual(outputs);
  });

  it("chooses only between a phone's own receiver and loudspeaker", () => {
    expect(choosesSpeakerphone(outputs)).toBe(true);
    expect(choosesSpeakerphone({ ...outputs, other: true })).toBe(false);
    expect(choosesSpeakerphone({ ...outputs, receiver: false })).toBe(false);
    expect(choosesSpeakerphone(null)).toBe(false);
  });

  it("reads the outputs, and follows what is connected and taken away", async () => {
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(<Outputs />);
      await Promise.resolve();
    });
    expect(seen.at(-1)).toEqual(outputs);
    act(() => {
      changed?.(headset);
    });
    expect(seen.at(-1)).toEqual({ ...outputs, other: true });
    act(() => {
      root.unmount();
    });
  });

  it("applies the choice to a call that exists, and withdraws only what it applied", async () => {
    const root = createRoot(document.createElement("div"));
    let version = 0;
    const draw = async () => {
      version += 1;
      await act(async () => {
        root.render(<Follower version={version} />);
        await Promise.resolve();
      });
    };
    await draw();
    // Outside a call nothing is applied, so the routes changing (a choice made elsewhere, as
    // the plugin's own test makes one) withdraws nothing.
    act(() => {
      changed?.({ ...phone, chosen: "loudspeaker" });
    });
    act(() => {
      changed?.(phone);
    });
    expect(commands).toEqual([]);
    // In a call the choice is applied, and again as it changes.
    callStatus = "connected";
    await draw();
    expect(commands).toEqual(["set receiver"]);
    speakerphone = true;
    await draw();
    expect(commands).toEqual(["set receiver", "set loudspeaker"]);
    // A headset connected: the page's choice is withdrawn, once, and the routes changing again
    // while it is connected withdraws nothing more.
    act(() => {
      changed?.(headset);
    });
    expect(commands).toEqual(["set receiver", "set loudspeaker", "clear"]);
    act(() => {
      changed?.({ ...headset, chosen: null });
    });
    expect(commands).toEqual(["set receiver", "set loudspeaker", "clear"]);
    // Taken away: applied again; the call's end withdraws it.
    act(() => {
      changed?.(phone);
    });
    expect(commands).toEqual(["set receiver", "set loudspeaker", "clear", "set loudspeaker"]);
    callStatus = "idle";
    await draw();
    expect(commands).toEqual([
      "set receiver",
      "set loudspeaker",
      "clear",
      "set loudspeaker",
      "clear",
    ]);
    act(() => {
      root.unmount();
    });
  });
});
