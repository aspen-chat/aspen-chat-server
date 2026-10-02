import type Picker from "emoji-picker-react";
import { useMemo, type ComponentProps } from "react";
import { useLanguageSetting } from "@/i18n/context";
import { platformLanguages } from "@/i18n/locales";

/**
 * The emoji picker's data in the reader's language: every emoji with its names and the
 * category headings, one file per language, which `emoji-picker-react` ships and takes back
 * through its `emojiData` prop. The same file gives the message box its names for completing
 * `:name`. Each is loaded as its own chunk the first time it is needed, and kept.
 */
export type EmojiData = NonNullable<ComponentProps<typeof Picker>["emojiData"]>;

// A file's own type is read from it, with the categories' names as strings where the picker's
// type names its enum; the one place that reads a file says what it is.
type Loader = () => Promise<{ default: unknown }>;

/** The languages the picker has data for, by their tags as the browser spells them. */
const LOADERS = {
  bn: () => import("emoji-picker-react/dist/data/emojis-bn.json"),
  da: () => import("emoji-picker-react/dist/data/emojis-da.json"),
  de: () => import("emoji-picker-react/dist/data/emojis-de.json"),
  en: () => import("emoji-picker-react/dist/data/emojis-en.json"),
  "en-gb": () => import("emoji-picker-react/dist/data/emojis-en-gb.json"),
  es: () => import("emoji-picker-react/dist/data/emojis-es.json"),
  "es-mx": () => import("emoji-picker-react/dist/data/emojis-es-mx.json"),
  et: () => import("emoji-picker-react/dist/data/emojis-et.json"),
  fi: () => import("emoji-picker-react/dist/data/emojis-fi.json"),
  fr: () => import("emoji-picker-react/dist/data/emojis-fr.json"),
  hi: () => import("emoji-picker-react/dist/data/emojis-hi.json"),
  hu: () => import("emoji-picker-react/dist/data/emojis-hu.json"),
  it: () => import("emoji-picker-react/dist/data/emojis-it.json"),
  ja: () => import("emoji-picker-react/dist/data/emojis-ja.json"),
  ko: () => import("emoji-picker-react/dist/data/emojis-ko.json"),
  lt: () => import("emoji-picker-react/dist/data/emojis-lt.json"),
  ms: () => import("emoji-picker-react/dist/data/emojis-ms.json"),
  nb: () => import("emoji-picker-react/dist/data/emojis-nb.json"),
  nl: () => import("emoji-picker-react/dist/data/emojis-nl.json"),
  pl: () => import("emoji-picker-react/dist/data/emojis-pl.json"),
  pt: () => import("emoji-picker-react/dist/data/emojis-pt.json"),
  ru: () => import("emoji-picker-react/dist/data/emojis-ru.json"),
  sv: () => import("emoji-picker-react/dist/data/emojis-sv.json"),
  th: () => import("emoji-picker-react/dist/data/emojis-th.json"),
  uk: () => import("emoji-picker-react/dist/data/emojis-uk.json"),
  vi: () => import("emoji-picker-react/dist/data/emojis-vi.json"),
  zh: () => import("emoji-picker-react/dist/data/emojis-zh.json"),
  "zh-hant": () => import("emoji-picker-react/dist/data/emojis-zh-hant.json"),
} satisfies Readonly<Record<string, Loader>>;

/** The language the emoji are named in, by the tag of its data file. */
export type EmojiLanguage = keyof typeof LOADERS;

function isEmojiLanguage(tag: string): tag is EmojiLanguage {
  return Object.hasOwn(LOADERS, tag);
}

/**
 * The data language for what the reader asked of the app: a catalogue they chose, or, for
 * `automatic`, the first of the browser's languages (most preferred first) the picker has,
 * each matched from its whole tag down to its language alone (`zh-Hant-TW` finds `zh-hant`,
 * `en-US` finds `en`); English when none is. A pseudo-locale's names are English, since it
 * is made from English.
 */
export function emojiLanguageFor(choice: string, platform: readonly string[]): EmojiLanguage {
  const wanted = choice === "automatic" ? platform : [choice];
  for (const tag of wanted) {
    const parts = tag.toLowerCase().split("-");
    for (let take = parts.length; take > 0; take -= 1) {
      const candidate = parts.slice(0, take).join("-");
      if (isEmojiLanguage(candidate)) {
        return candidate;
      }
    }
  }
  return "en";
}

/** The language the emoji are named in for this reader: see `emojiLanguageFor`. */
export function useEmojiLanguage(): EmojiLanguage {
  const { choice } = useLanguageSetting();
  return useMemo(() => emojiLanguageFor(choice, platformLanguages()), [choice]);
}

const loaded = new Map<EmojiLanguage, Promise<EmojiData>>();

/** The picker's data in `language`, loaded once. */
export function loadEmojiData(language: EmojiLanguage): Promise<EmojiData> {
  let loading = loaded.get(language);
  if (loading === undefined) {
    loading = LOADERS[language]().then((module) => module.default as EmojiData);
    loaded.set(language, loading);
  }
  return loading;
}
