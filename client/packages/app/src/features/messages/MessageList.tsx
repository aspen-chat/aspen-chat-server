import { HISTORY_PAGE_SIZE, WINDOW_MAX_MESSAGES, type MessageWindow } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import {
  Fragment,
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
import { flushSync } from "react-dom";
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
import { useMotion } from "@/features/layout/motion";
import { SCROLL_DEBUG, ScrollDiagnostics } from "@/features/messages/scrollDiagnostics";
import { ScrollDiagnosticsPanel } from "@/features/messages/ScrollDiagnosticsPanel";
import { HistorySkeleton, MessageSkeleton } from "@/features/messages/MessageSkeleton";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/**
 * The loaded window of a channel, oldest at the top. Stays pinned to the bottom while the user
 * is there, and otherwise keeps what is in view still through every change above it: a page of
 * older messages arriving, a picture loading, a deleted message's space closing.
 *
 * It does so without moving the scroll position, which is the one thing that cannot be done
 * safely: iOS scrolls in a process of its own, and its pan gesture places the view from where
 * the pan began plus the finger's travel, so a position set from the page while a finger drags
 * or a fling runs is overridden by the pan's next update, and the view lands wherever the change
 * put it. Instead the list keeps a reserve of space above its oldest message while there is
 * older history (`reserve`, drawn as skeleton rows), and whatever grows above the view takes
 * exactly its height from the reserve in the same layout, so nothing in view moves and there is
 * no position to override: a page can show at any moment, mid-drag and mid-fling included. The
 * browser's own scroll anchoring is off, so it never corrects the same change again. Only
 * refilling the reserve once pages have used it, and letting it go once the channel's start is
 * reached, move the position, and those wait for a quiet moment and take a frame, not a render.
 * History is read well ahead of the reader in the way they are heading, and the store keeps the
 * window bounded, so a long scroll drops what is far from the viewport, and the jump control
 * returns to the present.
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
 */
const PROGRAMMATIC_SCROLL_MS = 200;
/** How long after a wheel, touch, scrollbar press, or key a scroll event still counts as the reader's. */
const USER_SCROLL_MS = 500;
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
 * How much space is kept above the oldest message while there is older history, in pages the
 * size of the rows loaded so far: enough for the pages that may arrive before a quiet moment
 * refills it.
 */
const RESERVE_PAGES = 3;
/** How many skeleton rows are drawn at the bottom of the reserve; above them it is empty. */
const RESERVE_ROWS = 24;
/**
 * How long the list must have been quiet, with no finger on it and no scroll event, before its
 * position is moved to refill or let go of the reserve: short enough to come between a reader's
 * strokes, and the move takes a frame, so a finger landing in it is unlikely.
 */
const QUIET_MS = 300;
/** How long a deleted message's space takes to close at normal speed, as `--motion-base`. */
const COLLAPSE_MS = 200;
/** The ids of a list with no window yet, one array so its identity holds. */
const NO_IDS: readonly string[] = [];
/** The keys that move the list back through history; the other scroll keys move it forward. */
const OLDER_KEYS: ReadonlySet<string> = new Set(["ArrowUp", "PageUp", "Home"]);
const SCROLL_KEYS: ReadonlySet<string> = new Set([
  "ArrowUp",
  "ArrowDown",
  "PageUp",
  "PageDown",
  "Home",
  "End",
  " ",
]);

/** A message in the viewport and where it sat, so a change can be measured by it. */
interface Anchor {
  id: string;
  top: number;
}

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
  const scroller = useRef<HTMLDivElement>(null);
  /** The rows: messages, blocked runs, the new-messages line, and spaces closing. */
  const rows = useRef<HTMLDivElement>(null);
  /** The reserve above the oldest message, and its height in pixels. */
  const reserveBox = useRef<HTMLDivElement>(null);
  const reserve = useRef(0);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [loadingNewer, setLoadingNewer] = useState(false);
  /** Whether the newest page is being read for "Jump to latest". */
  const jumping = useRef(false);
  /** Where the view was before the window changed, so the change is measured by it. */
  const anchor = useRef<Anchor | null>(null);
  const stickToBottom = useRef(true);
  /** The highlighted message already scrolled to, so it is done once per link. */
  const highlightShown = useRef<string | null>(null);
  /** Scroll events before this time were caused by this component, not the user. */
  const programmaticUntil = useRef(0);
  /**
   * Where the list last scrolled itself to. The scroll event that lands there is its own, and
   * any other is someone else's: the reader's, find-in-page's, a screen reader's. It is told by
   * where the view lands rather than by when, because a reader's scroll in the midst of the
   * list's own is still theirs.
   */
  const ownScrollTop = useRef<number | null>(null);
  /** What the list does with its position, in a build made to find a jump (see the module). */
  const [diagnostics] = useState(() => (SCROLL_DEBUG ? new ScrollDiagnostics() : null));
  function scrollSelf(element: HTMLDivElement, move: () => void) {
    programmaticUntil.current = Date.now() + PROGRAMMATIC_SCROLL_MS;
    const before = element.scrollTop;
    move();
    ownScrollTop.current = element.scrollTop === before ? null : element.scrollTop;
    diagnostics?.noteMoved(element.scrollTop, "self");
  }
  /**
   * When the reader last acted on the list: a wheel or touch move, a press on its scrollbar, or a
   * navigation key. A scroll event is theirs only if it follows one of these closely; the browser
   * also fires scroll events when a modal opening changes the layout, when an image loads, or
   * when a script scrolls a message into view, and none of those mean the reader moved on.
   */
  const userScrollAt = useRef(0);
  /** When the list last scrolled, and whether a finger is on it. */
  const scrolledAt = useRef(0);
  const touching = useRef(false);
  /**
   * Which way the reader last moved the list, by their own input rather than by scroll
   * positions, which content changing size above the view moves too; a fling keeps the way of
   * the stroke that started it.
   */
  const heading = useRef<"older" | "newer" | null>(null);
  const touchY = useRef<number | null>(null);
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
  const { departing, measure, forget } = useDeparting(scroller, latest?.ids ?? NO_IDS, store);
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
  const hasOlderNow = useRef(hasOlder);
  hasOlderNow.current = hasOlder;

  /** The topmost message with any part in view, and its offset from the top of the scroller. */
  function captureAnchor(): Anchor | null {
    const element = scroller.current;
    if (element === null) {
      return null;
    }
    const origin = element.getBoundingClientRect().top;
    for (const article of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
      const rect = article.getBoundingClientRect();
      if (rect.bottom > origin) {
        return { id: article.dataset.messageId ?? "", top: rect.top - origin };
      }
    }
    return null;
  }

  function setReserve(px: number) {
    reserve.current = px;
    if (reserveBox.current !== null) {
      reserveBox.current.style.height = `${String(px)}px`;
    }
  }

  /** The reserve to keep: `RESERVE_PAGES` pages the size of the rows loaded so far. */
  function reserveTarget(): number {
    const box = rows.current;
    const count = ids?.length ?? 0;
    if (box === null || count === 0) {
      return 0;
    }
    return Math.round((box.offsetHeight / count) * HISTORY_PAGE_SIZE * RESERVE_PAGES);
  }

  /**
   * Keeps the view still through a change of `by` pixels in the height of what is above it:
   * the reserve gives up that much, or takes it back when what was above shrank. Only when the
   * reserve has nothing left to give is the position moved, by what remains; a pan under way
   * on iOS overrides that, so the reserve is kept from running out (`maintain`).
   */
  function absorb(element: HTMLDivElement, by: number, why: string) {
    if (Math.abs(by) < 0.5) {
      return;
    }
    let remaining = by;
    if (reserveBox.current !== null) {
      const left = reserve.current - by;
      if (left >= 0) {
        diagnostics?.note(
          `${why} ${String(Math.round(by))} from reserve ${String(Math.round(reserve.current))}->${String(Math.round(left))}`,
        );
        setReserve(left);
        return;
      }
      remaining = -left;
      diagnostics?.note(
        `${why} ${String(Math.round(by))} empties reserve ${String(Math.round(reserve.current))}, moving ${String(Math.round(remaining))}`,
      );
      setReserve(0);
    }
    scrollSelf(element, () => {
      element.scrollTop += remaining;
    });
  }

  // Shows the store's window, at once unless a deleted message's space is closing: rendering
  // the whole list again meanwhile would use its moment up before it is seen.
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
      if (!direct) {
        // A linked message waiting to be shown decides where the view goes, not what was in it.
        const linking = highlightId !== undefined && highlightShown.current !== highlightId;
        anchor.current =
          linking || (stickToBottom.current && latest?.atLatest === true) ? null : captureAnchor();
      }
      diagnostics?.note(
        `commit ${String(shown.window?.ids.length ?? 0)}->${String(latest?.ids.length ?? 0)} first ${shown.window?.ids[0]?.slice(-4) ?? "-"}->${latest?.ids[0]?.slice(-4) ?? "-"} at ${String(scroller.current?.scrollTop ?? 0)} anchor=${anchor.current?.id.slice(-4) ?? "-"}@${String(Math.round(anchor.current?.top ?? 0))}`,
      );
      // Rendered in this same task, so the view noted is the view the change lands in: nothing
      // the reader does can come between them.
      flushSync(() => {
        setShown({ channelId, window: latest });
      });
    };
    timer = setTimeout(commit, 0);
    return () => {
      clearTimeout(timer);
    };
  }, [channelId, latest, shown, highlightId, diagnostics]);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (element === null) {
      return;
    }
    position(element);
    // Positioned when the window's edges, the link, or being at the latest change; what it
    // reads besides is current in the refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [firstId, lastId, highlightId, atLatest]);

  /** Places the view after the window or the link changed: see the effect above. */
  function position(element: HTMLDivElement) {
    if (highlightId === undefined) {
      highlightShown.current = null;
    } else if (highlightShown.current !== highlightId) {
      // A linked message goes to the middle of the view once it is in the window, whatever
      // view was being kept, and the list lets go of the bottom so that nothing arriving or
      // growing later pulls the view away from it.
      const target = element.querySelector(`[data-message-id="${highlightId}"]`);
      if (target !== null) {
        highlightShown.current = highlightId;
        anchor.current = null;
        stickToBottom.current = false;
        scrollSelf(element, () => {
          target.scrollIntoView({ block: "center" });
        });
        scheduleMaintain();
        return;
      }
    }
    const measured = anchor.current;
    if (measured !== null) {
      anchor.current = null;
      const target = element.querySelector(`[data-message-id="${measured.id}"]`);
      if (target !== null) {
        const drift =
          target.getBoundingClientRect().top - element.getBoundingClientRect().top - measured.top;
        absorb(element, drift, "page");
      }
      scheduleMaintain();
      return;
    }
    if (highlightId !== undefined) {
      return;
    }
    if (stickToBottom.current && atLatest) {
      // A channel's first page is pinned to the bottom, with the reserve above it in place from
      // the start, so its first older pages take from it and move nothing.
      if (hasOlder && reserve.current === 0) {
        setReserve(reserveTarget());
      }
      scrollSelf(element, () => {
        element.scrollTop = element.scrollHeight;
      });
    }
  }

  // A different channel starts pinned to the bottom, with no reserve yet and no way known that
  // its reader is going; a linked message's own scroll unpins it.
  useEffect(() => {
    stickToBottom.current = true;
    heading.current = null;
    setReserve(0);
  }, [channelId]);

  /**
   * The moves of the position that keep the reserve able to absorb: refilling it once pages
   * have taken from it, and letting it go once the channel's start has been reached. Made only
   * after `QUIET_MS` with no finger on the list and no scroll event, in one frame, so a pan
   * that overrides them is unlikely to be under way.
   */
  const maintainTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  function scheduleMaintain() {
    clearTimeout(maintainTimer.current);
    maintainTimer.current = setTimeout(maintain, QUIET_MS);
  }
  function maintain() {
    maintainTimer.current = undefined;
    const element = scroller.current;
    if (element === null) {
      return;
    }
    const quietFor = Date.now() - scrolledAt.current;
    if (touching.current || quietFor < QUIET_MS) {
      maintainTimer.current = setTimeout(maintain, Math.max(QUIET_MS - quietFor, 16));
      return;
    }
    const pinned = stickToBottom.current && atLatest;
    if (!hasOlderNow.current) {
      if (reserve.current > 0) {
        const gone = reserve.current;
        diagnostics?.note(`reserve let go ${String(gone)}`);
        setReserve(0);
        scrollSelf(element, () => {
          element.scrollTop = pinned ? element.scrollHeight : element.scrollTop - gone;
        });
      }
      return;
    }
    const target = reserveTarget();
    if (reserve.current < target / 2) {
      const added = target - reserve.current;
      diagnostics?.note(`reserve refilled ${String(reserve.current)}->${String(target)}`);
      setReserve(target);
      scrollSelf(element, () => {
        element.scrollTop = pinned ? element.scrollHeight : element.scrollTop + added;
      });
    }
  }
  useEffect(
    () => () => {
      clearTimeout(maintainTimer.current);
    },
    [channelId],
  );

  // Rows change size without the window changing: pictures and link cards load, reactions come
  // and go, a deleted message's space closes. One wholly above the view takes the change from
  // the reserve, so nothing in view moves; pinned to the bottom, the list stays there. Each
  // row's size is followed from when it appears, so a change is known however the layout that
  // brought it is reached.
  useEffect(() => {
    const element = scroller.current;
    const box = rows.current;
    if (
      element === null ||
      box === null ||
      typeof ResizeObserver === "undefined" ||
      typeof MutationObserver === "undefined"
    ) {
      return;
    }
    const heights = new WeakMap<Element, number>();
    const observer = new ResizeObserver((entries) => {
      const viewTop = element.getBoundingClientRect().top;
      let above = 0;
      for (const entry of entries) {
        const height = entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height;
        const was = heights.get(entry.target);
        heights.set(entry.target, height);
        if (was === undefined || height === was) {
          continue;
        }
        // Where the row's bottom was before the change, which moved it by as much.
        const bottomBefore = entry.target.getBoundingClientRect().bottom - (height - was);
        if (bottomBefore <= viewTop + 1) {
          above += height - was;
        }
      }
      measure();
      if (stickToBottom.current && atLatest) {
        scrollSelf(element, () => {
          element.scrollTop = element.scrollHeight;
        });
        return;
      }
      if (above !== 0) {
        absorb(element, above, "rows above");
      }
    });
    const watch = (node: Node) => {
      if (node instanceof Element) {
        observer.observe(node);
      }
    };
    for (const child of box.children) {
      watch(child);
    }
    const arrivals = new MutationObserver((records) => {
      for (const record of records) {
        record.addedNodes.forEach(watch);
      }
    });
    arrivals.observe(box, { childList: true });
    return () => {
      arrivals.disconnect();
      observer.disconnect();
    };
    // The observers read the rest through refs; made again when being at the latest changes
    // what growth does.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, atLatest]);

  // The list itself shrinks when something takes the screen's space, as the keyboard does when
  // the message box is chosen on a phone. Its bottom edge stays where it was, so what was just
  // above the box, likely what is being answered, stays in view: pinned to the newest message,
  // or moved down by what the list lost. Growing back leaves the view where it is.
  useEffect(() => {
    const element = scroller.current;
    if (element === null || typeof ResizeObserver === "undefined") {
      return;
    }
    let height = element.clientHeight;
    const observer = new ResizeObserver(() => {
      const lost = height - element.clientHeight;
      height = element.clientHeight;
      if (lost <= 0) {
        return;
      }
      scrollSelf(element, () => {
        element.scrollTop =
          stickToBottom.current && atLatest ? element.scrollHeight : element.scrollTop + lost;
      });
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
    // Reads the pin through its ref; made again when being at the latest changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, atLatest]);

  /**
   * Whether a page read at one end leaves the other end's messages that a full window drops
   * well out of view: at least `LOAD_UNSURE_SCREENS` past the view, so nothing the reader can
   * see goes, and they meet the dropped end only after reading their way back towards it.
   */
  function roomFor(way: "older" | "newer"): boolean {
    const element = scroller.current;
    const dropped = (ids?.length ?? 0) + HISTORY_PAGE_SIZE - WINDOW_MAX_MESSAGES;
    if (element === null || ids === undefined || dropped <= 0) {
      return true;
    }
    // The dropped message nearest the view.
    const nearest = way === "older" ? ids[ids.length - dropped] : ids[dropped - 1];
    const row = element.querySelector(`[data-message-id="${nearest ?? ""}"]`);
    if (row === null) {
      return true;
    }
    const view = element.getBoundingClientRect();
    const rect = row.getBoundingClientRect();
    const margin = element.clientHeight * LOAD_UNSURE_SCREENS;
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
    const element = scroller.current;
    if (element === null) {
      return;
    }
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight;
    // The window's top is below the reserve.
    const distanceFromTop = element.scrollTop - reserve.current;
    const toward = heading.current;
    const screens = toward === null ? LOAD_UNSURE_SCREENS : LOAD_AHEAD_SCREENS;
    const near = element.clientHeight * screens;
    if (toward !== "newer" && distanceFromTop < near) {
      loadOlder();
    }
    if (toward !== "older" && distanceFromBottom < near && !jumping.current) {
      loadNewer();
    }
  }

  // A window shorter than the viewport gives no scroll events, so ask once it is rendered.
  useEffect(() => {
    if (loaded) {
      loadNearEnds();
    }
    // The ends are re-checked only when the window's edges move, not on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, firstId, lastId]);

  function onScroll() {
    const element = scroller.current;
    if (element === null) {
      return;
    }
    const own = ownScrollTop.current;
    const ours = own !== null && Math.abs(element.scrollTop - own) < 1;
    diagnostics?.noteScroll(element.scrollTop, element.clientHeight, ours);
    if (ours) {
      ownScrollTop.current = null;
    } else {
      scrolledAt.current = Date.now();
    }
    noteSeenSoon();
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight;
    const byUser = Date.now() - userScrollAt.current < USER_SCROLL_MS;
    // Whether the list is pinned to the bottom follows every scroll but its own: the reader's,
    // and those of find-in-page or a screen reader.
    if (!ours) {
      stickToBottom.current = atLatest && distanceFromBottom < 8;
    }
    loadNearEnds();
    if (highlightId !== undefined && byUser && Date.now() > programmaticUntil.current) {
      void navigate({ ...channelLink(home, channelId), replace: true });
    }
    scheduleMaintain();
  }

  function noteUserScroll() {
    userScrollAt.current = Date.now();
  }

  /** A press on the scroller's own scrollbar, which sits in the gap past its content box. */
  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    if (
      event.target === event.currentTarget &&
      event.nativeEvent.offsetX >= event.currentTarget.clientWidth
    ) {
      noteUserScroll();
      // A scrollbar may be dragged either way.
      heading.current = null;
    }
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (SCROLL_KEYS.has(event.key)) {
      noteUserScroll();
      heading.current = OLDER_KEYS.has(event.key) ? "older" : "newer";
    }
  }

  function onWheel(event: ReactWheelEvent<HTMLDivElement>) {
    noteUserScroll();
    if (event.deltaY !== 0) {
      heading.current = event.deltaY < 0 ? "older" : "newer";
    }
  }

  function onTouchMove(event: ReactTouchEvent<HTMLDivElement>) {
    noteUserScroll();
    const y = event.touches[0]?.clientY;
    if (y === undefined) {
      return;
    }
    // A finger moving down the screen draws older messages into view.
    if (touchY.current !== null && y !== touchY.current) {
      heading.current = y > touchY.current ? "older" : "newer";
    }
    touchY.current = y;
  }

  /** Back to the present: the newest page replaces the window and the view pins to the bottom. */
  function jumpToLatest(): Promise<void> {
    stickToBottom.current = true;
    heading.current = null;
    anchor.current = null;
    jumping.current = true;
    // The list goes to the end of what it holds while the newest are on their way; the next
    // page after it is not read, since the newest replace the window.
    const element = scroller.current;
    if (element !== null) {
      scrollSelf(element, () => {
        element.scrollTop = element.scrollHeight;
      });
    }
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
    const element = scroller.current;
    if (
      element === null ||
      readState === undefined ||
      ids === undefined ||
      document.visibilityState !== "visible" ||
      !document.hasFocus()
    ) {
      return;
    }
    const inWindow = new Set(ids);
    const view = element.getBoundingClientRect();
    let seen: string | undefined;
    for (const article of element.querySelectorAll<HTMLElement>("[data-message-id]")) {
      const id = article.dataset.messageId;
      if (id === undefined || !inWindow.has(id)) {
        continue;
      }
      const rect = article.getBoundingClientRect();
      if (rect.top >= view.bottom) {
        break;
      }
      if (rect.bottom > view.top) {
        seen = id;
      }
    }
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
      ref={scroller}
      onScroll={onScroll}
      onWheel={onWheel}
      onTouchStart={(event) => {
        touching.current = true;
        touchY.current = event.touches[0]?.clientY ?? null;
        diagnostics?.note("touch start");
      }}
      onTouchEnd={() => {
        touching.current = false;
        diagnostics?.note("touch end");
        scheduleMaintain();
      }}
      onTouchCancel={() => {
        touching.current = false;
        diagnostics?.note("touch cancel");
        scheduleMaintain();
      }}
      onTouchMove={onTouchMove}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      // The list keeps its own view still (see above); the browser's anchoring would correct
      // the same changes again.
      className="relative min-h-0 flex-1 overflow-y-auto [overflow-anchor:none]"
    >
      <div className="flex min-h-full flex-col justify-end gap-1 px-4 py-3">
        <div
          ref={reserveBox}
          aria-hidden="true"
          className="shrink-0 overflow-hidden"
          style={{ height: `${String(reserve.current)}px` }}
        >
          <div className="flex h-full flex-col justify-end gap-1">
            {Array.from({ length: RESERVE_ROWS }, (_, index) => (
              <MessageSkeleton key={index} index={index} />
            ))}
          </div>
        </div>
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
      {!window.atLatest && <JumpToLatest onJump={jumpToLatest} />}
      {diagnostics !== null && <ScrollDiagnosticsPanel diagnostics={diagnostics} />}
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
