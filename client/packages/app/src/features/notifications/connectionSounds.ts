import type { AspenSync } from "@aspen/protocol";
import type { Sound } from "./sounds";

/**
 * How long a deployment's event stream may be down before the user hears that it is: long
 * enough for the stream's first few reconnection attempts (immediately, then after half a
 * second, one, and two), so a blip nobody would otherwise notice makes no sound.
 */
export const DISCONNECTED_GRACE_MS = 5_000;

/** What `watchConnectionSounds` reads of a deployment's sync. */
export type ConnectionSoundSource = Pick<AspenSync, "status" | "subscribe">;

/**
 * Plays `disconnected` once when a deployment's event stream drops from live and has not come
 * back within `DISCONNECTED_GRACE_MS`, until the function it answers is called. Coming back
 * (live, or resyncing what it missed) or being stopped (signing out) calls it off.
 */
export function watchConnectionSounds(
  sync: ConnectionSoundSource,
  play: (sound: Sound) => void,
  timers: Pick<typeof globalThis, "setTimeout" | "clearTimeout"> = globalThis,
): () => void {
  let status = sync.status;
  let pending: ReturnType<typeof setTimeout> | null = null;
  const callOff = () => {
    if (pending !== null) {
      timers.clearTimeout(pending);
      pending = null;
    }
  };
  const stop = sync.subscribe(() => {
    const before = status;
    status = sync.status;
    if (before === "live" && status === "reconnecting") {
      callOff();
      pending = timers.setTimeout(() => {
        pending = null;
        play("disconnected");
      }, DISCONNECTED_GRACE_MS);
    } else if (status !== "reconnecting") {
      callOff();
    }
  });
  return () => {
    stop();
    callOff();
  };
}
