import { createContext, useContext } from "react";

/**
 * Whether a message list is at rest: no finger on it, and no scroll of the reader's for
 * `SETTLE_MS`. Content that must change its size without the reader's doing, such as a picture
 * of unknown size arriving, waits for rest (`whenStill`): the list holds what is in view still
 * through a change at rest, in the frame the change is drawn, but a browser without scroll
 * anchoring of its own (Safari) shows any change made while the list moves as a jump.
 */
export class Stillness {
  /** How long after its last scroll, with no finger down, the list counts as at rest. */
  static readonly SETTLE_MS = 150;

  #touching = false;
  #movingUntil = 0;
  readonly #waiters = new Set<() => void>();
  #timer: ReturnType<typeof setTimeout> | undefined;

  get still(): boolean {
    return !this.#touching && Date.now() >= this.#movingUntil;
  }

  /** The reader scrolled the list. */
  noteScroll(): void {
    this.#movingUntil = Date.now() + Stillness.SETTLE_MS;
    this.#schedule();
  }

  /** A finger came down on the list, or left it. */
  setTouching(touching: boolean): void {
    this.#touching = touching;
    this.#schedule();
  }

  /** Calls `then` once the list is at rest: at once if it is. Returns a way to stop waiting. */
  whenStill(then: () => void): () => void {
    if (this.still) {
      then();
      return () => undefined;
    }
    this.#waiters.add(then);
    this.#schedule();
    return () => {
      this.#waiters.delete(then);
    };
  }

  #schedule(): void {
    clearTimeout(this.#timer);
    if (this.#waiters.size === 0 || this.#touching) {
      return;
    }
    this.#timer = setTimeout(
      () => {
        if (!this.still) {
          this.#schedule();
          return;
        }
        const waiting = Array.from(this.#waiters);
        this.#waiters.clear();
        for (const then of waiting) {
          then();
        }
      },
      Math.max(this.#movingUntil - Date.now(), 16),
    );
  }
}

/** Outside a message list, as in the pins list or search results, everything is at rest. */
const ALWAYS_STILL = new Stillness();

export const StillnessContext = createContext<Stillness>(ALWAYS_STILL);

/** The rest of the message list this is drawn in. */
export function useStillness(): Stillness {
  return useContext(StillnessContext);
}
