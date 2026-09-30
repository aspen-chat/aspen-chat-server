import { numberFormat } from "@/i18n/format";

/**
 * How long a call lasted, in words of `locale`: hours, minutes, and seconds, each named in full
 * and joined as a list ("1 hour and 5 minutes"), leaving out parts that are zero, and seconds
 * once it reached an hour. A call of no time at all lasted "0 seconds".
 */
export function callLength(locale: string, seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const rest = whole % 60;
  const unit = (value: number, name: "hour" | "minute" | "second") =>
    numberFormat(locale, { style: "unit", unit: name, unitDisplay: "long" }).format(value);
  const parts = [
    ...(hours > 0 ? [unit(hours, "hour")] : []),
    ...(minutes > 0 ? [unit(minutes, "minute")] : []),
    ...(rest > 0 && hours === 0 ? [unit(rest, "second")] : []),
  ];
  if (parts.length === 0) {
    return unit(0, "second");
  }
  return new Intl.ListFormat(locale, { style: "long", type: "conjunction" }).format(parts);
}
