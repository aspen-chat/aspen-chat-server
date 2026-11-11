import type { Growth } from "@aspen/protocol";
import { dateFormat } from "@/i18n/format";
import { format, type Messages } from "@/i18n/messages";

/**
 * A point's `at` is the start of a day, a week, or a month of UTC's calendar, so it is named in
 * UTC: in the viewer's zone a midnight of UTC is the evening before anywhere west of it, and
 * every point would be named a day or a month early.
 */
const DAY: Intl.DateTimeFormatOptions = { month: "short", day: "numeric", timeZone: "UTC" };
const MONTH: Intl.DateTimeFormatOptions = { month: "short", year: "numeric", timeZone: "UTC" };
const SHORT_MONTH: Intl.DateTimeFormatOptions = {
  month: "short",
  year: "2-digit",
  timeZone: "UTC",
};

/** How a step of `unit` is named in `locale`, in the tooltip and the table. */
export function describer(m: Messages, locale: string, unit: Growth["unit"]) {
  const dayFormat = dateFormat(locale, DAY);
  const monthFormat = dateFormat(locale, MONTH);
  return (at: string) => {
    const date = new Date(at);
    switch (unit) {
      case "day":
        return dayFormat.format(date);
      case "week":
        return format(m.admin.weekOf, { date: dayFormat.format(date) });
      case "month":
        return monthFormat.format(date);
    }
  };
}

/** How a step of `unit` is named in `locale` along the chart's axis. */
export function axisDate(locale: string, unit: Growth["unit"]) {
  const dayFormat = dateFormat(locale, DAY);
  const shortMonthFormat = dateFormat(locale, SHORT_MONTH);
  return (at: string) =>
    unit === "month" ? shortMonthFormat.format(new Date(at)) : dayFormat.format(new Date(at));
}
