import type { MessageWindow } from "@aspen/protocol";
import { useState } from "react";
import { useReadState, useStore } from "@/api/hooks";

/**
 * Where the "New Messages" line goes in the shown window of a channel: under the message it
 * had been read up to when opened, kept there while it stays open though reading moves the
 * position at once, or nowhere when nothing was unread then. The line goes when the reader
 * leaves or posts. The index is `lineAt`'s.
 */
export function useNewMessagesLine(
  channelId: string,
  latest: MessageWindow | undefined,
  shown: MessageWindow | undefined,
): number | null {
  const store = useStore();
  const readState = useReadState(channelId);
  // Set once per channel, when its read state is first known.
  const [line, setLine] = useState<{ channelId: string; after: string | null } | null>(null);
  if (readState !== undefined && line?.channelId !== channelId) {
    const unread = readState.lastMessage != null && readState.lastMessage > readState.lastRead;
    setLine({ channelId, after: unread ? readState.lastRead : null });
  }
  const after = line?.channelId === channelId ? line.after : null;
  // Posting ends the line: what came before the reader's own message is read.
  const newest = latest?.ids[latest.ids.length - 1];
  if (
    after !== null &&
    newest !== undefined &&
    newest > after &&
    store.message(newest)?.author === store.myUserId
  ) {
    setLine({ channelId, after: null });
  }
  return shown === undefined ? null : lineAt(shown, after);
}

/**
 * The index of the message the "New Messages" line goes under, the last one at or before
 * `after`; `-1` for the top of the channel, when everything in it is new; `null` for no line in
 * this window, when nothing after `after` is loaded, or the line belongs above what is loaded.
 */
export function lineAt(window: MessageWindow, after: string | null): number | null {
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
