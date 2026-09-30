import type { MessageWindow } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import {
  Fragment,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
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
/** How close to either end of the window, in pixels, the next page is read. */
const LOAD_MORE_PX = 800;
/** How long after its last scroll event, with no finger down, the list counts as at rest. */
const SETTLE_MS = 150;
/** The ids of a list with no window yet, one array so its identity holds. */
const NO_IDS: readonly string[] = [];
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

  const { departing, measure } = useDeparting(scroller, window?.ids ?? NO_IDS, store);

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
        return { id: article.dataset.messageId ?? "", top: rect.top - origin };
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
        hold.current = { id: highlightId, top };
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
      if (!direct && (touching.current || quietFor < SETTLE_MS)) {
        timer = setTimeout(commit, Math.max(SETTLE_MS - quietFor, 16));
        return;
      }
      if (!direct) {
        // A linked message waiting to be shown decides where the view goes, not what was in it.
        const linking = highlightId !== undefined && highlightShown.current !== highlightId;
        anchor.current =
          linking || (stickToBottom.current && latest?.atLatest === true) ? null : captureAnchor();
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
        const top = target.getBoundingClientRect().top - element.getBoundingClientRect().top;
        scrollSelf(element, () => {
          element.scrollTop += top - restore.top;
        });
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
  // is being read never pushes it away. Browsers' own scroll anchoring would do some of this,
  // but not in Safari, and not always; the list does it itself, with the browser's turned off.
  // While the reader is moving the list, it is theirs to move: a correction then would fight a
  // finger or cut a fling short, and their next scroll holds wherever they leave it.
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

  // A page already read but not yet shown is waiting for the list to settle; reading the next
  // one before it shows would only pile changes up.
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
    if (element.scrollTop < LOAD_MORE_PX) {
      loadOlder();
    }
    if (distanceFromBottom < LOAD_MORE_PX) {
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
    }
  }

  /** Back to the present: the newest page replaces the window and the view pins to the bottom. */
  function jumpToLatest() {
    stickToBottom.current = true;
    anchor.current = null;
    void sync.loadLatest(channelId);
    if (highlightId !== undefined) {
      void navigate({ ...channelLink(home, channelId), replace: true });
    }
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
    (leavingAfter.get(after) ?? []).map((gone) => <DepartingSpace key={gone.id} gone={gone} />);

  return (
    <div
      ref={scroller}
      onScroll={onScroll}
      onWheel={noteUserScroll}
      onTouchStart={() => {
        touching.current = true;
      }}
      onTouchEnd={() => {
        touching.current = false;
      }}
      onTouchCancel={() => {
        touching.current = false;
      }}
      onTouchMove={noteUserScroll}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      className="relative min-h-0 flex-1 overflow-y-auto [overflow-anchor:none]"
    >
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
      {!window.atLatest && (
        <Button
          onPress={jumpToLatest}
          className="motion-rise sticky bottom-3 left-1/2 block w-fit -translate-x-1/2 rounded-full bg-accent px-4 py-1.5 text-sm font-medium text-accent-contrast shadow outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.jumpToLatest}
        </Button>
      )}
    </div>
  );
}

/**
 * The space a deleted message leaves, closing over a moment; the list's `gap-1` between messages
 * closes with it.
 */
function DepartingSpace({ gone }: { gone: Departing }) {
  return (
    <div
      aria-hidden="true"
      className="motion-collapse"
      style={
        { "--from-h": `${String(gone.height)}px`, "--collapse-gap": "-0.25rem" } as CSSProperties
      }
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
