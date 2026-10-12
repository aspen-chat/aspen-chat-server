import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useNumberFormat } from "@/i18n/format";

/** One line of a chart. */
export interface ChartSeries {
  /** What the line is, in the legend and the tooltip. */
  name: string;
  /** The line's color, a CSS value. */
  color: string;
  /** A value for each of the chart's steps; `null` where there is none, which breaks the line. */
  values: readonly (number | null)[];
}

/** The plot's height; the x-axis band is below it, inside the chart's own height. */
const PLOT_HEIGHT = 160;
const AXIS_BAND = 24;
const MARGIN = { top: 12, left: 44 };
/** The room to the right of the plot: enough for a value labelled at the line's end, or none. */
const END_LABEL_SPACE = 44;
const NO_END_LABEL_SPACE = 12;
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

/** The line through `values`, broken wherever one is missing. */
function linePath(
  values: readonly (number | null)[],
  x: (i: number) => number,
  y: (value: number) => number,
): string {
  let path = "";
  values.forEach((value, i) => {
    if (value === null) {
      return;
    }
    const joined = i > 0 && values[i - 1] != null;
    path += `${joined ? "L" : "M"}${x(i).toFixed(1)},${y(value).toFixed(1)}`;
    // A value with none on either side is a line of no length, which its round caps draw as
    // a dot.
    if (!joined && values[i + 1] == null) {
      path += "h0";
    }
  });
  return path;
}

/**
 * Lines over time on one scale, to the dataviz rules: 2px lines, solid hairline gridlines at
 * round values from zero, and a crosshair that snaps to the nearest step on hover or, from the
 * keyboard, with the arrow keys, reading out every line's value there and its date. A single
 * series sits over a 10% wash of its color with its latest value labelled at the line's end,
 * and is named by the chart's title, so it has no legend; several are named by a legend of line
 * keys, in the order given, and in the tooltip. Every value is also in the table view beside
 * the charts.
 */
