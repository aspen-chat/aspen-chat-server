const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/**
 * The first character of the name's first and last words, each a whole grapheme cluster, so an
 * emoji, a flag, or a letter with a combining accent is never cut in half.
 */
export function initials(name: string): string {
  const words = name
    .trim()
    .split(/\s+/)
    .filter((w) => w.length > 0);
  const firstOf = (word: string | undefined) =>
    word === undefined
      ? ""
      : (graphemes.segment(word)[Symbol.iterator]().next().value?.segment ?? "");
  const first = firstOf(words[0]) || "?";
  const second = words.length > 1 ? firstOf(words[words.length - 1]) : "";
  return (first + second).toUpperCase();
}
