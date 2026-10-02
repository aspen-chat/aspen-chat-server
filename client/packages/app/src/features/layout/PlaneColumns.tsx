import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

/**
 * Planes (`planeClass`) flowing down two balanced columns where there is room for two readable
 * ones, and down one where there is not. Their container's width decides, not the window's, so
 * a modal on a narrow window, or a pane beside others, gets one column.
 *
 * Two columns are a wrapping flex column whose height is set to the best balance of the planes'
 * measured heights, so the first ones fill the left column and the rest the right, in order. A
 * flex item is never split, where CSS multi-column layout splits whatever falls at a column's
 * end, and Firefox then leaves a stale break inside a plane as its content changes height,
 * stretching a control past the plane's edge. The planes stay children of one element, so none
 * is remade, losing what it holds, when the balance moves it to the other column.
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
      // overflow into a third column, with nothing to measure again, since a transform
      // changes no border box. The box's own rect over its layout width is the scale.
      const scale =
        element.offsetWidth > 0 ? element.getBoundingClientRect().width / element.offsetWidth : 1;
      const heights = Array.from(
        element.children,
        (child) => child.getBoundingClientRect().height / (scale || 1),
      );
      const total = heights.reduce((sum, h) => sum + h, 0);
      const column = (sum: number, count: number) => sum + gap * Math.max(count - 1, 0);
      // The first `k` planes on the left and the rest on the right, whichever `k` makes the
      // taller column shortest.
      let best = column(total, heights.length);
      let left = 0;
      heights.forEach((h, index) => {
        left += h;
        const k = index + 1;
        best = Math.min(best, Math.max(column(left, k), column(total - left, heights.length - k)));
      });
      // A pixel spare, so a plane measured at a fraction of one still fits its column.
      setHeight(Math.ceil(best) + 1);
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
        className="flex flex-col gap-4 *:shrink-0 @2xl:flex-wrap @2xl:content-start @2xl:*:w-[calc(50%-0.5rem)]"
      >
        {children}
      </div>
    </div>
  );
}
