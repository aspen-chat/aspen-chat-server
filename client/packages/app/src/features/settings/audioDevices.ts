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

export interface AudioDevices {
  readonly inputs: readonly AudioDevice[];
  readonly outputs: readonly AudioDevice[];
}

/**
 * Sorts the browser's device list into microphones and speakers. The browser's own `default`
 * entries are dropped, since the app offers the system default itself, and the id
 * `communications` (Windows' second default) with them. A device without a label, which is
 * what a browser gives before permission is granted, is named by its position.
 */
export function sortDevices(
  devices: readonly Pick<MediaDeviceInfo, "deviceId" | "kind" | "label">[],
  unnamed: (index: number) => string,
): AudioDevices {
  const inputs: AudioDevice[] = [];
  const outputs: AudioDevice[] = [];
  for (const device of devices) {
    if (
      device.deviceId === "" ||
      device.deviceId === "default" ||
      device.deviceId === "communications"
    ) {
      continue;
    }
    const list =
      device.kind === "audioinput" ? inputs : device.kind === "audiooutput" ? outputs : null;
    if (list === null) {
      continue;
    }
    list.push({
      id: device.deviceId,
      label: device.label === "" ? unnamed(list.length + 1) : device.label,
    });
  }
  return { inputs, outputs };
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
