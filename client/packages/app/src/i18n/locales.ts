import { en, type Messages } from "./messages";
import { accented, mirrored, pseudoCatalogue } from "./pseudo";

/** The catalogues the app has, by BCP 47 tag: English, and the two pseudo-locales made from it. */
export const LANGUAGES = ["en", "en-XA", "ar-XB"] as const;
export type Language = (typeof LANGUAGES)[number];

/** The language choice that follows the platform's languages. */
export const AUTOMATIC = "automatic";

const PSEUDO: readonly string[] = ["en-XA", "ar-XB"];

/** Languages written right to left, by primary subtag. */
const RIGHT_TO_LEFT = new Set(["ar", "ckb", "dv", "fa", "he", "ps", "sd", "ug", "ur", "yi"]);

const made = new Map<Language, Messages>([["en", en]]);

function catalogue(language: Language): Messages {
  let messages = made.get(language);
  if (messages === undefined) {
    messages = pseudoCatalogue(en, language === "ar-XB" ? mirrored : accented);
    made.set(language, messages);
  }
  return messages;
}

/** What a language choice comes to on this platform. */
export interface ResolvedLocale {
  /** The catalogue shown. */
  readonly language: Language;
  readonly messages: Messages;
  /**
   * The locale dates, times, and numbers are formatted in: the platform's own variant of the
   * catalogue's language when it has one (`en-GB`), so it never mixes two languages.
   */
  readonly locale: string;
  /** What servers are asked to write in, most preferred first. */
  readonly languages: readonly string[];
  readonly direction: "ltr" | "rtl";
}

const primary = (tag: string) => tag.split("-")[0]?.toLowerCase() ?? "";

function isLanguage(tag: string): tag is Language {
  return (LANGUAGES as readonly string[]).includes(tag);
}

/**
 * The catalogue `choice` names, or for `automatic` the first of `platform` (the browser's
 * languages, most preferred first) that the app has, matched whole and then by language
 * (`en-GB` shows `en`); English when none is. A pseudo-locale is only ever matched whole.
 */
export function resolveLocale(choice: string, platform: readonly string[]): ResolvedLocale {
  const language: Language = isLanguage(choice)
    ? choice
    : (platform.flatMap((tag): Language[] => {
        const exact = LANGUAGES.find((l) => l.toLowerCase() === tag.toLowerCase());
        if (exact !== undefined) {
          return [exact];
        }
        const same = LANGUAGES.find((l) => !PSEUDO.includes(l) && l === primary(tag));
        return same === undefined ? [] : [same];
      })[0] ?? "en");
  const locale = PSEUDO.includes(language)
    ? language
    : (platform.find((tag) => primary(tag) === language) ?? language);
  return {
    language,
    messages: catalogue(language),
    locale,
    languages: locale === language ? [language] : [locale, language],
    direction: language === "ar-XB" || RIGHT_TO_LEFT.has(primary(language)) ? "rtl" : "ltr",
  };
}
