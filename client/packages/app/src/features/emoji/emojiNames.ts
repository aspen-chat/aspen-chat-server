/**
 * Every unicode emoji by its names, for completing `:name` in a message box. The names are
 * the picker's own English list (`emoji-picker-react` ships one per language, and the picker
 * searches English too), loaded as its own chunk the first time a colon is typed.
 */

/** One emoji: its glyph and the names it answers to, the last the fullest. */
export interface NamedEmoji {
  readonly glyph: string;
  readonly names: readonly string[];
}

/** The picker's data file: emoji by category, each with its names and code points. */
interface EmojiData {
  emojis: Record<string, readonly { n: readonly string[]; u: string }[]>;
}

let loading: Promise<readonly NamedEmoji[]> | null = null;

/** Every emoji with its names, loaded once. */
export function loadEmojiNames(): Promise<readonly NamedEmoji[]> {
  loading ??= import("emoji-picker-react/dist/data/emojis-en.json").then((module) => {
    const data = module.default as EmojiData;
    return Object.values(data.emojis).flatMap((category) =>
      category.map((e) => ({
        glyph: String.fromCodePoint(...e.u.split("-").map((hex) => parseInt(hex, 16))),
        names: e.n,
      })),
    );
  });
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
