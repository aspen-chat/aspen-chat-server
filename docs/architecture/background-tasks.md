# Background tasks

`app::context::start_background_tasks` starts every task that runs alongside the request handlers. `api::start` calls it on every API server, private workers included.

## The tasks

| Task | Where | Described in |
| --- | --- | --- |
| Deployment settings watcher | `app::deployment_settings::spawn_watcher` | [Administration](administration/index.md) |
| Voice report listener | `app::voice::spawn_report_listener` | [Voice](voice/index.md) |
| Answerer of voice servers' token key requests | `app::voice::spawn_token_key_answerer` | [Voice](voice/index.md) |
| Fleet heartbeat | `app::fleet::spawn_heartbeat` | [Administration](administration/index.md) |
| Push dispatcher | `app::push::spawn_dispatcher` | [Push](push.md) |
| Plugins and their observers | `app::plugin::registry::start` | [Plugins](plugins/index.md) |
| Job runner | `app::jobs::spawn_runner` | [Jobs](jobs/index.md) |

Everything else that runs on a schedule (mail and digests, previews and held messages, upload
sweeps, moving evidence, standing passes, the voice reaper) is a job.

## Closing polls

A poll is closed at its deadline by a job saved with it (`closePoll`, keyed by the poll; see [Jobs](jobs/index.md)). The job runs `app::poll::close_at_deadline`, which:

1. marks the poll closed under its row's lock;
2. publishes its final tally;
3. posts the `poll_closed` message.

For a poll closed early or deleted, the job finds nothing to do.

Vote changes lock the poll row too, so the tallies in successive events never go backwards.

### Tallies

A tally is counted in SQL (`app::poll::load_results`):

- each answer's count, over the votes' key;
- each answer's first `SHOWN_VOTERS` (five) voters, through `poll_vote_by_option`.

So a vote costs the same however many have voted, and its event carries five names, not everyone.

`GET /polls/{poll}/votes/{option}` lists every voter of an answer, earliest first, a page at a time. It is refused on an anonymous poll.

### Closing early

`POST /polls/{poll}/close` closes one before its deadline the same way (`app::poll::close_poll`, under the row's lock).

- Its creator may close it.
- So may a holder of Manage messages in its channel. Their use of it as a deployment moderator is logged (`closePoll`).
- A poll already closed is answered as it stands.

### Voting

- Casting and withdrawing a vote both take viewing the poll's channel (`channel_access`; `404` otherwise).
- Both are refused in a blocked DM.
- So someone who loses the channel changes its tallies no more.

Plugins that intercept `message.create` where a poll is decide its question and answers, and each written-in answer, before they are saved (`intercept::decide_unchanged`; see [Plugins](plugins/index.md)).

## Message kinds

Messages have a `kind`:

| Kind | Notes |
| --- | --- |
| `standard` | The only kind that can be edited |
| `poll`, `poll_closed` | Carry no content and name their poll in `poll`. The client renders them from the poll record |
| `thread_echo` | See [Threads and DMs](threads-and-dms/index.md) |
| `call`, `missed_call` | See [Threads and DMs](threads-and-dms/index.md) |
| `command` | Text its bot has already answered |

Only a `standard` message can be edited, and only by its author (`app::message::update_message`). The other kinds hold no text of their author's, or, for a command, text its bot has already answered.

## Poll write-ins

A poll's creator may allow write-ins (`allowWriteIns`).

- Each voter who may post in its channel (Send messages, or Send in threads in a thread) may add one answer of their own with `POST /polls/{poll}/write-ins`. This also votes for it for them.
- An answer the poll already has, compared ignoring case and spacing, is voted for instead (`200` rather than `201`).
- Written-in answers are options numbered after the creator's, listed in `writeIns` with who wrote each.
- Who wrote each is left out on an anonymous poll, where it would say how they voted.
- `pollVotes` comes with `ownWriteIns`, the caller's own, so a writer can still find theirs.

### Removing a write-in

`DELETE /polls/{poll}/write-ins/{option}` is offered to its writer and the poll's creator. It takes the answer's votes and leaves `null` at its index.

**Why:** votes are cast by index, and renumbering would move a vote already on its way.
