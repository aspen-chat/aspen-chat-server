import remarkParse from "remark-parse";
import { unified } from "unified";
import { visit } from "unist-util-visit";
import { opensTooDeeply } from "@/features/messages/markdownLimits";
import { splice, type PickedText, type Span } from "@/features/messages/picks";

/**
 * Emoticons a message box turns into emoji as they are typed (`useEmoticons`). Each is one
 * people type as a face and rarely mean otherwise when it stands alone: `8)` and `B)` are left
 * out because they begin numbered and lettered lists, and `D:` because it is a drive.
 */
export const EMOTICONS: ReadonlyMap<string, string> = new Map([
  [":)", "🙂"],
  [":-)", "🙂"],
  [":(", "🙁"],
  [":-(", "🙁"],
  [":D", "😃"],
  [":-D", "😃"],
  [";)", "😉"],
  [";-)", "😉"],
  [":P", "😛"],
  [":-P", "😛"],
  [":p", "😛"],
  [":-p", "😛"],
  [":O", "😮"],
  [":-O", "😮"],
  [":o", "😮"],
  [":-o", "😮"],
  [":|", "😐"],
  [":-|", "😐"],
  [":/", "😕"],
  [":-/", "😕"],
  [":'(", "😢"],
  ["<3", "❤️"],
  ["</3", "💔"],
  [">:(", "😠"],
  [":*", "😘"],
  [":-*", "😘"],
  ["XD", "😆"],
  ["xD", "😆"],
]);

/** The emoticons longest first, so `>:(` is found before the `:(` it ends with. */
const LONGEST_FIRST = [...EMOTICONS.keys()].sort((a, b) => b.length - a.length);

/** An emoticon found in text, and the emoji it becomes. */
export interface FoundEmoticon extends Span {
  readonly emoji: string;
}

/** What may stand right after an emoticon for it to be one: a space, a line's end, punctuation. */
const ENDS_EMOTICON = /^[\s\p{P}]$/u;
const SPACE = /^\s$/u;

/** Whether `character` (one UTF-16 code unit, or none) ends a word an emoticon may close. */
export function endsEmoticon(character: string | undefined): boolean {
  return character === undefined || ENDS_EMOTICON.test(character);
}

/**
 * The emoticon whose last character is just before `end`, standing on its own: at the start
 * of the text or after a space, so `http:/` and `a:p` are not faces. Only the place before it is
 * judged here; what follows it is the caller's to judge (`endsEmoticon`).
 */
export function emoticonEndingAt(text: string, end: number): FoundEmoticon | null {
  for (const emoticon of LONGEST_FIRST) {
    const start = end - emoticon.length;
    if (start < 0 || text.slice(start, end) !== emoticon) {
      continue;
    }
    const before = text[start - 1];
    if (before !== undefined && !SPACE.test(before)) {
      continue;
    }
    return { start, end, emoji: EMOTICONS.get(emoticon) ?? emoticon };
  }
  return null;
}

/** Every emoticon in `text` that stands on its own, with a space, punctuation, or the end after. */
export function emoticonsIn(text: string): FoundEmoticon[] {
  const found: FoundEmoticon[] = [];
  for (let end = 1; end <= text.length; end++) {
    if (!endsEmoticon(text[end])) {
      continue;
    }
    const emoticon = emoticonEndingAt(text, end);
    if (emoticon !== null && emoticon.start >= (found.at(-1)?.end ?? 0)) {
      found.push(emoticon);
    }
  }
  return found;
}

const processor = unified().use(remarkParse);

/** Whether `offset` falls in a code span or a code block of `source` as Markdown reads it. */
function codeHolds(source: string, offset: number): boolean {
  if (opensTooDeeply(source)) {
    return false;
  }
  const tree = processor.parse(source);
  let held = false;
  visit(tree, (node) => {
    if (held) {
      return;
    }
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    if (start === undefined || end === undefined || offset < start || offset >= end) {
      return "skip";
    }
    if (node.type === "code" || node.type === "inlineCode") {
      held = true;
    }
    return undefined;
  });
  return held;
}

/** Each length of the runs of backticks in `text`, any of which may open a code span. */
function backtickRuns(text: string): Set<number> {
  return new Set([...text.matchAll(/`+/g)].map((run) => run[0].length));
}

/**
 * Whether the emoticon at `at` is code rather than a face: inside a code span or block, or inside
 * one still being written, which closing a run of backticks opened before it would make. A fence
 * left open runs to the end of the text already.
 */
export function inCode(text: string, at: Span): boolean {
  if (!text.includes("`") && !text.includes("~~~")) {
    return false;
  }
  if (codeHolds(text, at.start)) {
    return true;
  }
  const before = text.slice(0, at.end);
  for (const run of backtickRuns(text.slice(0, at.start))) {
    if (codeHolds(before + "`".repeat(run), at.start)) {
      return true;
    }
  }
  return false;
}

/** Whether the emoticon at `at` may become its emoji in `held`: not kept as typed, picked, or code. */
function convertible(held: PickedText, at: Span, kept: readonly Span[]): boolean {
  return (
    !kept.some((span) => span.start === at.start && span.end === at.end) &&
    !held.picks.some((pick) => pick.start < at.end && pick.end > at.start) &&
    !inCode(held.text, at)
  );
}

/** An emoticon turned into its emoji as it was typed, and what Backspace puts back. */
export interface Conversion {
  readonly held: PickedText;
  /** Where the caret stands after the emoji and what was typed after it. */
  readonly caret: number;
  /** The text as typed, before the emoticon became its emoji. */
  readonly typed: PickedText;
  readonly typedCaret: number;
  /** The emoticon, in `typed`. */
  readonly emoticon: Span;
}

/**
 * The conversion an edit makes, when it typed one character that ends an emoticon (a space, a
 * line's end, punctuation) just after one: `typed` is the box's text after the edit, `was` its
 * text before, and `caret` where the caret stands after it. Pastes, deletions, and characters
 * typed anywhere else convert nothing; neither does an emoticon among `kept`, which Backspace put
 * back as it was typed.
 */
export function convertTyped(
  typed: PickedText,
  was: string,
  caret: number,
  kept: readonly Span[],
): Conversion | null {
  const { text } = typed;
  if (
    text.length !== was.length + 1 ||
    caret < 1 ||
    !endsEmoticon(text[caret - 1]) ||
    text.slice(0, caret - 1) + text.slice(caret) !== was
  ) {
    return null;
  }
  const found = emoticonEndingAt(text, caret - 1);
  if (found === null || !convertible(typed, found, kept)) {
    return null;
  }
  return {
    held: splice(typed, found.start, found.end, found.emoji),
    caret: caret - (found.end - found.start) + found.emoji.length,
    typed,
    typedCaret: caret,
    emoticon: { start: found.start, end: found.end },
  };
}

/**
 * `held` as it is sent: an emoticon that ends it, with nothing typed after it to turn it into its
 * emoji, becomes one now, unless it is among `kept`.
 */
export function convertLast(held: PickedText, kept: readonly Span[]): PickedText {
  const found = emoticonEndingAt(held.text, held.text.trimEnd().length);
  if (found === null || !convertible(held, found, kept)) {
    return held;
  }
  return splice(held, found.start, found.end, found.emoji);
}
