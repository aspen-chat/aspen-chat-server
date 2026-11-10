import type { HeldMessage, ScheduledMessage } from "@aspen/protocol";
import { useEffect, useRef, useSyncExternalStore } from "react";
import type { PickTarget } from "@/features/messages/picks";

/** What a message box offers the rest of the page. */
interface ComposerTarget {
  /** Puts a tag in it, shown as `text` and picked as `target`, and the reader's focus with it. */
  insert: (text: string, target: PickTarget) => void;
  /** Makes what is written there a reply to the message, and brings the reader's focus there. */
  reply: (messageId: string) => void;
  /**
   * Takes a message the server holds until its files arrive back into the box, cancelling it
   * there; rejects, saying why, when the box holds something else or the server refuses.
   */
  takeBack: (waiting: HeldMessage | ScheduledMessage) => Promise<void>;
}

/** The message box open in each channel that someone may post in, by channel. */
const boxes = new Map<string, ComposerTarget>();
/** Whoever draws by whether a channel has a box (`useHasComposer`), told as boxes come and go. */
const watchers = new Set<() => void>();

function changed(): void {
  for (const watcher of watchers) {
    watcher();
  }
}

function watch(watcher: () => void): () => void {
  watchers.add(watcher);
  return () => {
    watchers.delete(watcher);
  };
}

/**
 * Offers the message box of `channelId` to `mentionIn`, `replyIn`, and `takeBackIn`, while it is mounted and
 * `on`: a box the reader may post from, not one starting a thread, which would share its
 * channel's id.
 */
export function useComposerTarget(channelId: string, on: boolean, target: ComposerTarget): void {
  const latest = useRef(target);
  useEffect(() => {
    latest.current = target;
  });
  useEffect(() => {
    if (!on) {
      return;
    }
    const own: ComposerTarget = {
      insert: (text, target) => {
        latest.current.insert(text, target);
      },
      reply: (messageId) => {
        latest.current.reply(messageId);
      },
      takeBack: (waiting) => latest.current.takeBack(waiting),
    };
    boxes.set(channelId, own);
    changed();
    return () => {
      if (boxes.get(channelId) === own) {
        boxes.delete(channelId);
        changed();
      }
    };
  }, [channelId, on]);
}

/** Whether `channelId` has a message box open that a tag can be put in, or a reply written in. */
export function hasComposer(channelId: string): boolean {
  return boxes.has(channelId);
}

/**
 * `hasComposer`, for what is drawn by it: a box registers once it has drawn, after whatever
 * drew beside it asked.
 */
export function useHasComposer(channelId: string): boolean {
  return useSyncExternalStore(watch, () => boxes.has(channelId));
}

/** Puts a tag of `target`, shown as `text`, in the message box of `channelId`. */
export function mentionIn(channelId: string, text: string, target: PickTarget): void {
  boxes.get(channelId)?.insert(text, target);
}

/** Starts a reply to `messageId` in the message box of `channelId`, its channel. */
export function replyIn(channelId: string, messageId: string): void {
  boxes.get(channelId)?.reply(messageId);
}

/**
 * Takes `waiting`, a message of `channelId` the server holds until its files arrive, back into
 * the channel's message box to be changed. Rejects when there is no box, or as the box says.
 */
export function takeBackIn(
  channelId: string,
  waiting: HeldMessage | ScheduledMessage,
): Promise<void> {
  const box = boxes.get(channelId);
  return box === undefined ? Promise.reject(new Error("no message box")) : box.takeBack(waiting);
}
