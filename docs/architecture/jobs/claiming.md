# Claiming and classes

Every server runs a runner that claims due jobs, orders them by class, and runs their steps.

## Code

| Piece | Code |
|---|---|
| The runner | `jobs::spawn_runner` |
| Waking runners | `jobs::wake`, `jobs::wake_from`, subject `aspen.jobs.wake` |
| Claim index | `job_next (kind, class, not_before, id)` |
| Classes | `JobClass` |

## Which servers run jobs

- Every API server runs a runner, unless `[jobs] run` is off.
- Private workers run one too.
- Some kinds run only on servers that can do their work (mail, previews, posters; see [Kinds](kinds.md)).

## Leases

- A runner claims due jobs by pushing their `not_before` past their lease (a minute), under `FOR UPDATE SKIP LOCKED`. No two servers run one job at once.
- It renews the lease every twenty seconds while a step runs.
- A job whose server stopped is claimed again once its lease runs out.

## The claim walk

A claim walks `job_next (kind, class, not_before, id)` once per kind the server runs. It never reads past jobs it cannot take, however many wait.

## Classes

Each job has a class (`JobClass`), which orders them:

| Class | For |
|---|---|
| `urgent` | Taking access away from what is open |
| `interactive` | What someone is watching a screen for |
| `normal` | Cleaning up after a decision made |
| `bulk` | Work for many at once |
| `maintenance` | Upkeep |

### Places

Every round, a runner claims each class in the order above, into the places it has free:

- one place each class keeps for itself, and
- `[jobs] concurrency` places that any class may take, the earlier classes first.

Urgent work is never queued behind a newsletter, and upkeep, holding its own place, is never starved.

## When a runner looks

A runner looks for jobs:

- every second,
- at once when any server publishes on `aspen.jobs.wake`, which it does through:
  - `jobs::wake`, for a job someone is waiting on, once what saved it commits, or
  - `jobs::wake_from`, after every operator command that succeeds, since what it decided may be a job; and
- at once when one of its own jobs ends.
