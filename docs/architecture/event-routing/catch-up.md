# Catch-up and connection limits

## Retained events

The dispatcher holds what the stream retains: the last `MAX_EVENT_AGE`, indexed by owner, loaded
from the stream at startup.

- It holds at most `event_retained_mib` (256) MiB of event text. Past that, the oldest events go
  early.
- A client resuming from before the oldest event held is answered `resumed: false`.
- It knows from when it holds every event (`Retained::holds_since`): from the moment the server
  began reading the stream, moved back to the whole window once the stream is read to its end and
  found well under its own `max_bytes` (NATS lets its oldest events go past it whatever their age),
  and moved forward by each event let go of early and by a gap.

It serves every catch-up from it:

- `resumeAfter`;
- a first connection's replay window.

It applies the membership, role, moderation, and DM changes it passes over, and delivers the profile
events of the partners the connection follows (see
[Subjects and the feed](subjects-and-feed.md#profiles-of-dm-partners)) from where each is followed.

## Judging a catch-up by its moment

The database the connection was read from may already show the user's own changes retained in that
window. So a community's events from before them are judged as they stood then. The catch-up gives:

- nothing from before the user's join;
- nothing of a community from before a change of their roles there (which roles they held before is
  not known);
- nothing that a deployment moderator alone reads from before a change of whether they moderate.

The model of a community a catch-up judges by must also have stood as it does at each retained
event. One the dispatcher adopts from the database (`Models::adopt`) after a retained change to who
may view a channel or category there already reflects the change, so the events before it carry a
model newer than their moment, and the model from before the change, which decided who was losing a
view, is lost. A catch-up through such a community from before that change is incomplete too. A
creation the adopted model already reflects is not counted: nobody viewed the channel before it.

## What `ready` says of the replay

Every `ready` frame carries what the catch-up could not vouch for, on a first connection as on a
resumed one:

| Field            | Says                                                                                                                                                                                             |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `resumed`        | Whether `resumeAfter` was honoured.                                                                                                                                                              |
| `replayComplete` | Whether the replay holds everything the connection may read of the window it covers: `false` when either of the two cases above left something out. The client reads its state again on `false`. |
| `replayedSince`  | When the window begins, when that is later than `MAX_EVENT_AGE` ago; `null` when the whole window was held, or the connection resumed. A client whose REST reads began before it reads again.    |
| `catchUpFollows` | Always `true`: the replay follows, ended by a `caughtUp` frame.                                                                                                                                  |

The replay is queued with the connection, so it is written right after `ready`, and a `caughtUp`
frame (`Delivery::CaughtUp`) follows it, written whether or not the replay held anything. Every
`event` after it happened live. A client connecting to a server whose `ready` lacks `catchUpFollows`
treats the stream as live from `ready`.

## Registering a connection

A connection is registered inside the dispatcher's loop:

1. Registering reads the user's communities, roles, models, and the people of their most recent DMs
   (`recent_dms`), then gives its database connection back before it waits for the dispatcher.
   Connections waiting their turn hold none of the pool.
2. The dispatcher decides, from the user's own events alone, whether the sign-in ended and what the
   connection reads once caught up.
3. It takes the retained events the catch-up may pass through (`Snapshot`): the user's own, each
   community's, and each followed profile's.
4. It sends them with the connection down its shard's ordered channel, ahead of the next event.
5. The shard judges and orders what the connection missed (`catch_up`, timed by
   `aspen_event_catch_up_duration_seconds`) and queues it.
6. The shard adds the connection before routing anything later.

So it sees each event once, a reconnect storm costs NATS nothing, and catch-ups are judged across
the shards rather than one after another on the dispatcher.

## Serializing frames

Each event's frame is serialized once, by the first connection to write it (`FeedEvent::frame`), and
shared by the rest.

## Limits

| Limit                                                                                                                         | Default | Close code and problem                            |
| ----------------------------------------------------------------------------------------------------------------------------- | ------- | ------------------------------------------------- |
| `[limits] max_event_streams_per_user`: streams one user holds on one API server                                               | 20      | 4429, `tooManyStreams`                            |
| `[limits] max_event_streams_per_address`: streams one client address holds that have not identified, counted from the upgrade | 200     | 4429, `tooManyStreamsFromAddress`                 |
| `event_queue_size`: events one connection's queue holds (at least two, for a catch-up and its `caughtUp`)                     | 512     | The connection is dropped and its client resumes. |
| `[limits] max_followed_dm_partners`: DM partners whose profiles one connection follows                                        | 100     | The oldest DMs' partners are not followed.        |

- The stream counts are kept in the process (`app::event_feed::StreamCaps`), so a server that stops
  leaves no count behind.
- One stream more than a cap allows is closed.
- A stream that identifies takes a place under its user's cap and gives back its address's. Its
  connection moves from its address's share of `[connections]` to its user's the same way
  (`max_per_user`; see [Taking on streams](#taking-on-streams)).

## Taking on streams

`app::stream_admission` decides whether a server takes on another stream now.

1. A stream's `identify` frame arrives.
2. If as many requests wait for a database connection as the pool holds, it is turned away.
3. Otherwise it waits for one of `[limits] max_identifying_event_streams` places (half the pool by
   default), for at most `ADMISSION_WAIT` (two seconds), and is turned away if none comes.
4. It holds the place while it is authenticated, rate limited, and registered with the feed, the
   last step that reads the database.
5. Its account's limits on `GET /events` (`user`, 30 a minute, 20 at once) are checked once the
   token says who it is: one more is closed with 4429, `rateLimited`, and `retryAfterSeconds`.

A stream turned away is closed with 1013, `serverBusy`, and `retryAfterSeconds` (`BUSY_RETRY_AFTER`,
ten). Its client waits that long and anywhere up to as long again, then connects as usual, and the
metric `aspen_event_stream_connects_total` counts it with `outcome="busy"`.

A stream is cheap to keep but dear to start: unless it resumes, its client reads its whole state
again right after (see [Design notes](design-notes.md#reconnect-storms)).

## Sequence jumps

- If the feed's sequence jumps (NATS lost events, or the in-memory stream was recreated), every
  local connection is dropped. Each learns from `resumed: false` to rebuild.
- Sequence numbers are the stream's, and stay global and monotonic. So `resumeAfter` works unchanged
  with gaps in it.
