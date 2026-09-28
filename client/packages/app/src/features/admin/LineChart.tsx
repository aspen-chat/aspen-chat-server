import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

/** One point of a series: when, and how many. */
export interface ChartPoint {
  at: string;
  value: number;
}

/** The plot's height; the x-axis band is below it, inside the chart's own height. */
const PLOT_HEIGHT = 160;
const AXIS_BAND = 24;
const MARGIN = { top: 12, right: 44, left: 44 };
const Y_TICKS = 4;
/** The room one date on the x-axis takes, so as many fit as the chart's width allows. */
const X_LABEL_SPACE = 90;

/** A round step for ticks spanning `span`: 1, 2, or 5 times a power of ten. */
function niceStep(span: number, ticks: number): number {
  const rough = Math.max(span / ticks, 1);
  const magnitude = 10 ** Math.floor(Math.log10(rough));
  const fraction = rough / magnitude;
  const nice = fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10;
  return Math.max(1, nice * magnitude);
}

const ticksFormat = new Intl.NumberFormat(undefined);

/**
 * A single-series line over time, to the dataviz rules: a 2px line over a 10% wash of its
 * color, solid hairline gridlines at round values from zero, the latest value labelled at the
 * line's end, and a crosshair that snaps to the nearest point on hover or, from the keyboard,
 * with the arrow keys, reading out the value and its date. The series is named by the chart's
 * title, so there is no legend. Every value is also in the table view beside the charts.
 */
