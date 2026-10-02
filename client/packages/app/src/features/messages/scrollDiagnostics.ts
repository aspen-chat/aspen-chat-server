/**
 * A record of what the message list does with its offset, for finding where a view jumps on
 * a device, where nothing else can watch: the list notes each touch, page shown, and change
 * absorbed, and tells of every move it makes on purpose (`moved`); a watcher samples a row in
 * view as each frame is about to be painted and counts as a jump any frame in which the row
 * moved by other than what the list meant to move it.
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

/** What the watcher reads of the first row in view after a frame. */
interface Sample {
  id: string;
  top: number;
  offsetTop: number;
  height: number;
  scrollTop: number;
  scrollHeight: number;
}

function change(what: string, before: number, after: number): string {
  return `${what} ${String(Math.round(before))}->${String(Math.round(after))}`;
}

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
   * Watches rows in `viewport` frame by frame, as each is about to be painted, for one that
   * moved by other than what the list meant. Returns a way to stop.
   *
   * A sample read in an ordinary task would force a layout of whatever changed since the last
   * frame, before the list's resize observers have kept the view still through it in the next,
   * and count as a jump a frame no one saw. So it is taken in a resize observer of its own, in
   * a later pass of the observers' loop than the list's: each frame resizes a sentinel, whose
   * callback resizes a child of it, which the loop delivers only after every callback of the
   * pass the list's observers run in, and still before the frame is painted.
   */
  watch(viewport: HTMLElement): () => void {
    let last: Sample | null = null;
    const outer = document.createElement("div");
    const inner = document.createElement("div");
    outer.setAttribute("aria-hidden", "true");
    outer.style.cssText =
      "position:fixed;top:0;left:0;height:1px;width:1px;visibility:hidden;pointer-events:none";
    inner.style.cssText = "height:1px;width:1px";
    outer.append(inner);
    document.body.append(outer);
    let tick = false;
    const sample = () => {
      const view = viewport.getBoundingClientRect();
      let seen: Sample | null = null;
      for (const row of viewport.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const rect = row.getBoundingClientRect();
        if (rect.bottom > view.top && rect.top < view.bottom) {
          seen = {
            id: row.dataset.messageId ?? "",
            top: rect.top,
            offsetTop: row.offsetTop,
            height: rect.height,
            scrollTop: viewport.scrollTop,
            scrollHeight: viewport.scrollHeight,
          };
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
            `JUMP row ${seen.id.slice(-4)} moved ${String(Math.round(actual))}, meant ${String(Math.round(expected))}: ${change("offsetTop", last.offsetTop, seen.offsetTop)} ${change("height", last.height, seen.height)} ${change("scrollTop", last.scrollTop, seen.scrollTop)} ${change("scrollHeight", last.scrollHeight, seen.scrollHeight)}`,
          );
          this.#firstJump ??= { before: this.#entries.slice(-40), after: [] };
        }
      }
      last = seen;
    };
    const first = new ResizeObserver(() => {
      inner.style.width = tick ? "2px" : "1px";
    });
    const second = new ResizeObserver(sample);
    first.observe(outer);
    second.observe(inner);
    let frame = 0;
    const everyFrame = () => {
      tick = !tick;
      outer.style.width = tick ? "2px" : "1px";
      frame = requestAnimationFrame(everyFrame);
    };
    frame = requestAnimationFrame(everyFrame);
    return () => {
      cancelAnimationFrame(frame);
      first.disconnect();
      second.disconnect();
      outer.remove();
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
