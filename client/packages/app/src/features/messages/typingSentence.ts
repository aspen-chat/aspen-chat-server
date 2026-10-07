/** One piece of the typing line: a person's name, by their place in the list, or words. */
export type SentencePart = { name: number } | { text: string };

/**
 * `template`'s `{names}` filled with `count` names joined as `locale` lists things ("A, B, and
 * C"), as pieces, so the line can shorten each name when it runs out of room and keep every word
 * around them whole, wherever the language puts them.
 */
export function typingSentence(template: string, locale: string, count: number): SentencePart[] {
  const list = new Intl.ListFormat(locale, { style: "long", type: "conjunction" }).formatToParts(
    Array.from({ length: count }, (_, index) => String(index)),
  );
  return template.split(/(\{names\})/).flatMap((part): SentencePart[] => {
    if (part === "{names}") {
      return list.map((piece) =>
        piece.type === "element" ? { name: Number(piece.value) } : { text: piece.value },
      );
    }
    return part === "" ? [] : [{ text: part }];
  });
}
