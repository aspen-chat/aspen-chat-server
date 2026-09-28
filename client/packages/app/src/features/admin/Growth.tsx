import type { Growth as GrowthSeries, GrowthRange } from "@aspen/protocol";
import { useCallback, useState } from "react";
import { ToggleButton, ToggleButtonGroup } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { count } from "@/features/admin/format";
import { LineChart } from "@/features/admin/LineChart";
import { useAdminRead } from "@/features/admin/useAdminRead";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";

const RANGES: readonly GrowthRange[] = [
  "threeMonths",
  "sixMonths",
  "oneYear",
  "fiveYears",
  "allTime",
];

const dayFormat = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
const monthFormat = new Intl.DateTimeFormat(undefined, { month: "short", year: "numeric" });
const shortMonthFormat = new Intl.DateTimeFormat(undefined, { month: "short", year: "2-digit" });

/** How a step of `unit` is named, in the tooltip and the table. */
function describer(m: Messages, unit: GrowthSeries["unit"]) {
  return (at: string) => {
    const date = new Date(at);
    switch (unit) {
      case "day":
        return dayFormat.format(date);
      case "week":
        return format(m.admin.weekOf, { date: dayFormat.format(date) });
      case "month":
        return monthFormat.format(date);
    }
  };
}

function axisDate(unit: GrowthSeries["unit"]) {
  return (at: string) =>
    unit === "month" ? shortMonthFormat.format(new Date(at)) : dayFormat.format(new Date(at));
}

/**
 * How many users and communities the deployment has had over a range, as two charts sharing
 * one set of range buttons. Two charts rather than one with two scales: a dual-axis chart
 * invents a relation between the lines, and on one scale the smaller series would lie flat.
 */
export function Growth() {
  const m = useMessages();
  const sync = useSync();
  const [range, setRange] = useState<GrowthRange>("threeMonths");
  const [asTable, setAsTable] = useState(false);
  const load = useCallback(() => sync.adminGrowth(range), [sync, range]);
  const { data, error, reload } = useAdminRead(load);
  // The last answer stays drawn, faded, while another range loads.
  const [shown, setShown] = useState<{ range: GrowthRange; series: GrowthSeries } | null>(null);
  if (data !== undefined && shown?.series !== data) {
    setShown({ range, series: data });
  }
  const series = shown?.series;
  const loading = shown?.range !== range;
  const describe = series === undefined ? () => "" : describer(m, series.unit);
  return (
    <Section id="admin-growth" title={m.admin.growth}>
      <div className="flex flex-wrap items-center gap-2">
        <ToggleButtonGroup
          aria-label={m.admin.growthRange}
          selectionMode="single"
          disallowEmptySelection
          selectedKeys={[range]}
          onSelectionChange={(keys) => {
            const [next] = Array.from(keys);
            if (next !== undefined) {
              setRange(next as GrowthRange);
            }
          }}
          className="flex flex-wrap gap-1"
        >
          {RANGES.map((r) => (
            <ToggleButton
              key={r}
              id={r}
              className="rounded-md border border-line px-3 py-1 text-sm text-ink-muted outline-none hover:bg-surface-hover pressed:bg-surface-hover selected:border-accent selected:bg-accent-soft selected:text-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              {m.admin.ranges[r]}
            </ToggleButton>
          ))}
        </ToggleButtonGroup>
        <ToggleButton
          isSelected={asTable}
          onChange={setAsTable}
          className="ml-auto rounded-md px-2 py-1 text-sm text-ink-muted underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {asTable ? m.admin.showCharts : m.admin.showTable}
        </ToggleButton>
      </div>
      {error !== null && <ReadFailed error={error} onRetry={reload} />}
      {series !== undefined &&
        (asTable ? (
          <Table
            label={m.admin.growth}
            headings={[m.admin.when, m.admin.users, m.admin.communities]}
            numeric={[1, 2]}
          >
            {[...series.points].reverse().map((point) => (
              <tr key={point.at}>
                <Cell>{describe(point.at)}</Cell>
                <Cell numeric>{count(point.users)}</Cell>
                <Cell numeric>{count(point.communities)}</Cell>
              </tr>
            ))}
          </Table>
        ) : (
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            {(["users", "communities"] as const).map((key) => (
              <figure
                key={key}
                className="flex min-w-0 flex-col gap-2 rounded-lg border border-line bg-surface-raised p-4"
              >
                <figcaption className="text-sm font-medium">{m.admin[key]}</figcaption>
                <LineChart
                  label={format(m.admin.chartLabel, {
                    series: m.admin[key],
                    range: m.admin.ranges[shown?.range ?? range],
                  })}
                  points={series.points.map((p) => ({ at: p.at, value: p[key] }))}
                  describe={describe}
                  axisDate={axisDate(series.unit)}
                  dimmed={loading}
                />
              </figure>
            ))}
          </div>
        ))}
    </Section>
  );
}
