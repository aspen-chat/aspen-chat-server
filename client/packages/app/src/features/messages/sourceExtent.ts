import type { Nodes } from "mdast";

/**
 * Where a node of a message's syntax tree was written, as offsets into the message's text. An
 * end left out is not known: a node a plugin made of part of another (`remarkSpoilers` splits
 * text around a spoiler) knows at most the end it shares with what it was made of.
 */
export interface Extent {
  start?: number | undefined;
  end?: number | undefined;
}

declare module "mdast" {
  interface Data {
    /** Where the node was written, when its `position` does not say (`extentOf`). */
    extent?: Extent;
  }
}

/** Where `node` was written: what a plugin recorded for it, or else its parsed position. */
export function extentOf(node: Nodes): Extent {
  return (
    node.data?.extent ?? {
      start: node.position?.start.offset,
      end: node.position?.end.offset,
    }
  );
}
