/** The most characters of a message a reply's line keeps; the line cuts it shorter to fit. */
export const REPLY_PREVIEW_MAX_CHARS = 200;

/** Discord's `||text||` and Reddit's `>!text!<`, as `remarkSpoilers` reads them, across lines. */
const SPOILER = /\|\|[\s\S]*?\|\||>![\s\S]*?!</g;
/** The lines that open and close a fenced code block. */
const FENCE = /^[ \t]*(?:```|~~~).*$/gm;

/**
 * What a reply's line shows of the message it answers, from that message's text with its tags
 * and custom emoji already written out as names: one line, each spoiler as `spoiler` so the line
 * never shows what it hides, and code fences left out.
 */
export function replyPreview(text: string, spoiler: string): string {
  const line = text.replace(FENCE, "").replace(SPOILER, spoiler).replace(/\s+/g, " ").trim();
  const chars = Array.from(line);
  return chars.length > REPLY_PREVIEW_MAX_CHARS
    ? chars.slice(0, REPLY_PREVIEW_MAX_CHARS).join("")
    : line;
}
