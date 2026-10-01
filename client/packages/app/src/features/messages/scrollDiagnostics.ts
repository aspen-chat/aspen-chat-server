/**
 * A record of what the message list does with its offset, for finding where a view jumps on
 * a device, where nothing else can watch: the list notes each touch, page shown, and change
 * absorbed, and tells of every move it makes on purpose (`moved`); a watcher samples a row in
 * view after each frame is painted and counts as a jump any frame in which the row moved by
 * other than what the list meant to move it.
 *
 * Built only with `VITE_SCROLL_DEBUG=1`, which `MessageList` shows the record under
 * (`ScrollDiagnosticsPanel`); otherwise `SCROLL_DEBUG` is false and every use of it is left
 * out of the bundle.
 */
export const SCROLL_DEBUG = import.meta.env.VITE_SCROLL_DEBUG === "1";

/** How many of the latest entries are kept. */
const KEPT = 400;
/** How far a row may be off what the list meant, in pixels, before a frame counts as a jump. */
const TOLERANCE = 2;

export class ScrollDiagnostics {
  readonly #entries: string[] = [];
  readonly #started = performance.now();
  readonly #listeners = new Set<() => void>();
  #jumps = 0;
  /** How far the list has meant to move the view since the last sample. */
  #meant = 0;
  /** The entries around the first jump: forty before it, and ten after. */
  #firstJump: { before: string[]; after: string[] } | null = null;

  /** Notes something the list did or saw. */
  note(what: string): void {
    const at = (performance.now() - this.#started) / 1000;
    const entry = `${at.toFixed(3)} ${what}`;
    this.#entries.push(entry);
    if (this.#entries.length > KEPT) {
      this.#entries.shift();
    }
    if (this.#firstJump !== null && this.#firstJump.after.length < 10) {
      this.#firstJump.after.push(entry);
    }
    for (const listener of this.#listeners) {
      listener();
    }
  }

  /** The list moved the view by `by` pixels on purpose: a finger, a fling, a key, a wheel. */
  moved(by: number): void {
    this.#meant += by;
  }

  /**
   * Watches rows in `viewport` frame by frame, after each is painted, for one that moved by
   * other than what the list meant. Returns a way to stop.
   */
  watch(viewport: HTMLElement): () => void {
    let stopped = false;
    let last: { id: string; top: number } | null = null;
    const sample = () => {
      if (stopped) {
        return;
      }
      const view = viewport.getBoundingClientRect();
      let seen: { id: string; top: number } | null = null;
      for (const row of viewport.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const rect = row.getBoundingClientRect();
        if (rect.bottom > view.top && rect.top < view.bottom) {
          seen = { id: row.dataset.messageId ?? "", top: rect.top };
          break;
        }
      }
      const meant = this.#meant;
      this.#meant = 0;
      if (seen !== null && last !== null && seen.id === last.id) {
        const actual = seen.top - last.top;
        const expected = -meant;
        if (Math.abs(actual - expected) > TOLERANCE) {
          this.#jumps += 1;
          this.note(
            `JUMP row ${seen.id.slice(-4)} moved ${String(Math.round(actual))}, meant ${String(Math.round(expected))}`,
          );
          this.#firstJump ??= { before: this.#entries.slice(-40), after: [] };
        }
      }
      last = seen;
      requestAnimationFrame(() => {
        setTimeout(sample, 0);
      });
    };
    requestAnimationFrame(() => {
      setTimeout(sample, 0);
    });
    return () => {
      stopped = true;
    };
  }

  get jumps(): number {
    return this.#jumps;
  }

  get firstJump(): string | null {
    if (this.#firstJump === null) {
      return null;
    }
    return [...this.#firstJump.before, ...this.#firstJump.after].join("\n");
  }

  get text(): string {
    return this.#entries.join("\n");
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }
}
