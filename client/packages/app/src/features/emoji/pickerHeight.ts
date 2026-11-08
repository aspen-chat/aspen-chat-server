/**
 * How tall a picker laid across the screen is drawn: `share` of the window's height, at most
 * `maxPx`, so it leaves the conversation room on a phone and does not tower on a monitor.
 */
export function pickerHeight(maxPx: number, share: number): number {
  return Math.min(maxPx, Math.round(window.innerHeight * share));
}
