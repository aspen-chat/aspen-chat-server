import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const phone = { receiver: true, speaker: true, other: false };
let changed: ((outputs: typeof phone) => void) | null = null;

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    isNativePlatform: () => true,
    isPluginAvailable: (name: string) => name === "AspenAudioRoute",
  },
  registerPlugin: () => ({
    outputs: () => Promise.resolve(phone),
    addListener: (_event: string, listener: (outputs: typeof phone) => void) => {
      changed = listener;
      return Promise.resolve({ remove: () => Promise.resolve() });
    },
  }),
}));

const { choosesSpeakerphone, usePhoneOutputs } = await import("./audioRoute");

/** What `usePhoneOutputs` answers each time it draws. */
const seen: unknown[] = [];

function Outputs() {
  seen.push(usePhoneOutputs());
  return null;
}

describe("audioRoute", () => {
  it("chooses only between a phone's own receiver and loudspeaker", () => {
    expect(choosesSpeakerphone(phone)).toBe(true);
    expect(choosesSpeakerphone({ ...phone, other: true })).toBe(false);
    expect(choosesSpeakerphone({ ...phone, receiver: false })).toBe(false);
    expect(choosesSpeakerphone(null)).toBe(false);
  });

  it("reads the outputs, and follows what is connected and taken away", async () => {
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(<Outputs />);
      await Promise.resolve();
    });
    expect(seen.at(-1)).toEqual(phone);
    act(() => {
      changed?.({ ...phone, other: true });
    });
    expect(seen.at(-1)).toEqual({ ...phone, other: true });
    act(() => {
      root.unmount();
    });
  });
});
