import type { Category, Channel } from "@aspen/protocol";
import type { DropAt } from "@/features/layout/reorder";

/** The key of the row that heads the top-level channels; no record's id can equal it. */
export const TOP_LEVEL_KEY = "top-level";

/**
 * One row of the layout list. The list is flat: the top level's heading and its channels, then
 * each category's heading followed by its channels. Keys are the records' ids.
 */
export type LayoutRow =
  | { kind: "topLevel"; key: typeof TOP_LEVEL_KEY }
  | { kind: "category"; key: string; category: Category }
  | { kind: "channel"; key: string; channel: Channel; parentCategory: string | null };

/** What a drop changes: one group's channels (and who is in it), or the categories' order. */
export type LayoutMove =
  | { kind: "channels"; parentCategory: string | null; order: string[] }
  | { kind: "categories"; order: string[] };

export function layoutRows(
  topLevel: readonly Channel[],
  categories: readonly Category[],
  byCategory: ReadonlyMap<string, readonly Channel[]>,
): LayoutRow[] {
  const channelRows = (channels: readonly Channel[], parentCategory: string | null) =>
    channels.map((channel): LayoutRow => ({
      kind: "channel",
      key: channel.id,
      channel,
      parentCategory,
    }));
  return [
    { kind: "topLevel", key: TOP_LEVEL_KEY },
    ...channelRows(topLevel, null),
    ...categories.flatMap((category): LayoutRow[] => [
      { kind: "category", key: category.id, category },
      ...channelRows(byCategory.get(category.id) ?? [], category.id),
    ]),
  ];
}

/** The group a heading row heads or a channel row is in: a category's id, or `null`. */
function groupOf(row: LayoutRow): string | null {
  return row.kind === "channel" ? row.parentCategory : row.kind === "category" ? row.key : null;
}

function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((id, index) => id === b[index]);
}

/**
 * Where a drop between rows falls among `rows`, which no longer hold what is dragged: the
 * index of the row it lands before, `rows.length` for the end, or `null` when the target is
 * not among them.
 */
function gapAt(rows: readonly LayoutRow[], target: DropAt): number | null {
  const at = rows.findIndex((row) => row.key === target.key);
  if (at === -1) {
    return null;
  }
  return target.dropPosition === "before" ? at : at + 1;
}

/**
 * What dropping the row `dragged` at `target` changes, or `null` when the drop means nothing
 * there or would leave everything where it is.
 *
 * A channel dropped between two rows joins the group of the row above the gap, right after
 * that row: first in the group under a heading, and so last in the group above when dropped
 * just over a category's heading. Dropped on a heading it goes to the end of that group.
 *
 * A category moves with its channels, and only to where a category can stand: just over
 * another category's heading, or at the end of the list.
 */
export function planMove(
  rows: readonly LayoutRow[],
  dragged: string,
  target: DropAt,
): LayoutMove | null {
  const row = rows.find((r) => r.key === dragged);
  if (row === undefined || row.kind === "topLevel") {
    return null;
  }
  if (row.kind === "channel") {
    return planChannelMove(rows, row.key, row.parentCategory, target);
  }
  if (target.dropPosition === "on") {
    return null;
  }
  const rest = rows.filter((r) => groupOf(r) !== row.key || r.kind === "topLevel");
  const gap = gapAt(rest, target);
  if (gap === null || (gap < rest.length && rest[gap]?.kind !== "category")) {
    return null;
  }
  const before = rest.slice(0, gap).filter((r) => r.kind === "category");
  const after = rest.slice(gap).filter((r) => r.kind === "category");
  const order = [...before.map((r) => r.key), row.key, ...after.map((r) => r.key)];
  const current = rows.filter((r) => r.kind === "category").map((r) => r.key);
  return sameOrder(order, current) ? null : { kind: "categories", order };
}

function planChannelMove(
  rows: readonly LayoutRow[],
  dragged: string,
  from: string | null,
  target: DropAt,
): LayoutMove | null {
  const rest = rows.filter((r) => r.key !== dragged);
  const channelsOf = (group: string | null) =>
    rest.filter((r) => r.kind === "channel" && r.parentCategory === group).map((r) => r.key);
  let parentCategory: string | null;
  let order: string[];
  if (target.dropPosition === "on") {
    const heading = rest.find((r) => r.key === target.key);
    if (heading === undefined || heading.kind === "channel") {
      return null;
    }
    parentCategory = groupOf(heading);
    order = [...channelsOf(parentCategory), dragged];
  } else {
    const gap = gapAt(rest, target);
    const above = gap === null ? undefined : rest[gap - 1];
    if (above === undefined) {
      return null;
    }
    parentCategory = groupOf(above);
    const group = channelsOf(parentCategory);
    const cut = above.kind === "channel" ? group.indexOf(above.key) + 1 : 0;
    order = [...group.slice(0, cut), dragged, ...group.slice(cut)];
  }
  if (parentCategory === from) {
    const current = rows
      .filter((r) => r.kind === "channel" && r.parentCategory === from)
      .map((r) => r.key);
    if (sameOrder(order, current)) {
      return null;
    }
  }
  return { kind: "channels", parentCategory, order };
}
