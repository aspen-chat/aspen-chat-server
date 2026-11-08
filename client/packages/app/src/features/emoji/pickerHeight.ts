import { pageHeight } from "@/theme/visibleArea";

/**
 * How tall a picker laid across the screen is drawn: `share` of the page's height with no
 * keyboard taking from it, at most `maxPx`, so it leaves the conversation room on a phone and does
 * not tower on a monitor.
 */
export function pickerHeight(maxPx: number, share: number): number {
  return Math.min(maxPx, Math.round(pageHeight() * share));
}
