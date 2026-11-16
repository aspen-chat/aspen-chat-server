import { describe, expect, it } from "vitest";
import type { PhoneOutputs } from "@/api/audioRoute";
import { listsMicrophones, listsSpeakers } from "./deviceControls";

/** A call that takes devices by id, as a browser's does. */
const browser = { hasAudioDeviceChoice: true };
/** A call that takes none, as the mobile apps' native calls do. */
const native = { hasAudioDeviceChoice: false };

const phone: PhoneOutputs = { receiver: true, speaker: true, other: false };
const tablet: PhoneOutputs = { receiver: false, speaker: true, other: false };

describe("listsMicrophones", () => {
  it("lists them on a computer whatever their number, where the call takes one", () => {
    expect(listsMicrophones(browser, false, 0)).toBe(true);
    expect(listsMicrophones(browser, false, 1)).toBe(true);
    expect(listsMicrophones(browser, false, 3)).toBe(true);
  });

  it("lists them on a phone or tablet only when there are several", () => {
    expect(listsMicrophones(browser, true, 0)).toBe(false);
    expect(listsMicrophones(browser, true, 1)).toBe(false);
    expect(listsMicrophones(browser, true, 2)).toBe(true);
  });

  it("lists none for a call that takes no microphone by id", () => {
    expect(listsMicrophones(native, false, 3)).toBe(false);
    expect(listsMicrophones(native, true, 3)).toBe(false);
  });
});

describe("listsSpeakers", () => {
  it("lists them on a computer whether or not the browser can choose, where the call takes one", () => {
    expect(listsSpeakers(browser, false, tablet, true, 1)).toBe(true);
    expect(listsSpeakers(browser, false, tablet, false, 0)).toBe(true);
  });

  it("lists them on a tablet only where the browser can choose among several", () => {
    expect(listsSpeakers(browser, true, tablet, true, 2)).toBe(true);
    expect(listsSpeakers(browser, true, tablet, true, 1)).toBe(false);
    expect(listsSpeakers(browser, true, tablet, false, 2)).toBe(false);
  });

  it("lists none where the Speaker button chooses between the receiver and loudspeaker", () => {
    expect(listsSpeakers(browser, true, phone, true, 2)).toBe(false);
  });

  it("lists none for a call that takes no speaker by id", () => {
    expect(listsSpeakers(native, false, tablet, true, 2)).toBe(false);
    expect(listsSpeakers(native, true, tablet, true, 2)).toBe(false);
  });
});
