import { HISTORY_PAGE_SIZE, WINDOW_MAX_MESSAGES, type MessageWindow } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import {
  Fragment,
  startTransition,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type TouchEvent as ReactTouchEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { Button } from "react-aria-components";
import {
  useBlockedUsers,
  useChannel,
  useMessageWindow,
  useReadState,
  useStore,
  useSync,
} from "@/api/hooks";
import { windowParts } from "@/features/messages/blocked";
import { BlockedRun, NewMessagesLine } from "@/features/messages/BlockedRun";
import { channelLink, type ChannelHome } from "@/features/messages/links";
import { MessageItem } from "@/features/messages/MessageItem";
import { useDeparting, type Departing } from "@/features/messages/departing";
import { KeepStillContext } from "@/features/messages/keepStill";
import { useMotion } from "@/features/layout/motion";
import { SCROLL_DEBUG, ScrollDiagnostics } from "@/features/messages/scrollDiagnostics";
import { ScrollDiagnosticsPanel } from "@/features/messages/ScrollDiagnosticsPanel";
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
} from "@/features/messages/scrollPhysics";
import { HistorySkeleton } from "@/features/messages/MessageSkeleton";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/**
 * The loaded window of a channel, oldest at the top. On iOS and iPadOS the list scrolls
 * itself (`OWNS_SCROLLING`): its box hides its overflow, so no finger, wheel, or key scrolls
 * it, and the list takes those itself and sets the box's scroll position from them, with its
 * own coasting, spring, and indicator (`scrollPhysics.ts` for the arithmetic). A box the
 * browser scrolls for the user is the one thing that cannot be kept still there: iOS scrolls
 * such a box in a process of its own and places it from where a pan began plus the finger's
 * travel, overriding whatever the page set meanwhile; a box only scripts scroll has no such
 * gesture, so nothing overrides what the list sets. Scripts still scroll it, so focus,
 * find-in-page, assistive technology, and tests bring things into view as they always did.
 * Everywhere else the browser scrolls the box, on its compositor, with its own indicator,
 * overscroll, and assistive gestures, and honours a position the list sets at any moment;
 * what the list does with the position is the same either way.
 *
 * So what is in view never moves when something above it changes: the list notes the topmost
 * message in view and where it stands in the content (`still`), and a page of older messages
 * arriving is measured by how far that message moved in the layout its arrival made, added to
 * the position in the same layout; a row growing above the view, as a picture loads or a
 * deleted message's space closes, is seen by a `ResizeObserver` on every row and added the
 * same way, before the frame is painted. A page renders as a transition, in slices between
 * which the finger is heard, so the list keeps moving while a page's rows are made. The
 * list stays pinned to the bottom while the user is there and keeps its bottom edge when the
 * keyboard takes the screen's space. History is read well ahead of the reader in the way they
 * are heading, the store keeps the window bounded, and the jump control returns to the
 * present.
 *
 * A linked message is scrolled to the middle of the view once, as soon as it is in the window,
 * even when a window around it had to be read first; staying at the bottom gives way to it.
 * The first scroll the user makes afterwards drops the message from the URL, so the link is
 * shareable but does not keep pulling the view back.
 *
 * In a channel that keeps a read position (any but a thread), the newest message on screen
 * counts as read while the page is visible and has focus. A channel opened with something
 * unread shows the "New Messages" line under the message it had been read up to, and keeps it
 * there while it stays open, though reading moves the position at once; the line goes when the
 * reader leaves or posts.
 *
 * Consecutive messages by people the reader blocked are collapsed into one row, which they may
 * open; a linked message among them opens its row. The row stands for its last message, both
 * as an anchor for the view and as what the reader has seen.
 *
 * When to give scrolling back to the browser on iOS too. The list scrolls itself there for one
 * reason, the pan's override above, and the browser's scrolling is better otherwise: it runs
 * on the compositor and keeps moving while the main thread is busy, draws the platform's own
 * indicator and overscroll, and gives VoiceOver its three-finger scroll, which a box that
 * hides its overflow does not. The day the oldest iOS the apps support honours a position set
 * while a finger drags or a fling runs, or applies scroll anchoring atomically with the
 * gesture, `OWNS_SCROLLING` can go, and the handlers, the physics, and the indicator with it.
 * The proof is `testReadingBackQuicklyNeverJumps` (the iOS UI tests, against a build made
 * with `VITE_SCROLL_DEBUG=1`) with `OWNS_SCROLLING` made false: ten runs in a row counting no
 * jump on the simulator of the oldest supported iOS. What stays whatever scrolls the list:
 * holding the view by the noted row through every change (`still`, `holdStill`), pictures
 * telling of their arrival in the same task, memoized rows, and pages rendered as transitions.
 * Nothing else in the client should assume either way; the list's box is the only place that
 * knows.
 */
/**
 * Whether the list scrolls its box itself rather than the browser: on iOS and iPadOS, the only
 * platforms with `-webkit-touch-callout`, where a pan overrides any position the page sets
 * (see above). A platform, not a behaviour, since the override cannot be felt without a
 * gesture; a test stands in for iOS by claiming the property.
 */
const OWNS_SCROLLING = typeof CSS !== "undefined" && CSS.supports("-webkit-touch-callout", "none");
/**
 * How close to the end of the window the reader is heading for, in screens of the list, the
 * next page is read: far enough that a reader flinging through history meets what is loaded
 * rather than its end.
 */
