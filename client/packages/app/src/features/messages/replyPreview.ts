import { maskSpoilers } from "@/features/messages/maskSpoilers";

/** The most characters of a message a reply's line keeps; the line cuts it shorter to fit. */
export const REPLY_PREVIEW_MAX_CHARS = 200;

/** The lines that open and close a fenced code block. */
const FENCE = /^[ \t]*(?:```|~~~).*$/gm;

/**
 * What a reply's line shows of the message it answers, from that message's body: one line,
 * each spoiler as `spoiler` so the line never shows what it hides (`maskSpoilers`, before the
 * line is cut, so no cut falls inside one), the tags and custom emoji of what is left written
 * out by `named`, and code fences left out. Names are written after spoilers are found, since
 * a name may hold what would read as a marker.
 */
export function replyPreview(
  content: string,
  spoiler: string,
  named: (text: string) => string,
): string {
  const line = named(maskSpoilers(content, spoiler)).replace(FENCE, "").replace(/\s+/g, " ").trim();
  const chars = Array.from(line);
  return chars.length > REPLY_PREVIEW_MAX_CHARS
    ? chars.slice(0, REPLY_PREVIEW_MAX_CHARS).join("")
    : line;
}
