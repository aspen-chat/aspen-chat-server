import type { Link, Root, Text } from "mdast";
import { visit } from "unist-util-visit";
import { linkify } from "@/features/messages/linkify";

/**
 * A remark plugin that turns bare domains and URLs left in text into links, using the same
 * tokenizer as the server's preview extractor. GitHub-flavoured Markdown already links
 * `http(s)` and `www.` literals; this catches `github.io/pages`-style mentions it leaves as
 * text. Code spans and blocks are not text nodes, so they are untouched.
 */
export function remarkBareLinks() {
  return (tree: Root) => {
    visit(tree, "text", (node: Text, index, parent) => {
      if (parent === undefined || index === undefined || parent.type === "link") {
        return;
      }
      const runs = linkify(node.value);
      if (!runs.some((run) => run.kind === "link")) {
        return;
      }
      const nodes: (Text | Link)[] = runs.map((run) =>
        run.kind === "text"
          ? { type: "text", value: run.text }
          : { type: "link", url: run.url, children: [{ type: "text", value: run.text }] },
      );
      parent.children.splice(index, 1, ...nodes);
      return index + nodes.length;
    });
  };
}
