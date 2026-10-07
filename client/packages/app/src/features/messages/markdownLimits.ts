import type { Nodes, Root } from "mdast";

/**
 * How deeply a message's syntax tree may nest (quotes in quotes, lists in lists, emphasis in
 * emphasis) before the message is shown as plain text instead. Parsing and turning the tree
 * into elements recurse once per level, so a few thousand levels, which a message of a few
 * kilobytes can write (`>>>>…`, `1. 1. 1. …`), would overflow the stack; no message a person
 * writes to be read comes near this.
 */
export const MAX_NESTING = 32;

/**
 * A quote or list marker at the start of a line, or after another one: up to three spaces of
 * indent, then `>`, or a bullet or number followed by a space or the line's end.
 */
const CONTAINER_MARKER = /[ \t]{0,3}(?:>|(?:[-*+]|\d{1,9}[.)])(?=[ \t\r\n]|$))[ \t]?/y;

/**
 * Whether a line of `source` opens more than `MAX_NESTING` quotes and lists at once, read from
 * the text alone, in one pass, before anything parses it. This is where nesting a message's
 * length could make deep comes from; nesting built some other way (indented lists, which cost
 * more text per level the deeper they go) is caught after parsing by `remarkLimits`.
 */
export function opensTooDeeply(source: string): boolean {
  let start = 0;
  while (start < source.length) {
    CONTAINER_MARKER.lastIndex = start;
    let markers = 0;
    while (CONTAINER_MARKER.exec(source) !== null) {
      markers += 1;
      if (markers > MAX_NESTING) {
        return true;
      }
    }
    const end = source.indexOf("\n", start);
    if (end === -1) {
      break;
    }
    start = end + 1;
  }
  return false;
}

/**
 * A remark plugin, run before every other transform, that replaces a tree nesting deeper than
 * `MAX_NESTING` with one paragraph holding the message's source as text. It walks the tree with
 * a stack of its own, since recursing is what it guards against.
 */
export function remarkLimits() {
  return (tree: Root, file: { value: unknown }) => {
    if (depthExceeds(tree, MAX_NESTING)) {
      const source = typeof file.value === "string" ? file.value : String(file.value);
      tree.children = [{ type: "paragraph", children: [{ type: "text", value: source }] }];
    }
  };
}

/** Whether any node lies more than `limit` levels below `tree`. */
export function depthExceeds(tree: Root, limit: number): boolean {
  const stack: [Nodes, number][] = [[tree, 0]];
  for (let next = stack.pop(); next !== undefined; next = stack.pop()) {
    const [node, depth] = next;
    if (depth > limit) {
      return true;
    }
    if ("children" in node) {
      for (const child of node.children) {
        stack.push([child, depth + 1]);
      }
    }
  }
  return false;
}
