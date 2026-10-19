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

/**
 * How many of one sound may play at once. One asked for while that many play is not heard, so a
 * burst of messages or of people joining cannot pile up a din.
 */
export const MAX_PLAYING_PER_SOUND = 2;

const playingCounts = new Map<Sound, number>();

/**
 * Takes one of `sound`'s `MAX_PLAYING_PER_SOUND` places, or answers `null` when every one is taken.
 * The function answered gives the place back, once however often it is called.
 */
function takePlace(sound: Sound): (() => void) | null {
  const playing = playingCounts.get(sound) ?? 0;
  if (playing >= MAX_PLAYING_PER_SOUND) {
    return null;
  }
  playingCounts.set(sound, playing + 1);
  let given = false;
  return () => {
    if (!given) {
      given = true;
      playingCounts.set(sound, (playingCounts.get(sound) ?? 1) - 1);
    }
  };
}

function urlOf(sound: Sound): Promise<string> {
  const load = recordings[`./sounds/${SOUND_FILES[sound]}`];
  if (load === undefined) {
    throw new Error(`No recording for ${sound}`);
  }
  return load();
}

/**
 * Plays `sound` through `device`, unless it played less than its `REPEAT_GAP_MS` ago or
 * `MAX_PLAYING_PER_SOUND` of it are playing.
 */
export async function playSound(sound: Sound, device: DeviceChoice | null): Promise<void> {
  const now = Date.now();
  if (now - (lastPlayed.get(sound) ?? -Infinity) < (REPEAT_GAP_MS[sound] ?? 0)) {
    return;
  }
  const release = takePlace(sound);
  if (release === null) {
    return;
  }
  lastPlayed.set(sound, now);
  try {
    await playThrough(await urlOf(sound), device, release);
  } catch (error) {
    release();
    throw error;
  }
}

/**
 * Plays `sound` through `device` over and over until the function it answers is called, unless
 * `MAX_PLAYING_PER_SOUND` of it are playing, when it is not heard at all.
 */
export function loopSound(sound: Sound, device: DeviceChoice | null): () => void {
  const release = takePlace(sound);
  if (release === null) {
    return () => undefined;
  }
  let stopped = false;
  let playing: HTMLAudioElement | null = null;
  void urlOf(sound)
    .then((url) => (stopped ? null : playThrough(url, device, release, true)))
    .then((audio) => {
      if (stopped) {
        audio?.pause();
      } else {
        playing = audio;
      }
    })
    .catch(release);
  return () => {
    stopped = true;
    playing?.pause();
    release();
  };
}

/**
 * An element playing `url` through `device`, or the system's default where the browser cannot
 * choose. A browser that has not yet been interacted with may refuse to play; the sound is then
 * simply not heard. `release` is called when the element ends, fails, or is refused; a looped
 * one never ends, so whoever stops it releases its place.
 */
async function playThrough(
  url: string,
  device: DeviceChoice | null,
  release: () => void,
  loop = false,
): Promise<HTMLAudioElement> {
  const audio = new Audio(url);
  audio.loop = loop;
  audio.addEventListener("ended", release);
  audio.addEventListener("error", release);
  if (device !== null && "setSinkId" in audio) {
    const outputs = (await navigator.mediaDevices.enumerateDevices()).filter(
      (d) => d.kind === "audiooutput",
    );
    const id = resolveDevice(device, outputs);
    if (id !== null) {
      await audio.setSinkId(id).catch(() => undefined);
    }
  }
  await audio.play().catch(release);
  return audio;
}
