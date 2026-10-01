/**
 * The arithmetic of a list that scrolls itself: how a finger's travel becomes an offset, how
 * fast the list coasts once the finger lifts, and how far past its ends it may be pulled. Pure
 * functions over numbers, so `MessageList` owns the offset and the DOM only shows it.
 *
 * Offsets grow downwards, like a scroll position: 0 shows the top of the content, and the
 * largest offset shows its bottom.
 */

/** What fraction of its speed a coasting list keeps each millisecond. */
export const DECELERATION = 0.998;
/** Below this speed, in pixels per millisecond, a coasting list has stopped. */
export const COASTING_STOPS = 0.03;
/** The fastest a list coasts, in pixels per millisecond: as fast as a finger can fling it. */
export const MAX_VELOCITY = 8;
/** How far back a release looks for the finger's speed, in milliseconds. */
export const VELOCITY_WINDOW_MS = 100;
/** How long a list pulled past an end takes to spring back, in milliseconds. */
export const SPRING_MS = 320;
/** How much of a pull past an end shows, as a fraction of the viewport, at the most. */
const PULL_LIMIT = 0.55;

/** The offsets a content height allows in a viewport height: one offset when it all fits. */
export function bounds(
  contentHeight: number,
  viewportHeight: number,
): { min: number; max: number } {
  const span = contentHeight - viewportHeight;
  return span >= 0 ? { min: 0, max: span } : { min: span, max: span };
}

export function clamp(offset: number, range: { min: number; max: number }): number {
  return Math.min(range.max, Math.max(range.min, offset));
}

/**
 * Where a list pulled `beyond` pixels past an end is shown: the pull shows less and less as it
 * grows, as iOS's lists do, and never more than `PULL_LIMIT` of the viewport.
 */
export function rubberBand(beyond: number, viewportHeight: number): number {
  const limit = viewportHeight * PULL_LIMIT;
  if (beyond === 0 || limit === 0) {
    return 0;
  }
  const sign = Math.sign(beyond);
  const pull = Math.abs(beyond);
  return sign * (1 - 1 / ((pull * PULL_LIMIT) / limit + 1)) * limit;
}

/** The offset to show for a list dragged to `offset`, which may lie past its ends. */
export function shown(
  offset: number,
  range: { min: number; max: number },
  viewportHeight: number,
): number {
  if (offset < range.min) {
    return range.min + rubberBand(offset - range.min, viewportHeight);
  }
  if (offset > range.max) {
    return range.max + rubberBand(offset - range.max, viewportHeight);
  }
  return offset;
}

/** A coasting list's speed after `elapsedMs`, and how far it travelled meanwhile. */
export function coast(
  velocity: number,
  elapsedMs: number,
): { velocity: number; travelled: number } {
  const kept = Math.pow(DECELERATION, elapsedMs);
  // The integral of v·k^t from 0 to the elapsed time.
  const travelled = (velocity * (kept - 1)) / Math.log(DECELERATION);
  return { velocity: velocity * kept, travelled };
}

/**
 * Where a list springing back from `from` to `to` is at `elapsedMs` into the spring: an
 * ease-out that reaches `to` at `SPRING_MS` and does not overshoot.
 */
export function spring(from: number, to: number, elapsedMs: number): number {
  const t = Math.min(1, elapsedMs / SPRING_MS);
  const eased = 1 - Math.pow(1 - t, 3);
  return from + (to - from) * eased;
}

/** The finger's recent positions, for its speed when it lifts. */
export class VelocityTracker {
  readonly #samples: { y: number; at: number }[] = [];

  reset(): void {
    this.#samples.length = 0;
  }

  add(y: number, at: number): void {
    this.#samples.push({ y, at });
    while (this.#samples.length > 0 && at - (this.#samples[0]?.at ?? at) > VELOCITY_WINDOW_MS) {
      this.#samples.shift();
    }
  }

  /**
   * The finger's speed over the window, in pixels per millisecond, positive when it moved down
   * the screen; zero for a finger that rested before lifting.
   */
  velocity(at: number): number {
    const first = this.#samples[0];
    const last = this.#samples[this.#samples.length - 1];
    if (first === undefined || last === undefined || last === first) {
      return 0;
    }
    if (at - last.at > VELOCITY_WINDOW_MS / 2) {
      return 0;
    }
    const elapsed = last.at - first.at;
    return elapsed <= 0 ? 0 : (last.y - first.y) / elapsed;
  }
}
