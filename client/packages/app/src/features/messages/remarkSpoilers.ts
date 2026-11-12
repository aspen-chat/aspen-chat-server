import type {
  Blockquote,
  Paragraph,
  Parent,
  PhrasingContent,
  Root,
  RootContent,
  Text,
} from "mdast";
import { visit } from "unist-util-visit";
import { markersOf, sliceText } from "@/features/messages/markerText";
import { extentOf } from "@/features/messages/sourceExtent";

/**
 * A spoiler in the syntax tree: inline content hidden until the reader reveals it. Rendered by
 * `Markdown.tsx` through the `data-spoiler` attribute on a `span`.
 */
export interface Spoiler extends Parent {
  type: "spoiler";
  children: PhrasingContent[];
}

/** A spelling of spoiler: what opens one and what closes it. */
export interface Marker {
  open: string;
  close: string;
}

/** The two spellings recognised: Discord's `||text||` and Reddit's `>!text!<`. */
const MARKERS: readonly Marker[] = [
  { open: "||", close: "||" },
  { open: ">!", close: "!<" },
];

/**
 * Whether a message body may hold a spoiler, told from its text alone: one without two pipes
 * together (in a table cell, written `\\|\\|`) and without Reddit's closing `!<` holds none,
 * so whoever only looks for spoilers need not parse it.
 */
export function mayHoldSpoiler(content: string): boolean {
  return content.includes("||") || content.includes("!<") || content.includes("\\|\\|");
}

/**
 * A remark plugin that wraps `||spoiler||` and `>!spoiler!<` spans in a `spoiler` node. The
 * markers may sit in different text nodes of the same paragraph, so `||**hidden**||` works, and
 * inline markup inside a spoiler is kept. Markers inside code are not text nodes and are left
 * alone, as is an opening marker with no closer.
 *
 * A marker is two characters written as themselves (`markersOf`): one written with a backslash
 * (`\|`) or as a character reference (`&#124;`) is text and opens and closes nothing. A table
 * cell is the exception for the pipe, which GFM lets a cell hold only as `\|`, so
 * `\|\|text\|\|` is how a cell writes a spoiler.
 *
 * Markers pair from left to right (`opening`): an opener closes at the nearest closer after
 * it, and one that makes no spoiler leaves the text after it to be read for the next.
 *
 * A line starting with `>!` parses as a blockquote before this plugin runs, so a blockquote
 * whose text starts with `!` and closes with `!<` is unwrapped first and its `>` restored.
 *
 * Each node made here records what is known of where it was written (`extentOf`): a spoiler
 * and the text left on either side of it share the ends of the text they were cut from, and a
 * quote's restored `>` is where the quote began. `maskSpoilers` writes a message out from them.
 *
 * The server's notification text hides the same spoilers
 * (`server/app/src/message/notification/spoilers.rs`), and `spec/spoiler_vectors.json` holds
 * texts both read alike. Change the two together.
 */
export function remarkSpoilers() {
  return (tree: Root) => {
    visit(tree, "blockquote", (node: Blockquote, index, parent) => {
      if (parent === undefined || index === undefined || !isRedditSpoilerQuote(node)) {
        return;
      }
      const paragraph = node.children[0] as Paragraph;
      const first = paragraph.children[0] as Text;
      if (first.data?.markers !== undefined) {
        first.data.markers = `>${first.data.markers}`;
      }
      first.value = `>${first.value}`;
      // The `>` is the quote's own, so what holds it starts where the quote did.
      for (const moved of [paragraph, first]) {
        moved.data = {
          ...moved.data,
          extent: { start: extentOf(node).start, end: extentOf(moved).end },
        };
      }
      parent.children.splice(index, 1, ...(node.children as RootContent[]));
      return index;
    });
    visit(tree, (node) => {
      if (!("children" in node) || (node.type as string) === "spoiler") {
        return;
      }
      wrapSpoilers(node);
    });
  };
}

function isRedditSpoilerQuote(node: Blockquote): boolean {
  const [paragraph] = node.children;
  if (paragraph?.type !== "paragraph") {
    return false;
  }
  const [first] = paragraph.children;
  return (
    first?.type === "text" &&
    markersOf(first).startsWith("!") &&
    paragraph.children.some((child) => child.type === "text" && markersOf(child).includes("!<"))
  );
}

