import {
  HISTORY_PAGE_SIZE,
  WINDOW_MAX_MESSAGES,
  type AspenSync,
  type MessageWindow,
} from "@aspen/protocol";
import { useRef, useState } from "react";
import type { Heading, ListScroller } from "@/features/messages/listScroller";

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
 * Reads the message list's history a page at a time, well ahead of the reader in the way they
 * are heading (`loadNearEnds`, whenever the view moves or is placed afresh, so a window shorter
 * than the viewport, which moves nothing, is read on as soon as it is rendered), and
 * brings it back to the present (`jumpToLatest`). A page already read but not yet shown
 * (`held`) is waiting for the next render; reading the next one before it shows would only
 * pile changes up.
 */
export function useHistoryPaging({
  scroller,
  sync,
  channelId,
  window,
  held,
}: {
  scroller: ListScroller;
  sync: AspenSync;
  channelId: string;
  window: MessageWindow | undefined;
  held: boolean;
}) {
  const [loading, setLoading] = useState<Record<Heading, boolean>>({
    older: false,
    newer: false,
  });
  /** Whether the newest page is being read for "Jump to latest". */
  const jumping = useRef(false);
  const ids = window?.ids;
  const more: Record<Heading, boolean> = {
    older: window?.hasOlder ?? false,
    newer: !(window?.atLatest ?? true),
  };

  /**
   * Whether a page read at one end leaves the other end's messages that a full window drops
   * well out of view: at least `LOAD_UNSURE_SCREENS` past the view, so nothing the reader can
   * see goes, and they meet the dropped end only after reading their way back towards it.
   */
  function roomFor(way: Heading): boolean {
    const box = scroller.viewport.current;
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

  function load(way: Heading) {
    if (loading[way] || !more[way] || held || !roomFor(way)) {
      return;
    }
    setLoading((now) => ({ ...now, [way]: true }));
    (way === "older" ? sync.loadOlder(channelId) : sync.loadNewer(channelId))
      .catch(() => undefined)
      .finally(() => {
        setLoading((now) => ({ ...now, [way]: false }));
      });
  }

  /** Reads the next page when the viewport is near an end of the window that has more. */
  function loadNearEnds() {
    const box = scroller.viewport.current;
    if (box === null) {
      return;
    }
    const { min, max } = scroller.range;
    const at = scroller.current();
    const toward = scroller.heading;
    const near = box.clientHeight * (toward === null ? LOAD_UNSURE_SCREENS : LOAD_AHEAD_SCREENS);
    if (toward !== "newer" && at - min < near) {
      load("older");
    }
    // While the newest are on their way for a jump, they replace the window: the next page
    // after it is not read.
    if (toward !== "older" && max - at < near && !jumping.current) {
      load("newer");
    }
  }

  /** Back to the present: the newest page replaces the window and the view pins to the bottom. */
  function jumpToLatest(): Promise<void> {
    jumping.current = true;
    // The list goes to the end of what it holds while the newest are on their way.
    scroller.jumpToEnd();
    return sync
      .loadLatest(channelId)
      .catch(() => undefined)
      .finally(() => {
        jumping.current = false;
      });
  }

  return { loadingOlder: loading.older, loadingNewer: loading.newer, loadNearEnds, jumpToLatest };
}
