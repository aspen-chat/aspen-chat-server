import type { Nodes, Root } from "mdast";
import remarkParse from "remark-parse";
import { unified } from "unified";
import { opensTooDeeply } from "@/features/messages/markdownLimits";
import { SPOILER_REMARK_PLUGINS } from "@/features/messages/remarkPipeline";
import { mayHoldSpoiler } from "@/features/messages/remarkSpoilers";
import { extentOf } from "@/features/messages/sourceExtent";

const processor = unified().use(remarkParse).use(SPOILER_REMARK_PLUGINS);

/**
 * A message body's syntax tree with its spoilers found, read as `Markdown` reads it, or `null`
 * for a body `Markdown` shows as it was written, whose markers hide nothing.
 */
export function spoilerTree(content: string): Root | null {
  if (opensTooDeeply(content)) {
    return null;
  }
  return processor.runSync(processor.parse(content), content) as Root;
}

/**
 * How many bodies' answers `maskSpoilers` keeps, so a line drawn again (a reply's line, in a
 * row of a list) is not parsed again.
 */
const CACHE_SIZE = 256;
const cache = new Map<string, { word: string; masked: string }>();

/**
 * A message body with each spoiler `Markdown` would hide written as `word`, for showing a
 * line of it where nothing can be revealed (a notification, a reply's line, a search result).
 * The rest stays as it was written, Markdown included, but for the text a spoiler was cut
 * from, which reads as the reader is shown it (an escaped `\*` as `*`), and for the markup
 * around text whose place in the body the tree does not record (a bare address GFM linked),
 * which is left out.
 *
 * A body that can hold no spoiler (`mayHoldSpoiler`) is answered without parsing it, and the
 * answers for the last `CACHE_SIZE` that were parsed are kept.
 */
export function maskSpoilers(content: string, word: string): string {
  if (!mayHoldSpoiler(content)) {
    return content;
  }
  const cached = cache.get(content);
  if (cached?.word === word) {
    return cached.masked;
  }
  const masked = parsedAndMasked(content, word);
  if (cached === undefined && cache.size >= CACHE_SIZE) {
    const oldest = cache.keys().next();
    if (oldest.done !== true) {
      cache.delete(oldest.value);
    }
  }
  cache.set(content, { word, masked });
  return masked;
}

function parsedAndMasked(content: string, word: string): string {
  const tree = spoilerTree(content);
  if (tree === null) {
    return content;
  }
  const holders = new Set<Nodes>();
  if (!holdsSpoiler(tree, holders)) {
    return content;
  }
  return written(tree, content, word, holders);
}

/** Whether a spoiler is `node` or under it, adding each node one is under to `holders`. */
function holdsSpoiler(node: Nodes, holders: Set<Nodes>): boolean {
  if ((node.type as string) === "spoiler") {
    return true;
  }
  if (!("children" in node)) {
    return false;
  }
  let holds = false;
  for (const child of node.children) {
    // Every child is visited, so every holder is found.
    holds = holdsSpoiler(child, holders) || holds;
  }
  if (holds) {
    holders.add(node);
  }
  return holds;
}

/**
 * `node` as it was written in `source`, with each spoiler as `word`. A node holding no spoiler
 * is its source, and one holding some is its children with the source between them, wherever
 * both ends of a stretch between are known (`extentOf`).
 */
function written(node: Nodes, source: string, word: string, holders: ReadonlySet<Nodes>): string {
  if ((node.type as string) === "spoiler") {
    return word;
  }
  const { start, end } = extentOf(node);
  if (!holders.has(node) && start !== undefined && end !== undefined) {
    return source.slice(start, end);
  }
  if (!("children" in node)) {
    return node.type === "text" ? node.value : "";
  }
  let out = "";
  let at = start;
  for (const child of node.children) {
    const extent = extentOf(child);
    if (at !== undefined && extent.start !== undefined) {
      out += source.slice(at, extent.start);
    }
    out += written(child, source, word, holders);
    at = extent.end;
  }
  if (at !== undefined && end !== undefined) {
    out += source.slice(at, end);
  }
  return out;
}
