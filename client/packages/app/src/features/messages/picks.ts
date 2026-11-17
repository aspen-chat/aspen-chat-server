import { useCallback, useState } from "react";
import { EMOJI_REFERENCE_SOURCE } from "@/features/emoji/customEmoji";
import { TAG_REFERENCE } from "@/features/mentions/tags";

/**
 * Picks in the text a person writes. A message box shows what was chosen from a list (a person
 * or role tagged, a custom emoji, a command's argument) as its plain name, and remembers each
 * pick by where it stands in the text, never by what it reads as: names repeat (two roles, a
 * local and a foreign account, the same word typed by hand in an address or a code span), so
 * only the place says which text was picked and what it stands for. The ranges are carried
 * along as the text changes, and sending replaces exactly them with what the server reads.
 */

export const PICK_KINDS = ["user", "role", "emoji", "channel", "community", "deployment"] as const;

/** What a pick stands for. A deployment's `id` is its domain. */
export type PickKind = (typeof PICK_KINDS)[number];

export interface PickTarget {
  readonly kind: PickKind;
  readonly id: string;
}

/** A stretch of text, from `start` to `end` (UTF-16 offsets). */
export interface Span {
  readonly start: number;
  readonly end: number;
}

/** A pick: the text of its span stands for its target. */
export interface Picked extends PickTarget, Span {}

/** Text and the picks in it, in order of place, none overlapping another. */
export interface PickedText {
  readonly text: string;
  readonly picks: readonly Picked[];
}

export const NOTHING_WRITTEN: PickedText = { text: "", picks: [] };

/**
 * The spans left once the text from `start` to `end` has given way to text `delta` longer:
 * those before it as they were, those after it moved along, and those it reached into gone,
 * their text now only text. Text put in at a span's edge is beside it, not in it.
 */
function carry<S extends Span>(
  picks: readonly S[],
  start: number,
  end: number,
  delta: number,
): S[] {
  return picks.flatMap((pick) => {
    if (pick.end <= start) {
      return [pick];
    }
    if (pick.start >= end) {
      return [{ ...pick, start: pick.start + delta, end: pick.end + delta }];
    }
    return [];
  });
}

/**
 * `held` with the text from `start` to `end` replaced by `shown` and then `after`. With
 * `target`, `shown` is a pick standing for it.
 */
export function splice(
  held: PickedText,
  start: number,
  end: number,
  shown: string,
  target?: PickTarget,
  after = "",
): PickedText {
  const inserted = shown + after;
  const picks = carry(held.picks, start, end, inserted.length - (end - start));
  if (target !== undefined && shown !== "") {
    picks.push({ kind: target.kind, id: target.id, start, end: start + shown.length });
    picks.sort((a, b) => a.start - b.start);
  }
  return { text: held.text.slice(0, start) + inserted + held.text.slice(end), picks };
}

interface Change {
  readonly start: number;
  /** Where the replaced text ended, in the text as it was. */
  readonly end: number;
}

function commonPrefix(a: string, b: string, limit: number): number {
  let n = 0;
  while (n < limit && a[n] === b[n]) {
    n++;
  }
  return n;
}

function commonSuffix(a: string, b: string, limit: number): number {
  let n = 0;
  while (n < limit && a[a.length - 1 - n] === b[b.length - 1 - n]) {
    n++;
  }
  return n;
}

/**
 * The change that made `old` into `next` when the caret was left at its end, as typing,
 * pasting, and deleting leave it; `null` when the text after the caret is not how `old` ended.
 */
function changeBefore(old: string, next: string, caret: number): Change | null {
  const tail = next.length - caret;
  if (tail < 0 || tail > old.length || old.slice(old.length - tail) !== next.slice(caret)) {
    return null;
  }
  const end = old.length - tail;
  return { start: commonPrefix(old, next, Math.min(end, caret)), end };
}

/**
 * `held` once its text has become `next` by an edit in the box, its picks carried along.
 * `caret` is where the box's caret stands after the edit, which says where the edit was when
 * the texts alone do not (a letter typed beside the same letter). Without it, or where it does
 * not account for the change, the change is taken to be the one stretch that differs, placed
 * wherever keeps the most picks: either placing leaves every kept pick's own text untouched.
 * Any spans of the text are carried the same way, picks or not.
 */
export function retype<S extends Span>(
  held: { readonly text: string; readonly picks: readonly S[] },
  next: string,
  caret?: number,
): { readonly text: string; readonly picks: readonly S[] } {
  const old = held.text;
  if (next === old) {
    return held;
  }
  const delta = next.length - old.length;
  const shorter = Math.min(old.length, next.length);
  const byCaret = caret === undefined ? null : changeBefore(old, next, caret);
  let picks: S[];
  if (byCaret !== null) {
    picks = carry(held.picks, byCaret.start, byCaret.end, delta);
  } else {
    const prefix = commonPrefix(old, next, shorter);
    const fromStart = carry(
      held.picks,
      prefix,
      old.length - commonSuffix(old, next, shorter - prefix),
      delta,
    );
    const suffix = commonSuffix(old, next, shorter);
    const fromEnd = carry(
      held.picks,
      commonPrefix(old, next, shorter - suffix),
      old.length - suffix,
      delta,
    );
    picks = fromEnd.length > fromStart.length ? fromEnd : fromStart;
  }
  return { text: next, picks };
}

