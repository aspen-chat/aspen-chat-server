import { useMemo } from "react";
import { useLocale } from "react-aria-components";
import { dateFormat, numberFormat, relativeTimeFormat } from "@/i18n/format";

/** Formatting the dashboard's figures in the app's locale. */
export function useFigures() {
  const { locale } = useLocale();
  return useMemo(() => figures(locale), [locale]);
}

export type Figures = ReturnType<typeof figures>;

function figures(locale: string) {
  const whole = numberFormat(locale);
  const compact = numberFormat(locale, { notation: "compact", maximumFractionDigits: 1 });
  const oneDecimal = numberFormat(locale, { maximumFractionDigits: 1 });
  const date = dateFormat(locale, { dateStyle: "medium" });
  const dateTime = dateFormat(locale, { dateStyle: "medium", timeStyle: "short" });
  const relative = relativeTimeFormat(locale, { numeric: "auto" });
  const inUnit = (name: string) =>
    numberFormat(locale, { style: "unit", unit: name, unitDisplay: "narrow" });

  /** A count as a headline figure: exact below ten thousand, compact above (12.9K). */
  function headline(n: number): string {
    return n < 10_000 ? whole.format(n) : compact.format(n);
  }

  /** A count in a table column. */
  function count(n: number): string {
    return whole.format(n);
  }

  /** A rate, to one decimal place. */
  function rate(n: number): string {
    return oneDecimal.format(n);
  }

  /** A size in bytes, in the largest binary unit that keeps it above one. */
  function bytes(n: number): string {
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let value = n;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${oneDecimal.format(value)} ${units[unit] ?? "B"}`;
  }

  function day(iso: string): string {
    return date.format(new Date(iso));
  }

  function moment(iso: string): string {
    return dateTime.format(new Date(iso));
  }

  /** How long ago `iso` was, from `now`: "5 minutes ago", "yesterday". */
  function ago(iso: string, now: number): string {
    const seconds = Math.round((Date.parse(iso) - now) / 1000);
    const steps: [Intl.RelativeTimeFormatUnit, number][] = [
      ["day", 86_400],
      ["hour", 3_600],
      ["minute", 60],
    ];
    for (const [unit, size] of steps) {
      if (Math.abs(seconds) >= size) {
        return relative.format(Math.round(seconds / size), unit);
      }
    }
    return relative.format(seconds, "second");
  }

  /** How long since `iso`, from `now`, in the largest two units: "3d 4h", "12m". */
  function since(iso: string, now: number): string {
    const total = Math.max(0, Math.floor((now - Date.parse(iso)) / 1000));
    const days = Math.floor(total / 86_400);
    const hours = Math.floor((total % 86_400) / 3_600);
    const minutes = Math.floor((total % 3_600) / 60);
    if (days > 0) {
      return `${inUnit("day").format(days)} ${inUnit("hour").format(hours)}`;
    }
    if (hours > 0) {
      return `${inUnit("hour").format(hours)} ${inUnit("minute").format(minutes)}`;
    }
    return inUnit("minute").format(minutes);
  }
  return { headline, count, rate, bytes, day, moment, ago, since };
}
