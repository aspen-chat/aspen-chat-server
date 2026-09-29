/**
 * The pseudo-locales, made from English so anyone can see which text has not been localized
 * and how the interface takes longer text and right-to-left layout. They match the server's
 * (`app::locale`); `spec/pseudo_locale_vectors.json` holds cases both pass.
 */

const PLAIN = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ACCENTED = Array.from("áƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ");
const VOWELS = "aeiouAEIOU";

/** A template's text and its `{name}` placeholders, which a pseudo-locale leaves alone. */
function pieces(template: string): { text: string; placeholder: boolean }[] {
  return template
    .split(/(\{\w+\})/)
    .filter((piece) => piece !== "")
    .map((piece) => ({ text: piece, placeholder: /^\{\w+\}$/.test(piece) }));
}

/** `template` accented: every ASCII letter accented, every vowel doubled, in brackets. */
export function accented(template: string): string {
  const body = pieces(template)
    .map(({ text, placeholder }) =>
      placeholder
        ? text
        : text.replace(/[A-Za-z]/g, (c) => {
            const accent = ACCENTED[PLAIN.indexOf(c)] ?? c;
            return VOWELS.includes(c) ? accent + accent : accent;
          }),
    )
    .join("");
  return `[${body}]`;
}

/** `template` mirrored: every word set right to left, between an override and a pop of it. */
export function mirrored(template: string): string {
  return pieces(template)
    .map(({ text, placeholder }) =>
      placeholder ? text : text.replace(/\S+/gu, (word) => `‮${word}‬`),
    )
    .join("");
}

interface Catalogue {
  readonly [key: string]: string | Catalogue;
}

/** Every string of `catalogue` passed through `transform`, keeping its shape. */
export function pseudoCatalogue<T extends Catalogue>(
  catalogue: T,
  transform: (template: string) => string,
): T {
  return Object.fromEntries(
    Object.entries(catalogue).map(([key, value]) => [
      key,
      typeof value === "string" ? transform(value) : pseudoCatalogue(value, transform),
    ]),
  ) as T;
}
