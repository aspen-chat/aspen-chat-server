import {
  createContext,
  useContext,
  useSyncExternalStore,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";

/**
 * Moving through a message list by keyboard. The list is one stop in the tab order: one row (a
 * message, a notice, or a run of blocked messages, each `[data-message-row]`) takes focus from
 * Tab, the last one focused while it is still drawn and otherwise the newest; from it the arrow
 * keys go to the row before or after, and Home and End to the first and last the list holds.
 * Tab goes on into the row's own controls, which focus shows. Each list keeps its own
 * `MessageRows`, and a row asks whether it is the stop with `useRowStop`, so moving re-renders
 * only the two rows the stop leaves and reaches.
 */
export class MessageRows {
  #chosen: string | null = null;
  #stop: string | null = null;
  readonly #listeners = new Set<() => void>();

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  get stop(): string | null {
    return this.#stop;
  }

  #setStop(id: string | null): void {
    if (id === this.#stop) {
      return;
    }
    this.#stop = id;
    for (const listener of this.#listeners) {
      listener();
    }
  }

  /** A row took focus: it is the stop from now on. */
  readonly focused = (id: string): void => {
    this.#chosen = id;
    this.#setStop(id);
  };

  /**
   * After the list is drawn: the stop is the row last focused, if it is still drawn in `box`,
   * or else the newest row.
   */
  settle(box: HTMLElement): void {
    const drawn = rowsIn(box);
    const chosen = drawn.find((row) => row.dataset.messageRow === this.#chosen);
    this.#setStop((chosen ?? drawn.at(-1))?.dataset.messageRow ?? null);
  }
}

function rowsIn(box: HTMLElement): HTMLElement[] {
  return Array.from(box.querySelectorAll<HTMLElement>("[data-message-row]"));
}

/** The keys that move between rows, from the row at `at` of `count`. */
function target(key: string, at: number, count: number): number | null {
  switch (key) {
    case "ArrowUp":
      return Math.max(0, at - 1);
    case "ArrowDown":
      return Math.min(count - 1, at + 1);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/**
 * Moves focus between the rows of `box` for a key pressed on a row itself (not on a control
 * inside it), answering whether it did, and which way, for the list to page history that way.
 */
export function moveBetweenRows(
  box: HTMLElement,
  event: ReactKeyboardEvent,
): "older" | "newer" | null {
  const row = event.target;
  if (
    !(row instanceof HTMLElement) ||
    row.dataset.messageRow === undefined ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey ||
    event.shiftKey
  ) {
    return null;
  }
  const drawn = rowsIn(box);
  const at = drawn.indexOf(row);
  const next = at === -1 ? null : target(event.key, at, drawn.length);
  if (next === null) {
    return null;
  }
  event.preventDefault();
  drawn[next]?.focus();
  return next < at ? "older" : "newer";
}

export const MessageRowsContext = createContext<MessageRows | null>(null);

const noRows = () => () => undefined;

/** Whether the row `id` is its list's stop in the tab order. */
export function useRowStop(id: string): boolean {
  const rows = useContext(MessageRowsContext);
  return useSyncExternalStore(rows?.subscribe ?? noRows, () => rows?.stop === id);
}

/** What a row needs to take part: its attribute, its place in the tab order, and noting focus. */
export function useRowProps(id: string): {
  "data-message-row": string;
  tabIndex: number;
  onFocus: () => void;
} {
  const rows = useContext(MessageRowsContext);
  const stop = useRowStop(id);
  return {
    "data-message-row": id,
    tabIndex: stop ? 0 : -1,
    onFocus: () => {
      rows?.focused(id);
    },
  };
}
