import type { Blockquote, Parent, PhrasingContent, Root, RootContent, Text } from "mdast";
import { visit } from "unist-util-visit";

/**
 * A spoiler in the syntax tree: inline content hidden until the reader reveals it. Rendered by
 * `Markdown.tsx` through the `data-spoiler` attribute on a `span`.
 */
export interface Spoiler extends Parent {
  type: "spoiler";
  children: PhrasingContent[];
}

/** The two spellings recognised: Discord's `||text||` and Reddit's `>!text!<`. */
const MARKERS: readonly { open: string; close: string }[] = [
  { open: "||", close: "||" },
  { open: ">!", close: "!<" },
];

/**
 * A remark plugin that wraps `||spoiler||` and `>!spoiler!<` spans in a `spoiler` node. The
 * markers may sit in different text nodes of the same paragraph, so `||**hidden**||` works, and
 * inline markup inside a spoiler is kept. Markers inside code are not text nodes and are left
 * alone, as is an opening marker with no closer.
 *
 * A line starting with `>!` parses as a blockquote before this plugin runs, so a blockquote
 * whose text starts with `!` and closes with `!<` is unwrapped first and its `>` restored.
 */
export function remarkSpoilers() {
  return (tree: Root) => {
    visit(tree, "blockquote", (node: Blockquote, index, parent) => {
      if (parent === undefined || index === undefined || !isRedditSpoilerQuote(node)) {
        return;
      }
      const [paragraph] = node.children;
      const [first] = (paragraph as Parent).children;
      (first as Text).value = `>${(first as Text).value}`;
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
    first.value.startsWith("!") &&
    paragraph.children.some((child) => child.type === "text" && child.value.includes("!<"))
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
    const before = (children[start] as Text).value.slice(0, openAt);
    const after = (children[end] as Text).value.slice(closeAt + marker.close.length);
    const inner: PhrasingContent[] = [];
    if (start === end) {
      inner.push({
        type: "text",
        value: (children[start] as Text).value.slice(openAt + marker.open.length, closeAt),
      });
    } else {
      const head = (children[start] as Text).value.slice(openAt + marker.open.length);
      if (head.length > 0) {
        inner.push({ type: "text", value: head });
      }
      inner.push(...children.slice(start + 1, end));
      const tail = (children[end] as Text).value.slice(0, closeAt);
      if (tail.length > 0) {
        inner.push({ type: "text", value: tail });
      }
    }
    const spoiler: Spoiler = {
      type: "spoiler",
      children: inner,
      data: { hName: "span", hProperties: { dataSpoiler: "" } },
    };
    const replacement: PhrasingContent[] = [];
    if (before.length > 0) {
      replacement.push({ type: "text", value: before });
    }
    replacement.push(spoiler as unknown as PhrasingContent);
    if (after.length > 0) {
      replacement.push({ type: "text", value: after });
    }
    children.splice(start, end - start + 1, ...replacement);
    i = start + replacement.length - (after.length > 0 ? 2 : 1);
  }
}

interface Found {
  start: number;
  openAt: number;
  end: number;
  closeAt: number;
  marker: { open: string; close: string };
}

/**
 * The first spoiler whose opening marker is in the text node at `start`: the closer is looked
 * for after the opener in the same node, then in the following text siblings. The content must
 * not be blank.
 */
function findSpoiler(children: readonly PhrasingContent[], start: number): Found | null {
  const text = (children[start] as Text).value;
  let best: Found | null = null;
  for (const marker of MARKERS) {
    const openAt = text.indexOf(marker.open);
    if (openAt === -1 || (best !== null && openAt >= best.openAt)) {
      continue;
    }
    const sameNode = text.indexOf(marker.close, openAt + marker.open.length);
    if (sameNode !== -1) {
      if (text.slice(openAt + marker.open.length, sameNode).trim().length > 0) {
        best = { start, openAt, end: start, closeAt: sameNode, marker };
      }
      continue;
    }
    for (let end = start + 1; end < children.length; end += 1) {
      const sibling = children[end];
      if (sibling?.type !== "text") {
        continue;
      }
      const closeAt = sibling.value.indexOf(marker.close);
      if (closeAt !== -1) {
        best = { start, openAt, end, closeAt, marker };
        break;
      }
    }
  }
  return best;
}
