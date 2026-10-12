import type { DatabaseLatency as LatencySeries, DatabaseLatencyRange } from "@aspen/protocol";
import { useCallback, useState } from "react";
import { ToggleButton, ToggleButtonGroup, useLocale } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { ReadFailed } from "@/features/admin/AdminDashboard";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { useFigures } from "@/features/admin/format";
import { LineChart } from "@/features/admin/LineChart";
import { useAdminRead } from "@/features/admin/useAdminRead";
import { toggleChipClass } from "@/features/invites/dialog";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { dateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const RANGES: readonly DatabaseLatencyRange[] = ["oneHour", "oneDay", "sevenDays"];

/** How often the latency is read again while the dashboard is visible, as the servers write it. */
const LATENCY_REFRESH_MS = 60_000;

const TIME: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };
const DAY: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };

/** What the servers time, each charted on a scale of its own. */
const KINDS = ["queries", "poolWaits"] as const;

/** A step's percentiles, in the order their lines are drawn and their colors given. */
const PERCENTILES = [
  { key: "medianMicros", name: "latencyMedian", color: "var(--color-chart-1)" },
  { key: "p95Micros", name: "latencyP95", color: "var(--color-chart-2)" },
  { key: "p99Micros", name: "latencyP99", color: "var(--color-chart-3)" },
] as const;

/**
 * How long the database has taken over a range, as every server together saw it: the median
 * and the 95th and 99th percentiles of statements' round trips, and of waits for a pooled
 * connection, as two charts sharing one set of range buttons. Two charts rather than one,
 * since the waits are usually far shorter than the statements and would lie flat beneath them.
 * Shows nothing on a deployment that predates the record.
 */
export function DatabaseLatency() {
  const m = useMessages();
  const { count, duration } = useFigures();
  const { locale } = useLocale();
  const sync = useSync();
  const [range, setRange] = useState<DatabaseLatencyRange>("oneHour");
  const [asTable, setAsTable] = useState(false);
  const load = useCallback(() => sync.admin.databaseLatency(range), [sync, range]);
  const { data, error, reload } = useAdminRead(load, LATENCY_REFRESH_MS);
  // The last answer stays drawn, faded, while another range loads.
  const [shown, setShown] = useState<{
    range: DatabaseLatencyRange;
    series: LatencySeries;
  } | null>(null);
  if (data != null && shown?.series !== data) {
    setShown({ range, series: data });
  }
  if (data === null) {
    return null;
  }
  const series = shown?.series;
  const loading = shown?.range !== range;
  const time = dateFormat(locale, TIME);
  const dayTime = dateFormat(locale, { ...DAY, ...TIME });
  const day = dateFormat(locale, DAY);
  const withinADay = shown?.range !== "sevenDays";
  const describe = (at: string) =>
    (shown?.range === "oneHour" ? time : dayTime).format(new Date(at));
  const axisDate = (at: string) => (withinADay ? time : day).format(new Date(at));
  const titles = { queries: m.admin.latencyQueries, poolWaits: m.admin.latencyPoolWaits };
  const hints = { queries: m.admin.latencyQueriesHint, poolWaits: m.admin.latencyPoolWaitsHint };
  return (
    <>
      <div>
        <h3 className="text-sm font-semibold text-ink-muted">{m.admin.databaseLatency}</h3>
        <p className="text-sm text-ink-muted">{m.admin.databaseLatencyHint}</p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <ToggleButtonGroup
          aria-label={m.admin.growthRange}
          selectionMode="single"
          disallowEmptySelection
          selectedKeys={[range]}
          onSelectionChange={(keys) => {
            const [next] = Array.from(keys);
            if (next !== undefined) {
              setRange(next as DatabaseLatencyRange);
            }
          }}
          className="flex flex-wrap gap-1"
        >
          {RANGES.map((r) => (
            <ToggleButton key={r} id={r} className={toggleChipClass}>
              {m.admin.latencyRanges[r]}
            </ToggleButton>
          ))}
        </ToggleButtonGroup>
        <ToggleButton
          isSelected={asTable}
          onChange={setAsTable}
          className="ms-auto rounded-md px-2 py-1 text-sm text-ink-muted underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {asTable ? m.admin.showCharts : m.admin.showTable}
        </ToggleButton>
      </div>
      {error !== null && <ReadFailed error={error} onRetry={reload} />}
      <div
        aria-busy={series === undefined && error === null ? true : undefined}
        className="grid grid-cols-1 gap-3 md:grid-cols-2"
      >
        {series === undefined && error === null && <LoadingLabel />}
        {KINDS.map((kind) => {
          const points = series?.points ?? [];
          const recorded = points.flatMap((point) => {
            const figures = point[kind];
            return figures == null ? [] : [{ at: point.at, figures }];
          });
          return (
            <figure
              key={kind}
              className="flex min-w-0 flex-col gap-2 rounded-lg border border-line bg-surface-raised p-4"
            >
              <figcaption>
                <span className="block text-sm font-medium">{titles[kind]}</span>
                <span className="block text-xs text-ink-muted">{hints[kind]}</span>
              </figcaption>
              {series === undefined ? (
                error === null && <Skeleton className="h-48 w-full" />
              ) : recorded.length === 0 ? (
                <p className="text-sm text-ink-muted">{m.admin.latencyNone}</p>
              ) : asTable ? (
                <Table
                  label={titles[kind]}
                  headings={[
                    m.admin.when,
                    m.admin.latencyCount,
                    ...PERCENTILES.map((p) => m.admin[p.name]),
                  ]}
                  numeric={[1, 2, 3, 4]}
                  dimmed={loading}
                >
                  {recorded.reverse().map(({ at, figures }) => (
                    <tr key={at}>
                      <Cell>{describe(at)}</Cell>
                      <Cell numeric>{count(figures.count)}</Cell>
                      {PERCENTILES.map((p) => (
                        <Cell key={p.key} numeric>
                          {duration(figures[p.key])}
                        </Cell>
                      ))}
                    </tr>
                  ))}
                </Table>
              ) : (
                <LineChart
                  label={format(m.admin.chartLabel, {
                    series: titles[kind],
                    range: m.admin.latencyRanges[shown?.range ?? range],
                  })}
                  ats={points.map((point) => point.at)}
                  series={PERCENTILES.map((p) => ({
                    name: m.admin[p.name],
                    color: p.color,
                    values: points.map((point) => point[kind]?.[p.key] ?? null),
                  }))}
                  describe={describe}
                  axisDate={axisDate}
                  formatValue={duration}
                  dimmed={loading}
                />
              )}
            </figure>
          );
        })}
      </div>
    </>
  );
}
