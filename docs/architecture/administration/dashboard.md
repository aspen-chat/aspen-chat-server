# The dashboard and fleet health

The Administration Dashboard is the deployment's console. This page covers what it reads: totals,
growth, the directories, and the fleet.

## What the dashboard reads

| Endpoint                                               | What it shows                                                                                                     | Permission                                                                                               |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `/admin/overview`                                      | The deployment's totals                                                                                           | View dashboard                                                                                           |
| `/admin/growth?range=…`                                | Growth of users and communities                                                                                   | View dashboard                                                                                           |
| User and community directories                         | Users and communities, searched, sorted, and paged                                                                | View dashboard, or any moderation permission but Message any user (`DeploymentPermissions::DIRECTORIES`) |
| `/admin/communities/{community}/reach` and `/takedown` | [Community takedowns](community-takedowns.md)                                                                     | Moderate any community, and Ban users to ban its members                                                 |
| `/admin/moderation-log`                                | The [moderation log](moderation.md#reading-the-log)                                                               | View dashboard                                                                                           |
| `/admin/file-transfers`                                | The record of files offered in calls ([File transfers](../file-transfers.md))                                     | Moderate any community                                                                                   |
| `/admin/registration-invites`                          | [Registration invites](registration-invites.md)                                                                   | Manage registration invites                                                                              |
| The fleet                                              | [Fleet health](#fleet-health), with [NATS memory](#nats-memory)                                                   | View dashboard                                                                                           |
| `/voice-servers`                                       | The registered voice servers, added, changed, and removed ([Voice servers](../voice/servers.md#managing-servers)) | Manage voice servers                                                                                     |
| `/admin/database-latency?range=…`                      | [Database latency](#database-latency)                                                                             | View dashboard                                                                                           |
| `/admin/jobs`                                          | A preview of background jobs ([Jobs](../jobs/index.md))                                                           | View jobs                                                                                                |

Powers too strong for even a privileged web API, such as suspending rate limits, stay terminal
commands.

## Totals

`/admin/overview` (`admin::current_totals`) reads the latest row of `deployment_stats`.

- The recurring job `recordStats` ([Jobs](../jobs/index.md)) writes it each hour, keeping one row a
  day.
- The totals are that row plus what was made and deleted since it was taken.
- Each of those is found through an index of its own, so the overview never counts a whole table.

## Growth

`range` is `threeMonths`, `sixMonths`, `oneYear`, `fiveYears`, or `allTime`.

- Points are users and communities at the end of each day up to half a year, each week up to two
  years, and each month beyond.
- Each step's point is the last day recorded in `deployment_stats` before the step ends.
- The last step's point is the totals now.
- The migration that made `deployment_stats` filled in every day before it.

## Directories

The user and community directories:

- search by any part of a name through `filter[name]`, or for one or two characters by its start
  (`admin::name_pattern`), served by trigram indexes either way;
- order by `sort`, a field, `-` prefixed for descending, newest first by default, each order walked
  through an index (`user_dashboard_name`, `community_dashboard_name`, `community_by_members`, the
  key for when it was made);
- count a community's members as `community.member_count`, which the recurring job `recountMembers`
  ([Jobs](../jobs/index.md)) recounts each hour;
- page by `offset`, at most `MAX_OFFSET` (10,000), and `limit`, 15 by default. **Why:** see
  [design notes](design-notes.md#directories-page-by-offset).

The user directory:

- leaves out the system account, as the totals and the growth chart do, since it is the deployment
  itself;
- takes `filter[banned]=true`, and gives each standing ban as `ban`;
- marks users of other deployments by their domain;
- is one of the directories every moderation permission opens;
- opens a person's profile card from their name (`ProfilePopover`), with what the reader may do to
  them across the deployment beneath it: their deployment roles, their DMs, a ban or its lifting,
  and deleting a bot whose owner is gone, each as the row's own controls offer it.

## Fleet health

Fleet health (`app::fleet`) comes from heartbeats.

1. Every `HEARTBEAT_INTERVAL` (ten seconds), each API server writes a heartbeat to the NATS
   key-value bucket `aspen_fleet`. Private workers do too, and show no streams or requests.
2. The heartbeat holds the server's uptime, open event streams, requests and server errors per
   minute, resident memory, and database connections.
3. Entries expire after thirty seconds, so a server that stops drops out.

The figures are counted where the matching Prometheus metrics are recorded. The heartbeat is how the
dashboard sees every server without reaching their loopback metrics endpoints.

### Voice servers

Voice servers' health is their registry rows: enabled, capacity, and whether their last report is
within `offer_silence_seconds`.

### Unregistered voice servers

A load report from an id no registered server has (a voice server given the wrong `id`) matches no
row.

1. The API server applying it notes the id in `aspen_fleet`, under `voice.unregistered.`
   (`fleet::note_unregistered_voice_server`).
2. The note lasts as long as a heartbeat does, so it stays while the voice server keeps reporting
   every fifteen seconds.
3. The fleet lists these as `unregisteredVoiceServers`.
4. The dashboard shows each with what to do, and shows every registered server's id beside them, to
   copy.

### Database latency

`GET /admin/database-latency` gives how long the database took over a range, as the servers saw it:
the median and the 95th and 99th percentiles of two things, on every server together.

- **Statements**: each statement's round trip on a pooled connection, from when it is handed to the
  connection until its answer has arrived. `BEGIN` and `COMMIT` are statements; the wait for NATS
  between them is not.
- **Waits for a connection**: each `Pool::get`, refused ones included: waiting for a connection to
  be free, opening one, and the pool's check that an idle one still answers. That check is one round
  trip, so a wait is never shorter than one.

**Why:** see [design notes](design-notes.md#database-latency).

How it is counted (`aspen_database::latency`):

1. `aspen_database::Pool` wraps the pool. `get` times the wait, and gives the connection a timer
   (Diesel's instrumentation) for as long as its caller holds it.
2. Statements sent together on one connection are answered in the order sent, so each answer ends
   the oldest statement under way. A statement whose future is dropped never ends; a new timer for
   each holder keeps the error to that holder.
3. Each duration is counted in a histogram of 104 fixed buckets, four to each doubling from a
   microsecond to about two minutes, so a percentile read from them is within an eighth of the
   truth.
4. The same durations are the Prometheus histograms `aspen_db_query_duration_seconds` and
   `aspen_db_pool_wait_duration_seconds`.

How it is kept (`app::database_latency`):

1. At the end of each minute by the clock, each server, private workers too, takes what it counted
   and adds it, bucket by bucket (`aspen_add_counts`), to rows of `database_latency`: one for the
   step that minute falls in, of each step length.
2. A row is one step, so it holds what every server counted in it, and its percentiles are the
   deployment's.
3. A minute that cannot be written, as while the database is away, is kept in memory and written
   when it can be, in its own place. A server keeps at most sixty.
4. The recurring job [`sweepExpired`](../jobs/kinds.md#sweepexpired) deletes steps older than their
   range reaches, so the table stays a few hundred rows.

| `range`     | Step                 | Steps |
| ----------- | -------------------- | ----- |
| `oneHour`   | a minute             | 60    |
| `oneDay`    | a quarter of an hour | 96    |
| `sevenDays` | an hour              | 168   |

A read is its range's rows, through the table's key. It gives every step of the range, oldest first,
ending with the step the last whole minute fell in; a step no server wrote has `null` figures, which
the chart draws as a gap.

Who sees it: holders of View dashboard, by this read alone. Nothing of it is in the event stream,
push, or a cache, so a holder who loses the permission is refused at their next read, and nothing
already open keeps showing more than they had read.

### NATS memory

`GET /admin/fleet` also gives `natsMemory` (`nats_memory::usage`), read from NATS as the request
arrives. See [NATS memory](../nats-memory.md).

- The scope's figures: bytes held, bytes reserved, and the limit. They are the NATS server's where
  `[nats] monitor_url` names its monitoring port and it answers, and this deployment's NATS
  account's otherwise, whose limit NATS gives only when the account has one.
- `monitorUnreachable` when a monitoring port is named and did not answer.
- Each stream: what it holds, the most it may, its messages, when its oldest was published, and
  whether its size is automatic. A stream NATS is without is `null` until the next `fitStreams`.
- The dashboard warns when the events stream holds nine tenths of its size, past which events go
  before their minute is up.
- `natsMemory` is `null` when NATS did not answer. The servers are still listed.
