import { loadEmojiData, type EmojiLanguage } from "@/features/emoji/emojiData";

/**
 * Every unicode emoji by its names in one language, for completing `:name` in a message box:
 * read from the picker's data for that language (`emojiData.ts`), so the names the box
 * completes are the ones the picker searches by.
 */

/** One emoji: its glyph and the names it answers to, the last the fullest. */
export interface NamedEmoji {
  readonly glyph: string;
  readonly names: readonly string[];
}

const loaded = new Map<EmojiLanguage, Promise<readonly NamedEmoji[]>>();

/** Every emoji with its names in `language`, loaded once. */
export function loadEmojiNames(language: EmojiLanguage): Promise<readonly NamedEmoji[]> {
  let loading = loaded.get(language);
  if (loading === undefined) {
    loading = loadEmojiData(language).then((data) =>
      Object.values(data.emojis).flatMap((category) =>
        category.map((e) => ({
          glyph: String.fromCodePoint(...e.u.split("-").map((hex) => parseInt(hex, 16))),
          names: e.n,
        })),
      ),
    );
    loaded.set(language, loading);
  }
  return loading;
}

/**
 * The emoji whose names begin with `query`, or hold it as a word, at most `limit`: the ones
 * whose fullest name begins with it first.
 */
export function searchEmojiNames(
  emoji: readonly NamedEmoji[],
  query: string,
  limit: number,
): NamedEmoji[] {
  const q = query.toLowerCase();
  const starts: NamedEmoji[] = [];
  const holds: NamedEmoji[] = [];
  for (const e of emoji) {
    const full = e.names[e.names.length - 1] ?? "";
    if (full.startsWith(q) || e.names.some((n) => n.startsWith(q))) {
      starts.push(e);
    } else if (e.names.some((n) => n.includes(q))) {
      holds.push(e);
    }
    if (starts.length >= limit) {
      break;
    }
  }
  return [...starts, ...holds].slice(0, limit);
}
