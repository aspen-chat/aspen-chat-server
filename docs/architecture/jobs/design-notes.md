# Jobs design notes

Why jobs are shaped as they are. The how is in the [jobs pages](index.md).

## Saving and steps

- **A job is saved in the transaction of the request that decides it.** The job then exists exactly when the decision commits, and survives any restart, while the request answers at once. ([Jobs](index.md#how-a-job-comes-to-be))
- **Each step writes its checkpoint in its own transaction.** What was done and what the job says was done commit together. ([Steps](steps.md#a-step))
- **Each step runs inside `events::settle_after`.** What it published is answered if it rolls back, and the rechecks of calls it noted run once it commits, as for a request. ([Steps](steps.md#a-step))
- **Every step can safely run again.** A lease can run out on a step that had in fact finished; the server that takes the job next finds the work done. ([Steps](steps.md#resuming))

## Claiming

- **Claims use `FOR UPDATE SKIP LOCKED` and a lease pushed into `not_before`.** No two servers run one job at once, and a job whose server stopped is claimed again once its lease runs out. ([Claiming](claiming.md#leases))
- **A claim walks `job_next` once per kind the server runs.** It never reads past jobs it cannot take, however many wait. ([Claiming](claiming.md#the-claim-walk))
- **Each class keeps one place for itself, and shared places go to earlier classes first.** Urgent work is never queued behind a newsletter, and upkeep is never starved. ([Claiming](claiming.md#places))

## Recurring jobs

- **A recurring job skips the periods it missed.** A run that took long, or a deployment that was down, does not then run them all at once. ([Steps](steps.md#recurring-jobs))
- **A recurring job is never given up.** It waits out its backoff and runs again. ([Steps](steps.md#failing))

## Kinds

- **A deleted role is marked deleted and stripped at once, and its rows go in a job.** Every read of roles leaves deleted ones out, so it grants nothing and ranks nobody while its rows go. ([Kinds](kinds.md#purgerole))
- **A deleted emoji is marked deleted and announced at once.** No read lists, resolves, or counts it after, and its name is free again, while its reactions go in a job. ([Kinds](kinds.md#purgecustomemoji))

- **The database queues `forgetIcon` by trigger.** Neither a sweep of every icon nor every path that changes a picture needs to remember it. ([Kinds](kinds.md#forgeticon))

## Watching

- **Each part of the overview is a bounded walk of an index.** It costs the same however many jobs wait. ([Watching](watching.md#the-overview))
- **No event announces a job.** What a job does is announced by its steps, as the same change made in a request would be. ([Watching](watching.md#when-access-is-given-or-taken-away))
