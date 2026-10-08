/**
 * Runs of one author's messages drawn as a group: the first with the author's picture, name,
 * and time, the rest under it with none of them. A message continues the group before it
 * when it is by the same author, on the same day, within `GROUP_SPAN_MS` of the group's first
 * message, nothing stands between them (`breakBefore`: the New Messages line, a run of blocked
 * messages), neither stands alone (notices, calls, closed polls, thread echoes, commands,
 * warnings), and the group's estimated height with it stays within the most it may take.
 *
 * Heights are estimated from what the messages hold (`estimateHeight`), never measured, so a
 * group is the same however its pictures load.
 *
 * Groups are worked out oldest first, so where one begins depends on the messages before it;
 * read alone, a page of older history arriving above the view could move every boundary below
 * it, into the view. So a message keeps what it was decided to be (`decided`) for as long as
 * its place allows: a continuation stays one while the message before it is still its author's
 * with nothing between, and a message that began a group still begins one. Only messages not
 * yet decided are decided by the rules.
 */

/** The longest a group spans, from its first message to its last. */
export const GROUP_SPAN_MS = 30 * 60 * 1000;
/** The share of the list's height a group may take at most, as estimated. */
export const GROUP_HEIGHT_SHARE = 0.75;
/** The estimated height a group's header (the name and time) adds to its first message. */
const HEADER_PX = 24;

/** One message as grouping sees it. */
export interface GroupCandidate {
  readonly id: string;
  readonly author: string;
  /** When it was sent, in milliseconds. */
  readonly at: number;
  /** Whether it is drawn with its header always, in a group of its own. */
  readonly alone: boolean;
  /** Its estimated height without a header, in CSS pixels. */
  readonly height: number;
  /** Whether something drawn just above it ends any group there. */
  readonly breakBefore: boolean;
}

interface Group {
  readonly author: string;
  readonly start: number;
  last: number;
  height: number;
  readonly alone: boolean;
}

/**
 * The messages of `messages`, oldest first, that continue the group before them, given the
 * most a group may take, `maxHeight` (none when it is not positive). A message taller than
 * that alone still begins a group of its own. `decided` holds what each message was decided to
 * be before, true for a continuation, which is kept where its place still allows it, and is
 * given what each is decided to be now.
 */
export function groupContinuations(
  messages: readonly GroupCandidate[],
  maxHeight: number,
  decided = new Map<string, boolean>(),
): Set<string> {
  const continuing = new Set<string>();
  let group: Group | null = null;
  for (const message of messages) {
    // Whether it may continue the group at all: the same author's, with nothing between.
    const may =
      group !== null &&
      !message.breakBefore &&
      !message.alone &&
      !group.alone &&
      message.author === group.author;
    const before = decided.get(message.id);
    const continues =
      before === undefined
        ? may &&
          group !== null &&
          message.at - group.start <= GROUP_SPAN_MS &&
          sameDay(group.last, message.at) &&
          (maxHeight <= 0 || group.height + message.height <= maxHeight)
        : before && may;
    decided.set(message.id, continues);
    if (continues && group !== null) {
      continuing.add(message.id);
      group.last = message.at;
      group.height += message.height;
    } else {
      group = {
        author: message.author,
        start: message.at,
        last: message.at,
        height: HEADER_PX + message.height,
        alone: message.alone,
      };
    }
  }
  return continuing;
}

/** Whether two moments fall on the same day where the reader is. */
function sameDay(a: number, b: number): boolean {
  return new Date(a).toDateString() === new Date(b).toDateString();
}

/** How the list sets text, for estimating heights. */
export interface TextMetrics {
  /** The width of a message's column, beside the picture, in CSS pixels. */
  readonly column: number;
  /** The height of a line of message text. */
  readonly lineHeight: number;
  /** The width of an average narrow character; a wide one (CJK, emoji) counts twice. */
  readonly charWidth: number;
}

/** What a message holds, as far as its height goes. */
export interface MessageContents {
  readonly content: string;
  /** The pictures and videos it shows, at their own sizes where known. */
  readonly pictures: readonly { readonly width?: number; readonly height?: number }[];
  /** How many of its files are drawn as a file's row rather than a picture. */
  readonly files: number;
  /** How many link previews it shows as cards. */
  readonly cards: number;
  readonly poll: boolean;
  readonly reactions: boolean;
  readonly thread: boolean;
}

/** The tallest a picture is drawn inline (`max-h-80`). */
const PICTURE_MAX_PX = 320;
/** A picture of unknown size, as its skeleton keeps (`h-48`). */
const PICTURE_UNKNOWN_PX = 192;
const FILE_PX = 56;
const CARD_PX = 110;
const POLL_PX = 220;
/** A row of reaction chips, or a thread's summary. */
const STRIP_PX = 32;
/** The padding and gap around a message in a group. */
const ROW_PX = 8;

/**
 * A message's height without its header, estimated from what it holds and how `metrics` sets
 * it: its text wrapped at the column's width, line by line, and each thing below the text at
 * the size it is drawn.
 */
export function estimateHeight(contents: MessageContents, metrics: TextMetrics): number {
  const perLine = Math.max(1, Math.floor(metrics.column / metrics.charWidth));
  let lines = 0;
  if (contents.content !== "") {
    for (const line of contents.content.split("\n")) {
      lines += Math.max(1, Math.ceil(textWidth(line) / perLine));
    }
  }
  let height = ROW_PX + lines * metrics.lineHeight;
  for (const picture of contents.pictures) {
    height += pictureHeight(picture.width, picture.height, metrics.column);
  }
  height += contents.files * FILE_PX + contents.cards * CARD_PX;
  if (contents.poll) {
    height += POLL_PX;
  }
  if (contents.reactions) {
    height += STRIP_PX;
  }
  if (contents.thread) {
    height += STRIP_PX;
  }
  return height;
}

/** How tall a picture is drawn: at its proportions, within the column and `PICTURE_MAX_PX`. */
function pictureHeight(width: number | undefined, height: number | undefined, column: number) {
  if (width === undefined || height === undefined || width <= 0 || height <= 0) {
    return PICTURE_UNKNOWN_PX;
  }
  const drawnWidth = Math.min(width, column, (PICTURE_MAX_PX * width) / height);
  return (drawnWidth * height) / width;
}

/** A line's width in narrow characters: CJK and other wide characters count as two. */
function textWidth(line: string): number {
  let width = 0;
  for (const char of line) {
    width += (char.codePointAt(0) ?? 0) >= 0x2e80 ? 2 : 1;
  }
  return width;
}