export function LineChart({
  label,
  ats,
  series,
  describe,
  axisDate,
  formatValue,
  dimmed = false,
}: {
  /** What the chart shows, for its accessible name. */
  label: string;
  /** When each step began, oldest first. */
  ats: readonly string[];
  series: readonly ChartSeries[];
  /** A step's date as the tooltip says it ("Sep 27", "Week of Sep 21"). */
  describe: (at: string) => string;
  /** A date as the axis says it, shorter. */
  axisDate: (at: string) => string;
  /** A value as the axis and the tooltip say it; a plain number when left out. */
  formatValue?: (value: number) => string;
  /** Shown faded, as while the next range loads. */
  dimmed?: boolean;
}) {
  const numbers = useNumberFormat();
  const say = formatValue ?? ((value: number) => numbers.format(value));
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

  const last = ats.length - 1;
  // The one line of a single-series chart, when it has a value at every step: only then is
  // there an area under it to wash and an end to label.
  const [only] = series;
  const alone =
    series.length === 1 && only?.values.every((value) => value !== null) === true ? only : null;
  const aloneLast = alone?.values[last] ?? null;
  const plotWidth = Math.max(
    width - MARGIN.left - (alone === null ? NO_END_LABEL_SPACE : END_LABEL_SPACE),
    1,
  );
  const top = Math.max(...series.flatMap((s) => s.values.map((value) => value ?? 0)), 0);
  const step = niceStep(top, Y_TICKS);
  const ceiling = Math.max(step * Math.ceil(top / step), step);
  const x = (i: number) =>
    MARGIN.left + (ats.length <= 1 ? plotWidth : (i / (ats.length - 1)) * plotWidth);
  const y = (value: number) => MARGIN.top + PLOT_HEIGHT - (value / ceiling) * PLOT_HEIGHT;
  const baseline = y(0);
  const ticks = Array.from({ length: Math.round(ceiling / step) + 1 }, (_, i) => i * step);
  const labels = Math.min(Math.max(Math.floor(plotWidth / X_LABEL_SPACE), 2), ats.length);
  const labelled = Array.from(
    new Set(
      Array.from({ length: labels }, (_, i) =>
        Math.round((i / Math.max(labels - 1, 1)) * (ats.length - 1)),
      ),
    ),
  );
  const shownAt = active === null ? undefined : ats[active];
  const height = MARGIN.top + PLOT_HEIGHT + AXIS_BAND;

  function nearest(event: PointerEvent<HTMLDivElement>) {
    const box = event.currentTarget.getBoundingClientRect();
    const offset = event.clientX - box.left - MARGIN.left;
    const index = Math.round((offset / plotWidth) * Math.max(ats.length - 1, 0));
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

  // Time runs left to right on the chart whichever way the page is written, as charts of time
  // conventionally do in right-to-left languages too; the keys follow the picture.
  return (
    <div className={"transition-opacity " + (dimmed ? "opacity-50" : "")}>
      {series.length > 1 && (
        <ul className="mb-1 flex flex-wrap gap-x-4 gap-y-1 text-xs text-ink-muted">
          {series.map((s) => (
            <li key={s.name} className="inline-flex items-center gap-1.5">
              <LineKey color={s.color} />
              {s.name}
            </li>
          ))}
        </ul>
      )}
      <div
        ref={frame}
        dir="ltr"
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
        className="relative rounded-md outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
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
                  {say(tick)}
                </text>
              </g>
            ))}
            {labelled.map((i) => {
              const at = ats[i];
              return at === undefined ? null : (
                <text
                  key={i}
                  x={x(i)}
                  y={MARGIN.top + PLOT_HEIGHT + 16}
                  textAnchor={i === 0 ? "start" : i === last ? "end" : "middle"}
                  className="fill-ink-muted text-[11px]"
                >
                  {axisDate(at)}
                </text>
              );
            })}
            {alone !== null && ats.length > 0 && (
              <path
                d={`${linePath(alone.values, x, y)}L${x(last).toFixed(1)},${baseline.toFixed(1)}L${x(0).toFixed(1)},${baseline.toFixed(1)}Z`}
                fill={alone.color}
                fillOpacity={0.1}
              />
            )}
            {series.map((s) => (
              <path
                key={s.name}
                d={linePath(s.values, x, y)}
                fill="none"
                stroke={s.color}
                strokeWidth={2}
                strokeLinejoin="round"
                strokeLinecap="round"
              />
            ))}
            {alone !== null && aloneLast !== null && (
              <>
                <circle
                  cx={x(last)}
                  cy={y(aloneLast)}
                  r={4}
                  fill={alone.color}
                  className="stroke-surface-raised"
                  strokeWidth={2}
                />
                <text
                  x={x(last) + 8}
                  y={y(aloneLast)}
                  dy="0.32em"
                  className="fill-ink text-xs font-semibold tabular-nums"
                >
                  {say(aloneLast)}
                </text>
              </>
            )}
            {active !== null && shownAt !== undefined && (
              <>
                <line
                  x1={x(active)}
                  x2={x(active)}
                  y1={MARGIN.top}
                  y2={MARGIN.top + PLOT_HEIGHT}
                  className="stroke-ink-muted"
                  strokeWidth={1}
                />
                {series.map((s) => {
                  const value = s.values[active];
                  return value == null ? null : (
                    <circle
                      key={s.name}
                      cx={x(active)}
                      cy={y(value)}
                      r={4}
                      fill={s.color}
                      className="stroke-surface-raised"
                      strokeWidth={2}
                    />
                  );
                })}
              </>
            )}
          </svg>
        )}
        {active !== null && shownAt !== undefined && (
          <div
            role="status"
            className="pointer-events-none absolute top-0 z-10 rounded-md border border-line bg-surface-raised px-2 py-1 whitespace-nowrap shadow-md"
            style={
              // Beside the crosshair, on whichever side has the room, so it never covers the
              // values it reads out.
              x(active) > width / 2 ? { right: width - x(active) + 12 } : { left: x(active) + 12 }
            }
          >
            {series.map((s) => {
              const value = s.values[active];
              return (
                <span key={s.name} className="flex items-center gap-1.5 text-sm">
                  {series.length > 1 && <LineKey color={s.color} />}
                  <span className="font-semibold tabular-nums">
                    {value == null ? "—" : say(value)}
                  </span>
                  {series.length > 1 && <span className="text-xs text-ink-muted">{s.name}</span>}
                </span>
              );
            })}
            <span className="block text-xs text-ink-muted">{describe(shownAt)}</span>
          </div>
        )}
      </div>
    </div>
  );
}

/** A short stroke of a line's color, naming it in the legend and the tooltip. */
function LineKey({ color }: { color: string }) {
  return (
    <span
      aria-hidden="true"
      className="inline-block h-0.5 w-3 shrink-0 rounded-full"
      style={{ backgroundColor: color }}
    />
  );
}
