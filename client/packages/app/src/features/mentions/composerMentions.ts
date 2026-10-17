import { useEffect, useRef } from "react";
import type { PickedTag } from "@/features/mentions/tags";

type Insert = (tag: PickedTag) => void;

/** The message box open in each channel that someone may post in, by channel. */
const boxes = new Map<string, Insert>();

/**
 * Offers the message box of `channelId` to `mentionIn`, while it is mounted and `on`: a box
 * the reader may post from, not one starting a thread, which would share its channel's id.
 */
export function useMentionTarget(channelId: string, on: boolean, insert: Insert): void {
  const latest = useRef(insert);
  useEffect(() => {
    latest.current = insert;
  });
  useEffect(() => {
    if (!on) {
      return;
    }
    const own: Insert = (tag) => {
      latest.current(tag);
    };
    boxes.set(channelId, own);
    return () => {
      if (boxes.get(channelId) === own) {
        boxes.delete(channelId);
      }
    };
  }, [channelId, on]);
}

/** Whether `channelId` has a message box open that a tag can be put in. */
export function canMentionIn(channelId: string): boolean {
  return boxes.has(channelId);
}

/** Puts `tag` in the message box of `channelId`, as a user's Mention does. */
export function mentionIn(channelId: string, tag: PickedTag): void {
  boxes.get(channelId)?.(tag);
}
