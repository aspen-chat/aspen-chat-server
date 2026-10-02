/**
 * The first and last message rows with any part in view, found by their offsets in the
 * content, which are in order, so a few reads find them among hundreds; called on every
 * move of the list.
 */
export function rowsInView(box: HTMLDivElement): {
  first: HTMLElement | null;
  last: HTMLElement | null;
} {
  const rows = box.querySelectorAll<HTMLElement>("[data-message-id]");
  const top = box.scrollTop;
  const bottom = top + box.clientHeight;
  // The first row whose bottom is below the view's top.
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    const row = rows[mid];
    if (row !== undefined && row.offsetTop + row.offsetHeight > top) {
      high = mid;
    } else {
      low = mid + 1;
    }
  }
  const first = rows[low] ?? null;
  if (first === null || first.offsetTop >= bottom) {
    return { first: null, last: null };
  }
  // The last row whose top is above the view's bottom.
  let last = low;
  high = rows.length;
  while (last < high) {
    const mid = (last + high + 1) >> 1;
    const row = rows[mid];
    if (row !== undefined && row.offsetTop < bottom) {
      last = mid;
    } else {
      high = mid - 1;
    }
  }
  return { first, last: rows[last] ?? null };
}
