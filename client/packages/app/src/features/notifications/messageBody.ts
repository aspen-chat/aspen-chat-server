/**
 * How long a system notification waits for the deployment's wording of its message before it
 * shows the app's own: a notification that comes a moment late is still news, and one that
 * waits on a slow deployment is not.
 */
export const SERVER_TEXT_WAIT_MS = 1500;

/**
 * How many readings of the deployment's wording may start at once, and how long until another
 * may: five, and one more every three seconds, so twenty a minute once a burst is spent. Each
 * is a `GET /messages/{message}`, which counts toward everything the user does on that
 * deployment (1200 requests a minute, `server/app/src/rate_limits.toml`), and a busy channel
 * set to tell of every message can post faster than that. Messages past the budget are worded
 * by the app.
 */
export const TEXT_READS_AT_ONCE = 5;
export const TEXT_READ_EVERY_MS = 3000;

/** One deployment's budget of readings (`TEXT_READS_AT_ONCE`, `TEXT_READ_EVERY_MS`). */
export class ReadBudget {
  #left = TEXT_READS_AT_ONCE;
  /** When the budget was last counted up, on the clock `take` is given. */
  #countedAt: number | undefined;

  /** Takes a reading from the budget at `now`, in milliseconds; `false` when none is left. */
  take(now: number): boolean {
    if (this.#countedAt === undefined) {
      this.#countedAt = now;
    }
    const earned = Math.floor((now - this.#countedAt) / TEXT_READ_EVERY_MS);
    if (earned > 0) {
      this.#left = Math.min(TEXT_READS_AT_ONCE, this.#left + earned);
      this.#countedAt += earned * TEXT_READ_EVERY_MS;
    }
    if (this.#left === 0) {
      return false;
    }
    // A full budget earns nothing, so its next reading is counted from when it was spent.
    if (this.#left === TEXT_READS_AT_ONCE) {
      this.#countedAt = now;
    }
    this.#left -= 1;
    return true;
  }
}

/**
 * What a notification says of a message: what `read` answers (the deployment's wording,
 * `AspenSync.notificationText`), or `fallback`, the app's own, when the read fails, answers
 * nothing (a deployment of a version that writes none), or takes longer than `waitMs`.
 */
export async function messageBody(
  read: () => Promise<string | undefined>,
  fallback: string,
  waitMs: number = SERVER_TEXT_WAIT_MS,
): Promise<string> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<undefined>((resolve) => {
    timer = setTimeout(() => {
      resolve(undefined);
    }, waitMs);
  });
  try {
    const text = await Promise.race([read().catch(() => undefined), late]);
    return text === undefined || text === "" ? fallback : text;
  } finally {
    clearTimeout(timer);
  }
}
