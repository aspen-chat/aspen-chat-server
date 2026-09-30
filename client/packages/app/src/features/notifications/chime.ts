import type { DeviceChoice } from "@aspen/protocol";
import { playThrough, wavOf } from "./tone";

/** The notification sound: two sine notes, rising a short fourth, each quick to fade. */
const CHIME = [
  { frequency: 659.25, start: 0, length: 0.12 },
  { frequency: 880, start: 0.09, length: 0.22 },
];

let url: string | null = null;

/** Plays the notification sound through `device` (`notificationOutputDevice`). */
export async function playChime(device: DeviceChoice | null): Promise<void> {
  url ??= URL.createObjectURL(wavOf(CHIME, { peak: 0.25, attack: 0.005 }));
  await playThrough(url, device);
}
