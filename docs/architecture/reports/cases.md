# Cases

Reports gather in cases (`report_case`).

## What a case is about

| `kind` | One case per | Also records |
| --- | --- | --- |
| `message` | Reported message | `subject`: whose it is |
| `profile` | Reported person's profile | `subject` |
| `nickname` | Member's nickname in one community | `subject`, and the `community` |

A nickname case goes with its community, as a message's case goes with its message.

## Statuses

| Status | Meaning |
| --- | --- |
| `open` | Awaiting a reviewer |
| `resolved` | A reviewer has acted on it. Final |
| `dismissed` | Hidden and kept. Opened again when restored or reported again |

See [Resolving cases](resolving-cases.md) for how cases change status.

## Joining a case

Partial unique indexes allow one unresolved case per message, per profile, and per member and community. So:

- A report of something with an open or dismissed case joins it.
- Concurrent first reports make one case: `ON CONFLICT DO NOTHING` and a locked re-read.
- A report of something whose case was resolved opens a new one.
- A person reports one case once. A second report is `409` `alreadyReported`.

## `reportsChanged`

Every change to what awaits review publishes the custom `reportsChanged` event. It carries the case, and how many of the cases the reader may see are open, counted up to `MAX_COUNTED_CASES` (1000) as `GET /admin/report-counts` counts them.

It goes to each holder of Review reports who may see the case (see [Who may review](reviewing-cases.md#who-may-review-a-case)), on their own subject.
