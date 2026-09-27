import { AUDIO_OUTPUT, NOTIFICATION_OUTPUT, PreferenceStore } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { notificationOutputDevice, sortDevices } from "./audioDevices";

describe("sortDevices", () => {
  it("splits microphones from speakers and drops the browser's own defaults", () => {
    const sorted = sortDevices(
      [
        { deviceId: "default", kind: "audioinput", label: "Default - Mic" },
        { deviceId: "communications", kind: "audiooutput", label: "Communications" },
        { deviceId: "m1", kind: "audioinput", label: "Blue Yeti" },
        { deviceId: "m2", kind: "audioinput", label: "" },
        { deviceId: "s1", kind: "audiooutput", label: "Speakers" },
        { deviceId: "c1", kind: "videoinput", label: "Webcam" },
        { deviceId: "", kind: "audioinput", label: "" },
      ],
      (index) => `Microphone ${String(index)}`,
    );
    expect(sorted.inputs).toEqual([
      { id: "m1", label: "Blue Yeti" },
      { id: "m2", label: "Microphone 2" },
    ]);
    expect(sorted.outputs).toEqual([{ id: "s1", label: "Speakers" }]);
  });
});

describe("notificationOutputDevice", () => {
  it("follows the voice output until a device of its own is chosen", async () => {
    const preferences = new PreferenceStore({ storage: null });
    expect(notificationOutputDevice(preferences)).toBeNull();
    await preferences.set(AUDIO_OUTPUT, { id: "s1", label: "Speakers" });
    expect(notificationOutputDevice(preferences)).toEqual({ id: "s1", label: "Speakers" });
    await preferences.set(NOTIFICATION_OUTPUT, { id: "s2", label: "Headset" });
    expect(notificationOutputDevice(preferences)).toEqual({ id: "s2", label: "Headset" });
    await preferences.set(NOTIFICATION_OUTPUT, "default");
    expect(notificationOutputDevice(preferences)).toBeNull();
  });
});
