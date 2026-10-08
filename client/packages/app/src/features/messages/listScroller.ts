import type { MessageWindow } from "@aspen/protocol";
import {
  useEffect,
  useLayoutEffect,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  type TouchEvent as ReactTouchEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { rowsInView } from "@/features/messages/rowsInView";
import { SCROLL_DEBUG, ScrollDiagnostics } from "@/features/messages/scrollDiagnostics";
import {
  COASTING_STOPS,
  MAX_VELOCITY,
  SPRING_MS,
  VelocityTracker,
  bounds,
  clamp,
  coast,
  shown as shownOffset,
  spring,
  thumb,
} from "@/features/messages/scrollPhysics";

/**
 * Whether the list scrolls its box itself rather than the browser: on iOS and iPadOS, the only
 * platforms with `-webkit-touch-callout`, where a pan overrides any position the page sets
 * (see `MessageList`). A platform, not a behaviour, since the override cannot be felt without a
 * gesture; a test stands in for iOS by claiming the property.
 */
export const OWNS_SCROLLING =
  typeof CSS !== "undefined" && CSS.supports("-webkit-touch-callout", "none");
/**
 * How far a finger moves before its touch is a drag rather than a tap: from then on nothing
 * under it may take a press, as nothing would under a pan the browser made.
 */
const DRAG_SLOP_PX = 8;
/** How far an arrow key moves the list, and how much of a screen a page key leaves in view. */
const ARROW_PX = 40;
const PAGE_OVERLAP_PX = 40;
/** How far one wheel line is, for a wheel that counts in lines. */
const WHEEL_LINE_PX = 16;
/** How long the indicator stays after the list last moved. */
const INDICATOR_MS = 700;
/** The least height the indicator's thumb is drawn at. */
const THUMB_MIN_PX = 24;
/** How close to the bottom, in pixels, a reader's move leaves the list pinned there. */
const PIN_SLACK_PX = 8;

export type Heading = "older" | "newer";

/** What the list tells the component it serves, which changes with every render. */
export interface ListScrollerEvents {
  /** The view moved, by the reader or by the list: what is seen and what is near changed. */
  moved(): void;
  /** The reader moved the list themselves, which drops a linked message from the URL. */
  steered(): void;
  /** Whether the view is at least a screen above the newest the list holds. */
  farBack(far: boolean): void;
}

/** The elements the list is drawn in. */
export interface ListElements {
  /** The box the list is seen through, whose scroll position the list sets. */
  viewport: RefObject<HTMLDivElement | null>;
  /** Everything the list holds; drawn past an end, while a finger pulls it there, by a transform. */
  content: RefObject<HTMLDivElement | null>;
  /** The scroll indicator, drawn where the list scrolls itself, and its thumb. */
  indicator: RefObject<HTMLDivElement | null>;
  thumb: RefObject<HTMLDivElement | null>;
}

/** The row kept still, and where it stood when noted. */
interface Still {
  id: string;
  /** Where the row's top stood in the view, which the box's own clamping cannot move. */
  screenTop: number;
  /** Where the row's top stood in the content, which only a change of the rows can move. */
  offsetTop: number;
  height: number;
  /** Whether the row is a linked message, held by its middle rather than its top. */
  linked: boolean;
}

/** The row with a message's id in the list, or `null` while it is not rendered. */
function rowOf(box: HTMLElement, id: string): HTMLElement | null {
  return box.querySelector<HTMLElement>(`[data-message-id="${id}"]`);
}

/**
 * The message list's scroll position and everything that sets it: holding what is in view
 * still through every change (see `MessageList` for why and how), staying pinned to the
 * bottom, going to a linked message, and, where the list scrolls itself (`OWNS_SCROLLING`),
 * the fingers, wheel, keys, coasting, spring, and indicator that move it. It reads and sets
 * the DOM directly, outside React's renders, since every change it makes must land in the
 * same layout as the change it answers; the component tells it what each render knows
 * (`update`) and calls it from its effects (`useListScroller`, `useListPosition`).
 */
export class ListScroller {
  /** The box the list is seen through, whose scroll position the list sets. */
  readonly viewport: RefObject<HTMLDivElement | null>;
  /** Everything the list holds; drawn past an end, while a finger pulls it there, by a transform. */
  readonly #content: RefObject<HTMLDivElement | null>;
  readonly #indicator: RefObject<HTMLDivElement | null>;
  readonly #thumb: RefObject<HTMLDivElement | null>;
  /** What the list does with its offset, in a build made to find a jump (see `MessageList`). */
  readonly diagnostics = SCROLL_DEBUG ? new ScrollDiagnostics() : null;

  /** Whether the window holds the newest messages, as of the last render. */
  #atLatest = true;
  /** The linked message, as of the last render. */
  #highlightId: string | undefined;
  #events: ListScrollerEvents = {
    moved: () => undefined,
    steered: () => undefined,
    farBack: () => undefined,
  };
  /** Whether the view follows the newest message as the list changes. */
  stickToBottom = true;
  /**
   * Which way the reader last moved the list, by their own input; a fling keeps the way of
   * the stroke that started it.
   */
  heading: Heading | null = null;
  /** The scroll positions the content allows in the viewport, as last measured. */
  range = { min: 0, max: 0 };

  /** How far past an end a finger has pulled the list, which its scroll position cannot hold. */
  #over = 0;
  /**
   * Where the list last scrolled its box to. The scroll event that lands there is its own, and
   * any other is a script's: focus, find-in-page, assistive technology.
   */
  #ownScrollTop: number | null = null;
  /** The box's position at its last scroll event, by which the browser's own scrolls are measured. */
  #lastScrollTop = 0;
  /**
   * The position the list last scrolled its box to, exactly. A box holds whole pixels, and
   * WebKit truncates a position set between them (999.7 reads back as 999), so a list that
   * built each move on what its box reads back would gain up to a pixel a move going up: a
   * finger's drag, many small moves, ran ahead of the finger by a few percent. Moves build on
   * this while the box is still within a pixel of it, and on the box's own position once
   * anything else has scrolled it.
   */
  #exactTop: number | null = null;
  /** The highlighted message already scrolled to, so it is done once per link. */
  #highlightShown: string | null = null;
  /**
   * The row kept still, and where it stands in the content, which does not change with
   * scrolling, noted after every move and change: a linked message while it is shown, so a
   * jump lands on it whatever loads around it, and otherwise the topmost row in view. Every
   * change of the rows' sizes is measured by how far it has moved (`settle`, and `#catchUp`
   * before every move notes it afresh).
   */
  #still: Still | null = null;
  /** A finger on the list, where it last was, how far it has gone, and whether it drags yet. */
  #drag: { y: number; travelled: number; dragging: boolean; tracker: VelocityTracker } | null =
    null;
  /**
   * The list coasting after a fling, or springing back from past an end. Its times are the
   * animation frames' clock, taken from the first frame that runs it.
   */
  #motion:
    | { kind: "coast"; velocity: number; at: number | null }
    | { kind: "spring"; from: number; to: number; startedAt: number | null }
    | null = null;
  #frame: number | null = null;
  #indicatorTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(elements: ListElements) {
    this.viewport = elements.viewport;
    this.#content = elements.content;
    this.#indicator = elements.indicator;
    this.#thumb = elements.thumb;
  }

  /** What the latest render knows, for the handlers and observers that outlive it. */
  update(render: {
    atLatest: boolean;
    highlightId: string | undefined;
    events: ListScrollerEvents;
  }) {
    this.#atLatest = render.atLatest;
    this.#highlightId = render.highlightId;
    this.#events = render.events;
  }

  /** Whether the view follows the newest message: pinned there, with the newest loaded. */
  get #pinned(): boolean {
    return this.stickToBottom && this.#atLatest;
  }

  /** The list's position: its box's, and what a finger has pulled it past an end by. */
  current(): number {
    return this.#boxTop() + this.#over;
  }

  /** Reads the content's and the viewport's heights afresh. */
  measureRange() {
    const box = this.viewport.current;
    if (box === null) {
      return;
    }
    this.range = bounds(box.scrollHeight, box.clientHeight);
    this.#noteDistance();
  }

  /**
   * Holds the view through a change of the rows' sizes that has just been laid out, before it
   * is painted: by the noted row, or at the bottom while pinned there (unless a finger holds
   * the list). Rows that change their own height call it through `KeepStillContext` in the
   * same task; the rows' observer calls it for every other change.
   */
  readonly settle = (why = "row told") => {
    this.measureRange();
    // What changed above the view is absorbed, not a move; pinned to the bottom, what changed
    // below is then followed, which is one.
    this.#holdStill(why);
    if (this.#pinned && this.#drag === null) {
      this.#moveTo(this.range.max, false);
    }
    this.#noteStill();
  };

  /**
   * Places the view after the window or the link changed: a linked message goes to the middle
   * of the view once it is in the window, whatever view was being kept, and the list lets go
   * of the bottom so that nothing arriving or growing later pulls the view away from it. A
   * linked message not yet in the window keeps the view for it. Otherwise the view holds
   * through the change, at the bottom or by the noted row, a linked message shown included:
   * the pages read around it arrive just after it is centred. Either way, what is seen and
   * what is near have changed, which `moved` is told.
   */
  position() {
    const box = this.viewport.current;
    if (box === null) {
      return;
    }
    this.measureRange();
    const link = this.#highlightId;
    if (link === undefined) {
      this.#highlightShown = null;
    } else if (this.#highlightShown !== link) {
      const target = rowOf(box, link);
      if (target !== null) {
        this.#highlightShown = link;
        this.stickToBottom = false;
        this.#stopMotion();
        this.#over = 0;
        target.scrollIntoView({ block: "center" });
        this.#ownScrollTop = box.scrollTop;
        this.#exactTop = null;
        this.#afterMove(false);
        return;
      }
      this.#events.moved();
      return;
    }
    if (this.#pinned) {
      this.#stopMotion();
      this.#moveTo(this.range.max, false);
    } else {
      this.#holdStill("page");
      this.#events.moved();
    }
    this.#noteStill();
  }

  /** Back to the end of what the list holds, pinned there, while the newest are read. */
  jumpToEnd() {
    this.stickToBottom = true;
    this.heading = null;
    this.#stopMotion();
    this.measureRange();
    this.#moveTo(this.range.max, false);
  }

  /** A different channel starts pinned to the bottom, with no way known that its reader is going. */
  reset() {
    this.stickToBottom = true;
    this.heading = null;
    this.#stopMotion();
    this.#drag = null;
    this.#letRowsTakePointers();
  }

  dispose() {
    this.#stopMotion();
    clearTimeout(this.#indicatorTimer);
  }

  /** Notes, in a build made to find a jump, a new window about to be shown. */
  readonly noteCommit = (from: MessageWindow | undefined, to: MessageWindow | undefined) => {
    const still = this.#still;
    this.diagnostics?.note(
      `commit ${String(from?.ids.length ?? 0)}->${String(to?.ids.length ?? 0)} first ${from?.ids[0]?.slice(-4) ?? "-"}->${to?.ids[0]?.slice(-4) ?? "-"} still=${still?.id.slice(-4) ?? "-"}@${String(Math.round(still?.screenTop ?? 0))}`,
    );
  };

  /**
   * Rows change size without the window changing: pictures and link cards load, reactions come
   * and go, a deleted message's space closes. What stands at the beginning changes the same
   * way. Each change is settled before the frame is painted, after `measured` has run.
   */
  observeRows(
    rows: HTMLElement | null,
    start: HTMLElement | null,
    measured: () => void,
  ): () => void {
    if (rows === null || typeof ResizeObserver === "undefined") {
      return () => undefined;
    }
    let height = rows.offsetHeight;
    const observer = new ResizeObserver(() => {
      if (this.diagnostics !== null && rows.offsetHeight !== height) {
        this.diagnostics.note(`rows ${String(height)}->${String(rows.offsetHeight)}`);
      }
      height = rows.offsetHeight;
      measured();
      this.settle("rows");
    });
    observer.observe(rows);
    if (start !== null) {
      observer.observe(start);
    }
    return () => {
      observer.disconnect();
    };
  }

  /**
   * The viewport changes size: something takes the screen's space, as the keyboard does when
   * the message box is chosen on a phone, or gives it back. Its bottom edge stays where it was,
   * so what was just above the box, likely what is being answered, stays in view: pinned to
   * the newest message, or moved by what the viewport lost or gained.
   */
  observeViewport(): () => void {
    const box = this.viewport.current;
    if (box === null || typeof ResizeObserver === "undefined") {
      return () => undefined;
    }
    let height = box.clientHeight;
    const observer = new ResizeObserver(() => {
      const lost = height - box.clientHeight;
      height = box.clientHeight;
      this.measureRange();
      if (lost === 0) {
        return;
      }
      this.#moveTo(this.#pinned ? this.range.max : clamp(this.current() + lost, this.range), false);
    });
    observer.observe(box);
    return () => {
      observer.disconnect();
    };
  }

  /**
   * The box scrolled: by the list, which is nothing new; by the browser for the reader, where
   * it scrolls the box; or by a script, as focus, find-in-page, and assistive technology do.
   * The list follows the last two.
   */
  readonly onScroll = () => {
    const box = this.viewport.current;
    if (box === null) {
      return;
    }
    const by = box.scrollTop - this.#lastScrollTop;
    this.#lastScrollTop = box.scrollTop;
    const own = this.#ownScrollTop;
    if (own !== null && Math.abs(box.scrollTop - own) < 1) {
      this.#ownScrollTop = null;
      return;
    }
    this.diagnostics?.moved(by);
    this.diagnostics?.note(
      `${OWNS_SCROLLING ? "scrolled by a script" : "scrolled"} to ${String(Math.round(box.scrollTop))}`,
    );
    if (!OWNS_SCROLLING && by !== 0) {
      this.heading = by < 0 ? "older" : "newer";
    }
    this.#stopMotion();
    this.#exactTop = null;
    this.#over = 0;
    // Where the browser left the box decides the pin before anything moves it again: one left
    // at the bottom is pinned there, the reader's doing or the browser's own when the bottom
    // moved up (what lies beneath the list shrank), and whatever the rows did meanwhile is then
    // followed rather than held, as the next settle does.
    this.measureRange();
    if (this.#atLatest && this.range.max - box.scrollTop < PIN_SLACK_PX) {
      this.stickToBottom = true;
      this.#noteStill();
      this.#showIndicator();
      this.#noteDistance();
      this.#events.moved();
      return;
    }
    this.#afterMove(true);
  };

  /**
   * Whether `event` happened in the list itself. React passes events up through what a row
   * opened in a layer of its own (a sheet, a picker, a dialog) as though it were inside the
   * row, though it is drawn over the page, so a finger, wheel, or key there would otherwise
   * move the list behind it.
   */
  #inList(event: { target: EventTarget }): boolean {
    const box = this.viewport.current;
    return box !== null && event.target instanceof Node && box.contains(event.target);
  }

  readonly onTouchStart = (event: ReactTouchEvent<HTMLDivElement>) => {
    const touch = event.touches[0];
    if (touch === undefined || !this.#inList(event)) {
      return;
    }
    this.#stopMotion();
    const tracker = new VelocityTracker();
    tracker.add(touch.clientY, Date.now());
    this.#drag = { y: touch.clientY, travelled: 0, dragging: false, tracker };
    this.diagnostics?.note("touch start");
  };

  readonly onTouchMove = (event: ReactTouchEvent<HTMLDivElement>) => {
    const touch = event.touches[0];
    const drag = this.#drag;
    if (touch === undefined || drag === null || !this.#inList(event)) {
      return;
    }
    const dy = drag.y - touch.clientY;
    drag.y = touch.clientY;
    drag.travelled += Math.abs(dy);
    drag.tracker.add(touch.clientY, Date.now());
    if (!drag.dragging && drag.travelled >= DRAG_SLOP_PX) {
      // A drag, not a tap: whatever the finger lifts over, a picture or a button, must not
      // take the lift as a press. The browser's own pan would have cancelled the pointer; the
      // rows take no pointer until the finger has lifted, which comes after the pointer has.
      drag.dragging = true;
      if (this.#content.current !== null) {
        this.#content.current.style.pointerEvents = "none";
      }
    }
    if (dy !== 0) {
      // A finger moving down the screen draws older messages into view.
      this.heading = dy < 0 ? "older" : "newer";
      this.#moveTo(this.current() + dy, true);
    }
  };

  readonly onTouchEnd = (event: ReactTouchEvent<HTMLDivElement>) => {
    const drag = this.#drag;
    if (drag !== null && event.touches.length === 0) {
      // The finger's speed down the screen is the content's speed up it.
      this.#lift("touch end", -drag.tracker.velocity(Date.now()));
    }
  };

  readonly onTouchCancel = (event: ReactTouchEvent<HTMLDivElement>) => {
    if (this.#inList(event)) {
      this.#lift("touch cancel", 0);
    }
  };

  readonly onWheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    const box = this.viewport.current;
    if (box === null || event.deltaY === 0 || !this.#inList(event)) {
      return;
    }
    const by =
      event.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? event.deltaY * WHEEL_LINE_PX
        : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? event.deltaY * box.clientHeight
          : event.deltaY;
    const at = this.current();
    this.#steer(at + by, at);
  };

  /** Focus went to a row that way, by key (`messageRows.ts`); history pages ahead of it. */
  readonly focusMoved = (heading: Heading) => {
    this.heading = heading;
  };

  readonly onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const box = this.viewport.current;
    // A key a row took, to move between rows (`messageRows.ts`), is not for scrolling.
    if (
      box === null ||
      event.defaultPrevented ||
      event.target instanceof HTMLTextAreaElement ||
      !this.#inList(event)
    ) {
      return;
    }
    const page = box.clientHeight - PAGE_OVERLAP_PX;
    const at = this.current();
    const moves: Record<string, number | undefined> = {
      ArrowUp: at - ARROW_PX,
      ArrowDown: at + ARROW_PX,
      PageUp: at - page,
      PageDown: at + page,
      Home: this.range.min,
      End: this.range.max,
      " ": event.shiftKey ? at - page : at + page,
    };
    const next = moves[event.key];
    if (next === undefined) {
      return;
    }
    event.preventDefault();
    this.#steer(next, at);
  };

  /** The indicator's thumb dragged with a pointer: the view follows it along the track. */
  readonly onThumbPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    const track = this.#indicator.current;
    const bar = this.#thumb.current;
    if (track === null || bar === null) {
      return;
    }
    event.preventDefault();
    this.#stopMotion();
    const startY = event.clientY;
    const startOffset = clamp(this.current(), this.range);
    const travel = track.clientHeight - bar.clientHeight;
    const { min, max } = this.range;
    const onMove = (move: PointerEvent) => {
      if (travel > 0) {
        this.#steer(startOffset + ((move.clientY - startY) / travel) * (max - min), this.current());
      }
    };
    const onUp = () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
      document.removeEventListener("pointercancel", onUp);
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    document.addEventListener("pointercancel", onUp);
  };

  /** Where the box is scrolled to: exactly where the list put it, unless something since moved it. */
  #boxTop(): number {
    const box = this.viewport.current;
    if (box === null) {
      return 0;
    }
    const exact = this.#exactTop;
    return exact !== null && Math.abs(exact - box.scrollTop) < 1 ? exact : box.scrollTop;
  }

  /** Tells whether the view is at least a screen above the newest the list holds. */
  #noteDistance() {
    const box = this.viewport.current;
    if (box !== null) {
      this.#events.farBack(
        box.clientHeight > 0 && this.range.max - box.scrollTop >= box.clientHeight,
      );
    }
  }

  /** Notes the row to keep still: the linked message while it is shown, else the topmost in view. */
  #noteStill() {
    const box = this.viewport.current;
    if (box === null) {
      this.#still = null;
      return;
    }
    const linked = this.#highlightId;
    const shownLink = linked !== undefined && this.#highlightShown === linked;
    const row = shownLink ? rowOf(box, linked) : rowsInView(box).first;
    this.#still =
      row === null
        ? null
        : {
            id: row.dataset.messageId ?? "",
            screenTop: row.offsetTop - box.scrollTop,
            offsetTop: row.offsetTop,
            height: row.offsetHeight,
            linked: shownLink,
          };
  }

  /**
   * The noted row as it stands now, with how much of its growth counts as a move of what is
   * in view: half, for a linked message held by its middle, so it stays centred as what it
   * holds, a picture of its own, takes its size; none for a row held by its top.
   */
  #stillRow(): { box: HTMLDivElement; noted: Still; row: HTMLElement; grown: number } | null {
    const box = this.viewport.current;
    const noted = this.#still;
    const row = box === null || noted === null ? null : rowOf(box, noted.id);
    if (box === null || noted === null || row === null) {
      return null;
    }
    const grown = noted.linked ? (row.offsetHeight - noted.height) / 2 : 0;
    return { box, noted, row, grown };
  }

  /**
   * Puts the noted row back where it stood in the view, whatever moved it: content changing
   * above it, or the box clamping its own position as content shrank.
   */
  #holdStill(why: string) {
    const still = this.#stillRow();
    if (still !== null) {
      const { box, noted, row, grown } = still;
      this.#absorb(row.offsetTop - noted.screenTop + grown - box.scrollTop, why);
    }
  }

  /**
   * Absorbs whatever moved the noted row in the content since it was noted, before a move
   * notes a row afresh. A change of the rows' sizes is told to the rows' observer only after
   * the frame's animation callbacks have run, and a fling moves the list in those, as touches
   * may come before it: a move that noted the row where it now stood would leave the observer
   * nothing to find, and the change would show as a jump. Scrolling does not move a row in the
   * content, so what has is a change of the rows alone.
   */
  #catchUp() {
    const still = this.#stillRow();
    if (still === null) {
      return;
    }
    const { noted, row, grown } = still;
    const by = row.offsetTop - noted.offsetTop + grown;
    if (Math.abs(by) >= 0.5) {
      this.measureRange();
      this.#absorb(by, "unseen change");
    }
  }

  /** Keeps the view still through a change of `by` pixels in what is above it. */
  #absorb(by: number, why: string) {
    const box = this.viewport.current;
    if (box === null || Math.abs(by) < 0.5) {
      return;
    }
    this.diagnostics?.note(
      `${why} ${String(Math.round(by))} absorbed at ${String(Math.round(box.scrollTop))}`,
    );
    this.#scrollBoxTo(this.#boxTop() + by);
    this.#noteStill();
  }

  /**
   * Scrolls the box to `at`, which is the list's own doing: the position it then has, which
   * the box may have rounded or clamped, is what its scroll event will say.
   */
  #scrollBoxTo(at: number) {
    const box = this.viewport.current;
    if (box === null) {
      return;
    }
    this.#exactTop = at;
    if (box.scrollTop === at) {
      return;
    }
    box.scrollTop = at;
    this.#ownScrollTop = box.scrollTop;
    this.#lastScrollTop = box.scrollTop;
  }

  /** Shows the content at `next`, and the indicator with it. */
  #place(next: number) {
    const body = this.#content.current;
    const box = this.viewport.current;
    if (body === null || box === null) {
      return;
    }
    const within = clamp(next, this.range);
    this.#scrollBoxTo(within);
    this.#over = next - within;
    const pulled = shownOffset(next, this.range, box.clientHeight) - within;
    body.style.transform = pulled === 0 ? "" : `translate3d(0, ${String(-pulled)}px, 0)`;
    const bar = this.#thumb.current;
    const track = this.#indicator.current;
    if (bar !== null && track !== null) {
      const { height, top, shown } = thumb(within, this.range, {
        viewportHeight: box.clientHeight,
        contentHeight: box.scrollHeight,
        trackHeight: track.clientHeight,
        minHeight: THUMB_MIN_PX,
      });
      bar.style.height = `${String(height)}px`;
      bar.style.transform = `translate3d(0, ${String(top)}px, 0)`;
      track.style.opacity = shown ? "1" : "0";
    }
  }

  /** Shows the indicator for a moment: the list moved. */
  #showIndicator() {
    const track = this.#indicator.current;
    if (track === null) {
      return;
    }
    track.dataset.moving = "";
    clearTimeout(this.#indicatorTimer);
    this.#indicatorTimer = setTimeout(() => {
      delete track.dataset.moving;
    }, INDICATOR_MS);
  }

  /**
   * Moves the view to `next` on purpose: a finger, a fling, a key, the wheel, or the list
   * following the bottom. A move of the reader's drops a linked message from the URL.
   */
  #moveTo(next: number, byUser: boolean) {
    const box = this.viewport.current;
    if (box === null) {
      return;
    }
    const before = shownOffset(this.current(), this.range, box.clientHeight);
    this.#place(next);
    const after = shownOffset(this.current(), this.range, box.clientHeight);
    this.diagnostics?.moved(after - before);
    this.#afterMove(byUser);
    if (byUser) {
      this.#events.steered();
    }
  }

  /** A move of the reader's from `at` to `next` by wheel, key, or thumb, kept within the ends. */
  #steer(next: number, at: number) {
    this.#stopMotion();
    this.heading = next < at ? "older" : "newer";
    this.#moveTo(clamp(next, this.range), true);
  }

  /** What follows a move of the view: the noted row, the indicator, the pin to the bottom. */
  #afterMove(byUser: boolean) {
    const box = this.viewport.current;
    if (box === null) {
      return;
    }
    this.#catchUp();
    this.#noteStill();
    this.#showIndicator();
    this.#noteDistance();
    if (byUser) {
      // Measured afresh, as the move may have come with a change of the content or the box.
      this.measureRange();
      this.stickToBottom = this.#atLatest && this.range.max - box.scrollTop < PIN_SLACK_PX;
    }
    this.#events.moved();
  }

  #stopMotion() {
    this.#motion = null;
    if (this.#frame !== null) {
      cancelAnimationFrame(this.#frame);
      this.#frame = null;
    }
  }

  /** Runs the list's coasting or spring, a frame at a time, until it comes to rest. */
  #animate() {
    this.#frame = requestAnimationFrame((now) => {
      this.#frame = null;
      const moving = this.#motion;
      if (moving === null || this.viewport.current === null) {
        return;
      }
      if (moving.kind === "coast") {
        // By the time passed, however long a frame took: a list stalled by a long render coasts
        // on to where it would have been.
        const { velocity, travelled } = coast(moving.velocity, now - (moving.at ?? now));
        const { min, max } = this.range;
        let next = this.current() + travelled;
        let done = Math.abs(velocity) < COASTING_STOPS;
        if (next < min || next > max) {
          // Coasting ends at an end; there is nothing past it to see.
          next = clamp(next, this.range);
          done = true;
        }
        this.#motion = done ? null : { kind: "coast", velocity, at: now };
        this.#moveTo(next, true);
      } else {
        const startedAt = moving.startedAt ?? now;
        const elapsed = now - startedAt;
        this.#motion = elapsed >= SPRING_MS ? null : { ...moving, startedAt };
        this.#moveTo(spring(moving.from, moving.to, elapsed), true);
      }
      if (this.#motion !== null) {
        this.#animate();
      }
    });
  }

  /**
   * The finger has lifted, or its touch was cancelled: the rows take pointers again, and a
   * list pulled past an end springs back; otherwise it coasts with the finger's speed.
   */
  #lift(why: string, velocity: number) {
    this.#drag = null;
    this.#letRowsTakePointers();
    this.diagnostics?.note(why);
    const at = this.current();
    const within = clamp(at, this.range);
    if (within !== at) {
      this.#motion = { kind: "spring", from: at, to: within, startedAt: null };
      this.#animate();
    } else if (Math.abs(velocity) >= COASTING_STOPS) {
      const capped = Math.sign(velocity) * Math.min(Math.abs(velocity), MAX_VELOCITY);
      this.#motion = { kind: "coast", velocity: capped, at: null };
      this.#animate();
    }
  }

  #letRowsTakePointers() {
    if (this.#content.current !== null) {
      this.#content.current.style.pointerEvents = "";
    }
  }
}