/** `held` without the whitespace around its text, as it is sent. */
export function trimPicked(held: PickedText): PickedText {
  const text = held.text.trim();
  const lead = held.text.length - held.text.trimStart().length;
  return {
    text,
    picks: held.picks.flatMap((pick) =>
      pick.start >= lead && pick.end - lead <= text.length
        ? [{ ...pick, start: pick.start - lead, end: pick.end - lead }]
        : [],
    ),
  };
}

/**
 * The pick of `kind` that is exactly the text from `start` to `end`, as a word of a command
 * line is, if that word was picked.
 */
export function pickAt(
  picks: readonly Picked[],
  start: number,
  end: number,
  kind: PickKind,
): Picked | undefined {
  return picks.find((pick) => pick.start === start && pick.end === end && pick.kind === kind);
}

/**
 * The picks among `stored` that are whole and lie in `text`, in order, for what was read from
 * storage: anything else there (another version's shape, a range past the text) is left out,
 * and its text stays text.
 */
export function soundPicks(text: string, stored: unknown): Picked[] {
  if (!Array.isArray(stored)) {
    return [];
  }
  const kinds: readonly string[] = PICK_KINDS;
  const whole = stored.flatMap((entry: unknown): Picked[] => {
    if (typeof entry !== "object" || entry === null) {
      return [];
    }
    const { start, end, kind, id } = entry as Record<string, unknown>;
    return typeof start === "number" &&
      typeof end === "number" &&
      Number.isInteger(start) &&
      Number.isInteger(end) &&
      start >= 0 &&
      start < end &&
      end <= text.length &&
      typeof kind === "string" &&
      kinds.includes(kind) &&
      typeof id === "string"
      ? [{ start, end, kind: kind as PickKind, id }]
      : [];
  });
  whole.sort((a, b) => a.start - b.start);
  const sound: Picked[] = [];
  for (const pick of whole) {
    if (pick.start >= (sound.at(-1)?.end ?? 0)) {
      sound.push(pick);
    }
  }
  return sound;
}

/**
 * What a pick is sent as in a message's text, or `null` for one a message has no form for (a
 * command's channel, community, or deployment), which stays as written.
 */
function referenceOf(pick: PickTarget): string | null {
  switch (pick.kind) {
    case "user":
      return `<@${pick.id}>`;
    case "role":
      return `<@&${pick.id}>`;
    case "emoji":
      return `<:${pick.id}>`;
    case "channel":
    case "community":
    case "deployment":
      return null;
  }
}

/**
 * `held` as a message's text is sent: each pick as its reference, and the text between picks
 * as `between` makes of it (names typed by hand, which are looked up).
 */
export function encodePicked(
  held: PickedText,
  between: (text: string) => string = (text) => text,
): string {
  let sent = "";
  let at = 0;
  for (const pick of held.picks) {
    const reference = referenceOf(pick);
    if (reference !== null) {
      sent += between(held.text.slice(at, pick.start)) + reference;
      at = pick.end;
    }
  }
  return sent + between(held.text.slice(at));
}

/** The names a sent message's references are shown by; `undefined` for one not known. */
export interface PickNames {
  readonly usernameOf: (id: string) => string | undefined;
  readonly roleNameOf: (id: string) => string | undefined;
  readonly emojiNameOf: (id: string) => string | undefined;
}

/**
 * A sent message's text as a message box shows it: each tag as `@name` and each custom emoji as
 * `:name:`, a pick of what it named, so the text sent back unchanged is `content` again. A
 * reference to someone or something unknown is left as it was sent.
 */
export function decodePicked(content: string, names: PickNames): PickedText {
  const reference = new RegExp(`${TAG_REFERENCE}|${EMOJI_REFERENCE_SOURCE}`, "g");
  const picks: Picked[] = [];
  let text = "";
  let at = 0;
  for (const match of content.matchAll(reference)) {
    const [whole, role, tagged, emoji] = match;
    const target: PickTarget =
      emoji !== undefined
        ? { kind: "emoji", id: emoji }
        : { kind: role === "&" ? "role" : "user", id: tagged ?? "" };
    const name =
      target.kind === "emoji"
        ? names.emojiNameOf(target.id)
        : target.kind === "role"
          ? names.roleNameOf(target.id)
          : names.usernameOf(target.id);
    if (name === undefined) {
      continue;
    }
    const shown = target.kind === "emoji" ? `:${name}:` : `@${name}`;
    text += content.slice(at, match.index);
    picks.push({ ...target, start: text.length, end: text.length + shown.length });
    text += shown;
    at = match.index + whole.length;
  }
  return { text: text + content.slice(at), picks };
}

/** Replaces the text from `start` to `end`, as `splice` does. */
export type Splice = (
  start: number,
  end: number,
  shown: string,
  target?: PickTarget,
  after?: string,
) => void;

/**
 * What a message box holds: its text and the picks in it. `type` takes the box's own edits
 * (`retype`), `splice` what a completion or a keyboard writes, and `set` text put in whole.
 */
export function usePickedText(initial: PickedText) {
  const [held, set] = useState(initial);
  const type = useCallback((next: string, caret?: number) => {
    set((was) => retype(was, next, caret));
  }, []);
  const spliceIn: Splice = useCallback((start, end, shown, target, after) => {
    set((was) => splice(was, start, end, shown, target, after));
  }, []);
  return { held, type, splice: spliceIn, set };
}
