import { MESSAGE_SPACING, MESSAGE_TEXT_SIZE } from "@aspen/protocol";
import { useEffect } from "react";
import { usePreference } from "@/api/hooks";

/** The message text size at which messages are drawn as Tailwind draws them, in CSS pixels. */
export const NORMAL_MESSAGE_TEXT_SIZE = 16;

/** How much larger than normal `size` draws messages, which `.message-text` multiplies by. */
export function messageTextScale(size: number): number {
  return size / NORMAL_MESSAGE_TEXT_SIZE;
}

/**
 * Keeps <html> in step with the reader's message text size and line spacing:
 * `--message-text-scale` and `--message-line-scale`, which every type size and line height
 * inside `.message-text` (`styles.css`) multiplies.
 */
export function useFollowMessageTextSize(): void {
  const size = usePreference(MESSAGE_TEXT_SIZE);
  const spacing = usePreference(MESSAGE_SPACING);
  useEffect(() => {
    const root = document.documentElement.style;
    root.setProperty("--message-text-scale", String(messageTextScale(size)));
    root.setProperty("--message-line-scale", String(spacing));
  }, [size, spacing]);
}