/** Replaces each marked run among `parent`'s children with a `spoiler` node, left to right. */
function wrapSpoilers(parent: Parent): void {
  const children = parent.children as PhrasingContent[];
  for (let i = 0; i < children.length; i += 1) {
    const node = children[i];
    if (node?.type !== "text") {
      continue;
    }
    const found = findSpoiler(children, i);
    if (found === null) {
      continue;
    }
    const { start, openAt, end, closeAt, marker } = found;
    const first = children[start] as Text;
    const last = children[end] as Text;
    const before = sliceText(first, 0, openAt);
    const after = sliceText(last, closeAt + marker.close.length);
    const inner: PhrasingContent[] = [];
    if (start === end) {
      inner.push(sliceText(first, openAt + marker.open.length, closeAt));
    } else {
      const head = sliceText(first, openAt + marker.open.length);
      if (head.value.length > 0) {
        inner.push(head);
      }
      inner.push(...children.slice(start + 1, end));
      const tail = sliceText(last, 0, closeAt);
      if (tail.value.length > 0) {
        inner.push(tail);
      }
    }
    const startsAt = extentOf(first).start;
    const endsAt = extentOf(last).end;
    const spoiler: Spoiler = {
      type: "spoiler",
      children: inner,
      data: {
        hName: "span",
        hProperties: { dataSpoiler: "" },
        extent: {
          start: before.value.length > 0 ? undefined : startsAt,
          end: after.value.length > 0 ? undefined : endsAt,
        },
      },
    };
    const replacement: PhrasingContent[] = [];
    if (before.value.length > 0) {
      before.data = { ...before.data, extent: { start: startsAt } };
      replacement.push(before);
    }
    replacement.push(spoiler as unknown as PhrasingContent);
    if (after.value.length > 0) {
      after.data = { ...after.data, extent: { end: endsAt } };
      replacement.push(after);
    }
    children.splice(start, end - start + 1, ...replacement);
    i = start + replacement.length - (after.value.length > 0 ? 2 : 1);
  }
}

interface Found {
  start: number;
  openAt: number;
  end: number;
  closeAt: number;
  marker: Marker;
}

/**
 * The first place `marker` opens something in a run of text, read as `markers` for where the
 * markers are and as `value` for what stands between them: where it opens and where it closes,
 * `closeAt` `null` when nothing in the run closes it, or `null` when it opens nothing there.
 *
 * Openers are tried from left to right, each closing at the nearest closer after it:
 *
 * - Something between the two that is not blank is a spoiler.
 * - Only blank text between them (`|| ||`) is none, and both markers are text: the search goes
 *   on after the closer.
 * - Nothing between them (`||||`) is none, and the first marker is text: the second is tried
 *   as an opener.
 * - No closer after it leaves it open, as it leaves every later opener of the run, so that is
 *   the answer: it may close in a later run of the same parent.
 *
 * Every step ends at least a marker's length further on, so a run is read once.
 */
export function opening(
  markers: string,
  value: string,
  marker: Marker,
): { openAt: number; closeAt: number | null } | null {
  let from = 0;
  for (;;) {
    const openAt = markers.indexOf(marker.open, from);
    if (openAt === -1) {
      return null;
    }
    const inside = openAt + marker.open.length;
    const closeAt = markers.indexOf(marker.close, inside);
    if (closeAt === -1) {
      return { openAt, closeAt: null };
    }
    if (closeAt === inside) {
      from = inside;
    } else if (value.slice(inside, closeAt).trim().length > 0) {
      return { openAt, closeAt };
    } else {
      from = closeAt + marker.close.length;
    }
  }
}

/**
 * The first spoiler whose opening marker is in the text node at `start`: each spelling's first
 * opening (`opening`), closed in that node or else in the next text sibling holding its closer,
 * whatever stands between. Of the two spellings the one that opens first wins.
 */
function findSpoiler(children: readonly PhrasingContent[], start: number): Found | null {
  const node = children[start] as Text;
  const text = markersOf(node);
  let best: Found | null = null;
  for (const marker of MARKERS) {
    const opened = opening(text, node.value, marker);
    if (opened === null || (best !== null && opened.openAt >= best.openAt)) {
      continue;
    }
    const { openAt, closeAt } = opened;
    if (closeAt !== null) {
      best = { start, openAt, end: start, closeAt, marker };
      continue;
    }
    for (let end = start + 1; end < children.length; end += 1) {
      const sibling = children[end];
      if (sibling?.type !== "text") {
        continue;
      }
      const closesAt = markersOf(sibling).indexOf(marker.close);
      if (closesAt !== -1) {
        best = { start, openAt, end, closeAt: closesAt, marker };
        break;
      }
    }
  }
  return best;
}
