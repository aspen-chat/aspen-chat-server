import type { ApiServerHealth, VoiceServerHealth } from "@aspen/protocol";
import { CheckCircleIcon, ProhibitIcon, WarningIcon, XCircleIcon } from "@phosphor-icons/react";
import type { ComponentType, ReactNode } from "react";
import { useCallback } from "react";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { ago, bytes, count, rate, since } from "@/features/admin/format";
import { useAdminRead } from "@/features/admin/useAdminRead";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How often the fleet is read again while the dashboard is visible, as the servers report. */
const FLEET_REFRESH_MS = 10_000;

/**
 * The deployment's servers: every API server that has sent a heartbeat in the last half
 * minute, and every registered voice server with its last report.
 */
export function FleetHealth() {
  const m = useMessages();
  const sync = useSync();
  const load = useCallback(() => sync.fleet(), [sync]);
  const { data, at: now, error, reload } = useAdminRead(load, FLEET_REFRESH_MS);
  return (
    <Section id="admin-fleet" title={m.admin.fleet} hint={m.admin.fleetUpdated}>
      {error !== null && <ReadFailed error={error} onRetry={reload} />}
      <h3 className="text-sm font-semibold text-ink-muted">{m.admin.apiServers}</h3>
      {data?.apiServers.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.admin.noApiServers}</p>
      ) : (
        <Table
          label={m.admin.apiServers}
          headings={[
            m.admin.host,
            m.admin.status,
            m.admin.uptime,
            m.admin.streams,
            m.admin.requests,
            m.admin.errors,
            m.admin.memory,
            m.admin.database,
          ]}
          numeric={[3, 4, 5, 6]}
        >
          {(data?.apiServers ?? []).map((server) => (
            <ApiServerRow key={server.instance} server={server} now={now} />
          ))}
        </Table>
      )}
      <h3 className="text-sm font-semibold text-ink-muted">{m.admin.voiceServers}</h3>
      {data?.voiceServers.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.admin.noVoiceServers}</p>
      ) : (
        <Table
          label={m.admin.voiceServers}
          headings={[m.admin.name, m.admin.status, m.admin.load, m.admin.lastReport]}
          numeric={[2]}
        >
          {(data?.voiceServers ?? []).map((server) => (
            <VoiceServerRow key={server.id} server={server} now={now} />
          ))}
        </Table>
      )}
    </Section>
  );
}

function ApiServerRow({ server, now }: { server: ApiServerHealth; now: number }) {
  const m = useMessages();
  const failing = server.serverErrorsPerMinute > 0;
  return (
    <tr>
      <Cell>
        <span className="font-medium">{server.host}</span>
        <span className="block text-xs text-ink-muted">{server.version}</span>
      </Cell>
      <Cell>
        {failing ? (
          <Status icon={XCircleIcon} tone="text-danger" label={m.admin.failing} />
        ) : (
          <Status icon={CheckCircleIcon} tone="text-online" label={m.admin.healthy} />
        )}
      </Cell>
      <Cell>{since(server.startedAt, now)}</Cell>
      <Cell numeric>{count(server.eventStreams)}</Cell>
      <Cell numeric>{rate(server.requestsPerMinute)}</Cell>
      <Cell numeric>{rate(server.serverErrorsPerMinute)}</Cell>
      <Cell numeric>{server.residentBytes == null ? "—" : bytes(server.residentBytes)}</Cell>
      <Cell>
        {format(m.admin.dbConnections, {
          busy: count(server.dbConnections - server.dbConnectionsIdle),
          total: count(server.dbConnections),
        })}
      </Cell>
    </tr>
  );
}

function VoiceServerRow({ server, now }: { server: VoiceServerHealth; now: number }) {
  const m = useMessages();
  return (
    <tr>
      <Cell>
        <span className="font-medium">{server.name}</span>
        <span className="block text-xs break-all text-ink-muted">{server.url}</span>
      </Cell>
      <Cell>
        {!server.enabled ? (
          <Status icon={ProhibitIcon} tone="text-ink-muted" label={m.admin.disabled} />
        ) : server.reporting ? (
          <Status icon={CheckCircleIcon} tone="text-online" label={m.admin.reporting} />
        ) : (
          <Status icon={WarningIcon} tone="text-away" label={m.admin.silent} />
        )}
      </Cell>
      <Cell numeric>
        {format(m.admin.loadOf, {
          participants: count(server.participants),
          capacity: count(server.capacity),
        })}
      </Cell>
      <Cell>{server.lastReportAt == null ? m.admin.never : ago(server.lastReportAt, now)}</Cell>
    </tr>
  );
}

/** A state, never by color alone: its icon in the state's color, and its name. */
export function Status({
  icon: Icon,
  tone,
  label,
}: {
  icon: ComponentType<{
    size?: number;
    weight?: "fill";
    "aria-hidden"?: boolean;
    className?: string;
  }>;
  tone: string;
  label: string;
}) {
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
      <Icon size={16} weight="fill" aria-hidden className={tone} />
      {label}
    </span>
  );
}

/** A column heading with more than text: a control, and the order it sorts the table in. */
export interface Heading {
  content: ReactNode;
  sort?: "ascending" | "descending" | "none";
}

/**
 * A read-only table that scrolls sideways within itself on a narrow screen, so the page never
 * does. `numeric` names the columns whose figures line up.
 */
export function Table({
  label,
  headings,
  numeric = [],
  dimmed = false,
  children,
}: {
  label: string;
  headings: readonly (string | Heading)[];
  numeric?: number[];
  /** Shown faded, as while the next page loads. */
  dimmed?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={
        "overflow-x-auto rounded-lg border border-line bg-surface-raised transition-opacity" +
        (dimmed ? " opacity-50" : "")
      }
    >
      <table aria-label={label} className="w-full text-left text-sm">
        <thead className="border-b border-line text-xs text-ink-muted">
          <tr>
            {headings.map((heading, i) => (
              <th
                key={i}
                scope="col"
                aria-sort={typeof heading === "string" ? undefined : heading.sort}
                className={
                  "px-3 py-2 font-medium whitespace-nowrap" +
                  (numeric.includes(i) ? " text-right" : "")
                }
              >
                {typeof heading === "string" ? heading : heading.content}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-line">{children}</tbody>
      </table>
    </div>
  );
}

export function Cell({ numeric = false, children }: { numeric?: boolean; children: ReactNode }) {
  return (
    <td
      className={
        "px-3 py-2 align-top" + (numeric ? " text-right whitespace-nowrap tabular-nums" : "")
      }
    >
      {children}
    </td>
  );
}
