import type { Nodes, Root, Text } from "mdast";
import type { Data, Processor } from "unified";
import { SKIP, visit } from "unist-util-visit";
import { extentOf } from "@/features/messages/sourceExtent";

declare module "mdast" {
  interface TextData {
    /**
     * Where in the text's value a character a spoiler's marker is spelled with was written as
     * an escape or a character reference, as the parser reports it. `remarkMarkerText` turns
     * these into `markers` before any other plugin runs.
     */
    literal?: number[];
    /** The text as spoiler markers are looked for in it, when not its value (`markersOf`). */
    markers?: string;
  }
}

/** The characters a spoiler's markers are spelled with. */
const MARKER_CHARACTERS = "|>!<";
/** What stands in `markersOf` for a character that opens and closes nothing. */
const LITERAL = "\0";

/**
 * A text node's value as spoiler markers are looked for in it: the same length, with a
 * character that can spell a marker but was written as a backslash escape (`\|`) or a character
 * reference (`&#124;`) replaced by U+0000, which no marker holds and no parsed text does. In a
 * table cell an escaped `\|` stays a `|`, since GFM writes a cell's pipe no other way.
 */
export function markersOf(node: Text): string {
  return node.data?.markers ?? node.value;
}

/** The text from `from` to `to` of `node`'s value, with the same stretch of its markers. */
export function sliceText(node: Text, from: number, to?: number): Text {
  const text: Text = { type: "text", value: node.value.slice(from, to) };
  if (node.data?.markers !== undefined) {
    text.data = { markers: node.data.markers.slice(from, to) };
  }
  return text;
}

/** Adds `added`'s text to the end of `node`'s, markers included. */
export function appendText(node: Text, added: Text): void {
  if (node.data?.markers !== undefined || added.data?.markers !== undefined) {
    node.data = { ...node.data, markers: markersOf(node) + markersOf(added) };
  }
  node.value += added.value;
}

type FromMarkdownExtension = Exclude<
  NonNullable<Data["fromMarkdownExtensions"]>[number],
  unknown[]
>;
type Handle = NonNullable<FromMarkdownExtension["exit"]>[string];
type CompileContext = ThisParameterType<Handle>;

/** Notes that `text`'s value from `from` on was not written as itself. */
function noteLiteral(text: Text, from: number): void {
  for (let at = from; at < text.value.length; at += 1) {
    if (MARKER_CHARACTERS.includes(text.value.charAt(at))) {
      text.data = { ...text.data, literal: [...(text.data?.literal ?? []), at] };
    }
  }
}

/**
 * A remark plugin, listed before `remark-gfm`, that keeps what the parser alone knows of a
 * message's text: which characters were written as an escape or a character reference, so that
 * `remarkSpoilers` reads no marker in them (`markersOf`). Each such character becomes a text
 * node of its own, which GFM's search for bare addresses, run after parsing and remaking the
 * text nodes it finds one in, leaves as it is; `remarkLiteralLinks` joins the text again.
 */
export function remarkMarkerText(this: Processor) {
  // Where the value of the text each open character reference is in stood when it began.
  const references = new WeakMap<Text, number>();
  const extension: FromMarkdownExtension = {
    exit: {
      // The escaped character has just been added to the text before this.
      characterEscape(this: CompileContext) {
        const parent = this.stack.at(-1);
        const text = parent !== undefined && "children" in parent ? parent.children.at(-1) : null;
        if (text?.type !== "text") {
          return;
        }
        const at = text.value.length - 1;
        const inCell = this.stack.some((node) => node.type === "tableCell");
        if (!(inCell && text.value.endsWith("|"))) {
          noteLiteral(text, at);
        }
      },
      // A reference's `&` and its `;`, with what it stands for added between the two.
      characterReferenceMarker(this: CompileContext) {
        const text = this.stack.at(-1);
        if (text?.type !== "text") {
          return;
        }
        const from = references.get(text);
        if (from === undefined) {
          references.set(text, text.value.length);
        } else {
          references.delete(text);
          noteLiteral(text, from);
        }
      },
    },
    transforms: [splitLiterals],
  };
  const data = this.data();
  data.fromMarkdownExtensions = [...(data.fromMarkdownExtensions ?? []), extension];
}

/** Makes each character noted as `literal` a text node of its own, its markers set. */
function splitLiterals(tree: Root): undefined {
  visit(tree, "text", (node, index, parent) => {
    const literal = node.data?.literal;
    if (literal === undefined || parent === undefined || index === undefined) {
      return;
    }
    const pieces: Text[] = [];
    let at = 0;
    for (const position of literal) {
      if (position > at) {
        pieces.push({ type: "text", value: node.value.slice(at, position) });
      }
      pieces.push({ type: "text", value: node.value.charAt(position), data: { markers: LITERAL } });
      at = position + 1;
    }
    if (at < node.value.length) {
      pieces.push({ type: "text", value: node.value.slice(at) });
    }
    // Only the ends the pieces share with the text they were cut from are known.
    const { start, end } = extentOf(node);
    pieces.forEach((piece, i) => {
      piece.data = {
        ...piece.data,
        extent: {
          start: i === 0 ? start : undefined,
          end: i === pieces.length - 1 ? end : undefined,
        },
      };
    });
    (parent.children as Nodes[]).splice(index, 1, ...pieces);
    return [SKIP, index + pieces.length];
  });
}
