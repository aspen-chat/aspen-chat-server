import { useNavigate } from "@tanstack/react-router";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { Button } from "react-aria-components";
import { useChannel, useMessageWindow, useSync } from "@/api/hooks";
import { channelLink, type ChannelHome } from "@/features/messages/links";
import { MessageItem } from "@/features/messages/MessageItem";
import { useMessages } from "@/i18n/context";

/**
 * The loaded window of a channel, oldest at the top. Stays pinned to the bottom while the user
 * is there and keeps the message under the viewport still when the window changes around it.
 * Nearing the top reads the previous page of history; nearing the bottom of a window that is
 * not at the latest reads the next one. The store keeps the window bounded, so a long scroll
 * drops what is far from the viewport, and the jump control returns to the present.
 *
 * A linked message is scrolled into view once. The first scroll the user makes afterwards drops
 * the message from the URL, so the link is shareable but does not keep pulling the view back.
 */
const PROGRAMMATIC_SCROLL_MS = 200;
/** How long after a wheel, touch, scrollbar press, or key a scroll event still counts as the reader's. */
const USER_SCROLL_MS = 500;
/** How close to either end of the window, in pixels, the next page is read. */
const LOAD_MORE_PX = 800;
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
  const window = useMessageWindow(channelId);
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
   * When the reader last acted on the list: a wheel or touch move, a press on its scrollbar, or a
   * navigation key. A scroll event is theirs only if it follows one of these closely; the browser
   * also fires scroll events when a modal opening changes the layout, when an image loads, or
   * when a script scrolls a message into view, and none of those mean the reader moved on.
   */
  const userScrollAt = useRef(0);

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

  useLayoutEffect(() => {
    const element = scroller.current;
    if (element === null) {
      return;
    }
    const restore = anchor.current;
    if (restore !== null) {
      anchor.current = null;
      const target = element.querySelector(`[data-message-id="${restore.id}"]`);
      if (target !== null) {
        const top = target.getBoundingClientRect().top - element.getBoundingClientRect().top;
        programmaticUntil.current = Date.now() + PROGRAMMATIC_SCROLL_MS;
        element.scrollTop += top - restore.top;
      }
      return;
    }
    if (highlightId === undefined) {
      highlightShown.current = null;
    } else {
      if (highlightShown.current !== highlightId) {
        const target = element.querySelector(`[data-message-id="${highlightId}"]`);
        if (target !== null) {
          highlightShown.current = highlightId;
          programmaticUntil.current = Date.now() + PROGRAMMATIC_SCROLL_MS;
          target.scrollIntoView({ block: "center" });
        }
      }
      return;
    }
    if (stickToBottom.current && atLatest) {
      programmaticUntil.current = Date.now() + PROGRAMMATIC_SCROLL_MS;
      element.scrollTop = element.scrollHeight;
    }
  }, [firstId, lastId, highlightId, atLatest]);

  // A different channel starts pinned to the bottom; a linked message's own scroll unpins it.
  useEffect(() => {
    stickToBottom.current = true;
  }, [channelId]);

  // Content can grow without the window changing, for example when a link preview card arrives
  // for the newest message. Stay pinned through that too.
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
      if (stickToBottom.current && atLatest) {
        programmaticUntil.current = Date.now() + PROGRAMMATIC_SCROLL_MS;
        element.scrollTop = element.scrollHeight;
      }
    });
    observer.observe(content);
    return () => {
      observer.disconnect();
    };
  }, [loaded, atLatest]);

  function loadOlder() {
    if (loadingOlder || !hasOlder) {
      return;
    }
    anchor.current = captureAnchor();
    setLoadingOlder(true);
    sync
      .loadOlder(channelId)
      .catch(() => {
        anchor.current = null;
      })
      .finally(() => {
        setLoadingOlder(false);
      });
  }

  function loadNewer() {
    if (loadingNewer || atLatest) {
      return;
    }
    anchor.current = captureAnchor();
    setLoadingNewer(true);
    sync
      .loadNewer(channelId)
      .catch(() => {
        anchor.current = null;
      })
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
    const element = scroller.current;
    if (element === null) {
      return;
    }
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight;
    stickToBottom.current = atLatest && distanceFromBottom < 8;
    loadNearEnds();
    const byUser = Date.now() - userScrollAt.current < USER_SCROLL_MS;
    if (highlightId !== undefined && byUser && Date.now() > programmaticUntil.current) {
      void navigate({ ...channelLink(home, channelId), replace: true });
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

  if (window === undefined) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-muted">{m.loading}</div>
    );
  }

  return (
    <div
      ref={scroller}
      onScroll={onScroll}
      onWheel={noteUserScroll}
      onTouchMove={noteUserScroll}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      className="relative min-h-0 flex-1 overflow-y-auto"
    >
      <div className="flex min-h-full flex-col justify-end gap-1 px-4 py-3">
        {window.hasOlder ? (
          <p aria-live="polite" className="py-2 text-center text-sm text-ink-faint">
            {loadingOlder ? m.loading : ""}
          </p>
        ) : (
          <p className="py-2 text-center text-sm text-ink-faint">{m.channelStart}</p>
        )}
        {window.ids.map((id) => (
          <MessageItem
            key={id}
            id={id}
            home={home}
            channelId={channelId}
            parentId={parentId}
            highlighted={id === highlightId}
          />
        ))}
        {!window.atLatest && (
          <p aria-live="polite" className="py-2 text-center text-sm text-ink-faint">
            {loadingNewer ? m.loading : ""}
          </p>
        )}
      </div>
      {!window.atLatest && (
        <Button
          onPress={jumpToLatest}
          className="sticky bottom-3 left-1/2 block w-fit -translate-x-1/2 rounded-full bg-accent px-4 py-1.5 text-sm font-medium text-accent-contrast shadow outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.jumpToLatest}
        </Button>
      )}
    </div>
  );
}
