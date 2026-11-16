import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { PhoneRoutes } from "./nativeCall";

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

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    isPluginAvailable: (name: string) => name === "AspenCall",
  },
  registerPlugin: () => ({
    routes: () => Promise.resolve(phone),
    addListener: (_event: string, listener: (routes: typeof phone) => void) => {
      changed = listener;
      return Promise.resolve({ remove: () => Promise.resolve() });
    },
  }),
}));

const { choosesSpeakerphone, outputsOf, usePhoneOutputs } = await import("./audioRoute");

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
});
