# Jobs

Work that may take longer than a request should (because it touches however many rows a
decision covers, or reaches another service), and work that must happen on a schedule with
nothing to cause it, is a job: a row of `job`, run in the background by whichever server claims
it (`app::jobs`). A request that decides something saves the job in its own transaction
(`jobs::enqueue`), so the job exists exactly when the decision commits, and survives any restart;
the request answers at once, and the job's work follows within moments.

## Steps

A job's kind (`JobKind`) decides what it does. Each kind works in steps, each bounded (a few
hundred rows, one mail, one preview) and each its own transaction, which publishes the events
its writes need before it commits, as every write does (see Event routing), inside
`events::settle_after`, as a request's work is, so what a step published is answered if it rolls
back and the rechecks of calls it noted run once it commits; and it writes how far
the job has come (`jobs::checkpoint`, the job's `progress`) in the same transaction, so what was
done and what the job says was done commit together. A server that stops part way leaves the
job where its last step left it, and the next to claim it carries on from there. Every step of
every kind can safely run again: a lease that runs out lets another server take a job whose step
had in fact finished, which then finds that work done. A step answers `Outcome::Done` (a one-shot
job is deleted, a recurring one due again), `Continue` (it checkpointed itself) or `Progress`
(the runner checkpoints it), after which the next step follows at once, or `Later` (it cannot go
on yet; tried again after the time it gives, counting no attempt).

## Claiming and classes

Every API server runs a runner (`jobs::spawn_runner`), unless `[jobs] run` is off; private
workers do too. A runner claims due jobs by pushing their `not_before` past their lease (a
minute) under `FOR UPDATE SKIP LOCKED`, so no two servers run one job at once, and renews the
lease every twenty seconds while a step runs; a job whose server stopped is claimed again once
its lease runs out. A claim walks `job_next (kind, class, not_before, id)` once per kind the
server runs, so it never reads past jobs it cannot take, however many wait.

Each job has a class (`JobClass`), which orders them: `urgent` (taking access away from what is
open), `interactive` (what someone is watching a screen for), `normal` (cleaning up after a
decision made), `bulk` (work for many at once), and `maintenance` (upkeep). A runner claims each
class in that order every round, into the places it has free: one each class keeps for itself,
and `[jobs] concurrency` that any class may take, the earlier classes first. So urgent work is
never queued behind a newsletter, and upkeep, holding its own place, is never starved. A runner
looks for jobs every second, and at once when any server publishes on `aspen.jobs.wake`
(`jobs::wake`, for a job someone is waiting on, once what saved it commits) or one of its own jobs
ends.

## Failing

A step that fails is tried again after a backoff (ten seconds, doubling, at most an hour), from
the job's last checkpoint; each claim counts an attempt, and progress clears the count. A
one-shot job whose attempts run out (`max_attempts`, five) is given up: kept with `failed_at`
and its last error, so the operator can see it and try it again (`aspen-chat-server jobs retry
<id>`) or cancel it (`jobs cancel <id>`). A recurring job is never given up; it waits out its
backoff and runs again.

## Recurring jobs

A recurring job is declared in code (`jobs::recurring`, its period from the configuration where
there is one), saved by each server as it starts, with `every` and its class brought up to what
the code says. Once done it is due a period after it was last due (`due`), as many periods on as
put it in the future, so a run that took long, or a deployment that was down, skips the periods
it missed rather than running them all at once.

## Kinds

| Kind | Class | What it does |
| --- | --- | --- |
| `closePoll` | interactive | Closing one poll at its deadline (see Background tasks), saved with the poll and keyed by it. |
| `reapVoice` | normal, every fifteen seconds | Ending the calls of voice servers silent for `session_silence_seconds` and calls alone for `idle_session_seconds`, fifty of each a step, each call in a transaction of its own, and clearing spent rings (see Voice). |
| `pruneFailedJobs` | maintenance, daily | Deleting jobs given up more than `FAILED_KEPT_DAYS` (30) ago, a thousand a step. |
| `sweepSignIns` | maintenance, hourly | Deleting sessions an hour after they expire and sign-ins a day after theirs (revoked ones included, which revoking expires), a thousand a step, through `session_expires` and `refresh_token_expires`. |
| `purgeRole` | normal | Taking a deleted role off its holders and the tags of it, a thousand a step, then deleting it (`role::purge_step`). Deleting a role (`role::retire_role`) marks it deleted, takes its permissions, hue, and showing apart away and puts it below every role at once, deletes its overrides, and announces it; every read of roles leaves deleted ones out, so it grants nothing and ranks nobody while its rows go. |
| `shutOut` | urgent | Signing out users from elsewhere: those whose homes the gates no longer admit, decided per home from the gates as each step runs, after a change to a gate or a list, or everyone of one home suspended for an unvouched key; a hundred a step, each in a transaction of its own (see Federation). |
| `purgeCustomEmoji` | normal | Taking a deleted custom emoji's reactions off, a thousand a step, then deleting it and its picture unless something took the picture up (`custom_emoji::purge_step`). Deleting an emoji marks it deleted and announces it at once; no read lists it, resolves it, or counts its reactions after, and its name is free again. |
| `forgetPluginScope` | normal | Deleting what plugins kept in a deleted community, channel, or account, and the timers set there (`plugin::storage::forget_step`): the scope's own values, then its channels' (or a channel's threads') a batch of five hundred channels at a time by id, each batch's values taken off their owner's share in the statement that deletes them; then a community's or an account's share. |
| `retirePlugin` | normal | Taking a removed plugin's notes away, which no read shows once it is removed, and its account out of its communities fifty at a time, announced as any member leaving is; also its account alone, for a plugin installed again without one (`plugin::install::retire_step`). |
| `purgePlugin` | bulk | Deleting everything a removed plugin kept, a thousand rows of each table a step (`plugin::install::purge_step`). |
| `recheckAllCalls` | urgent | Rechecking every call on the deployment, two hundred seats a step in order of session and user, for a change that touches them all: turning file transfers on or off (`voice::recheck_all_step`). Other rechecks, of one community, channel, category, or user, run on their own tasks after the change commits, at most `RECHECKS_AT_ONCE` (16) at a time on each server. |
| `deleteMessagesBy` | normal | A ban's deletion window (`message::queue_deletion_of_recent`): the banned person's messages after the window's start and before the ban, in the channels the banner could view then and their threads, or anywhere for a ban from the deployment. Two hundred a step, newest first, each batch deleted as `message::soft_delete_many` deletes (one statement for the messages, one for their echoes, their files kept as evidence, each thread's summary taken down once, and every deletion announced together), and the job's place moved past it in the same transaction. The ban answers how many it covers. |

