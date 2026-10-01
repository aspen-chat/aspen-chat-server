/**
 * A record of what the message list does with its scroll position, for finding where a view
 * jumps on a device, where nothing else can watch: the list notes each touch, scroll, page
 * shown, and correction, and a scroll event that moves the view by more than a screen at once,
 * which no finger or fling does between two events, is noted as a jump.
 *
 * Built only with `VITE_SCROLL_DEBUG=1`, which `MessageList` shows the record under
 * (`ScrollDiagnosticsPanel`); otherwise `SCROLL_DEBUG` is false and every use of it is left
 * out of the bundle.
 */
export const SCROLL_DEBUG = import.meta.env.VITE_SCROLL_DEBUG === "1";

/** How many of the latest entries are kept. */
const KEPT = 400;

export class ScrollDiagnostics {
  readonly #entries: string[] = [];
  readonly #started = performance.now();
  readonly #listeners = new Set<() => void>();
  #lastScrollTop: number | null = null;
  #jumps = 0;

  /** Notes something the list did or saw. */
  note(what: string): void {
    const at = (performance.now() - this.#started) / 1000;
    this.#entries.push(`${at.toFixed(3)} ${what}`);
    if (this.#entries.length > KEPT) {
      this.#entries.shift();
    }
    if (this.#firstJump !== null && this.#firstJump.after.length < 10) {
      this.#firstJump.after.push(`${at.toFixed(3)} ${what}`);
    }
    for (const listener of this.#listeners) {
      listener();
    }
  }

  /**
   * Notes a scroll event. One that moves the view by more than the screen's height since the
   * last known position, however that position was reached, is a jump.
   */
  noteScroll(scrollTop: number, clientHeight: number, ours: boolean): void {
    const last = this.#lastScrollTop;
    const delta = last === null ? 0 : scrollTop - last;
    this.#lastScrollTop = scrollTop;
    const jumped = Math.abs(delta) > clientHeight;
    if (jumped) {
      this.#jumps += 1;
    }
    this.note(
      `${jumped ? "JUMP " : ""}scroll ${String(Math.round(scrollTop))} by ${String(Math.round(delta))}${ours ? " (own)" : ""}`,
    );
    if (jumped && this.#firstJump === null) {
      // What led to it, kept as it stood, and what follows added over the next entries.
      this.#firstJump = { before: this.#entries.slice(-40), after: [] };
    }
  }

  /** The entries around the first jump: forty before it, and ten after. */
  #firstJump: { before: string[]; after: string[] } | null = null;

  get firstJump(): string | null {
    if (this.#firstJump === null) {
      return null;
    }
    return [...this.#firstJump.before, ...this.#firstJump.after].join("\n");
  }

  /** The position the list set or saw set, so the next scroll event is measured from it. */
  noteMoved(scrollTop: number, why: string): void {
    const last = this.#lastScrollTop;
    this.#lastScrollTop = scrollTop;
    this.note(
      `${why} ${last === null ? "" : String(Math.round(last)) + "->"}${String(Math.round(scrollTop))}`,
    );
  }

  get jumps(): number {
    return this.#jumps;
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
