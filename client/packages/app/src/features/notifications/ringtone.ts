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
  return loop(url, device);
}

/**
 * The dial tone a caller hears while their call rings someone: the telephone's ringback, 440
 * and 480 Hz together, held for 1.2 seconds in every 4, and quiet, since it plays under the
 * call itself.
 */
const RINGBACK = [
  { frequency: 440, start: 0, length: 1.2 },
  { frequency: 480, start: 0, length: 1.2 },
];

let ringbackUrl: string | null = null;

/**
 * Starts the dial tone through `device` (the voice chat's speaker), looping until the function
 * it answers is called.
 */
export function startDialTone(device: DeviceChoice | null): () => void {
  ringbackUrl ??= URL.createObjectURL(
    wavOf(RINGBACK, { peak: 0.05, attack: 0.03, seconds: 4, sustain: true }),
  );
  return loop(ringbackUrl, device);
}

/** Plays `url` through `device` over and over until the function it answers is called. */
function loop(url: string, device: DeviceChoice | null): () => void {
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
