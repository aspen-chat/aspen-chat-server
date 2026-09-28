/** Formatting the dashboard's figures in the reader's locale. */

const whole = new Intl.NumberFormat(undefined);
const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
const oneDecimal = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const date = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** A count as a headline figure: exact below ten thousand, compact above (12.9K). */
export function headline(n: number): string {
  return n < 10_000 ? whole.format(n) : compact.format(n);
}

/** A count in a table column. */
export function count(n: number): string {
  return whole.format(n);
}

/** A rate, to one decimal place. */
export function rate(n: number): string {
  return oneDecimal.format(n);
}

/** A size in bytes, in the largest binary unit that keeps it above one. */
export function bytes(n: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${oneDecimal.format(value)} ${units[unit] ?? "B"}`;
}

export function day(iso: string): string {
  return date.format(new Date(iso));
}

export function moment(iso: string): string {
  return dateTime.format(new Date(iso));
}

/** How long ago `iso` was, from `now`: "5 minutes ago", "yesterday". */
export function ago(iso: string, now: number): string {
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
export function since(iso: string, now: number): string {
  const total = Math.max(0, Math.floor((now - Date.parse(iso)) / 1000));
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3_600);
  const minutes = Math.floor((total % 3_600) / 60);
  if (days > 0) {
    return `${String(days)}d ${String(hours)}h`;
  }
  if (hours > 0) {
    return `${String(hours)}h ${String(minutes)}m`;
  }
  return `${String(minutes)}m`;
}
