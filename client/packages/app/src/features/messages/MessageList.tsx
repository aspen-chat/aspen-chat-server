import { useNavigate } from "@tanstack/react-router";
import { Fragment, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  useBlockedUsers,
  useChannel,
  useMessageWindow,
  useReadState,
  useStore,
  useSync,
} from "@/api/hooks";
import { useAnnounceArrivals } from "@/features/messages/announceArrivals";
import { windowParts } from "@/features/messages/blocked";
import { BlockedRun, NewMessagesLine } from "@/features/messages/BlockedRun";
import { DepartingSpace } from "@/features/messages/DepartingSpace";
import { useDeparting, type Departing } from "@/features/messages/departing";
import { useHistoryPaging } from "@/features/messages/historyPaging";
import { JumpToLatest } from "@/features/messages/JumpToLatest";
import { MessageRows, MessageRowsContext, moveBetweenRows } from "@/features/messages/messageRows";
import { KeepStillContext } from "@/features/messages/keepStill";
import { channelLink, type ChannelHome } from "@/features/messages/links";
import { OWNS_SCROLLING, useListPosition, useListScroller } from "@/features/messages/listScroller";
import { MessageItem } from "@/features/messages/MessageItem";
import { HistorySkeleton } from "@/features/messages/MessageSkeleton";
import { useNewMessagesLine } from "@/features/messages/newMessagesLine";
import { ScrollDiagnosticsPanel } from "@/features/messages/ScrollDiagnosticsPanel";
import { useShownWindow } from "@/features/messages/shownWindow";
import { useGroupContinuations } from "@/features/messages/useMessageGroups";
import { useReadMarking, useReadReports } from "@/features/messages/useReadMarking";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/**
 * The loaded window of a channel, oldest at the top. On iOS and iPadOS the list scrolls
 * itself (`OWNS_SCROLLING`): its box hides its overflow, so no finger, wheel, or key scrolls
 * it, and the list takes those itself and sets the box's scroll position from them, with its
 * own coasting, spring, and indicator (`ListScroller`, with `scrollPhysics.ts` for the
 * arithmetic). A box the
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
 * are heading (`useHistoryPaging`), the store keeps the window bounded, and the jump control
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
 * reader leaves or posts (`useNewMessagesLine`).
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
 * Nothing else in the client should assume either way; the list's box and its `ListScroller`
 * are the only places that know.
 */

/** The ids of a list with no window yet, one array so its identity holds. */
const NO_IDS: readonly string[] = [];