const LOAD_AHEAD_SCREENS = 20;
/**
 * How close to either end the next page is read while which way the reader is going is
 * unknown. The end behind a reader is never read: each page read at one end of a full window
 * drops messages from the other, so reading at both would trade pages back and forth for as
 * long as the reader is near both, changing what is above the view each time.
 */
const LOAD_UNSURE_SCREENS = 2;
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
/** How long a deleted message's space takes to close at normal speed, as `--motion-base`. */
const COLLAPSE_MS = 200;
/** The ids of a list with no window yet, one array so its identity holds. */
const NO_IDS: readonly string[] = [];

export function MessageList({
  channelId,
  home,
  highlightId,
}: {
  channelId: string;
  home: ChannelHome;
  highlightId: string | undefined;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const latest = useMessageWindow(channelId);
  // What is on screen: the store's window, except that a change to it waits while a deleted
  // message's space is closing. A different channel, or the first page of one, is shown at once.
  const [shown, setShown] = useState({ channelId, window: latest });
  const window =
    shown.channelId === channelId && shown.window !== undefined ? shown.window : latest;
  const held = window !== latest;
  const channel = useChannel(channelId);
  // A thread's messages cannot start threads, and link to the thread rather than to a place in
  // a channel's history.
  const parentId = channel?.ty === "thread" ? (channel.parentChannel ?? null) : null;
  /** The box the list is seen through, whose scroll position the list sets. */
  const viewport = useRef<HTMLDivElement>(null);
  /** Everything the list holds; drawn past an end, while a finger pulls it there, by a transform. */
  const content = useRef<HTMLDivElement>(null);
  /** The rows: messages, blocked runs, the new-messages line, and spaces closing. */
  const rows = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLDivElement>(null);
  const indicator = useRef<HTMLDivElement>(null);
  /** How far past an end a finger has pulled the list, which its scroll position cannot hold. */
  const over = useRef(0);
  /** The scroll positions the content allows in the viewport, as last measured. */
  const range = useRef({ min: 0, max: 0 });
  /**
   * Where the list last scrolled its box to. The scroll event that lands there is its own, and
   * any other is a script's: focus, find-in-page, assistive technology.
   */
  const ownScrollTop = useRef<number | null>(null);
  /** The box's position at its last scroll event, by which the browser's own scrolls are measured. */
  const lastScrollTop = useRef(0);
  /** A finger on the list, where it last was, how far it has gone, and whether it drags yet. */
  const drag = useRef<{
    y: number;
    travelled: number;
    dragging: boolean;
    tracker: VelocityTracker;
  } | null>(null);
  /**
   * The list coasting after a fling, or springing back from past an end. Its times are the
   * animation frames' clock, taken from the first frame that runs it.
   */
  const motion = useRef<
    | { kind: "coast"; velocity: number; at: number | null }
    | { kind: "spring"; from: number; to: number; startedAt: number | null }
    | null
  >(null);
  const frame = useRef<number | null>(null);
  const indicatorTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [loadingNewer, setLoadingNewer] = useState(false);
  /** Whether the newest page is being read for "Jump to latest". */
  const jumping = useRef(false);
  const stickToBottom = useRef(true);
  /** The highlighted message already scrolled to, so it is done once per link. */
  const highlightShown = useRef<string | null>(null);
  /**
   * Which way the reader last moved the list, by their own input; a fling keeps the way of
   * the stroke that started it.
   */
  const heading = useRef<"older" | "newer" | null>(null);
  /**
   * The row kept still, and where it stands in the content, which does not change with
   * scrolling, noted after every move and change: a linked message while it is shown, so a
   * jump lands on it whatever loads around it, and otherwise the topmost row in view. Every
   * change of the rows' sizes is measured by how far it has moved (`keepStillNow`, and the
   * rows' observer).
   */
  const still = useRef<{
    id: string;
    /** Where the row's top stood in the view, which the box's own clamping cannot move. */
    screenTop: number;
    height: number;
    linked: boolean;
  } | null>(null);
  function noteStill() {
    const box = viewport.current;
    if (box === null) {
      still.current = null;
      return;
    }
    const linked = highlightNow.current;
    const shownLink = linked !== undefined && highlightShown.current === linked;
    const row = shownLink
      ? box.querySelector<HTMLElement>(`[data-message-id="${linked}"]`)
      : rowsInView(box).first;
    still.current =
      row === null
        ? null
        : {
            id: row.dataset.messageId ?? "",
            screenTop: row.offsetTop - box.scrollTop,
            height: row.offsetHeight,
            linked: shownLink,
          };
  }
  /**
   * Puts the noted row back where it stood in the view, whatever moved it: content changing
   * above it, or the box clamping its own position as content shrank. A linked message is
   * held by its middle rather than its top, so it stays centred as what it holds, a picture of
   * its own, takes its size.
   */
  function holdStill(why: string) {
    const box = viewport.current;
    const noted = still.current;
    if (box === null || noted === null) {
      return;
    }
    const row = box.querySelector<HTMLElement>(`[data-message-id="${noted.id}"]`);
    if (row !== null) {
      const grown = noted.linked ? (row.offsetHeight - noted.height) / 2 : 0;
      absorb(row.offsetTop - noted.screenTop + grown - box.scrollTop, why);
    }
  }

  /**
   * The first and last message rows with any part in view, found by their offsets in the
   * content, which are in order, so a few reads find them among hundreds; called on every
   * move of the list.
   */
  function rowsInView(box: HTMLDivElement): {
    first: HTMLElement | null;
    last: HTMLElement | null;
  } {
    const rows = box.querySelectorAll<HTMLElement>("[data-message-id]");
    const top = box.scrollTop;
    const bottom = top + box.clientHeight;
    // The first row whose bottom is below the view's top.
    let low = 0;
    let high = rows.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      const row = rows[mid];
      if (row !== undefined && row.offsetTop + row.offsetHeight > top) {
        high = mid;
      } else {
        low = mid + 1;
      }
    }
    const first = rows[low] ?? null;
    if (first === null || first.offsetTop >= bottom) {
      return { first: null, last: null };
    }
    // The last row whose top is above the view's bottom.
    let last = low;
    high = rows.length;
    while (last < high) {
      const mid = (last + high + 1) >> 1;
      const row = rows[mid];
      if (row !== undefined && row.offsetTop < bottom) {
        last = mid;
      } else {
        high = mid - 1;
      }
    }
    return { first, last: rows[last] ?? null };
  }
  /**
   * A row's height just changed in the DOM: keeps the view still through it now, in the same
   * task, by what the noted row has moved in the content; pinned to the bottom, the list stays
   * there. The rows' observer then finds nothing left to do. Rows get one function for the
   * list's life, which runs this render's.
   */
  function keepStillNow() {
    measureRange();
    // What changed above the view is absorbed, not a move; pinned to the bottom, what changed
    // below is then followed, which is one.
    holdStill("row told");
    if (stickToBottom.current && atLatestNow.current && drag.current === null) {
      moveTo(range.current.max, false);
    }
    noteStill();
  }
  const keepStillLatest = useRef(keepStillNow);
  const keepStill = useCallback(() => {
    keepStillLatest.current();
  }, []);
  /** What the list does with its offset, in a build made to find a jump (see the module). */
  const [diagnostics] = useState(() => (SCROLL_DEBUG ? new ScrollDiagnostics() : null));
  const store = useStore();
  const readState = useReadState(channelId);
  /**
   * Where the "New Messages" line goes in this channel: after the read position it had when
   * opened, or nowhere when nothing was unread then. Set once per channel, when its read state
   * is first known.
   */
  const [line, setLine] = useState<{ channelId: string; after: string | null } | null>(null);
  if (readState !== undefined && line?.channelId !== channelId) {
    const unread = readState.lastMessage != null && readState.lastMessage > readState.lastRead;
    setLine({ channelId, after: unread ? readState.lastRead : null });
  }
  const lineAfter = line?.channelId === channelId ? line.after : null;
  // Posting ends the line: what came before the reader's own message is read.
  const newest = latest?.ids[latest.ids.length - 1];
  if (
    lineAfter !== null &&
    newest !== undefined &&
    newest > lineAfter &&
    store.message(newest)?.author === store.myUserId
  ) {
    setLine({ channelId, after: null });
  }
  const seenFrame = useRef<number | null>(null);
  const blockedUsers = useBlockedUsers();
  const parts = useMemo(() => {
    const blocked = new Set(blockedUsers);
    return windowParts(window?.ids ?? [], (id) => {
      const author = store.message(id)?.author;
      return author !== undefined && blocked.has(author);
    });
  }, [window, blockedUsers, store]);

  // The store's window, not the one shown: a deleted message's row empties as the store drops
  // it, before the shown window catches up, and its space must be there in that same frame.
  const { departing, measure, forget } = useDeparting(viewport, latest?.ids ?? NO_IDS, store);
  const closing = useRef(false);
  useEffect(() => {
    closing.current = departing.length > 0;
  });

  const loaded = window !== undefined;
  const ids = window?.ids;
  const firstId = ids?.[0];
  const lastId = ids?.[ids.length - 1];
  const hasOlder = window?.hasOlder ?? false;
  const atLatest = window?.atLatest ?? true;
  const atLatestNow = useRef(atLatest);
  const highlightNow = useRef(highlightId);
  // What this render knows, for the handlers and observers that outlive it.
  useLayoutEffect(() => {
    atLatestNow.current = atLatest;
    highlightNow.current = highlightId;
    keepStillLatest.current = keepStillNow;
  });

  /** Reads the content's and the viewport's heights afresh. */
  function measureRange() {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    range.current = bounds(box.scrollHeight, box.clientHeight);
  }

  /** The list's position: its box's, and what a finger has pulled it past an end by. */
  function current(): number {
    return (viewport.current?.scrollTop ?? 0) + over.current;
  }

  /**
   * Scrolls the box to `at`, which is the list's own doing: the position it then has, which
   * the box may have rounded or clamped, is what its scroll event will say.
   */
  function scrollBoxTo(at: number) {
    const box = viewport.current;
    if (box === null || box.scrollTop === at) {
      return;
    }
    box.scrollTop = at;
    ownScrollTop.current = box.scrollTop;
    lastScrollTop.current = box.scrollTop;
  }

  /** Shows the content at `next`, and the indicator with it. */
  function place(next: number) {
    const body = content.current;
    const box = viewport.current;
    if (body === null || box === null) {
      return;
    }
    const within = clamp(next, range.current);
    scrollBoxTo(within);
    over.current = next - within;
    const pulled = shownOffset(next, range.current, box.clientHeight) - within;
    body.style.transform = pulled === 0 ? "" : `translate3d(0, ${String(-pulled)}px, 0)`;
    const bar = thumb.current;
    const track = indicator.current;
    if (bar !== null && track !== null) {
      const { min, max } = range.current;
      const span = max - min;
      const trackHeight = track.clientHeight;
      const height =
        span <= 0
          ? trackHeight
          : Math.max(THUMB_MIN_PX, (box.clientHeight / box.scrollHeight) * trackHeight);
      const top = span <= 0 ? 0 : ((within - min) / span) * (trackHeight - height);
      bar.style.height = `${String(height)}px`;
      bar.style.transform = `translate3d(0, ${String(top)}px, 0)`;
      track.style.opacity = span <= 0 ? "0" : "1";
    }
  }

  /** Shows the indicator for a moment: the list moved. */
  function showIndicator() {
    const track = indicator.current;
    if (track === null) {
      return;
    }
    track.dataset.moving = "";
    clearTimeout(indicatorTimer.current);
    indicatorTimer.current = setTimeout(() => {
      delete track.dataset.moving;
    }, INDICATOR_MS);
  }

  /** What follows a move of the view: the pin to the bottom, reading, the next page. */
  function afterMove(byUser: boolean) {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    noteStill();
    showIndicator();
    if (byUser) {
      stickToBottom.current = atLatestNow.current && range.current.max - box.scrollTop < 8;
    }
    noteSeenSoon();
    loadNearEnds();
  }

  /**
   * Moves the view to `next` on purpose: a finger, a fling, a key, the wheel. A move of the
   * reader's drops a linked message from the URL.
   */
  function moveTo(next: number, byUser: boolean) {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    const before = shownOffset(current(), range.current, box.clientHeight);
    place(next);
    const after = shownOffset(current(), range.current, box.clientHeight);
    diagnostics?.moved(after - before);
    afterMove(byUser);
    if (byUser && highlightId !== undefined) {
      void navigate({ ...channelLink(home, channelId), replace: true });
    }
  }

  /** Keeps the view still through a change of `by` pixels in what is above it. */
  function absorb(by: number, why: string) {
    const box = viewport.current;
    if (box === null || Math.abs(by) < 0.5) {
      return;
    }
    diagnostics?.note(
      `${why} ${String(Math.round(by))} absorbed at ${String(Math.round(box.scrollTop))}`,
    );
    scrollBoxTo(box.scrollTop + by);
    noteStill();
  }

  /**
   * The box scrolled: by the list, which is nothing new; by the browser for the reader, where
   * it scrolls the box; or by a script, as focus, find-in-page, and assistive technology do.
   * The list follows the last two.
   */
  function onScroll() {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    const by = box.scrollTop - lastScrollTop.current;
    lastScrollTop.current = box.scrollTop;
    const own = ownScrollTop.current;
    if (own !== null && Math.abs(box.scrollTop - own) < 1) {
      ownScrollTop.current = null;
      return;
    }
    diagnostics?.moved(by);
    diagnostics?.note(
      `${OWNS_SCROLLING ? "scrolled by a script" : "scrolled"} to ${String(Math.round(box.scrollTop))}`,
    );
    if (!OWNS_SCROLLING && by !== 0) {
      heading.current = by < 0 ? "older" : "newer";
    }
    stopMotion();
    over.current = 0;
    afterMove(true);
  }

  function stopMotion() {
    motion.current = null;
    if (frame.current !== null) {
      cancelAnimationFrame(frame.current);
      frame.current = null;
    }
  }

  /** Runs the list's coasting or spring, a frame at a time, until it comes to rest. */
  function animate() {
    frame.current = requestAnimationFrame((now) => {
      frame.current = null;
      const moving = motion.current;
      const box = viewport.current;
      if (moving === null || box === null) {
        return;
      }
      if (moving.kind === "coast") {
        // By the time passed, however long a frame took: a list stalled by a long render coasts
        // on to where it would have been.
        const { velocity, travelled } = coast(moving.velocity, now - (moving.at ?? now));
        const { min, max } = range.current;
        let next = current() + travelled;
        let done = Math.abs(velocity) < COASTING_STOPS;
        if (next < min || next > max) {
          // Coasting ends at an end; there is nothing past it to see.
          next = clamp(next, range.current);
          done = true;
        }
        motion.current = done ? null : { kind: "coast", velocity, at: now };
        moveTo(next, true);
      } else {
        const startedAt = moving.startedAt ?? now;
        const elapsed = now - startedAt;
        const next = spring(moving.from, moving.to, elapsed);
        motion.current = elapsed >= SPRING_MS ? null : { ...moving, startedAt };
        moveTo(next, true);
      }
      if (motion.current !== null) {
        animate();
      }
    });
  }

  /** Lets a list pulled past an end go: it springs back. Otherwise it coasts with the finger's speed. */
  function release(velocity: number) {
    const at = current();
    const past = clamp(at, range.current);
    if (past !== at) {
      motion.current = { kind: "spring", from: at, to: past, startedAt: null };
      animate();
      return;
    }
    if (Math.abs(velocity) >= COASTING_STOPS) {
      const capped = Math.sign(velocity) * Math.min(Math.abs(velocity), MAX_VELOCITY);
      motion.current = { kind: "coast", velocity: capped, at: null };
      animate();
    }
  }

  function onTouchStart(event: ReactTouchEvent<HTMLDivElement>) {
    const touch = event.touches[0];
    if (touch === undefined) {
      return;
    }
    stopMotion();
    const tracker = new VelocityTracker();
    tracker.add(touch.clientY, Date.now());
    drag.current = { y: touch.clientY, travelled: 0, dragging: false, tracker };
    diagnostics?.note("touch start");
  }

  function onTouchMove(event: ReactTouchEvent<HTMLDivElement>) {
    const touch = event.touches[0];
    const dragging = drag.current;
    if (touch === undefined || dragging === null) {
      return;
    }
    const dy = dragging.y - touch.clientY;
    dragging.y = touch.clientY;
    dragging.travelled += Math.abs(dy);
    dragging.tracker.add(touch.clientY, Date.now());
    if (!dragging.dragging && dragging.travelled >= DRAG_SLOP_PX) {
      // A drag, not a tap: whatever the finger lifts over, a picture or a button, must not
      // take the lift as a press. The browser's own pan would have cancelled the pointer; the
      // rows take no pointer until the finger has lifted, which comes after the pointer has.
      dragging.dragging = true;
      if (content.current !== null) {
        content.current.style.pointerEvents = "none";
      }
    }
    if (dy !== 0) {
      // A finger moving down the screen draws older messages into view.
      heading.current = dy < 0 ? "older" : "newer";
      moveTo(current() + dy, true);
    }
  }

  function onTouchEnd(event: ReactTouchEvent<HTMLDivElement>) {
    const dragging = drag.current;
    if (dragging === null || event.touches.length > 0) {
      return;
    }
    drag.current = null;
    letRowsTakePointers();
    diagnostics?.note("touch end");
    // The finger's speed down the screen is the content's speed up it.
    release(-dragging.tracker.velocity(Date.now()));
  }

  function onTouchCancel() {
    drag.current = null;
    letRowsTakePointers();
    diagnostics?.note("touch cancel");
    release(0);
  }

  function letRowsTakePointers() {
    if (content.current !== null) {
      content.current.style.pointerEvents = "";
    }
  }

  function onWheel(event: ReactWheelEvent<HTMLDivElement>) {
    const box = viewport.current;
    if (box === null || event.deltaY === 0) {
      return;
    }
    stopMotion();
    const by =
      event.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? event.deltaY * WHEEL_LINE_PX
        : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? event.deltaY * box.clientHeight
          : event.deltaY;
    heading.current = by < 0 ? "older" : "newer";
    moveTo(clamp(current() + by, range.current), true);
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const box = viewport.current;
    if (box === null || event.target instanceof HTMLTextAreaElement) {
      return;
    }
    const page = box.clientHeight - PAGE_OVERLAP_PX;
    const { min, max } = range.current;
    const at = current();
    const moves: Record<string, number | undefined> = {
      ArrowUp: at - ARROW_PX,
      ArrowDown: at + ARROW_PX,
      PageUp: at - page,
      PageDown: at + page,
      Home: min,
      End: max,
      " ": event.shiftKey ? at - page : at + page,
    };
    const next = moves[event.key];
    if (next === undefined) {
      return;
    }
    event.preventDefault();
    stopMotion();
    heading.current = next < at ? "older" : "newer";
    moveTo(clamp(next, range.current), true);
  }

  /** The indicator's thumb dragged with a pointer: the view follows it along the track. */
  function onThumbPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    const track = indicator.current;
    const bar = thumb.current;
    if (track === null || bar === null) {
      return;
    }
    event.preventDefault();
    stopMotion();
    const startY = event.clientY;
    const startOffset = clamp(current(), range.current);
    const travel = track.clientHeight - bar.clientHeight;
    const { min, max } = range.current;
    const onMove = (move: PointerEvent) => {
      if (travel <= 0) {
        return;
      }
      const next = startOffset + ((move.clientY - startY) / travel) * (max - min);
      heading.current = next < current() ? "older" : "newer";
      moveTo(clamp(next, range.current), true);
    };
    const onUp = () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
      document.removeEventListener("pointercancel", onUp);
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    document.addEventListener("pointercancel", onUp);
  }

  // Shows the store's window, as a transition: React renders its rows in slices, between
  // which the finger is heard. A change waits while a deleted message's space is closing:
  // rendering the whole list again meanwhile would use its moment up before it is seen.
  useEffect(() => {
    if (shown.channelId === channelId && shown.window === latest) {
      return;
    }
    // Shown directly already (see `window`); the state only catches up.
    const direct = shown.channelId !== channelId || shown.window === undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const commit = () => {
      if (!direct && closing.current) {
        timer = setTimeout(commit, 16);
        return;
      }
      diagnostics?.note(
        `commit ${String(shown.window?.ids.length ?? 0)}->${String(latest?.ids.length ?? 0)} first ${shown.window?.ids[0]?.slice(-4) ?? "-"}->${latest?.ids[0]?.slice(-4) ?? "-"} still=${still.current?.id.slice(-4) ?? "-"}@${String(Math.round(still.current?.screenTop ?? 0))}`,
      );
      startTransition(() => {
        setShown({ channelId, window: latest });
      });
    };
    timer = setTimeout(commit, 0);
    return () => {
      clearTimeout(timer);
    };
  }, [channelId, latest, shown, diagnostics]);

  useLayoutEffect(() => {
    if (viewport.current !== null) {
      position();
    }
    // Positioned when the window's edges, the link, or being at the latest change; what it
    // reads besides is current in the refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [firstId, lastId, highlightId, atLatest]);

  /** Places the view after the window or the link changed: see the effect above. */
  function position() {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    measureRange();
    if (highlightId === undefined) {
      highlightShown.current = null;
    } else if (highlightShown.current !== highlightId) {
      // A linked message goes to the middle of the view once it is in the window, whatever
      // view was being kept, and the list lets go of the bottom so that nothing arriving or
      // growing later pulls the view away from it.
      const target = box.querySelector(`[data-message-id="${highlightId}"]`);
      if (target !== null) {
        highlightShown.current = highlightId;
        stickToBottom.current = false;
        stopMotion();
        over.current = 0;
        target.scrollIntoView({ block: "center" });
        ownScrollTop.current = box.scrollTop;
        afterMove(false);
        return;
      }
    }
    // A linked message not yet in the window keeps the view for it; otherwise the view holds
    // through the change, at the bottom or by the noted message, a linked message shown
    // included: the pages read around it arrive just after it is centred.
    if (highlightId !== undefined && highlightShown.current !== highlightId) {
      return;
    }
    if (stickToBottom.current && atLatest) {
      stopMotion();
      moveTo(range.current.max, false);
    } else {
      holdStill("page");
    }
    noteStill();
  }

  // A different channel starts pinned to the bottom, with no way known that its reader is going;
  // a linked message's own scroll unpins it.
  useEffect(() => {
    stickToBottom.current = true;
    heading.current = null;
    stopMotion();
    drag.current = null;
    letRowsTakePointers();
    // Deliberately runs when the channel changes.
  }, [channelId]);

  useEffect(
    () => () => {
      stopMotion();
      clearTimeout(indicatorTimer.current);
    },
    [],
  );

  // Rows change size without the window changing: pictures and link cards load, reactions come
  // and go, a deleted message's space closes. The view holds by the noted row, before the frame
  // is painted; pinned to the bottom, the list stays there.
  useEffect(() => {
    const body = rows.current;
    if (body === null || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(() => {
      measure();
      measureRange();
      holdStill("rows");
      if (stickToBottom.current && atLatestNow.current && drag.current === null) {
        moveTo(range.current.max, false);
      }
      noteStill();
    });
    observer.observe(body);
    return () => {
      observer.disconnect();
    };
    // Reads the rest through refs; made once there is a list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  // The viewport changes size: something takes the screen's space, as the keyboard does when
  // the message box is chosen on a phone, or gives it back. Its bottom edge stays where it was,
  // so what was just above the box, likely what is being answered, stays in view: pinned to
  // the newest message, or moved by what the viewport lost or gained.
  useEffect(() => {
    const box = viewport.current;
    if (box === null || typeof ResizeObserver === "undefined") {
      return;
    }
    let height = box.clientHeight;
    const observer = new ResizeObserver(() => {
      const lost = height - box.clientHeight;
      height = box.clientHeight;
      measureRange();
      if (lost === 0) {
        return;
      }
      const next =
        stickToBottom.current && atLatestNow.current
          ? range.current.max
          : clamp(current() + lost, range.current);
      moveTo(next, false);
    });
    observer.observe(box);
    return () => {
      observer.disconnect();
    };
    // Reads the pin through its ref; made once there is a list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  /**
   * Whether a page read at one end leaves the other end's messages that a full window drops
   * well out of view: at least `LOAD_UNSURE_SCREENS` past the view, so nothing the reader can
   * see goes, and they meet the dropped end only after reading their way back towards it.
   */
  function roomFor(way: "older" | "newer"): boolean {
    const box = viewport.current;
    const dropped = (ids?.length ?? 0) + HISTORY_PAGE_SIZE - WINDOW_MAX_MESSAGES;
    if (box === null || ids === undefined || dropped <= 0) {
      return true;
    }
    // The dropped message nearest the view.
    const nearest = way === "older" ? ids[ids.length - dropped] : ids[dropped - 1];
    const row = box.querySelector(`[data-message-id="${nearest ?? ""}"]`);
    if (row === null) {
      return true;
    }
    const view = box.getBoundingClientRect();
    const rect = row.getBoundingClientRect();
    const margin = box.clientHeight * LOAD_UNSURE_SCREENS;
    return way === "older" ? rect.top > view.bottom + margin : rect.bottom < view.top - margin;
  }

  // A page already read but not yet shown is waiting for the next render; reading the next one
  // before it shows would only pile changes up.
  function loadOlder() {
    if (loadingOlder || !hasOlder || held || !roomFor("older")) {
      return;
    }
    setLoadingOlder(true);
    sync
      .loadOlder(channelId)
      .catch(() => undefined)
      .finally(() => {
        setLoadingOlder(false);
      });
  }

  function loadNewer() {
    if (loadingNewer || atLatest || held || !roomFor("newer")) {
      return;
    }
    setLoadingNewer(true);
    sync
      .loadNewer(channelId)
      .catch(() => undefined)
      .finally(() => {
        setLoadingNewer(false);
      });
  }

  /** Reads the next page when the viewport is near an end of the window that has more. */
  function loadNearEnds() {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    const { min, max } = range.current;
    const at = current();
    const distanceFromTop = at - min;
    const distanceFromBottom = max - at;
    const toward = heading.current;
    const screens = toward === null ? LOAD_UNSURE_SCREENS : LOAD_AHEAD_SCREENS;
    const near = box.clientHeight * screens;
    if (toward !== "newer" && distanceFromTop < near) {
      loadOlder();
    }
    if (toward !== "older" && distanceFromBottom < near && !jumping.current) {
      loadNewer();
    }
  }

  // A window shorter than the viewport moves nothing, so ask once it is rendered.
  useEffect(() => {
    if (loaded) {
      loadNearEnds();
    }
    // The ends are re-checked only when the window's edges move, not on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, firstId, lastId]);

  /** Back to the present: the newest page replaces the window and the view pins to the bottom. */
  function jumpToLatest(): Promise<void> {
    stickToBottom.current = true;
    heading.current = null;
    jumping.current = true;
    stopMotion();
    // The list goes to the end of what it holds while the newest are on their way; the next
    // page after it is not read, since the newest replace the window.
    measureRange();
    moveTo(range.current.max, false);
    if (highlightId !== undefined) {
      void navigate({ ...channelLink(home, channelId), replace: true });
    }
    return sync
      .loadLatest(channelId)
      .catch(() => undefined)
      .finally(() => {
        jumping.current = false;
      });
  }

  /**
   * Marks the newest message with any part on screen as read, when the reader can see the page.
   * Messages of other channels drawn inside this one's, such as the reply an echo shows, are
   * not this channel's to mark.
   */
  function noteSeen() {
    const box = viewport.current;
    if (
      box === null ||
      readState === undefined ||
      ids === undefined ||
      document.visibilityState !== "visible" ||
      !document.hasFocus()
    ) {
      return;
    }
    // The newest row in view that is this channel's: a reply an echo shows, which is not,
    // stands under its echo's row.
    let row = rowsInView(box).last;
    const inWindow = new Set(ids);
    while (row !== null && !inWindow.has(row.dataset.messageId ?? "")) {
      row = row.parentElement?.closest<HTMLElement>("[data-message-id]") ?? null;
    }
    const seen = row?.dataset.messageId;
    if (seen !== undefined) {
      sync.markRead(channelId, seen);
    }
  }

  // Once a frame at most, however fast the list scrolls.
  function noteSeenSoon() {
    seenFrame.current ??= requestAnimationFrame(() => {
      seenFrame.current = null;
      noteSeen();
    });
  }

  // What is on screen changes with the window, and becomes seen when the page is looked at.
  useEffect(() => {
    noteSeenSoon();
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        sync.flushReads();
      } else {
        noteSeenSoon();
      }
    };
    globalThis.addEventListener("focus", noteSeenSoon);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      globalThis.removeEventListener("focus", noteSeenSoon);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  // Leaving the channel reports what was read in it straight away.
  useEffect(
    () => () => {
      if (seenFrame.current !== null) {
        cancelAnimationFrame(seenFrame.current);
        seenFrame.current = null;
      }
      sync.flushReads();
    },
    [sync, channelId],
  );

  if (window === undefined) {
    return <HistorySkeleton />;
  }

  const lineIndex = lineAt(window, lineAfter);
  const leavingAfter = new Map<string | null, Departing[]>();
  for (const gone of departing) {
    leavingAfter.set(gone.after, [...(leavingAfter.get(gone.after) ?? []), gone]);
  }
  const leaving = (after: string | null) =>
    (leavingAfter.get(after) ?? []).map((gone) => (
      <DepartingSpace
        key={gone.id}
        gone={gone}
        onClosed={() => {
          forget(gone.id);
        }}
      />
    ));

  return (
    <div
      ref={viewport}
      data-message-list=""
      data-owns-scrolling={OWNS_SCROLLING ? "" : undefined}
      onScroll={onScroll}
      {...(OWNS_SCROLLING
        ? { onTouchStart, onTouchMove, onTouchEnd, onTouchCancel, onWheel, onKeyDown }
        : {})}
      // Where the list scrolls itself, hidden overflow: only scripts scroll the box, the
      // list's own among them, and fingers pan it through the handlers. Everywhere else the
      // browser scrolls it, with its anchoring off, since the list keeps its own view still.
      className={
        OWNS_SCROLLING
          ? "relative min-h-0 flex-1 touch-none overflow-hidden [overflow-anchor:none]"
          : "relative min-h-0 flex-1 overflow-y-auto overscroll-contain [overflow-anchor:none]"
      }
    >
      <KeepStillContext.Provider value={keepStill}>
        <div ref={content} className="flex min-h-full flex-col justify-end gap-1 px-4 py-3">
          {window.hasOlder ? (
            // Its height does not change with its text: the text comes and goes above what is being
            // read, and a line appearing there would push the view down.
            <p aria-live="polite" className="flex min-h-9 items-center justify-center py-2">
              {loadingOlder && (
                <>
                  <LoadingLabel />
                  <Skeleton className="h-3 w-24" />
                </>
              )}
            </p>
          ) : (
            <p className="py-2 text-center text-sm text-ink-faint">{m.channelStart}</p>
          )}
          <div ref={rows} className="flex flex-col gap-1">
            {lineIndex === -1 && <NewMessagesLine />}
            {leaving(null)}
            {parts.map((part) => {
              const item = (id: string) => (
                <MessageItem
                  id={id}
                  home={home}
                  channelId={channelId}
                  parentId={parentId}
                  highlighted={id === highlightId}
                />
              );
              if (part.kind === "message") {
                return (
                  <Fragment key={part.id}>
                    {item(part.id)}
                    {leaving(part.id)}
                    {part.index === lineIndex && <NewMessagesLine />}
                  </Fragment>
                );
              }
              const lineOffset =
                lineIndex !== null &&
                lineIndex >= part.index &&
                lineIndex < part.index + part.ids.length
                  ? lineIndex - part.index
                  : null;
              return (
                <Fragment key={part.ids[0]}>
                  <BlockedRun
                    ids={part.ids}
                    lineOffset={lineOffset}
                    highlightId={highlightId}
                    item={item}
                  />
                  {part.ids.flatMap((id) => leaving(id))}
                </Fragment>
              );
            })}
          </div>
          {!window.atLatest && (
            <p aria-live="polite" className="flex min-h-9 items-center justify-center py-2">
              {loadingNewer && (
                <>
                  <LoadingLabel />
                  <Skeleton className="h-3 w-24" />
                </>
              )}
            </p>
          )}
        </div>
      </KeepStillContext.Provider>
      {OWNS_SCROLLING && (
        <div
          ref={indicator}
          aria-hidden="true"
          // Over the list's own padding, taking no pointer but its thumb's, and that only where
          // there is a pointer to drag it with.
          className="pointer-events-none absolute end-0.5 top-1 bottom-1 w-1.5 opacity-0 transition-opacity duration-300 data-moving:opacity-100 pointer-fine:w-2.5 pointer-fine:hover:opacity-100"
        >
          <div
            ref={thumb}
            onPointerDown={onThumbPointerDown}
            className="absolute inset-x-0 top-0 rounded-full bg-ink/35 pointer-fine:pointer-events-auto"
          />
        </div>
      )}
      {!window.atLatest && <JumpToLatest onJump={jumpToLatest} />}
      {diagnostics !== null && (
        <ScrollDiagnosticsPanel diagnostics={diagnostics} viewport={viewport} />
      )}
    </div>
  );
}

/**
 * The pill that brings a channel back to its newest messages. It keeps its own state, so a press
 * repaints the pill alone, saying the newest are on their way, and `onJump`, which sets the
 * whole list moving, starts only once that is on screen.
 */
function JumpToLatest({ onJump }: { onJump: () => Promise<void> }) {
  const m = useMessages();
  const [jumping, setJumping] = useState(false);
  return (
    <Button
      onPress={() => {
        setJumping(true);
        requestAnimationFrame(() => {
          setTimeout(() => {
            void onJump().finally(() => {
              setJumping(false);
            });
          }, 0);
        });
      }}
      isPending={jumping}
      className="motion-rise sticky bottom-3 left-1/2 flex w-fit -translate-x-1/2 items-center gap-2 rounded-full bg-accent px-4 py-1.5 text-sm font-medium text-accent-contrast shadow outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {jumping && (
        <span
          aria-hidden="true"
          className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-accent-contrast/40 border-t-accent-contrast"
        />
      )}
      {jumping ? m.jumpingToLatest : m.jumpToLatest}
    </Button>
  );
}

/**
 * The space a deleted message leaves, closing over a moment; the list's `gap-1` between messages
 * closes with it. The closing starts a frame after the list has rendered without the message,
 * not as the space is made: a long render would otherwise spend most of it before anything is
 * painted.
 */
function DepartingSpace({ gone, onClosed }: { gone: Departing; onClosed: () => void }) {
  const space = useRef<HTMLDivElement>(null);
  const motion = useMotion();
  useLayoutEffect(() => {
    const element = space.current;
    if (element === null) {
      return;
    }
    let closing: Animation | undefined;
    // Two frames on: the frame the space is made in may have begun long before a slow render
    // ended, and an animation started in it would count from then.
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(start);
    });
    const start = () => {
      closing = element.animate(
        [
          { height: `${String(gone.height)}px`, marginTop: "0px" },
          { height: "0px", marginTop: "-0.25rem" },
        ],
        {
          duration: COLLAPSE_MS * motion.scale,
          easing: "cubic-bezier(0.4, 0, 0.2, 1)",
          fill: "forwards",
        },
      );
      closing.finished.then(onClosed, () => undefined);
    };
    return () => {
      cancelAnimationFrame(frame);
      closing?.cancel();
    };
    // Closes once, from where it was made.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div
      ref={space}
      aria-hidden="true"
      className="overflow-hidden"
      style={{ height: `${String(gone.height)}px` }}
    />
  );
}

/**
 * The index of the message the "New Messages" line goes under, the last one at or before
 * `after`; `-1` for the top of the channel, when everything in it is new; `null` for no line in
 * this window, when nothing after `after` is loaded, or the line belongs above what is loaded.
 */
function lineAt(window: MessageWindow, after: string | null): number | null {
  if (after === null) {
    return null;
  }
  let index = -1;
  for (const [i, id] of window.ids.entries()) {
    if (id > after) {
      break;
    }
    index = i;
  }
  if (index === window.ids.length - 1 || (index === -1 && window.hasOlder)) {
    return null;
  }
  return index;
}
