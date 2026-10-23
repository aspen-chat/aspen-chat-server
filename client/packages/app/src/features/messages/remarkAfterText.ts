import type { Parent, Root, RootContent } from "mdast";

/**
 * Where what follows a message's text goes (its edited mark, its saved mark, a grouped
 * message's time): a node rendered by `Markdown.tsx` through the `data-after-text` attribute on
 * a `span`, whose value says whether it ends the text's last line (`inline`) or stands under
 * the last block (`block`).
 */
interface AfterTextNode {
  type: "afterText";
  children: [];
  data: { hName: "span"; hProperties: { dataAfterText: "inline" | "block" } };
}

/** Blocks whose last line is text, which the marks can end. */
const LINES = new Set(["paragraph", "heading"]);
/** Blocks whose last line is their last child's. */
const HOLDERS = new Set(["blockquote", "list", "listItem"]);

/**
 * A remark plugin that places what follows the text (`AfterTextNode`) at the end of its last
 * line, reaching into the last of the quotes and lists it ends with, so the marks sit after the
 * last word wherever the line has room, and wrap under it, as a word would, where it has not.
 * Text that ends with a block whose lines are not text (code, a table, a rule) has them under
 * that block.
 */
export function remarkAfterText() {
  return (tree: Root) => {
    const host = lastLine(tree);
    if (host === null) {
      tree.children.push(slot("block") as unknown as RootContent);
    } else {
      host.children.push(slot("inline") as never);
    }
  };
}

function lastLine(parent: Parent): Parent | null {
  const last = parent.children.at(-1);
  if (last === undefined || !("children" in last)) {
    return null;
  }
  if (LINES.has(last.type)) {
    return last;
  }
  return HOLDERS.has(last.type) ? lastLine(last) : null;
}

function slot(place: "inline" | "block"): AfterTextNode {
  return {
    type: "afterText",
    children: [],
    data: { hName: "span", hProperties: { dataAfterText: place } },
  };
}
