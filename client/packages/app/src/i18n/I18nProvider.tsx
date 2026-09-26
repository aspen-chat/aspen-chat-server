import type { ReactNode } from "react";
import { I18nProvider as AriaI18nProvider } from "react-aria-components";
import { MessagesContext } from "./context";
import { en } from "./messages";

/**
 * Provides both React Aria's locale (dates, numbers, keyboard hints, screen-reader strings) and
 * the app's own message catalogue. Only English exists today; adding a locale means adding a
 * catalogue and selecting it here.
 */
export function I18nProvider({ children }: { children: ReactNode }) {
  const locale = typeof navigator === "undefined" ? "en-US" : navigator.language;
  return (
    <AriaI18nProvider locale={locale}>
      <MessagesContext.Provider value={en}>{children}</MessagesContext.Provider>
    </AriaI18nProvider>
  );
}
