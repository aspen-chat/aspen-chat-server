import { ApiProblemError, type JobEntry, type JobsOverview } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "medium" };

/** How often the preview is read again while the tab is open and the page shown. */
export const JOBS_REFRESH_MS = 5_000;

/** The most a count goes to; the server stops counting there. */
const MAX_COUNTED = 1000;

function countText(count: number): string {
  return count >= MAX_COUNTED ? `${String(MAX_COUNTED - 1)}+` : String(count);
}

/**
 * A preview of the deployment's background jobs, read again every few seconds while the tab is
 * open and the page visible: those running, those waiting by class and then by how long, and the
 * latest given up with why, at most 100 in all. A glance at what the server is doing, not a tool
 * for finding out why; the operator has `jobs list` and the logs for that.
 */
export function Jobs() {
  const m = useMessages();
  const sync = useSync();
  const [overview, setOverview] = useState<JobsOverview | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = () => {
      if (document.visibilityState !== "visible") {
        timer = setTimeout(read, JOBS_REFRESH_MS);
        return;
      }
      sync.admin.jobs().then(
        (read) => {
          if (current) {
            setOverview(read);
            setError(null);
            timer = setTimeout(readAgain, JOBS_REFRESH_MS);
          }
        },
        (e: unknown) => {
          if (current) {
            setError(e instanceof ApiProblemError ? e.message : String(e));
          }
        },
      );
    };
    const readAgain = () => {
      read();
    };
    read();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [sync, attempt]);

  return (
    <Section id="admin-jobs" title={m.admin.jobs} hint={m.admin.jobsHint}>
      {error !== null && (
        <ReadFailed
          error={error}
          onRetry={() => {
            setAttempt((n) => n + 1);
          }}
        />
      )}
      <JobList
        title={format(m.admin.jobsRunning, {
          count: overview === undefined ? "…" : countText(overview.runningCount),
        })}
        jobs={overview?.running}
        when="started"
      />
      <JobList
        title={format(m.admin.jobsWaiting, {
          count:
            overview === undefined
              ? "…"
              : countText(overview.waitingCounts.reduce((sum, c) => sum + c.count, 0)),
        })}
        jobs={overview?.waiting}
        when="due"
      />
      <JobList
        title={format(m.admin.jobsFailed, {
          count: overview === undefined ? "…" : countText(overview.failedCount),
        })}
        jobs={overview?.failed}
        when="failed"
      />
    </Section>
  );
}

/** One part of the preview: its jobs, with when each started, fell due, or was given up. */
function JobList({
  title,
  jobs,
  when,
}: {
  title: string;
  jobs: readonly JobEntry[] | undefined;
  when: "started" | "due" | "failed";
}) {
  const m = useMessages();
  const timeFormat = useDateFormat(TIME);
  const at = (job: JobEntry) =>
    when === "started" ? job.runningSince : when === "failed" ? job.failedAt : job.due;
  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-sm font-semibold">{title}</h3>
      {jobs?.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.admin.jobsNone}</p>
      ) : (
        <Table
          label={title}
          headings={[
            { content: m.admin.jobKind },
            { content: m.admin.jobClass },
            {
              content:
                when === "started"
                  ? m.admin.jobStarted
                  : when === "failed"
                    ? m.admin.jobFailedAt
                    : m.admin.jobAge,
            },
            { content: when === "failed" ? m.admin.jobError : m.admin.jobAttempts },
          ]}
          numeric={when === "failed" ? [] : [3]}
          skeletonRows={jobs === undefined ? 3 : 0}
        >
          {(jobs ?? []).map((job) => {
            const time = at(job);
            return (
              <tr key={job.id}>
                <Cell>
                  <code className="text-xs">{job.kind}</code>
                  {job.recurring && (
                    <span className="ms-2 text-xs text-ink-muted">{m.admin.jobRecurring}</span>
                  )}
                </Cell>
                <Cell>{m.admin.jobClasses[job.class]}</Cell>
                <Cell>{time == null ? "" : timeFormat.format(new Date(time))}</Cell>
                {when === "failed" ? (
                  <Cell>
                    <span className="break-words">{job.error ?? ""}</span>
                  </Cell>
                ) : (
                  <Cell numeric>{job.attempts}</Cell>
                )}
              </tr>
            );
          })}
        </Table>
      )}
    </div>
  );
}