/**
 * A `ListScroller` for the life of a message list. A different channel starts pinned to the
 * bottom, with no way known that its reader is going; a linked message's own scroll unpins it.
 */
export function useListScroller(channelId: string, elements: ListElements): ListScroller {
  const [scroller] = useState(() => new ListScroller(elements));
  useEffect(() => {
    scroller.reset();
  }, [scroller, channelId]);
  useEffect(
    () => () => {
      scroller.dispose();
    },
    [scroller],
  );
  return scroller;
}

/**
 * Tells the list's `scroller` what this render knows, before it positions the view whenever
 * the window's edges, the link, or being at the latest change, and observes the rows and the
 * viewport once there is a list.
 */
export function useListPosition(
  scroller: ListScroller,
  {
    loaded,
    atLatest,
    highlightId,
    firstId,
    lastId,
    events,
    rows,
    startBox,
    startShown,
    measured,
  }: {
    /** Whether the channel has a window to show yet. */
    loaded: boolean;
    atLatest: boolean;
    highlightId: string | undefined;
    /** The window's edges. */
    firstId: string | undefined;
    lastId: string | undefined;
    events: ListScrollerEvents;
    rows: RefObject<HTMLElement | null>;
    /** What stands at the beginning of the history, observed with the rows while shown. */
    startBox: RefObject<HTMLElement | null>;
    startShown: boolean;
    /** Runs before each change of the rows' sizes is settled. */
    measured: () => void;
  },
) {
  // What this render knows, for the handlers and observers that outlive it; first, so the
  // positioning below reads it.
  useLayoutEffect(() => {
    scroller.update({ atLatest, highlightId, events });
  });

  useLayoutEffect(() => {
    scroller.position();
  }, [scroller, firstId, lastId, highlightId, atLatest]);

  useEffect(
    () => scroller.observeRows(rows.current, startBox.current, measured),
    // Made once there is a list, and again as the beginning comes and goes; `measured` reads
    // through refs, so the first one serves.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scroller, loaded, startShown],
  );

  useEffect(() => scroller.observeViewport(), [scroller, loaded]);
}
