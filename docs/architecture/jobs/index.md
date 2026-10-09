# Jobs

A job is a row of `job` that whichever server claims it runs in the background (`app::jobs`). Work that may take longer than a request should, and work that must happen on a schedule with nothing to cause it, is a job.

Work may take too long for a request because it touches however many rows a decision covers, or because it reaches another service.

## How a job comes to be

1. A request that decides something saves the job in its own transaction (`jobs::enqueue`).
2. The job exists exactly when the decision commits, and survives any restart.
3. The request answers at once.
4. The job's work follows within moments.

## Pages

- [Steps and failing](steps.md): kinds, bounded steps and their transactions, checkpoints, outcomes, retries and giving up, and recurring jobs.
- [Claiming and classes](claiming.md): the runner, leases, the `job_next` walk, classes and their places, and waking.
- [Kinds](kinds.md): every `JobKind`, its class, and what it does.
- [Watching jobs](watching.md): `GET /admin/jobs`, the dashboard's Jobs tab, the `jobs` operator command, metrics, and who may see jobs.
- [Design notes](design-notes.md): why jobs are shaped as they are.

## Key files

| Part | Where it lives |
|---|---|
| Saving, checkpointing, running, the overview | `app::jobs` (`server/app/src/jobs/mod.rs`) |
| Kinds and classes | `JobKind`, `JobClass` (`aspen_wire::job`, re-exported from `app::jobs`) |
| Recurring jobs | `jobs::recurring` |
| Operator command | `aspen-chat-server jobs` (`list`, `retry`, `cancel`) (`server/src/operator/jobs.rs`) |
| Settings | `[jobs] run`, `[jobs] concurrency` in `aspen.toml` |
