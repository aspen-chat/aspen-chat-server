import type { MessageWindow } from "@aspen/protocol";
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
import { Stillness, StillnessContext } from "@/features/messages/stillness";
import { HistorySkeleton } from "@/features/messages/MessageSkeleton";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/**
 * The loaded window of a channel, oldest at the top. Stays pinned to the bottom while the user
 * is there and keeps the message under the viewport still when the window changes around it,
 * and when content changes size under it, as pictures and link cards load: the list holds its
 * view itself (`hold`), since Safari has no scroll anchoring and other browsers' is unreliable
 * across a jump, and turns the browser's off so the two never fight.
 * Keeping it still takes a scroll correction, which is made against where the view is when the
 * change is shown, and only while the list is at rest: iOS Safari has no scroll anchoring of its
 * own, and a correction made while a finger drags the list or it coasts afterwards is lost or
 * fought, so a change that arrives meanwhile waits until the list settles.
 * Nearing the top reads the previous page of history; nearing the bottom of a window that is
 * not at the latest reads the next one. The store keeps the window bounded, so a long scroll
 * drops what is far from the viewport, and the jump control returns to the present.
 *
 * A linked message is scrolled to the middle of the view once, as soon as it is in the window,
 * even when a window around it had to be read first; keeping the old view still gives way to it,
 * and so does staying at the bottom. The first scroll the user makes afterwards drops
 * the message from the URL, so the link is shareable but does not keep pulling the view back.
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
 * How close to the other end, or to either while which way the reader is going is unknown, the
 * next page is read. Reading far that way too would drop what the reader is heading into, in a
 * window of short messages, for a page they are leaving behind.
 */
const LOAD_BEHIND_SCREENS = 2;
/** How long after its last scroll event, with no finger down, the list counts as at rest. */
const SETTLE_MS = 150;

/**
 * Whether the browser keeps the view still itself when content above it grows
 * (`overflow-anchor`), found once by trying it, since an engine may know the property without
 * doing it.
 */
let browserAnchors: boolean | undefined;

function tryAnchoring(): boolean {
  const box = document.createElement("div");
  box.style.cssText =
    "position:fixed;left:0;top:0;width:10px;height:100px;overflow:auto;opacity:0;pointer-events:none";
  const above = document.createElement("div");
  above.style.height = "10px";
  const rest = document.createElement("div");
  rest.style.height = "1000px";
  box.append(above, rest);
  document.body.append(box);
  box.scrollTop = 500;
  // Laid out at that position before the content above it grows.
  box.getBoundingClientRect();
  above.style.height = "110px";
  const anchored = box.scrollTop === 600;
  box.remove();
  return anchored;
}

/** Whether the browser keeps this list's view still itself (see `browserAnchors`). */
function anchors(element: HTMLElement | null): boolean {
  if (element === null || getComputedStyle(element).overflowAnchor !== "auto") {
    return false;
  }
  browserAnchors ??= tryAnchoring();
  return browserAnchors;
}
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

