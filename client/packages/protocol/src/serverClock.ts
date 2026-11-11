/**
 * The largest offset taken from a `ClockOffsetStore`, thirty days either way. A device's clock is
 * wrong by minutes, hours, or a mistyped day or month; anything further is taken for a damaged
 * store and the clock starts from zero.
 */
export const CLOCK_OFFSET_MAX_MS = 30 * 24 * 60 * 60 * 1000;

/**
 * Where a deployment's clock offset is kept between runs, one store per deployment. Like the
 * device credential it outlives sign-outs: it describes the device and the deployment, not who
 * is signed in.
 */
export interface ClockOffsetStore {
  /** The offset last saved, in milliseconds, or `null` when none is kept or it cannot be read. */
  load(): number | null;
  save(offsetMs: number): void;
}

export class MemoryClockOffsetStore implements ClockOffsetStore {
  #offsetMs: number | null = null;

  load(): number | null {
    return this.#offsetMs;
  }

  save(offsetMs: number): void {
    this.#offsetMs = offsetMs;
  }
}

/** Keeps the offset as a JSON number under one key of a `Storage` (localStorage). */
export class WebStorageClockOffsetStore implements ClockOffsetStore {
  readonly #storage: Storage;
  readonly #key: string;

  constructor(storage: Storage, key = "aspen.clock") {
    this.#storage = storage;
    this.#key = key;
  }

  load(): number | null {
    try {
      const raw = this.#storage.getItem(this.#key);
      const parsed: unknown = raw === null ? null : JSON.parse(raw);
      return typeof parsed === "number" ? parsed : null;
    } catch {
      return null;
    }
  }

  save(offsetMs: number): void {
    try {
      this.#storage.setItem(this.#key, JSON.stringify(offsetMs));
    } catch {
      // A full or refused store leaves the offset to this run: the next starts from zero.
    }
  }
}

/** A stored offset worth starting from: a finite number within `CLOCK_OFFSET_MAX_MS` of zero. */
function trusted(stored: number | null): number | null {
  return stored !== null && Number.isFinite(stored) && Math.abs(stored) <= CLOCK_OFFSET_MAX_MS
    ? stored
    : null;
}

/**
 * One deployment's clock as this device reckons it: the device's own, moved by how far the
 * deployment's stood from it when its event stream last said (`serverTime` on `ready`). Every
 * time the deployment stamped (a ring's `until`, a token's expiry, a poll's `closesAt`) is
 * judged against this clock, never the device's own, so a device whose clock is wrong reaches
 * the answer one whose clock is right does.
 *
 * Until this run's first `ready` the offset is the one the last run saved (`restored`), so the
 * sign-in screen and a restored session's first requests are corrected too, or zero where none
 * is kept: the two clocks are then taken to agree, as they are for a deployment whose `ready`
 * carries no `serverTime`. Each `ready` replaces the offset and saves it. It is short of the
 * truth by the time the frame spent on its way, which makes this clock late by that much and no
 * more.
 */
export class ServerClock {
  readonly #deviceNow: () => number;
  readonly #store: ClockOffsetStore | null;
  #offsetMs: number;
  #restored: boolean;
  #heard = false;

  /**
   * `deviceNow` reads the device's clock, as milliseconds since the epoch; tests replace it.
   * `store` is where this deployment's offset is kept between runs; without one the clock
   * starts from zero every run.
   */
  constructor(deviceNow: () => number = () => Date.now(), store: ClockOffsetStore | null = null) {
    this.#deviceNow = deviceNow;
    this.#store = store;
    const kept = trusted(store?.load() ?? null);
    this.#offsetMs = kept ?? 0;
    this.#restored = kept !== null;
  }

  /** How far the deployment's clock is ahead of the device's, in milliseconds; negative behind. */
  get offsetMs(): number {
    return this.#offsetMs;
  }

  /**
   * Whether a `ready` has been taken this run, so the offset is the deployment's answer (its
   * silence included) and not the assumption made before one or an earlier run's (`restored`).
   * What cannot be undone, such as dropping a mute as run out, waits for this.
   */
  get heard(): boolean {
    return this.#heard;
  }

  /**
   * Whether the offset is the one an earlier run saved, which no `ready` of this run has
   * replaced yet. It is the best reading there is for what can be judged again (a token's
   * expiry, a countdown), and never stands in for `heard`.
   */
  get restored(): boolean {
    return this.#restored;
  }

  /**
   * Takes the deployment's time from a `ready` frame received at `receivedAt` by the device's
   * clock, and saves the offset for the next run. `null`, or text that is not a time, is a
   * deployment that does not say: the offset returns to zero.
   */
  note(serverTime: string | null, receivedAt: number = this.#deviceNow()): void {
    const said = serverTime === null ? Number.NaN : Date.parse(serverTime);
    this.#offsetMs = Number.isNaN(said) ? 0 : said - receivedAt;
    this.#heard = true;
    this.#restored = false;
    this.#store?.save(this.#offsetMs);
  }

  /** The deployment's time at `deviceTime`, a reading of the device's clock. */
  at(deviceTime: number): number {
    return deviceTime + this.#offsetMs;
  }

  /** The deployment's time now, as milliseconds since the epoch. */
  now(): number {
    return this.at(this.#deviceNow());
  }
}
