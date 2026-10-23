import type { DeviceChoice } from "@aspen/protocol";
import { play, type Playing } from "./speakers";

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
 * The recordings as `data:` URLs, which are decoded without a request, so no Content Security
 * Policy can refuse them (the desktop app's page may connect to no `data:` URL). Each is a chunk
 * of its own, loaded the first time it plays, so none is part of the page's first load.
 */
const recordings = import.meta.glob<string>("./sounds/*.wav", {
  query: "?inline",
  import: "default",
});

/** Each recording decoded, kept from the first time it plays. */
const decoded = new Map<Sound, Promise<AudioBuffer>>();

async function decode(sound: Sound, context: BaseAudioContext): Promise<AudioBuffer> {
  const load = recordings[`./sounds/${SOUND_FILES[sound]}`];
  if (load === undefined) {
    throw new Error(`No recording for ${sound}`);
  }
  const url = await load();
  const text = atob(url.slice(url.indexOf(",") + 1));
  const bytes = Uint8Array.from(text, (c) => c.charCodeAt(0));
  return context.decodeAudioData(bytes.buffer);
}

/**
 * `sound` decoded by `context`. A buffer plays in any context, resampled where the rates differ,
 * so each recording is decoded once. A failure is not kept, so the next play tries again.
 */
function bufferOf(sound: Sound, context: BaseAudioContext): Promise<AudioBuffer> {
  let buffer = decoded.get(sound);
  if (buffer === undefined) {
    buffer = decode(sound, context);
    decoded.set(sound, buffer);
    buffer.catch(() => {
      decoded.delete(sound);
    });
  }
  return buffer;
}

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

/**
 * Plays `sound` through `device` (`play`), unless it played less than its `REPEAT_GAP_MS` ago or
 * `MAX_PLAYING_PER_SOUND` of it are playing. Settles once it has started; a sound the browser
 * refuses to play is simply not heard.
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
    const playing = await play((context) => bufferOf(sound, context), device);
    void playing.ended.then(release);
  } catch {
    release();
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
  let playing: Playing | null = null;
  play((context) => bufferOf(sound, context), device, true).then((started) => {
    playing = started;
    void started.ended.then(release);
    if (stopped) {
      started.stop();
    }
  }, release);
  return () => {
    stopped = true;
    playing?.stop();
    release();
  };
}
