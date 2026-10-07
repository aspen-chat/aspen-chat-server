/**
 * Characters a file name is shown without: format characters (`\p{Cf}`: the bidirectional
 * overrides and isolates, zero-width spaces and joiners), which can make `gpj.exe` read as
 * `exe.jpg` or hide part of a name, and control characters (`\p{Cc}`), which have no place in
 * one.
 */
const HIDDEN = /[\p{Cf}\p{Cc}]/gu;

/**
 * A file name someone else chose (an attachment's, a file sent in a call) as it is shown and
 * saved: without the characters that change how the rest reads (`HIDDEN`).
 */
export function plainFileName(name: string): string {
  return name.replace(HIDDEN, "");
}
