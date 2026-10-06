/**
 * The longest a message's text may be, as the server's limit: counted in characters (Unicode
 * scalar values, so an emoji made of one code point is one) of the text as sent, its tags and
 * custom emoji written out as references.
 */
export const MESSAGE_MAX_CHARS = 10_000;

/** How long a message's text is, counted as `MESSAGE_MAX_CHARS` is. */
export function messageLength(content: string): number {
  return Array.from(content).length;
}
