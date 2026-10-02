import type { Parent, PhrasingContent, Root, Text } from "mdast";
import { visit } from "unist-util-visit";
import { EMOJI_REFERENCE } from "@/features/emoji/customEmoji";

/**
 * A custom emoji in the syntax tree: `<:id>`, as the server stores it. Rendered by
 * `Markdown.tsx` through the `data-emoji` attribute on a `span`, whose text is the reference.
 */
export interface CustomEmojiNode extends Parent {
  type: "customEmoji";
  children: PhrasingContent[];
}

/**
 * A remark plugin that turns references to custom emoji in text into `customEmoji` nodes.
 * Code is not text in the tree, so a reference written in code stays as it was.
 */
export function remarkCustomEmoji() {
  return (tree: Root) => {
    visit(tree, "text", (node: Text, index, parent) => {
      if (parent === undefined || index === undefined) {
        return;
      }
      const pieces = splitReferences(node.value);
      if (pieces.length === 1 && pieces[0]?.type === "text") {
        return;
      }
      parent.children.splice(index, 1, ...(pieces as never[]));
      return index + pieces.length;
    });
  };
}

function splitReferences(value: string): PhrasingContent[] {
  const pieces: PhrasingContent[] = [];
  let last = 0;
  for (const match of value.matchAll(EMOJI_REFERENCE)) {
    const at = match.index;
    if (at > last) {
      pieces.push({ type: "text", value: value.slice(last, at) });
    }
    const node: CustomEmojiNode = {
      type: "customEmoji",
      children: [{ type: "text", value: match[0] }],
      data: {
        hName: "span",
        hProperties: { dataEmoji: match[1]?.toLowerCase() ?? "" },
      },
    };
    pieces.push(node as unknown as PhrasingContent);
    last = at + match[0].length;
  }
  if (last < value.length) {
    pieces.push({ type: "text", value: value.slice(last) });
  }
  return pieces;
}