/** A message at the top of the viewport and where it sat, so it can be put back after a change. */
interface Anchor {
  id: string;
  top: number;
  /** The scroller's position then, by which to tell how far the reader moved it since. */
  scrollTop: number;
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
  // What is on screen: the store's window, except that a change to it waits while the list
  // moves. A different channel, or the first page of one, is shown at once.
  const [shown, setShown] = useState({ channelId, window: latest });
  const window =
    shown.channelId === channelId && shown.window !== undefined ? shown.window : latest;
  const held = window !== latest;
  const channel = useChannel(channelId);
  // A thread's messages cannot start threads, and link to the thread rather than to a place in
  // a channel's history.
  const parentId = channel?.ty === "thread" ? (channel.parentChannel ?? null) : null;
  const scroller = useRef<HTMLDivElement>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [loadingNewer, setLoadingNewer] = useState(false);
  /** Whether the newest page is being read for "Jump to latest". */
  const jumping = useRef(false);
  /** Where the viewport was before a page was read, restored once the window has changed. */
  const anchor = useRef<Anchor | null>(null);
  const stickToBottom = useRef(true);
  /** The highlighted message already scrolled to, so it is done once per link. */
  const highlightShown = useRef<string | null>(null);
  /** Scroll events before this time were caused by this component, not the user. */
  const programmaticUntil = useRef(0);
  /**
   * Where the list last scrolled itself to. The scroll event that lands there is its own, and
   * any other is someone else's: the reader's, find-in-page's, a screen reader's. It is told by
   * where the view lands rather than by when, because the list scrolls itself again and again
   * while content loads, and a reader's scroll in the midst of that is still theirs.
   */
  const ownScrollTop = useRef<number | null>(null);
  function scrollSelf(element: HTMLDivElement, move: () => void) {
    programmaticUntil.current = Date.now() + PROGRAMMATIC_SCROLL_MS;
    const before = element.scrollTop;
    move();
    ownScrollTop.current = element.scrollTop === before ? null : element.scrollTop;
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
  /** The list's rest, which what changes size by itself waits for (`stillness.ts`). */
  const [stillness] = useState(() => new Stillness());
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
        return {
          id: article.dataset.messageId ?? "",
          top: rect.top - origin,
          scrollTop: element.scrollTop,
        };
      }
    }
    return null;
  }

  /**
   * Where the view is, held still whenever content changes size under it: the linked message
   * while it is being shown, or else the topmost message in view, at its offset from the top.
   * Nothing is held while the list is pinned to the bottom, which the bottom holds instead.
   */
  const hold = useRef<Anchor | null>(null);
  function captureHold() {
    const element = scroller.current;
    if (element === null || (stickToBottom.current && atLatest)) {
      hold.current = null;
      return;
    }
    if (highlightId !== undefined && highlightShown.current === highlightId) {
      const target = element.querySelector(`[data-message-id="${highlightId}"]`);
      if (target !== null) {
        const top = target.getBoundingClientRect().top - element.getBoundingClientRect().top;
        hold.current = { id: highlightId, top, scrollTop: element.scrollTop };
        return;
      }
    }
    hold.current = captureAnchor();
  }

  // Shows the store's window once the list is at rest, noting where the view was so the layout
  // effect below can keep it there.
  useEffect(() => {
    if (shown.channelId === channelId && shown.window === latest) {
      return;
    }
    // Shown directly already (see `window`); the state only catches up.
    const direct = shown.channelId !== channelId || shown.window === undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const commit = () => {
      const quietFor = Date.now() - scrolledAt.current;
      // A deleted message's space closing waits for nothing: rendering the whole list again
      // meanwhile would use its moment up before it is seen.
      // Where the browser anchors scrolling, it holds the view through the change, so the page
      // shows at once even mid-fling and nothing is noted to put back; elsewhere it waits for
      // the list to rest. No browser anchors a view at the very top, which a reader who
      // outran the page is at, so there the list puts it back itself.
      const element = scroller.current;
      const anchored = anchors(element) && element !== null && element.scrollTop > 0;
      const moving = !anchored && (touching.current || quietFor < SETTLE_MS);
      if (!direct && (moving || closing.current)) {
        timer = setTimeout(commit, Math.max(SETTLE_MS - quietFor, 16));
        return;
      }
      if (!direct) {
        // A linked message waiting to be shown decides where the view goes, not what was in it.
        const linking = highlightId !== undefined && highlightShown.current !== highlightId;
        anchor.current =
          linking || anchored || (stickToBottom.current && latest?.atLatest === true)
            ? null
            : captureAnchor();
      }
      setShown({ channelId, window: latest });
    };
    timer = setTimeout(commit, 0);
    return () => {
      clearTimeout(timer);
    };
  }, [channelId, latest, shown, highlightId]);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (element === null) {
      return;
    }
    position(element);
    captureHold();
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
        return;
      }
    }
    const restore = anchor.current;
    if (restore !== null) {
      anchor.current = null;
      const target = element.querySelector(`[data-message-id="${restore.id}"]`);
      if (target !== null) {
        // The reader may have moved the list while the change rendered, a finger landing just
        // as a page shows; what they moved is theirs to keep, and only the change is undone.
        // Assigning the position even unchanged would cut a running fling short.
        const moved = element.scrollTop - restore.scrollTop;
        const drift =
          target.getBoundingClientRect().top -
          element.getBoundingClientRect().top -
          (restore.top - moved);
        if (Math.abs(drift) >= 1) {
          scrollSelf(element, () => {
            element.scrollTop += drift;
          });
        }
      }
      return;
    }
    if (highlightId !== undefined) {
      return;
    }
    if (stickToBottom.current && atLatest) {
      scrollSelf(element, () => {
        element.scrollTop = element.scrollHeight;
      });
    }
  }

  // A different channel starts pinned to the bottom; a linked message's own scroll unpins it.
  useEffect(() => {
    stickToBottom.current = true;
  }, [channelId]);

  // Content changes size without the window changing: pictures and link cards load, reactions
  // come and go. The view stays where it is through it: pinned to the bottom there, and
  // otherwise with the held message (see `hold`) where it was, so a picture loading above what
  // is being read never pushes it away. Where the browser anchors scrolling itself
  // (`overflow-anchor`, everywhere but Safari), it holds the view through such changes even
  // while a finger drags or a fling runs, and this finds nothing left to correct. Where it does
  // not, the list corrects only at rest: while the reader is moving the list, a correction
  // would fight a finger or cut a fling short, and their next scroll holds wherever they leave
  // it. `e2e/historyScroll.spec.ts` drags back through a long history to check it.
  useEffect(() => {
    const element = scroller.current;
    if (element === null || typeof ResizeObserver === "undefined") {
      return;
    }
    const content = element.firstElementChild;
    if (content === null) {
      return;
    }
    const observer = new ResizeObserver(() => {
      measure();
      if (stickToBottom.current && atLatest) {
        scrollSelf(element, () => {
          element.scrollTop = element.scrollHeight;
        });
        return;
      }
      const held = hold.current;
      const moving = touching.current || Date.now() - scrolledAt.current < SETTLE_MS;
      if (held === null || moving) {
        return;
      }
      const target = element.querySelector(`[data-message-id="${held.id}"]`);
      if (target === null) {
        captureHold();
        return;
      }
      const drift =
        target.getBoundingClientRect().top - element.getBoundingClientRect().top - held.top;
      if (Math.abs(drift) >= 1) {
        scrollSelf(element, () => {
          element.scrollTop += drift;
        });
      }
    });
    observer.observe(content);
    return () => {
      observer.disconnect();
    };
    // The observer reads the held position and the rest through refs; it is made again when
    // being at the latest, or the link, changes what it holds.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, atLatest, highlightId]);

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
      if (!(stickToBottom.current && atLatest)) {
        captureHold();
      }
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
    // Reads the pin through its ref; made again when being at the latest changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, atLatest]);

  // A page already read but not yet shown is waiting for the list to settle, or for the next
  // render; reading the next one before it shows would only pile changes up.
  function loadOlder() {
    if (loadingOlder || !hasOlder || held) {
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
    if (loadingNewer || atLatest || held) {
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
    const screens = (way: "older" | "newer") =>
      element.clientHeight * (heading.current === way ? LOAD_AHEAD_SCREENS : LOAD_BEHIND_SCREENS);
    if (element.scrollTop < screens("older")) {
      loadOlder();
    }
    if (distanceFromBottom < screens("newer") && !jumping.current) {
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
    // The list's own scrolls are not movement, and do not move what it holds: one that
    // corrects for a picture loading may be read only after the next picture has grown, and
    // holding the view where it then is would keep that picture's push.
    const element = scroller.current;
    if (element === null) {
      return;
    }
    const own = ownScrollTop.current;
    const ours = own !== null && Math.abs(element.scrollTop - own) < 1;
    if (ours) {
      ownScrollTop.current = null;
    } else {
      scrolledAt.current = Date.now();
      stillness.noteScroll();
    }
    noteSeenSoon();
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight;
    const byUser = Date.now() - userScrollAt.current < USER_SCROLL_MS;
    // Whether the list is pinned to the bottom follows every scroll but its own: the reader's,
    // and those of find-in-page or a screen reader. Content loading moves nothing by itself
    // with the browser's anchoring off, so it never unpins the list.
    if (!ours) {
      stickToBottom.current = atLatest && distanceFromBottom < 8;
    }
    loadNearEnds();
    if (highlightId !== undefined && byUser && Date.now() > programmaticUntil.current) {
      void navigate({ ...channelLink(home, channelId), replace: true });
    }
    if (!ours) {
      captureHold();
    }
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
        stillness.setTouching(true);
      }}
      onTouchEnd={() => {
        touching.current = false;
        stillness.setTouching(false);
      }}
      onTouchCancel={() => {
        touching.current = false;
        stillness.setTouching(false);
      }}
      onTouchMove={onTouchMove}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      className="relative min-h-0 flex-1 overflow-y-auto"
    >
      <StillnessContext.Provider value={stillness}>
        <div className="flex min-h-full flex-col justify-end gap-1 px-4 py-3">
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
      </StillnessContext.Provider>
      {!window.atLatest && <JumpToLatest onJump={jumpToLatest} />}
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
