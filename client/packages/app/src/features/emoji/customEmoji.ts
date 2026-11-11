import type { CustomEmoji } from "@aspen/protocol";

/**
 * How a community's own emoji are written where the server stores them: `<:id>`, the id
 * alone, in a message's text and as a reaction's key (`app::custom_emoji::reference`). A
 * reader resolves the id from the community's list, so a rename changes no message, and the
 * message box shows one as `:name:`: one picked from a list is remembered by its place in the
 * text (`picks.ts`), and one typed by hand is looked up by its name as the message is sent.
 */

const UUID = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
/** A reference, its id captured, as a pattern's source. */
export const EMOJI_REFERENCE_SOURCE = `<:(${UUID})>`;
/** A reference in text, its id captured. */
export const EMOJI_REFERENCE = new RegExp(EMOJI_REFERENCE_SOURCE, "g");
/**
 * A `:name:` the message box writes, its name captured: a word with no space or colon, its
 * length counted in code points, as the server counts a name's.
 */
const SHOWN = /:([^\s:]{2,32}):/gu;

export function referenceOf(id: string): string {
  return `<:${id.toLowerCase()}>`;
}

/** The emoji a reaction key or reference names, or `null` for a unicode emoji. */
export function emojiIdOf(key: string): string | null {
  const match = new RegExp(`^<:(${UUID})>$`).exec(key);
  return match?.[1]?.toLowerCase() ?? null;
}

/**
 * Text typed by hand as it is sent: each `:name:` naming one of `emoji` as its reference. A
 * name is looked up as written first, and then ignoring case when exactly one emoji has it so:
 * which names are the same ignoring case is the server's database's to say, and it may keep
 * apart two that `toLowerCase` folds together.
 */
export function encodeCustomEmoji(text: string, emoji: readonly CustomEmoji[]): string {
  if (emoji.length === 0) {
    return text;
  }
  const byName = new Map(emoji.map((e) => [e.name.toLowerCase(), e.id]));
  return text.replace(SHOWN, (shown, name: string) => {
    const id = byName.get(name.toLowerCase());
    return id === undefined ? shown : referenceOf(id);
  });
}

/**
 * A sent message's text as the message box shows it: each reference to one of `emoji` as
 * `:name:`. A reference naming none of them is left as it was sent.
 */
export function decodeCustomEmoji(content: string, emoji: readonly CustomEmoji[]): string {
  if (emoji.length === 0) {
    return content;
  }
  const byId = new Map(emoji.map((e) => [e.id.toLowerCase(), e.name]));
  return content.replace(EMOJI_REFERENCE, (reference, id: string) => {
    const name = byId.get(id.toLowerCase());
    return name === undefined ? reference : `:${name}:`;
  });
}
