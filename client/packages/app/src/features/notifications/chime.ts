import { resolveDevice, type DeviceChoice } from "@aspen/protocol";

/** The chime's sample rate, and its two notes, rising: a short fourth. */
const RATE = 24_000;
const NOTES: readonly { frequency: number; start: number; length: number }[] = [
  { frequency: 659.25, start: 0, length: 0.12 },
  { frequency: 880, start: 0.09, length: 0.22 },
];
const PEAK = 0.25;

/**
 * The notification sound, made here rather than shipped as a recording: two sine notes, each
 * rising in 5 ms and fading away, as 16-bit mono WAV.
 */
function chimeWav(): Blob {
  const seconds = Math.max(...NOTES.map((n) => n.start + n.length));
  const samples = Math.ceil(seconds * RATE);
  const pcm = new Float32Array(samples);
  for (const { frequency, start, length } of NOTES) {
    const first = Math.floor(start * RATE);
    const count = Math.floor(length * RATE);
    for (let i = 0; i < count && first + i < samples; i++) {
      const t = i / RATE;
      const attack = Math.min(1, t / 0.005);
      const decay = Math.exp(-t / (length / 4));
      pcm[first + i] =
        (pcm[first + i] ?? 0) + Math.sin(2 * Math.PI * frequency * t) * attack * decay;
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
    const value = Math.max(-1, Math.min(1, (pcm[i] ?? 0) * PEAK));
    view.setInt16(44 + i * 2, value * 0x7fff, true);
  }
  return new Blob([buffer], { type: "audio/wav" });
}

let url: string | null = null;

/**
 * Plays the notification sound through `device` (`notificationOutputDevice`), or the system's
 * default where the browser cannot choose. A browser that has not yet been interacted with may
 * refuse to play; the sound is then simply not heard.
 */
export async function playChime(device: DeviceChoice | null): Promise<void> {
  url ??= URL.createObjectURL(chimeWav());
  const audio = new Audio(url);
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
}
