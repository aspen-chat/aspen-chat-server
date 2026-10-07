import type { Parent, Root, RootContent, Text } from "mdast";
import { SKIP, visit } from "unist-util-visit";

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
 * (`remarkBareLinks`), an address of the deployment as a name path (`SelfLink`). A link whose
 * text is its address (`<https://…>`, or GFM's literal `https://…` and `www.…`) is kept, since
 * its words cannot differ from where it leads. A definition line, a block of its own, becomes
 * a paragraph. The literal text joins the text beside it, so it is read on as typed text is.
 */
export function remarkLiteralLinks() {
  return (tree: Root, file: { value: unknown }) => {
    const source = String(file.value);
    visit(tree, (node, index, parent) => {
      if (parent === undefined || index === undefined || !NAMED.has(node.type)) {
        return;
      }
      const start = node.position?.start.offset;
      const end = node.position?.end.offset;
      if (start === undefined || end === undefined) {
        return;
      }
      const written = source.slice(start, end);
      // An autolink starts with `<`, and a literal one with its address.
      if (node.type === "link" && !written.startsWith("[")) {
        return;
      }
      const text: Text = { type: "text", value: written };
      const literal: RootContent =
        node.type === "definition" ? { type: "paragraph", children: [text] } : text;
      parent.children.splice(index, 1, literal);
      return [SKIP, index + 1];
    });
    joinText(tree);
  };
}

/** Joins each run of adjacent text nodes into one. */
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
        before.value += here.value;
        children.splice(i, 1);
      }
    }
  });
}
