/**
 * The times a message may be scheduled for, and the ones the schedule dialog offers outright.
 * The bounds mirror the server's `app::message::scheduled`, so the form refuses what it would.
 */

import {
  type CalendarDateTime,
  fromDate,
  getLocalTimeZone,
  toCalendarDateTime,
} from "@internationalized/date";
import { format, type Messages } from "@/i18n/messages";

export const MIN_AHEAD_MS = 60 * 1000;
export const MAX_AHEAD_MS = 730 * 24 * 60 * 60 * 1000;

/** How a time a message is scheduled for is written wherever one is shown. */
export const SCHEDULED_TIME_FORMAT: Intl.DateTimeFormatOptions = {
  weekday: "short",
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
};

/** The hour of the morning the dialog's "morning" choices mean, on the reader's own clock. */
const MORNING_HOUR = 9;

export type QuickTime = "inAnHour" | "tomorrow" | "nextMonday";

/**
 * The times offered outright as of `now`, on the reader's clock: an hour from now, tomorrow
 * morning, and the coming Monday's morning, which is left out when tomorrow is that Monday.
 */
export function quickTimes(now: Date): { key: QuickTime; at: Date }[] {
  const inAnHour = new Date(now.getTime() + 60 * 60 * 1000);
  inAnHour.setSeconds(0, 0);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(MORNING_HOUR, 0, 0, 0);
  const monday = new Date(now);
  // Days until the next Monday (day 1), a week on when today is one.
  monday.setDate(monday.getDate() + ((8 - monday.getDay()) % 7 || 7));
  monday.setHours(MORNING_HOUR, 0, 0, 0);
  const times: { key: QuickTime; at: Date }[] = [
    { key: "inAnHour", at: inAnHour },
    { key: "tomorrow", at: tomorrow },
  ];
  if (monday.getTime() !== tomorrow.getTime()) {
    times.push({ key: "nextMonday", at: monday });
  }
  return times;
}

/** `date` as the date field holds it: the reader's own date and time, with no time zone. */
export function localDateTime(date: Date): CalendarDateTime {
  return toCalendarDateTime(fromDate(date, getLocalTimeZone()));
}

/** The moment the date field's value names on the reader's clock. */
export function toDate(value: CalendarDateTime): Date {
  return value.toDate(getLocalTimeZone());
}

/** Why `at` cannot be scheduled for as of `now`, if it cannot. */
export function timeProblem(at: Date, now: Date): "tooSoon" | "tooFar" | null {
  const ahead = at.getTime() - now.getTime();
  if (ahead < MIN_AHEAD_MS) {
    return "tooSoon";
  }
  return ahead > MAX_AHEAD_MS ? "tooFar" : null;
}

/** What a time out of bounds (`timeProblem`) is refused with, wherever one is. */
export function describeTimeProblem(m: Messages, problem: "tooSoon" | "tooFar"): string {
  return problem === "tooSoon"
    ? format(m.scheduled.tooSoon, { minutes: MIN_AHEAD_MS / 60_000 })
    : format(m.scheduled.tooFar, { years: Math.floor(MAX_AHEAD_MS / (365 * 86_400_000)) });
}
