import type { DeviceChoice } from "@aspen/protocol";
import { playThrough, wavOf } from "./tone";

/**
 * The ringtone: a soft rising D major arpeggio, warmed by a quiet octave overtone and let ring
 * out, then a rest, 2.4 seconds in all, looped. Gentler than the chime: a slower attack and a
 * lower peak, since it repeats.
 */
const RING = [
  { frequency: 587.33, start: 0, length: 0.9 },
  { frequency: 739.99, start: 0.16, length: 0.9 },
  { frequency: 880, start: 0.32, length: 1.1 },
];

let url: string | null = null;

/**
 * Starts the ringtone through `device` (`notificationOutputDevice`), looping until the function
 * it answers is called.
 */
export function startRingtone(device: DeviceChoice | null): () => void {
  url ??= URL.createObjectURL(
    wavOf(RING, { peak: 0.14, attack: 0.02, overtone: 0.15, seconds: 2.4 }),
  );
  let stopped = false;
  let playing: HTMLAudioElement | null = null;
  void playThrough(url, device, true).then((audio) => {
    if (stopped) {
      audio.pause();
    } else {
      playing = audio;
    }
  });
  return () => {
    stopped = true;
    playing?.pause();
  };
}
