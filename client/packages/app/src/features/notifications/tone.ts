import { resolveDevice, type DeviceChoice } from "@aspen/protocol";

/** The sample rate the app's sounds are made at. */
const RATE = 24_000;

/** A sine note: its pitch, when it starts and how long it rings, in seconds. */
export interface Note {
  readonly frequency: number;
  readonly start: number;
  readonly length: number;
}

/**
 * Notes as 16-bit mono WAV, `seconds` long (or as long as the notes need): each a sine,
 * with `overtone` of its octave above for warmth, rising in `attack` seconds and fading away.
 * The app's sounds are made this way rather than shipped as recordings.
 */
export function wavOf(
  notes: readonly Note[],
  {
    peak,
    attack,
    overtone = 0,
    seconds,
  }: { peak: number; attack: number; overtone?: number; seconds?: number },
): Blob {
  const total = seconds ?? Math.max(...notes.map((n) => n.start + n.length));
  const samples = Math.ceil(total * RATE);
  const pcm = new Float32Array(samples);
  for (const { frequency, start, length } of notes) {
    const first = Math.floor(start * RATE);
    const count = Math.floor(length * RATE);
    for (let i = 0; i < count && first + i < samples; i++) {
      const t = i / RATE;
      const envelope = Math.min(1, t / attack) * Math.exp(-t / (length / 4));
      const wave =
        Math.sin(2 * Math.PI * frequency * t) +
        overtone * Math.sin(2 * Math.PI * frequency * 2 * t);
      pcm[first + i] = (pcm[first + i] ?? 0) + wave * envelope;
    }
  }
  const buffer = new ArrayBuffer(44 + samples * 2);
  const view = new DataView(buffer);
  const text = (at: number, value: string) => {
    for (let i = 0; i < value.length; i++) {
      view.setUint8(at + i, value.charCodeAt(i));
    }
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + samples * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, RATE, true);
  view.setUint32(28, RATE * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, samples * 2, true);
  for (let i = 0; i < samples; i++) {
    const value = Math.max(-1, Math.min(1, (pcm[i] ?? 0) * peak));
    view.setInt16(44 + i * 2, value * 0x7fff, true);
  }
  return new Blob([buffer], { type: "audio/wav" });
}

/**
 * An element playing `url` through `device` (`notificationOutputDevice`), or the system's
 * default where the browser cannot choose. A browser that has not yet been interacted with may
 * refuse to play; the sound is then simply not heard.
 */
export async function playThrough(
  url: string,
  device: DeviceChoice | null,
  loop = false,
): Promise<HTMLAudioElement> {
  const audio = new Audio(url);
  audio.loop = loop;
  if (device !== null && "setSinkId" in audio) {
    const outputs = (await navigator.mediaDevices.enumerateDevices()).filter(
      (d) => d.kind === "audiooutput",
    );
    const id = resolveDevice(device, outputs);
    if (id !== null) {
      await audio.setSinkId(id).catch(() => undefined);
    }
  }
  await audio.play().catch(() => undefined);
  return audio;
}
