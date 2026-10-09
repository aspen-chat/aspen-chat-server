# Catch-up and connection limits

## Retained events

The dispatcher holds what the stream retains: the last `MAX_EVENT_AGE`, indexed by owner, loaded from the stream at startup.

- It holds at most `event_retained_mib` (256) MiB of event text. Past that, the oldest events go early.
- A client resuming from before the oldest event held is answered `resumed: false`.

It serves every catch-up from it:

- `resumeAfter`;
- a first connection's replay window.

It applies the membership changes it passes over.

## Judging a catch-up by its moment

The database the connection was read from may already show the user's own changes retained in that window. So a community's events from before them are judged as they stood then. The catch-up gives:

- nothing from before the user's join;
- nothing of a community from before a change of their roles there (which roles they held before is not known);
- nothing that a deployment moderator alone reads from before a change of whether they moderate.

When the last two leave out what the database's reading would have shown, a resuming connection is answered `resumed: false`. Its client reads its state again rather than miss it.

## Registering a connection

A connection is registered inside the dispatcher's loop:

1. Registering gives its database connection back before it waits for the dispatcher. Connections waiting their turn hold none of the pool.
2. The dispatcher decides, from the user's own events alone, whether the sign-in ended and what the connection reads once caught up.
3. It takes the retained events the catch-up may pass through (`Snapshot`).
4. It sends them with the connection down its shard's ordered channel, ahead of the next event.
5. The shard judges and orders what the connection missed (`catch_up`, timed by `aspen_event_catch_up_duration_seconds`) and queues it.
6. The shard adds the connection before routing anything later.

So it sees each event once, a reconnect storm costs NATS nothing, and catch-ups are judged across the shards rather than one after another on the dispatcher.

## Serializing frames

Each event's frame is serialized once, by the first connection to write it (`FeedEvent::frame`), and shared by the rest.

## Limits

| Limit | Default | Close code and problem |
| --- | --- | --- |
| `[limits] max_event_streams_per_user`: streams one user holds on one API server | 20 | 4429, `tooManyStreams` |
| `[limits] max_event_streams_per_address`: streams one client address holds, counted from the upgrade, before `identify` | 200 | 4429, `tooManyStreamsFromAddress` |
| `event_queue_size`: events one connection's queue holds | 512 | The connection is dropped and its client resumes. |

- The stream counts are kept in the process (`app::event_feed::StreamCaps`), so a server that stops leaves no count behind.
- One stream more than a cap allows is closed.

## Sequence jumps

- If the feed's sequence jumps (NATS lost events, or the in-memory stream was recreated), every local connection is dropped. Each learns from `resumed: false` to rebuild.
- Sequence numbers are the stream's, and stay global and monotonic. So `resumeAfter` works unchanged with gaps in it.
