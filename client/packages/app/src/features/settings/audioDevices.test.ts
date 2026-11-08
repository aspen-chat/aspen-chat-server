import { AUDIO_OUTPUT, NOTIFICATION_OUTPUT, PreferenceStore } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import {
  type CameraDevice,
  cameraFacing,
  flippedCamera,
  flipsBetweenSides,
  notificationOutputDevice,
  sortDevices,
  trackFacing,
} from "./audioDevices";

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

describe("cameraFacing", () => {
  const camera = (label: string, facingMode?: string[]) => ({
    deviceId: "c",
    kind: "videoinput" as const,
    label,
    ...(facingMode === undefined ? {} : { getCapabilities: () => ({ facingMode }) }),
  });

  it("takes the browser's word first, and the name where it gives none", () => {
    expect(cameraFacing(camera("Back Camera", ["user"]))).toBe("front");
    expect(cameraFacing(camera("", ["environment"]))).toBe("back");
    expect(cameraFacing(camera("Front Camera", ["left"]))).toBeNull();
    expect(cameraFacing(camera("camera2 1, facing front", []))).toBe("front");
    expect(cameraFacing(camera("Back Ultra Wide Camera"))).toBe("back");
    expect(cameraFacing(camera("Rear camera"))).toBe("back");
    expect(cameraFacing(camera("FaceTime HD Camera"))).toBeNull();
    expect(cameraFacing(camera("Caméra avant"))).toBeNull();
  });
});

describe("trackFacing", () => {
  const track = (label: string, facingMode?: string) => ({
    label,
    getSettings: () => (facingMode === undefined ? {} : { facingMode }),
  });

  it("takes the open camera's settings first, and its name where they say nothing", () => {
    expect(trackFacing(track("Front Camera", "environment"))).toBe("back");
    expect(trackFacing(track("", "user"))).toBe("front");
    expect(trackFacing(track("Back Ultra Wide Camera"))).toBe("back");
    expect(trackFacing(track("Integrated Webcam"))).toBeNull();
  });
});

describe("flipping between a phone's sides", () => {
  const phone: CameraDevice[] = [
    { id: "f", label: "Front Camera", facing: "front" },
    { id: "b", label: "Back Camera", facing: "back" },
    { id: "w", label: "Back Ultra Wide Camera", facing: "back" },
  ];

  it("flips where every camera faces front or back, to the first of the other side", () => {
    expect(flipsBetweenSides(phone)).toBe(true);
    expect(flippedCamera(phone, "front")?.id).toBe("b");
    expect(flippedCamera(phone, "back")?.id).toBe("f");
  });

  it("does not flip with one side missing or a camera placed nowhere", () => {
    expect(flipsBetweenSides(phone.slice(1))).toBe(false);
    expect(flipsBetweenSides([...phone, { id: "u", label: "USB Camera", facing: null }])).toBe(
      false,
    );
  });

  it("keeps each camera's side as it sorts them", () => {
    const sorted = sortDevices(
      [
        { deviceId: "f", kind: "videoinput", label: "Front Camera" },
        { deviceId: "x", kind: "videoinput", label: "" },
      ],
      (index) => `Camera ${String(index)}`,
    );
    expect(sorted.cameras).toEqual([
      { id: "f", label: "Front Camera", facing: "front" },
      { id: "x", label: "Camera 2", facing: null },
    ]);
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
