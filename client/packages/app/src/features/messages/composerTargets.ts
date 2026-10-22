import { useEffect, useRef } from "react";
import type { PickedTag } from "@/features/mentions/tags";

/** What a message box offers the rest of the page. */
interface ComposerTarget {
  /** Puts a tag in it where the reader is writing, and their focus with it. */
  insert: (tag: PickedTag) => void;
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
      insert: (tag) => {
        latest.current.insert(tag);
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

/** Puts `tag` in the message box of `channelId`, as a user's Mention does. */
export function mentionIn(channelId: string, tag: PickedTag): void {
  boxes.get(channelId)?.insert(tag);
}

/** Starts a reply to `messageId` in the message box of `channelId`, its channel. */
export function replyIn(channelId: string, messageId: string): void {
  boxes.get(channelId)?.reply(messageId);
}
