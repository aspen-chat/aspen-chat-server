import type { Root } from "mdast";
import remarkParse from "remark-parse";
import { unified } from "unified";
import { visit } from "unist-util-visit";
import { opensTooDeeply } from "@/features/messages/markdownLimits";
import { MESSAGE_REMARK_PLUGINS } from "@/features/messages/remarkPipeline";

const processor = unified().use(remarkParse).use(MESSAGE_REMARK_PLUGINS);

/** How many texts' answers `spoileredLinks` keeps, so a message drawn again is not parsed again. */
const CACHE_SIZE = 256;
const cache = new Map<string, ReadonlySet<string>>();
const NONE: ReadonlySet<string> = new Set();

/**
 * The addresses linked from inside a spoiler in a message body, read as `Markdown` reads it,
 * each as `normalizedUrl` writes it, so a link preview of one can be hidden as its link is. A
 * body with no spoiler marker is not parsed; nor is one `Markdown` shows as plain text, whose
 * markers hide nothing. The answers for the last `CACHE_SIZE` texts are kept and shared, so
 * they must not be changed.
 */
export function spoileredLinks(content: string): ReadonlySet<string> {
  if ((!content.includes("||") && !content.includes(">!")) || opensTooDeeply(content)) {
    return NONE;
  }
  const cached = cache.get(content);
  if (cached !== undefined) {
    return cached;
  }
  const found = collect(content);
  if (cache.size >= CACHE_SIZE) {
    const oldest = cache.keys().next();
    if (oldest.done !== true) {
      cache.delete(oldest.value);
    }
  }
  cache.set(content, found);
  return found;
}

function collect(content: string): ReadonlySet<string> {
  const tree = processor.runSync(processor.parse(content), content) as Root;
  const found = new Set<string>();
  visit(tree, (node) => {
    if ((node.type as string) !== "spoiler") {
      return;
    }
    visit(node, "link", (link) => {
      const url = normalizedUrl(link.url);
      if (url !== null) {
        found.add(url);
      }
    });
  });
  return found.size === 0 ? NONE : found;
}

/**
 * `url` as a WHATWG URL serializes it, which is how the server's `url` crate writes a link
 * preview's `url` (`https://example.org` as `https://example.org/`); `null` when it is no URL.
 */
export function normalizedUrl(url: string): string | null {
  try {
    return new URL(url).href;
  } catch {
    return null;
  }
}
