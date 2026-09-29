import { useLocale } from "react-aria-components";

/**
 * Intl formatters in the locale the app shows (`I18nProvider`), made once per locale and
 * options: dates, times, and numbers read the same language as the text around them.
 */
const made = new Map<string, Intl.DateTimeFormat | Intl.NumberFormat | Intl.RelativeTimeFormat>();

function cached<T extends Intl.DateTimeFormat | Intl.NumberFormat | Intl.RelativeTimeFormat>(
  kind: string,
  locale: string,
  options: object,
  make: () => T,
): T {
  const key = `${kind}|${locale}|${JSON.stringify(options)}`;
  let formatter = made.get(key);
  if (formatter === undefined) {
    formatter = make();
    made.set(key, formatter);
  }
  return formatter as T;
}

export function dateFormat(locale: string, options: Intl.DateTimeFormatOptions) {
  return cached("date", locale, options, () => new Intl.DateTimeFormat(locale, options));
}

export function numberFormat(locale: string, options: Intl.NumberFormatOptions = {}) {
  return cached("number", locale, options, () => new Intl.NumberFormat(locale, options));
}

export function relativeTimeFormat(locale: string, options: Intl.RelativeTimeFormatOptions) {
  return cached("relative", locale, options, () => new Intl.RelativeTimeFormat(locale, options));
}

/** Formats dates and times in the app's locale. */
export function useDateFormat(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  return dateFormat(useLocale().locale, options);
}

/** Formats numbers in the app's locale. */
export function useNumberFormat(options: Intl.NumberFormatOptions = {}): Intl.NumberFormat {
  return numberFormat(useLocale().locale, options);
}
