import { resolveDevice, type DeviceChoice } from "@aspen/protocol";

/** The app's sounds, each a recording in `sounds/` (`SOUND_FILES`). */
export type Sound =
  /** A message or plugin notice telling of itself. */
  | "chime"
  /** A call ringing the user, looped. */
  | "ringtone"
  /** The user's call ringing someone, looped under the call. */
  | "dialTone"
  | "callJoined"
  | "callLeft"
  | "disconnected";

/** Each sound's file in `sounds/`. */
const SOUND_FILES: Record<Sound, string> = {
  chime: "chime.wav",
  ringtone: "ringtone.wav",
  dialTone: "dial-tone.wav",
  callJoined: "call-joined.wav",
  callLeft: "call-left.wav",
  disconnected: "disconnected.wav",
};

/**
 * The recordings as `data:` URLs, which every build's Content Security Policy lets play (the
 * desktop app's page, loaded from a file, may play nothing from its own origin). Each is a chunk
 * of its own, loaded the first time it plays, so none is part of the page's first load.
 */
const recordings = import.meta.glob<string>("./sounds/*.wav", {
  query: "?inline",
  import: "default",
});

/**
 * How soon a sound may play again, for those that may not repeat at once. Several people
 * joining at once are heard as one, and the disconnected sound is said once when a network
 * failure drops a call and its deployment together.
 */
const REPEAT_GAP_MS: Partial<Record<Sound, number>> = {
  callJoined: 250,
  callLeft: 250,
  disconnected: 30_000,
};

const lastPlayed = new Map<Sound, number>();

function urlOf(sound: Sound): Promise<string> {
  const load = recordings[`./sounds/${SOUND_FILES[sound]}`];
  if (load === undefined) {
    throw new Error(`No recording for ${sound}`);
  }
  return load();
}

/** Plays `sound` through `device`, unless it played less than its `REPEAT_GAP_MS` ago. */
export async function playSound(sound: Sound, device: DeviceChoice | null): Promise<void> {
  const now = Date.now();
  if (now - (lastPlayed.get(sound) ?? -Infinity) < (REPEAT_GAP_MS[sound] ?? 0)) {
    return;
  }
  lastPlayed.set(sound, now);
  await playThrough(await urlOf(sound), device);
}

/** Plays `sound` through `device` over and over until the function it answers is called. */
export function loopSound(sound: Sound, device: DeviceChoice | null): () => void {
  let stopped = false;
  let playing: HTMLAudioElement | null = null;
  void urlOf(sound)
    .then((url) => (stopped ? null : playThrough(url, device, true)))
    .then((audio) => {
      if (stopped) {
        audio?.pause();
      } else {
        playing = audio;
      }
    });
  return () => {
    stopped = true;
    playing?.pause();
  };
}

/**
 * An element playing `url` through `device`, or the system's default where the browser cannot
 * choose. A browser that has not yet been interacted with may refuse to play; the sound is then
 * simply not heard.
 */
async function playThrough(
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
