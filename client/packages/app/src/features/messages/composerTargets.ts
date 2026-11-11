import { useEffect, useRef } from "react";
import type { PickTarget } from "@/features/messages/picks";

/** What a message box offers the rest of the page. */
interface ComposerTarget {
  /** Puts a tag in it, shown as `text` and picked as `target`, and the reader's focus with it. */
  insert: (text: string, target: PickTarget) => void;
  /** Makes what is written there a reply to the message, and brings the reader's focus there. */
  reply: (messageId: string) => void;
}

/** The message box open in each channel that someone may post in, by channel. */
const boxes = new Map<string, ComposerTarget>();

/**
 * Offers the message box of `channelId` to `mentionIn` and `replyIn`, while it is mounted and
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
    };
    boxes.set(channelId, own);
    return () => {
      if (boxes.get(channelId) === own) {
        boxes.delete(channelId);
      }
    };
  }, [channelId, on]);
}

/** Whether `channelId` has a message box open that a tag can be put in, or a reply written in. */
export function hasComposer(channelId: string): boolean {
  return boxes.has(channelId);
}

/** Puts a tag of `target`, shown as `text`, in the message box of `channelId`. */
export function mentionIn(channelId: string, text: string, target: PickTarget): void {
  boxes.get(channelId)?.insert(text, target);
}

/** Starts a reply to `messageId` in the message box of `channelId`, its channel. */
export function replyIn(channelId: string, messageId: string): void {
  boxes.get(channelId)?.reply(messageId);
}