export function MessageList({
  channelId,
  home,
  highlightId,
  start,
}: {
  channelId: string;
  home: ChannelHome;
  highlightId: string | undefined;
  /**
   * What stands at the beginning of the history, in place of the line saying it is the
   * beginning, once the window reaches it: a thread's starter, which scrolls with its replies.
   */
  start?: ReactNode;
}) {
  const m = useMessages();
  const sync = useSync();
  const store = useStore();
  const navigate = useNavigate();
  /** The box the list is seen through, whose scroll position the list sets. */
  const viewport = useRef<HTMLDivElement>(null);
  /** Everything the list holds; drawn past an end, while a finger pulls it there, by a transform. */
  const content = useRef<HTMLDivElement>(null);
  const indicator = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLDivElement>(null);
  const scroller = useListScroller(channelId, { viewport, content, indicator, thumb });
  /** The rows: messages, blocked runs, the new-messages line, and spaces closing. */
  const rows = useRef<HTMLDivElement>(null);
  /** What stands at the beginning of the history, while it is shown. */
  const startBox = useRef<HTMLDivElement>(null);
  /**
   * Whether the view is at least a screen above the newest the list holds, which offers the
   * Jump to latest pill even with the newest messages in the window.
   */
  const [farBack, setFarBack] = useState(false);
  const channel = useChannel(channelId);
  // A thread's messages cannot start threads, and link to the thread rather than to a place in
  // a channel's history.
  const parentId = channel?.ty === "thread" ? (channel.parentChannel ?? null) : null;

  const latest = useMessageWindow(channelId);
  // The store's window, not the one shown: a deleted message's row empties as the store drops
  // it, before the shown window catches up, and its space must be there in that same frame.
  const { departing, measure, forget } = useDeparting(viewport, latest?.ids ?? NO_IDS, store);
  const { window, held } = useShownWindow({
    channelId,
    latest,
    closing: departing.length > 0,
    onCommit: scroller.noteCommit,
  });
  const ids = window?.ids;
  const lastId = ids?.[ids.length - 1];
  const atLatest = window?.atLatest ?? true;
  const startShown = window !== undefined && !window.hasOlder && start !== undefined;

  const readState = useReadState(channelId);
  const { seenFrame, noteSeenSoon } = useReadMarking({
    viewport,
    readState,
    ids,
    sync,
    channelId,
  });
  const { loadingOlder, loadingNewer, loadNearEnds, jumpToLatest } = useHistoryPaging({
    scroller,
    sync,
    channelId,
    window,
    held,
  });
  /** Drops a linked message from the URL, once the reader has moved on from it. */
  function dropLink() {
    if (highlightId !== undefined) {
      void navigate({ ...channelLink(home, channelId), replace: true });
    }
  }
  useListPosition(scroller, {
    loaded: window !== undefined,
    atLatest,
    highlightId,
    firstId: ids?.[0],
    lastId,
    events: {
      moved: () => {
        noteSeenSoon();
        loadNearEnds();
      },
      steered: dropLink,
      farBack: setFarBack,
    },
    rows,
    startBox,
    startShown,
    measured: measure,
  });
  useReadReports({ seenFrame, noteSeenSoon, sync, channelId });
  useAnnounceArrivals(sync, window?.ids);
  const [messageRows] = useState(() => new MessageRows());
  // Every render may draw or drop rows, the one the tab order stops at among them.
  useLayoutEffect(() => {
    if (content.current !== null) {
      messageRows.settle(content.current);
    }
  });

  const blockedUsers = useBlockedUsers();
  const parts = useMemo(() => {
    const blocked = new Set(blockedUsers);
    return windowParts(window?.ids ?? [], (id) => {
      const author = store.message(id)?.author;
      return author !== undefined && blocked.has(author);
    });
  }, [window, blockedUsers, store]);

  const lineIndex = useNewMessagesLine(channelId, latest, window);
  const continuing = useGroupContinuations(
    parts,
    lineIndex,
    store,
    viewport,
    content,
    window !== undefined,
  );

  if (window === undefined) {
    return <HistorySkeleton />;
  }

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
      onScroll={scroller.onScroll}
      {...(OWNS_SCROLLING
        ? {
            onTouchStart: scroller.onTouchStart,
            onTouchMove: scroller.onTouchMove,
            onTouchEnd: scroller.onTouchEnd,
            onTouchCancel: scroller.onTouchCancel,
            onWheel: scroller.onWheel,
            onKeyDown: scroller.onKeyDown,
          }
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
      <KeepStillContext.Provider value={scroller.settle}>
        <div
          ref={content}
          onKeyDown={(event) => {
            const heading = moveBetweenRows(event.currentTarget, event);
            if (heading !== null) {
              scroller.focusMoved(heading);
            }
          }}
          className="message-text flex min-h-full flex-col justify-end gap-1 px-4 py-3"
        >
          <MessageRowsContext.Provider value={messageRows}>
            {window.hasOlder ? (
              <LoadingEdge loading={loadingOlder} />
            ) : start !== undefined ? (
              <div ref={startBox}>{start}</div>
            ) : (
              <p className="py-2 text-center text-sm text-ink-faint">{m.channelStart}</p>
            )}
            <div ref={rows} className="flex flex-col gap-1">
              {lineIndex === -1 && <NewMessagesLine />}
              {leaving(null)}
              {parts.map((part) => {
                const item = (id: string, next?: string) => (
                  <MessageItem
                    id={id}
                    home={home}
                    channelId={channelId}
                    parentId={parentId}
                    highlighted={id === highlightId}
                    grouped={continuing.has(id)}
                    continued={next !== undefined && continuing.has(next)}
                  />
                );
                if (part.kind === "message") {
                  const next = ids?.[part.index + 1];
                  return (
                    <Fragment key={part.id}>
                      {item(part.id, next)}
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
            {!window.atLatest && <LoadingEdge loading={loadingNewer} />}
          </MessageRowsContext.Provider>
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
            onPointerDown={scroller.onThumbPointerDown}
            className="absolute inset-x-0 top-0 rounded-full bg-ink/35 pointer-fine:pointer-events-auto"
          />
        </div>
      )}
      {(!window.atLatest || farBack) && (
        // Over the bottom of the view, taking no room in the list: the pill comes and goes as
        // the reader nears the newest, and room of its own going at the bottom would move
        // everything in view down by its height.
        <div className="pointer-events-none sticky bottom-3 flex h-0 items-end justify-center">
          <JumpToLatest
            onJump={() => {
              const jumped = jumpToLatest();
              dropLink();
              return jumped;
            }}
          />
        </div>
      )}
      {scroller.diagnostics !== null && (
        <ScrollDiagnosticsPanel diagnostics={scroller.diagnostics} viewport={viewport} />
      )}
    </div>
  );
}

/**
 * Where a page is read at an end of the window that has more. Its height does not change with
 * its text: the text comes and goes above or below what is being read, and a line appearing
 * there would move the view.
 */
function LoadingEdge({ loading }: { loading: boolean }) {
  return (
    <p aria-live="polite" className="flex min-h-9 items-center justify-center py-2">
      {loading && (
        <>
          <LoadingLabel />
          <Skeleton className="h-3 w-24" />
        </>
      )}
    </p>
  );
}
