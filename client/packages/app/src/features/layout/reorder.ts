/** A drop target as React Aria reports it for a reorder: the item dropped on and which side. */
export interface DropAt {
  key: string;
  dropPosition: "before" | "after" | "on";
}

/**
 * The order of `ids` after inserting `arriving`, items from another list, at `target`. They
 * land together, in the order given, before or after the target; a drop "on" an item is
 * treated as after it. An unknown target appends them.
 */
export function insertIds(
  ids: readonly string[],
  arriving: readonly string[],
  target: DropAt,
): string[] {
  const rest = ids.filter((id) => !arriving.includes(id));
  const at = rest.indexOf(target.key);
  if (at === -1) {
    return [...rest, ...arriving];
  }
  const cut = target.dropPosition === "before" ? at : at + 1;
  return [...rest.slice(0, cut), ...arriving, ...rest.slice(cut)];
}

/**
 * The order of `ids` after moving `moved` to `target`, as a drag and drop asks. The moved items
 * keep their relative order and land together before or after the target; a drop "on" an item
 * is treated as after it. Moving onto itself leaves the order alone.
 */
export function reorderIds(
  ids: readonly string[],
  moved: ReadonlySet<string>,
  target: DropAt,
): string[] {
  if (moved.has(target.key)) {
    return [...ids];
  }
  const picked = ids.filter((id) => moved.has(id));
  const rest = ids.filter((id) => !moved.has(id));
  const at = rest.indexOf(target.key);
  if (at === -1) {
    return [...ids];
  }
  const cut = target.dropPosition === "before" ? at : at + 1;
  return [...rest.slice(0, cut), ...picked, ...rest.slice(cut)];
}
