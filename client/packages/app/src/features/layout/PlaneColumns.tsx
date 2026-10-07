import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

/**
 * Planes (`planeClass`) flowing down balanced columns: three where there is room for three
 * readable ones, two where there is room for two, and one where there is not. Their container's
 * width decides, not the window's, so a modal on a narrow window, or a pane beside others, gets
 * one column. The container queries set `--plane-columns`, which both sizes the planes and tells
 * the balance how many columns to fill.
 *
 * Several columns are a wrapping flex column whose height is set to the best balance of the
 * planes' measured heights, so the first ones fill the first column, the next the second, and so
 * on, in order. A flex item is never split, where CSS multi-column layout splits whatever falls
 * at a column's end, and Firefox then leaves a stale break inside a plane as its content changes
 * height, stretching a control past the plane's edge. The planes stay children of one element, so
 * none is remade, losing what it holds, when the balance moves it to another column.
 */
export function PlaneColumns({ children }: { children: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState<number | null>(null);
  useLayoutEffect(() => {
    const element = box.current;
    if (element === null) {
      return;
    }
    const balance = () => {
      const style = getComputedStyle(element);
      if (style.flexWrap !== "wrap") {
        setHeight(null);
        return;
      }
      const gap = Number.parseFloat(style.rowGap) || 0;
      // Measured in layout pixels: a modal arrives scaled up from 0.96 (`motion-dialog`), and
      // a rect read mid-scale is that much short, which would set a height the planes
      // overflow into a column too many, with nothing to measure again, since a transform
      // changes no border box. The box's own rect over its layout width is the scale.
      const scale =
        element.offsetWidth > 0 ? element.getBoundingClientRect().width / element.offsetWidth : 1;
      const heights = Array.from(
        element.children,
        (child) => child.getBoundingClientRect().height / (scale || 1),
      );
      const columns = Number.parseInt(style.getPropertyValue("--plane-columns"), 10) || 1;
      // A pixel spare, so a plane measured at a fraction of one still fits its column.
      setHeight(Math.ceil(balancedHeight(heights, columns, gap)) + 1);
    };
    balance();
    const observer = new ResizeObserver(balance);
    observer.observe(element);
    for (const child of Array.from(element.children)) {
      observer.observe(child);
    }
    return () => {
      observer.disconnect();
    };
  }, [children]);
  return (
    <div className="@container">
      <div
        ref={box}
        style={height === null ? undefined : { height }}
        className={
          "flex flex-col gap-4 *:shrink-0 " +
          "@2xl:flex-wrap @2xl:content-start @2xl:[--plane-columns:2] @5xl:[--plane-columns:3] " +
          "@2xl:*:w-[calc((100%_-_(var(--plane-columns)_-_1)_*_1rem)_/_var(--plane-columns))]"
        }
      >
        {children}
      </div>
    </div>
  );
}

/**
 * The shortest height of `columns` columns holding `heights` in order, each column a run of
 * consecutive planes `gap` apart: the tallest column of the best split. A wrapping flex column
 * of this height fills each column greedily, which never needs more columns than the best split.
 */
function balancedHeight(heights: number[], columns: number, gap: number): number {
  // `starts[i]`: the planes before the `i`th, summed.
  const starts = heights.reduce((sums, h) => [...sums, (sums.at(-1) ?? 0) + h], [0]);
  const sum = (to: number) => starts[to] ?? 0;
  // The height of a column holding planes `from` up to, not including, `to`.
  const column = (from: number, to: number) =>
    sum(to) - sum(from) + gap * Math.max(to - from - 1, 0);
  // `best[i]`: the shortest tallest column holding the first `i` planes in the columns so far.
  let best = starts.map((_, to) => column(0, to));
  for (let used = 2; used <= columns; used++) {
    const previous = best;
    best = previous.map((alone, to) =>
      previous
        .slice(0, to)
        .reduce(
          (shortest, before, from) => Math.min(shortest, Math.max(before, column(from, to))),
          alone,
        ),
    );
  }
  return best.at(-1) ?? 0;
}
