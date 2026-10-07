import type { Code, Nodes, Parent, Root, Table, TableRow } from "mdast";
import type { Options } from "react-markdown";

/**
 * How deeply a message's syntax tree may nest (quotes in quotes, lists in lists, emphasis in
 * emphasis) before the message is shown as plain text instead. Parsing and turning the tree
 * into elements recurse once per level, so a few thousand levels, which a message of a few
 * kilobytes can write (`>>>>…`, `1. 1. 1. …`), would overflow the stack; no message a person
 * writes to be read comes near this.
 */
export const MAX_NESTING = 32;

/**
 * The widest table, in columns, and the largest, in cells, drawn as a table; a bigger one shows
 * as its source in a code block. Tables cost the page one element per cell however little text
 * each holds.
 */
export const MAX_TABLE_COLUMNS = 64;
export const MAX_TABLE_CELLS = 5000;

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
 * `MAX_NESTING` with one paragraph holding the message's source as text, and a table wider than
 * `MAX_TABLE_COLUMNS` or larger than `MAX_TABLE_CELLS` with a code block holding its source. It
 * walks the tree with a stack of its own, since recursing is what it guards against.
 */
export function remarkLimits() {
  return (tree: Root, file: { value: unknown }) => {
    const source = typeof file.value === "string" ? file.value : String(file.value);
    const tables: { parent: Parent; index: number; table: Table }[] = [];
    const stack: [Nodes, number][] = [[tree, 0]];
    for (let next = stack.pop(); next !== undefined; next = stack.pop()) {
      const [node, depth] = next;
      if (depth > MAX_NESTING) {
        tree.children = [{ type: "paragraph", children: [{ type: "text", value: source }] }];
        return;
      }
      if ("children" in node) {
        node.children.forEach((child, index) => {
          if (child.type === "table" && tooLarge(child)) {
            tables.push({ parent: node, index, table: child });
          } else {
            stack.push([child, depth + 1]);
          }
        });
      }
    }
    for (const { parent, index, table } of tables) {
      const start = table.position?.start.offset;
      const end = table.position?.end.offset;
      const code: Code = {
        type: "code",
        lang: null,
        value: start === undefined || end === undefined ? "" : source.slice(start, end),
      };
      parent.children[index] = code;
    }
  };
}

function tooLarge(table: Table): boolean {
  let cells = 0;
  for (const row of table.children) {
    if (row.children.length > MAX_TABLE_COLUMNS) {
      return true;
    }
    cells += row.children.length;
  }
  return (table.align?.length ?? 0) > MAX_TABLE_COLUMNS || cells > MAX_TABLE_CELLS;
}

type Handler = NonNullable<
  NonNullable<NonNullable<Options["remarkRehypeOptions"]>["handlers"]>["tableRow"]
>;
type Element = Extract<NonNullable<ReturnType<Handler>>, { type: "element" }>;

/**
 * A table row as the cells it has. The default handler pads every row to the header's width,
 * so a wide header over many short rows (a few kilobytes of `|`) would make millions of empty
 * cells; a short row here is left short.
 */
export const tableRow: Handler = (state, node, parent) => {
  const row = node as TableRow;
  const table = parent?.type === "table" ? parent : undefined;
  const tagName = table?.children[0] === row ? "th" : "td";
  const cells = row.children.map((cell, index) => {
    const align = table?.align?.[index];
    const element: Element = {
      type: "element",
      tagName,
      properties: align == null ? {} : { align },
      children: state.all(cell),
    };
    state.patch(cell, element);
    return state.applyData(cell, element);
  });
  const element: Element = {
    type: "element",
    tagName: "tr",
    properties: {},
    children: state.wrap(cells, true),
  };
  state.patch(row, element);
  return state.applyData(row, element);
};
