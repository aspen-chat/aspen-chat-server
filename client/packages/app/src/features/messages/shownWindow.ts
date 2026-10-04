import type { MessageWindow } from "@aspen/protocol";
import { startTransition, useEffect, useRef, useState } from "react";

/** How often a change of the window waiting for a deleted message's space to close looks again. */
const CLOSING_RETRY_MS = 16;

/**
 * What the message list shows of a channel: the store's window (`latest`), except that a
 * change to it waits while a deleted message's space is `closing`, since rendering the whole
 * list again meanwhile would use its moment up before it is seen. A different channel, or the
 * first page of one, is shown at once. A change is shown as a transition: React renders its
 * rows in slices, between which the finger is heard. `held` says a change is waiting.
 */
export function useShownWindow({
  channelId,
  latest,
  closing,
  onCommit,
}: {
  channelId: string;
  latest: MessageWindow | undefined;
  closing: boolean;
  /** Told of each change about to be shown, from the window shown before. */
  onCommit: (from: MessageWindow | undefined, to: MessageWindow | undefined) => void;
}): { window: MessageWindow | undefined; held: boolean } {
  const [shown, setShown] = useState({ channelId, window: latest });
  const window =
    shown.channelId === channelId && shown.window !== undefined ? shown.window : latest;
  const closingNow = useRef(closing);
  useEffect(() => {
    closingNow.current = closing;
  });

  useEffect(() => {
    if (shown.channelId === channelId && shown.window === latest) {
      return;
    }
    // Shown directly already (see `window`); the state only catches up.
    const direct = shown.channelId !== channelId || shown.window === undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const commit = () => {
      if (!direct && closingNow.current) {
        timer = setTimeout(commit, CLOSING_RETRY_MS);
        return;
      }
      onCommit(shown.window, latest);
      startTransition(() => {
        setShown({ channelId, window: latest });
      });
    };
    timer = setTimeout(commit, 0);
    return () => {
      clearTimeout(timer);
    };
    // `onCommit` only notes the change; a new one each render must not restart the wait.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channelId, latest, shown]);

  return { window, held: window !== latest };
}
