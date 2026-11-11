import { TYPE, type MessageFormatElement } from "@formatjs/icu-messageformat-parser";
import { IntlMessageFormat } from "intl-messageformat";
import catalogue from "./locales/en.json";

/**
 * User-facing strings: `locales/en.json`, the English catalogue every other language is
 * translated from, nested by feature. Keys are camelCase to match the server's `locales/*.yml`
 * convention. Server-produced Problem `title`/`detail` values are already localized and are
 * shown as-is.
 *
 * Each string is an ICU MessageFormat template, the syntax translation platforms check and
 * help with: `{name}` for a value, and `{count, plural, one {# vote} other {# votes}}` for
 * words that change with a number, which each language writes with the forms it has.
 */
export const en = catalogue;

/** A catalogue: the shape of `en`, with any text in place of its strings. */
type Catalogue<T> = { readonly [K in keyof T]: T[K] extends string ? string : Catalogue<T[K]> };

export type Messages = Catalogue<typeof en>;

/** Strings by key, nested: what a catalogue is, whatever its keys. */
export interface StringTree {
  readonly [key: string]: string | StringTree;
}

/**
 * The templates written in each language, by its tag: English's own, each translation's, and
 * each pseudo-locale's. A catalogue's language picks its templates' plural forms and how `#`
 * and `{n, number}` write their number.
 */
const written = new Map<string, Set<string>>();

/**
 * Templates parsed, by the language each was parsed in and then by text: every one some
 * catalogue's, so they number at most what the catalogues hold.
 */
const parsed = new Map<string, Map<string, Parsed>>();

/**
 * The language of English's own templates, of a string a translation leaves in English, and of
 * templates no catalogue holds.
 */
const DEFAULT_LANGUAGE = "en";

/**
 * The catalogue `format` reads templates as being of. A text two catalogues hold is written in
 * each one's language, so only the catalogue shown can say which a template is in.
 */
let shown = DEFAULT_LANGUAGE;

/** Notes `messages`' templates as written in `language`; `locales.ts` does so for each it makes. */
export function register(messages: StringTree, language: string): void {
  let templates = written.get(language);
  if (templates === undefined) {
    templates = new Set();
    written.set(language, templates);
  }
  for (const value of Object.values(messages)) {
    if (typeof value === "string") {
      templates.add(value);
    } else {
      register(value, language);
    }
  }
}

register(en, DEFAULT_LANGUAGE);

/**
 * Has `format` read templates as the catalogue of `language`'s from here on. `I18nProvider`
 * calls it with the language it shows before anything it holds formats.
 */
export function showLanguage(language: string): void {
  shown = language;
}

interface Parsed {
  /** `null` when the template is not valid MessageFormat, and is shown as written. */
  readonly message: IntlMessageFormat | null;
  /** Its plain `{name}` placeholders, which are left as written when given no value. */
  readonly placeholders: readonly string[];
}

function placeholdersOf(elements: readonly MessageFormatElement[], into: Set<string>): Set<string> {
  for (const element of elements) {
    if (element.type === TYPE.argument) {
      into.add(element.value);
    } else if (element.type === TYPE.plural || element.type === TYPE.select) {
      for (const option of Object.values(element.options)) {
        placeholdersOf(option.value, into);
      }
    }
  }
  return into;
}

function parse(template: string): Parsed {
  const language = written.get(shown)?.has(template) === true ? shown : DEFAULT_LANGUAGE;
  const known = parsed.get(language)?.get(template);
  if (known !== undefined) {
    return known;
  }
  let result: Parsed;
  try {
    // Text in angle brackets is text: no template marks anything up.
    const message = new IntlMessageFormat(template, language, undefined, { ignoreTag: true });
    result = { message, placeholders: [...placeholdersOf(message.getAst(), new Set())] };
  } catch (error) {
    console.error(`Not a valid message template: ${JSON.stringify(template)}`, error);
    result = { message: null, placeholders: [] };
  }
  if (written.get(language)?.has(template) === true) {
    let ofLanguage = parsed.get(language);
    if (ofLanguage === undefined) {
      ofLanguage = new Map();
      parsed.set(language, ofLanguage);
    }
    ofLanguage.set(template, result);
  }
  return result;
}

/**
 * `template` with `values` in its placeholders, and a plural's form chosen by the number given
 * for it. A plain placeholder given no value is left as written; a template that cannot be
 * formatted (a plural given no number, a translation that is not valid MessageFormat) is shown as
 * written, and the console says why.
 */
export function formatParts<T>(
  template: string,
  values: Readonly<Record<string, string | number | T>>,
): string | T | (string | T)[] {
  const { message, placeholders } = parse(template);
  if (message === null) {
    return template;
  }
  const filled: Record<string, string | number | T> = { ...values };
  for (const name of placeholders) {
    if (!(name in filled)) {
      filled[name] = `{${name}}`;
    }
  }
  try {
    return message.format<T>(filled);
  } catch (error) {
    console.error(`Could not format ${JSON.stringify(template)}`, error);
    return template;
  }
}

export function format(
  template: string,
  values: Readonly<Record<string, string | number>>,
): string {
  const formatted = formatParts<never>(template, values);
  return Array.isArray(formatted) ? formatted.join("") : formatted;
}
