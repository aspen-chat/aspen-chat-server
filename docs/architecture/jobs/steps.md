# Steps and failing

A job's kind (`JobKind`) decides what it does (see [Kinds](kinds.md)). Each kind works in steps.

## Code

| Piece | Code |
|---|---|
| Saving a job | `jobs::enqueue`, `jobs::enqueue_many` |
| Recording progress | `jobs::checkpoint` (the job's `progress`) |
| A step's answer | `jobs::Outcome` |
| Answering a rolled-back step | `events::settle_after` |
| Recurring jobs | `jobs::recurring` |

## A step

Each step is bounded: a few hundred rows, one mail, one preview. Each step is its own transaction, and in it:

1. It makes its writes.
2. It publishes the events its writes need before it commits, as every write does (see [Event routing](../event-routing/index.md)).
3. It writes how far the job has come (`jobs::checkpoint`, the job's `progress`).
4. It commits.

The step runs inside `events::settle_after`, as a request's work is. What a step published is answered if it rolls back, and the rechecks of calls it noted run once it commits.

### Resuming

- A server that stops part way leaves the job where its last step left it.
- The next server to claim it carries on from there.
- **Every step of every kind can safely run again.** A lease that runs out lets another server take a job whose step had in fact finished (see [Claiming](claiming.md#leases)). That server then finds the work done.

## Outcomes

| `Outcome` | What follows |
|---|---|
| `Done` | A one-shot job is deleted; a recurring one is due again. |
| `Continue` | The step checkpointed itself; the next step follows at once. |
| `Progress` | The runner checkpoints it; the next step follows at once. |
| `Later` | It cannot go on yet. It is tried again after the time it gives, counting no attempt. |

## Failing

- A step that fails is tried again after a backoff, from the job's last checkpoint. The backoff is ten seconds, doubling, at most an hour.
- Each claim counts an attempt. Progress clears the count.
- A one-shot job whose attempts run out (`max_attempts`, five) is given up. It is kept with `failed_at` and its last error.
- The operator can see a job given up and try it again (`aspen-chat-server jobs retry <id>`) or cancel it (`jobs cancel <id>`).
- A recurring job is never given up. It waits out its backoff and runs again.

Some kinds have backoffs and attempt counts of their own (see [Kinds](kinds.md)). Jobs given up are deleted after `FAILED_KEPT_DAYS` by `pruneFailedJobs`.

## Recurring jobs

1. A recurring job is declared in code (`jobs::recurring`). Its period comes from the configuration where there is one.
2. Each server saves it as it starts, bringing `every` and its class up to what the code says.
   A period made shorter takes effect at once: a job waiting longer than its new period is made due within it.
3. Once done, it is due a period after it was last due (`due`), moved on as many periods as put it in the future.

A run that took long, or a deployment that was down, skips the periods it missed rather than running them all at once.
