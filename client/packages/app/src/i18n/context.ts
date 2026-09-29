import { createContext, useContext } from "react";
import type { ResolvedLocale } from "./locales";
import { en, type Messages } from "./messages";

export const MessagesContext = createContext<Messages>(en);

export function useMessages(): Messages {
  return useContext(MessagesContext);
}

/** The language the app shows: what the user chose, and what that comes to here. */
export interface LanguageSetting {
  /** A catalogue's tag, or `AUTOMATIC`. */
  readonly choice: string;
  readonly setChoice: (choice: string) => void;
  readonly resolved: ResolvedLocale;
}

export const LanguageContext = createContext<LanguageSetting | null>(null);

export function useLanguageSetting(): LanguageSetting {
  const setting = useContext(LanguageContext);
  if (setting === null) {
    throw new Error("useLanguageSetting must be used inside <I18nProvider>");
  }
  return setting;
}
