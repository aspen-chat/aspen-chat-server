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
 * The language each catalogue's templates are written in, which picks a template's plural
 * forms and how `#` writes its number. The first catalogue to hold a text names it: a string a
 * translation leaves in English is English, and two languages that write a template
 * identically write it alike.
 */
const languages = new Map<string, string>();

/** Templates parsed, each of some catalogue's, so they number at most what the catalogues hold. */
const parsed = new Map<string, Parsed>();

/** The language of templates no catalogue holds. */
const DEFAULT_LANGUAGE = "en";

/** Notes `messages`' templates as written in `language`; `locales.ts` does so for each it makes. */
export function register(messages: StringTree, language: string): void {
  for (const value of Object.values(messages)) {
    if (typeof value === "string") {
      if (!languages.has(value)) {
        languages.set(value, language);
      }
    } else {
      register(value, language);
    }
  }
}

register(en, "en");

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
  const known = parsed.get(template);
  if (known !== undefined) {
    return known;
  }
  const language = languages.get(template);
  let result: Parsed;
  try {
    // Text in angle brackets is text: no template marks anything up.
    const message = new IntlMessageFormat(template, language ?? DEFAULT_LANGUAGE, undefined, {
      ignoreTag: true,
    });
    result = { message, placeholders: [...placeholdersOf(message.getAst(), new Set())] };
  } catch (error) {
    console.error(`Not a valid message template: ${JSON.stringify(template)}`, error);
    result = { message: null, placeholders: [] };
  }
  if (language !== undefined) {
    parsed.set(template, result);
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
