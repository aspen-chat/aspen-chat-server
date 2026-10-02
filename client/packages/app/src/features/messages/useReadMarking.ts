import type { AspenSync, MessageWindow, ReadState } from "@aspen/protocol";
import { useEffect, useRef, type RefObject } from "react";
import { rowsInView } from "@/features/messages/rowsInView";

/**
 * How the message list marks what the reader has seen as read: `noteSeenSoon` marks the newest
 * message on screen, at most once a frame, and `seenFrame` is the frame it waits for.
 */
export function useReadMarking({
  viewport,
  readState,
  ids,
  sync,
  channelId,
}: {
  viewport: RefObject<HTMLDivElement | null>;
  readState: ReadState | undefined;
  ids: MessageWindow["ids"] | undefined;
  sync: AspenSync;
  channelId: string;
}) {
  const seenFrame = useRef<number | null>(null);

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

  return { seenFrame, noteSeenSoon };
}

/**
 * Reports what the message list has seen: when the page is looked at again, when it is hidden,
 * and when the reader leaves the channel.
 */
export function useReadReports({
  seenFrame,
  noteSeenSoon,
  sync,
  channelId,
}: {
  seenFrame: RefObject<number | null>;
  noteSeenSoon: () => void;
  sync: AspenSync;
  channelId: string;
}) {
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
    // `seenFrame` is a ref, which the rule cannot tell from a parameter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sync, channelId],
  );
}
