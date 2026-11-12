import type { Nodes, Parent, Root, RootContent, Text } from "mdast";
import { SKIP, visit } from "unist-util-visit";
import { appendText } from "@/features/messages/markerText";
import { extentOf } from "@/features/messages/sourceExtent";

/**
 * The syntax that names a link with words of its author's choosing: `[text](url)`,
 * `[text][ref]` and `[ref]`, pictures (`![alt](url)`), and the `[ref]: url` lines that give
 * references their addresses.
 */
const NAMED = new Set(["link", "linkReference", "image", "imageReference", "definition"]);

/**
 * A remark plugin that shows a link named by its author's words as the text it was written
 * as, so a message's links read as the addresses they open: `[my bank](https://evil.example)`
 * shows those characters, and the address in it is then linked as a bare one is
 * (`remarkBareLinks`), an address of the deployment as a name path (`SelfLink`). An autolink
 * (`<https://…>`) is kept, since its words cannot differ from where it leads, and so is an
 * email address GFM linked. GFM's literal `https://…` and `www.…` become text too, to be linked
 * by `remarkBareLinks` with the server's rules. GFM ends one only at whitespace or `<`, so in
 * `||https://example.org||` it would take the spoiler's closing marker into the address. A
 * definition line, a block of its own, becomes a paragraph. The literal text joins the text
 * beside it, so it is read on as typed text is, and the text made knows where it was written
 * (`extentOf`), for whoever writes the message out again (`maskSpoilers`).
 */
export function remarkLiteralLinks() {
  return (tree: Root, file: { value: unknown }) => {
    const source = String(file.value);
    visit(tree, (node, index, parent) => {
      if (parent === undefined || index === undefined || !NAMED.has(node.type)) {
        return;
      }
      const written = writtenAs(node, source);
      if (written === null) {
        return;
      }
      // An autolink starts with `<`; an email address GFM linked is kept, since
      // `remarkBareLinks` does not link those.
      if (
        node.type === "link" &&
        (written.startsWith("<") || (!written.startsWith("[") && node.url.startsWith("mailto:")))
      ) {
        return;
      }
      const text: Text = { type: "text", value: written };
      const literal: RootContent =
        node.type === "definition" ? { type: "paragraph", children: [text] } : text;
      // A literal link GFM found after parsing has no position to hand on.
      if (node.position !== undefined) {
        text.position = node.position;
        literal.position = node.position;
      }
      parent.children.splice(index, 1, literal);
      return [SKIP, index + 1];
    });
    joinText(tree);
  };
}

/**
 * The source `node` was parsed from. A literal link GFM found in text after parsing (an
 * address right after a character it does not start one after, such as `|`) has no position,
 * and is its one text child; any other node without a position is `null`.
 */
function writtenAs(node: Nodes, source: string): string | null {
  const start = node.position?.start.offset;
  const end = node.position?.end.offset;
  if (start !== undefined && end !== undefined) {
    return source.slice(start, end);
  }
  const [only] = node.type === "link" && node.children.length === 1 ? node.children : [];
  return only?.type === "text" ? only.value : null;
}

/**
 * Joins each run of adjacent text nodes into one, written from the first's start to the
 * last's end, reading for spoiler markers as its parts did (`markersOf`).
 */
function joinText(tree: Root) {
  visit(tree, (node) => {
    if (!("children" in node)) {
      return;
    }
    const children = (node as Parent).children;
    for (let i = children.length - 1; i > 0; i -= 1) {
      const here = children[i];
      const before = children[i - 1];
      if (here?.type === "text" && before?.type === "text") {
        before.data = {
          ...before.data,
          extent: { start: extentOf(before).start, end: extentOf(here).end },
        };
        appendText(before, here);
        children.splice(i, 1);
      }
    }
  });
}
