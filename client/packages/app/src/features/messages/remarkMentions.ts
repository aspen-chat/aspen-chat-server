import type { Parent, PhrasingContent, Root, Text } from "mdast";
import { visit } from "unist-util-visit";

/**
 * A tag in the syntax tree: `<@user-id>`, `<@&role-id>`, or `@everyone`, as the server reads
 * them (`server/app/src/mention.rs`). Rendered by `Markdown.tsx` through the `data-mention`
 * and `data-id` attributes on a `span`, whose text is what was written.
 */
export interface MentionNode extends Parent {
  type: "mention";
  children: PhrasingContent[];
}

/** The kinds of tag, as `data-mention` names them. */
export type MentionKind = "user" | "role" | "everyone";

const UUID = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
/** A tag, or `@everyone` standing on its own: not inside a word on either side. */
const TAG = new RegExp(`<@(&?)(${UUID})>|(?<![\\p{L}\\p{N}_])@everyone(?![\\p{L}\\p{N}_])`, "gu");

/**
 * A remark plugin that turns tags in text into `mention` nodes. Code is not text in the tree,
 * so a tag written in code stays as it was.
 */
export function remarkMentions() {
  return (tree: Root) => {
    visit(tree, "text", (node: Text, index, parent) => {
      if (parent === undefined || index === undefined) {
        return;
      }
      const pieces = splitTags(node.value);
      if (pieces.length === 1 && pieces[0]?.type === "text") {
        return;
      }
      parent.children.splice(index, 1, ...(pieces as never[]));
      return index + pieces.length;
    });
  };
}

function splitTags(value: string): PhrasingContent[] {
  const pieces: PhrasingContent[] = [];
  let last = 0;
  for (const match of value.matchAll(TAG)) {
    const at = match.index;
    if (at > last) {
      pieces.push({ type: "text", value: value.slice(last, at) });
    }
    const kind: MentionKind =
      match[2] === undefined ? "everyone" : match[1] === "&" ? "role" : "user";
    const node: MentionNode = {
      type: "mention",
      children: [{ type: "text", value: match[0] }],
      data: {
        hName: "span",
        hProperties: { dataMention: kind, dataId: match[2]?.toLowerCase() ?? "" },
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
