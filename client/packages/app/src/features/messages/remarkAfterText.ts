import type { Parent, Root, RootContent } from "mdast";

/**
 * Where what follows a message's text goes (its edited mark, its saved mark, a grouped
 * message's time): a node rendered by `Markdown.tsx` through the `data-after-text` attribute on
 * a `span`.
 */
interface AfterTextNode {
  type: "afterText";
  children: [];
  data: { hName: "span"; hProperties: { dataAfterText: true } };
}

/**
 * A block whose lines are not text (code, a table, a rule) with what follows the text beside
 * it, rendered as a `div` of the `message-after-row` class, which `styles.css` lays out.
 */
interface AfterTextRowNode {
  type: "afterTextRow";
  children: [RootContent, AfterTextNode];
  data: { hName: "div"; hProperties: { className: ["message-after-row"] } };
}

/** Blocks whose last line is text, which the marks can end. */
const LINES = new Set(["paragraph", "heading"]);
/** Blocks whose last line is their last child's. */
const HOLDERS = new Set(["blockquote", "list", "listItem"]);

/**
 * A remark plugin that places what follows the text (`AfterTextNode`) at the end of its last
 * line, reaching into the last of the quotes and lists it ends with, so the marks sit after the
 * last word wherever the line has room, and wrap under it, as a word would, where it has not.
 * Text that ends with a block whose lines are not text (code, a table, a rule) has them beside
 * that block in a row (`AfterTextRowNode`), wrapping under it only where the line has no room.
 */
export function remarkAfterText() {
  return (tree: Root) => {
    place(tree);
  };
}

function place(parent: Parent) {
  const last = parent.children.at(-1);
  if (last === undefined) {
    parent.children.push(slot() as never);
  } else if (LINES.has(last.type)) {
    (last as Parent).children.push(slot() as never);
  } else if (HOLDERS.has(last.type)) {
    place(last as Parent);
  } else {
    parent.children[parent.children.length - 1] = row(last) as never;
  }
}

function slot(): AfterTextNode {
  return {
    type: "afterText",
    children: [],
    data: { hName: "span", hProperties: { dataAfterText: true } },
  };
}

function row(block: RootContent): AfterTextRowNode {
  return {
    type: "afterTextRow",
    children: [block, slot()],
    data: { hName: "div", hProperties: { className: ["message-after-row"] } },
  };
}
