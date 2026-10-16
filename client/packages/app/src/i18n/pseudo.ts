import { parse, TYPE, type MessageFormatElement } from "@formatjs/icu-messageformat-parser";
import type { StringTree } from "./messages";

/**
 * The pseudo-locales, made from English so anyone can see which text has not been localized
 * and how the interface takes longer text and right-to-left layout. They match the server's
 * (`app::locale`); `spec/pseudo_locale_vectors.json` holds cases both pass.
 */

const PLAIN = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ACCENTED = Array.from("áƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ");
const VOWELS = "aeiouAEIOU";

/** Where each run of text lies in `elements`, inside plurals' and selects' forms too. */
function textRuns(elements: readonly MessageFormatElement[], into: [number, number][]) {
  for (const element of elements) {
    if (element.type === TYPE.literal && element.location !== undefined) {
      into.push([element.location.start.offset, element.location.end.offset]);
    } else if (element.type === TYPE.plural || element.type === TYPE.select) {
      for (const option of Object.values(element.options)) {
        textRuns(option.value, into);
      }
    }
  }
  return into;
}

/**
 * `template` with each run of its text passed through `transform`, and its MessageFormat
 * syntax (placeholders, plurals' keywords, `#`) left as written. A template that is not valid
 * MessageFormat is all text.
 */
function eachText(template: string, transform: (text: string) => string): string {
  let runs: [number, number][];
  try {
    runs = textRuns(parse(template, { ignoreTag: true, captureLocation: true }), []);
  } catch {
    runs = [[0, template.length]];
  }
  runs.sort((a, b) => a[0] - b[0]);
  let result = "";
  let written = 0;
  for (const [start, end] of runs) {
    result += template.slice(written, start) + transform(template.slice(start, end));
    written = end;
  }
  return result + template.slice(written);
}

/** `template` accented: every ASCII letter accented, every vowel doubled, in brackets. */
export function accented(template: string): string {
  const body = eachText(template, (text) =>
    text.replace(/[A-Za-z]/g, (c) => {
      const accent = ACCENTED[PLAIN.indexOf(c)] ?? c;
      return VOWELS.includes(c) ? accent + accent : accent;
    }),
  );
  return `[${body}]`;
}

/** `template` mirrored: every word set right to left, between an override and a pop of it. */
export function mirrored(template: string): string {
  return eachText(template, (text) => text.replace(/\S+/gu, (word) => `‮${word}‬`));
}

/** Every string of `catalogue` passed through `transform`, keeping its shape. */
export function pseudoCatalogue<T extends StringTree>(
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