export function LineChart({
  label,
  points,
  describe,
  axisDate,
  dimmed = false,
}: {
  /** What the series is, for its accessible name. */
  label: string;
  points: readonly ChartPoint[];
  /** A point's date as the tooltip says it ("Sep 27", "Week of Sep 21"). */
  describe: (at: string) => string;
  /** A date as the axis says it, shorter. */
  axisDate: (at: string) => string;
  /** Shown faded, as while the next range loads. */
  dimmed?: boolean;
}) {
  const frame = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [active, setActive] = useState<number | null>(null);

  useLayoutEffect(() => {
    const element = frame.current;
    if (element === null) {
      return;
    }
    setWidth(element.clientWidth);
    const observer = new ResizeObserver(() => {
      setWidth(element.clientWidth);
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, []);

  const plotWidth = Math.max(width - MARGIN.left - MARGIN.right, 1);
  const top = Math.max(...points.map((p) => p.value), 0);
  const step = niceStep(top, Y_TICKS);
  const ceiling = Math.max(step * Math.ceil(top / step), step);
  const x = (i: number) =>
    MARGIN.left + (points.length <= 1 ? plotWidth : (i / (points.length - 1)) * plotWidth);
  const y = (value: number) => MARGIN.top + PLOT_HEIGHT - (value / ceiling) * PLOT_HEIGHT;
  const line = points
    .map((p, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(p.value).toFixed(1)}`)
    .join("");
  const baseline = y(0);
  const area =
    points.length === 0
      ? ""
      : `${line}L${x(points.length - 1).toFixed(1)},${baseline.toFixed(1)}L${x(0).toFixed(1)},${baseline.toFixed(1)}Z`;
  const ticks = Array.from({ length: Math.round(ceiling / step) + 1 }, (_, i) => i * step);
  const labels = Math.min(Math.max(Math.floor(plotWidth / X_LABEL_SPACE), 2), points.length);
  const labelled = Array.from(
    new Set(
      Array.from({ length: labels }, (_, i) =>
        Math.round((i / Math.max(labels - 1, 1)) * (points.length - 1)),
      ),
    ),
  );
  const last = points.length - 1;
  const shown = active === null ? null : points[active];
  const height = MARGIN.top + PLOT_HEIGHT + AXIS_BAND;

  function nearest(event: PointerEvent<HTMLDivElement>) {
    const box = event.currentTarget.getBoundingClientRect();
    const offset = event.clientX - box.left - MARGIN.left;
    const index = Math.round((offset / plotWidth) * Math.max(points.length - 1, 0));
    setActive(Math.min(Math.max(index, 0), last));
  }

  function onKey(event: KeyboardEvent<HTMLDivElement>) {
    const moves: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1 };
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      setActive(event.key === "Home" ? 0 : last);
      return;
    }
    const move = moves[event.key];
    if (move !== undefined) {
      event.preventDefault();
      setActive((current) => Math.min(Math.max((current ?? last) + move, 0), last));
    }
  }

  return (
    <div
      ref={frame}
      role="group"
      tabIndex={0}
      aria-label={label}
      onPointerMove={nearest}
      onPointerLeave={() => {
        setActive(null);
      }}
      onFocus={() => {
        setActive(last);
      }}
      onBlur={() => {
        setActive(null);
      }}
      onKeyDown={onKey}
      className={
        "relative rounded-md outline-none focus-visible:ring-2 focus-visible:ring-accent/50 transition-opacity " +
        (dimmed ? "opacity-50" : "")
      }
      style={{ height }}
    >
      {width > 0 && (
        <svg width={width} height={height} aria-hidden="true" className="block">
          {ticks.map((tick) => (
            <g key={tick}>
              <line
                x1={MARGIN.left}
                x2={MARGIN.left + plotWidth}
                y1={y(tick)}
                y2={y(tick)}
                className="stroke-line"
                strokeWidth={1}
                shapeRendering="crispEdges"
              />
              <text
                x={MARGIN.left - 8}
                y={y(tick)}
                dy="0.32em"
                textAnchor="end"
                className="fill-ink-muted text-[11px] tabular-nums"
              >
                {ticksFormat.format(tick)}
              </text>
            </g>
          ))}
          {labelled.map((i) => {
            const point = points[i];
            return point === undefined ? null : (
              <text
                key={i}
                x={x(i)}
                y={MARGIN.top + PLOT_HEIGHT + 16}
                textAnchor={i === 0 ? "start" : i === last ? "end" : "middle"}
                className="fill-ink-muted text-[11px]"
              >
                {axisDate(point.at)}
              </text>
            );
          })}
          <path d={area} className="fill-accent/10" />
          <path
            d={line}
            fill="none"
            className="stroke-accent"
            strokeWidth={2}
            strokeLinejoin="round"
            strokeLinecap="round"
          />
          {points[last] !== undefined && (
            <>
              <circle
                cx={x(last)}
                cy={y(points[last].value)}
                r={4}
                className="fill-accent stroke-surface-raised"
                strokeWidth={2}
              />
              <text
                x={x(last) + 8}
                y={y(points[last].value)}
                dy="0.32em"
                className="fill-ink text-xs font-semibold tabular-nums"
              >
                {ticksFormat.format(points[last].value)}
              </text>
            </>
          )}
          {shown !== null && shown !== undefined && active !== null && (
            <>
              <line
                x1={x(active)}
                x2={x(active)}
                y1={MARGIN.top}
                y2={MARGIN.top + PLOT_HEIGHT}
                className="stroke-ink-muted"
                strokeWidth={1}
              />
              <circle
                cx={x(active)}
                cy={y(shown.value)}
                r={4}
                className="fill-accent stroke-surface-raised"
                strokeWidth={2}
              />
            </>
          )}
        </svg>
      )}
      {shown !== null && shown !== undefined && active !== null && (
        <div
          role="status"
          className="pointer-events-none absolute top-0 z-10 rounded-md border border-line bg-surface-raised px-2 py-1 shadow-md"
          style={{
            left: Math.min(Math.max(x(active) - 60, 0), Math.max(width - 120, 0)),
          }}
        >
          <span className="block text-sm font-semibold tabular-nums">
            {ticksFormat.format(shown.value)}
          </span>
          <span className="block text-xs text-ink-muted">{describe(shown.at)}</span>
        </div>
      )}
    </div>
  );
}
