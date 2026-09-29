import { setPreferredLanguages } from "@aspen/protocol";
import { useCallback, useEffect, useLayoutEffect, useMemo, useState, type ReactNode } from "react";
import { I18nProvider as AriaI18nProvider } from "react-aria-components";
import { LanguageContext, MessagesContext } from "./context";
import { AUTOMATIC, resolveLocale } from "./locales";

/**
 * Where this install remembers the language last chosen, so the sign-in screen shows it too;
 * signed in, the account's choice (`LANGUAGE`) replaces it.
 */
const STORAGE_KEY = "aspen.language";

function storedChoice(): string {
  try {
    return window.localStorage.getItem(STORAGE_KEY) ?? AUTOMATIC;
  } catch {
    return AUTOMATIC;
  }
}

function platformLanguages(): readonly string[] {
  if (typeof navigator === "undefined") {
    return [];
  }
  return navigator.languages.length > 0 ? navigator.languages : [navigator.language];
}

/**
 * Provides the language the app shows: the app's catalogue, React Aria's locale (dates,
 * numbers, keyboard hints, screen-reader strings, and direction), the document's `lang` and
 * `dir`, and the languages every request asks servers to write in.
 */
export function I18nProvider({ children }: { children: ReactNode }) {
  const [choice, setChoiceState] = useState(storedChoice);
  const [platform, setPlatform] = useState(platformLanguages);
  useEffect(() => {
    const update = () => {
      setPlatform(platformLanguages());
    };
    window.addEventListener("languagechange", update);
    return () => {
      window.removeEventListener("languagechange", update);
    };
  }, []);
  const resolved = useMemo(() => resolveLocale(choice, platform), [choice, platform]);
  // Before any child's effects, so the first requests already name the language.
  useLayoutEffect(() => {
    document.documentElement.lang = resolved.locale;
    document.documentElement.dir = resolved.direction;
    setPreferredLanguages(resolved.languages);
  }, [resolved]);
  const setChoice = useCallback((next: string) => {
    setChoiceState(next);
    try {
      window.localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Storage may be unavailable; the choice still holds until the page is closed.
    }
  }, []);
  const setting = useMemo(() => ({ choice, setChoice, resolved }), [choice, setChoice, resolved]);
  return (
    <LanguageContext.Provider value={setting}>
      <AriaI18nProvider locale={resolved.locale}>
        <MessagesContext.Provider value={resolved.messages}>{children}</MessagesContext.Provider>
      </AriaI18nProvider>
    </LanguageContext.Provider>
  );
}