## Watching jobs

`GET /admin/jobs` (`jobs::overview`), which takes the deployment permission View jobs, shows the
dashboard a preview: those running, then those waiting by class and then by how long they have
waited, then the latest given up, at most `LISTED` (100) in all, and how many of each, each count
stopping at `MAX_COUNTED` (1000); recurring jobs are listed only while they run. It never shows
what a job was given. Each part is a bounded walk of an index (`job_running`, `job_waiting`,
`job_failed`), so it costs the same however many jobs wait. The dashboard's Jobs tab reads it
every five seconds while open. `aspen-chat-server jobs list` prints the same. Each server
exports `aspen_jobs_running` and `aspen_jobs_finished_total` by kind, and how long jobs waited
to start by class (`aspen_job_wait_duration_seconds`) and ran by kind
(`aspen_job_duration_seconds`).

## When access is given or taken away

1. **Who can observe it?** Holders of View jobs, through `GET /admin/jobs`, and the operator. No
   event announces a job; what a job does is announced by its steps, as the same change made in
   a request would be.
2. **What decides it?** `jobs::overview` requires View jobs; the terminal is the operator's.
3. **When it is lost:** the next read is refused; nothing stays open, since the preview is read
   afresh every few seconds.
4. **When it is gained:** the dashboard shows the tab when it next reads the caller's deployment
   permissions, as every tab does.
5. **Does every path announce it?** A job's steps publish their events inside their own
   transactions; the job itself is not announced.
6. **Published inside the transaction?** Each step's events are, and a step rolled back after it
   published is answered as any rollback is (`app::events::settle`).
