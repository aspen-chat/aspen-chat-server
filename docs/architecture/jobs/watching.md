# Watching jobs

Holders of View jobs and the operator can see what jobs are running, waiting, and given up.

## The overview

`GET /admin/jobs` (`jobs::overview`) takes the deployment permission View jobs. It gives the dashboard a preview of:

1. the jobs running,
2. then those waiting, by class and then by how long they have waited,
3. then the latest given up.

| Limit | Value |
|---|---|
| Jobs listed in all | `LISTED` (100) |
| Each count stops at | `MAX_COUNTED` (1000) |

- It gives how many of each there are.
- Recurring jobs are listed only while they run.
- It never shows what a job was given.
- Each part is a bounded walk of an index (`job_running`, `job_waiting`, `job_failed`), so it costs the same however many jobs wait.

The dashboard's Jobs tab reads it every five seconds while open. `aspen-chat-server jobs list` prints the same.

## Metrics

| Metric | By |
|---|---|
| `aspen_jobs_running` | kind |
| `aspen_jobs_finished_total` | kind |
| `aspen_job_wait_duration_seconds` (how long jobs waited to start) | class |
| `aspen_job_duration_seconds` (how long jobs ran) | kind |

## When access is given or taken away

1. **Who can observe it?** Holders of View jobs, through `GET /admin/jobs`, and the operator. No event announces a job. What a job does is announced by its steps, as the same change made in a request would be.
2. **What decides it?** `jobs::overview` requires View jobs. The terminal is the operator's.
3. **When it is lost:** the next read is refused. Nothing stays open, since the preview is read afresh every few seconds.
4. **When it is gained:** the dashboard shows the tab when it next reads the caller's deployment permissions, as every tab does.
5. **Does every path announce it?** A job's steps publish their events inside their own transactions. The job itself is not announced.
6. **Published inside the transaction?** Each step's events are. A step rolled back after it published is answered as any rollback is (`app::events::settle`).
