import {
  AUDIO_OUTPUT,
  DEFAULT_DEVICE,
  NOTIFICATION_OUTPUT,
  SAME_AS_VOICE,
  type DeviceChoice,
  type PreferenceStore,
} from "@aspen/protocol";

/** A microphone or speaker as the browser lists it. */
export interface AudioDevice {
  readonly id: string;
  readonly label: string;
}

/** Which way a camera looks: toward its user, or away from them. */
export type CameraFacing = "front" | "back";

export interface CameraDevice extends AudioDevice {
  /** `null` for a camera that looks neither way, or that neither the browser nor its name places. */
  readonly facing: CameraFacing | null;
}

export interface AudioDevices {
  readonly inputs: readonly AudioDevice[];
  readonly outputs: readonly AudioDevice[];
  readonly cameras: readonly CameraDevice[];
}

/** A device as `enumerateDevices` lists it; cameras say which way they face where the browser can. */
type ListedDevice = Pick<MediaDeviceInfo, "deviceId" | "kind" | "label"> & {
  getCapabilities?: () => { facingMode?: string[] };
};

/**
 * Which way a camera faces: as the browser says (`facingMode`, which Chromium gives), or else as
 * its name says, which is how Safari tells ("Front Camera", "Back Ultra Wide Camera") and
 * Chromium on Android too ("camera2 1, facing front"). A name in another language places none.
 */
export function cameraFacing(device: ListedDevice): CameraFacing | null {
  const modes =
    typeof device.getCapabilities === "function" ? (device.getCapabilities().facingMode ?? []) : [];
  if (modes.length > 0) {
    if (modes.length === 1 && modes[0] === "user") {
      return "front";
    }
    return modes.length === 1 && modes[0] === "environment" ? "back" : null;
  }
  const front = /\bfront\b/i.test(device.label);
  const back = /\b(back|rear)\b/i.test(device.label);
  return front === back ? null : front ? "front" : "back";
}

/**
 * Whether the cameras are a phone's: some facing front, some back, and none either way unknown,
 * however many lenses the back has. Then one button flips between the two sides.
 */
export function flipsBetweenSides(cameras: readonly CameraDevice[]): boolean {
  return (
    cameras.every((camera) => camera.facing !== null) &&
    cameras.some((camera) => camera.facing === "front") &&
    cameras.some((camera) => camera.facing === "back")
  );
}

/**
 * The camera a flip from `facing` switches to: the first listed of the other side, which is
 * the main camera of that side on the phones that list several (iPhones list "Back Camera"
 * before their other lenses, Android its lowest numbered).
 */
export function flippedCamera(
  cameras: readonly CameraDevice[],
  facing: CameraFacing,
): CameraDevice | undefined {
  return cameras.find((camera) => camera.facing !== null && camera.facing !== facing);
}

/**
 * Sorts the browser's device list into microphones, speakers, and cameras. The browser's own `default`
 * entries are dropped, since the app offers the system default itself, and the id
 * `communications` (Windows' second default) with them. A device without a label, which is
 * what a browser gives before permission is granted, is named by its position.
 */
export function sortDevices(
  devices: readonly ListedDevice[],
  unnamed: (index: number) => string,
): AudioDevices {
  const inputs: AudioDevice[] = [];
  const outputs: AudioDevice[] = [];
  const cameras: CameraDevice[] = [];
  for (const device of devices) {
    if (
      device.deviceId === "" ||
      device.deviceId === "default" ||
      device.deviceId === "communications"
    ) {
      continue;
    }
    if (device.kind === "videoinput") {
      cameras.push({
        id: device.deviceId,
        label: device.label === "" ? unnamed(cameras.length + 1) : device.label,
        facing: cameraFacing(device),
      });
      continue;
    }
    const list = device.kind === "audioinput" ? inputs : outputs;
    list.push({
      id: device.deviceId,
      label: device.label === "" ? unnamed(list.length + 1) : device.label,
    });
  }
  return { inputs, outputs, cameras };
}

/**
 * The speaker notification sounds should play through, or `null` for the system default.
 * "Same as voice chat" follows the voice output preference. A sound player resolves the choice
 * against the devices at hand with `resolveDevice` when it plays.
 */
export function notificationOutputDevice(preferences: PreferenceStore): DeviceChoice | null {
  const chosen = preferences.get(NOTIFICATION_OUTPUT);
  const resolved = chosen === SAME_AS_VOICE ? preferences.get(AUDIO_OUTPUT) : chosen;
  return resolved === DEFAULT_DEVICE ? null : resolved;
}

/** The speaker voice chat plays through, or `null` for the system default. */
export function voiceOutputDevice(preferences: PreferenceStore): DeviceChoice | null {
  const chosen = preferences.get(AUDIO_OUTPUT);
  return chosen === DEFAULT_DEVICE ? null : chosen;
}
