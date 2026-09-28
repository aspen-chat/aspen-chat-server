import { format, type Messages } from "@/i18n/messages";

/** How many tags, as words: for an accessible name that says what a badge shows. */
export function mentionsText(m: Messages, count: number): string {
  return count === 1 ? m.oneMention : format(m.mentions, { count: String(count) });
}
