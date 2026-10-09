# Event routing: design notes

The reasons behind the [event routing](index.md) design.

## The feed

### One consumer per server

- **Each API server reads the stream once, with one ordered consumer.** NATS does work in proportion to events times API servers, however many people are connected. See [Subjects and the feed](subjects-and-feed.md#the-event-feed).
- **Routing is split over `event_feed_shards` tasks.** The per-reader cost of fanning an event out to a big community is spread across cores.
- **Registration goes through the dispatcher's loop.** Each connection sees each event once, and a reconnect storm costs NATS nothing. See [Catch-up](catch-up.md#registering-a-connection).
- **Shards judge catch-ups, not the dispatcher.** The dispatcher only takes the retained events a catch-up may pass through. Catch-ups are then judged across the shards rather than one after another on the dispatcher.
- **Registering gives its database connection back before waiting for the dispatcher.** Connections waiting their turn hold none of the pool.

## Visibility

- **The model is attached to each event as it stood when routed.** A shard judges every reader by the permissions of the event's moment, in stream order. See [Visibility](visibility.md).
- **Events that change who may view a channel carry the model before as well.** Those losing view learn they have, those gaining it learn they may look the channel up, and nobody else learns the channel exists.
- **An unknown channel is viewed by nobody.** A new channel's overrides, published ahead of it, reach only those they let view it.
- **An unknown permission name is passed over.** A newer server's permission does not cost the whole change.

## Publishing

- **`publish_event` resolves the scope on the caller's connection.** An event published inside a transaction can name rows that transaction wrote. See [Publishing](publishing.md).
- **`expected_kind` has no wildcard arm.** An entity added without deciding its routing does not compile.
- **Events about a user carry one `Aspen-Event-Id` on every copy.** The client drops repeats.
- **`publish_events` sends every copy before awaiting any acknowledgement.** The caller waits one round trip for them all, and the stream still keeps the given order.

## Settling

- **Each request runs in a task of its own that finishes even when the client goes away.** Abandoning a request between its publishing and its commit rolls nothing back. See [Settling and resyncs](settling.md).
- **Events are published before their transaction commits, and a failure announces a resync.** One rolled back after publishing leaves events the database does not bear out; the resync tells readers to read again.
- **A failure after the publishing transaction committed announces nothing.** What it published is true.

## Ending sign-ins

- **`signInsEnded` and `accountBanned` carry `at`.** A new sign-in's first connection, replaying the window, is not closed by an end or a ban from before it existed. See [Ending sign-ins](sign-in-ends.md).
- **A registration resuming past a retained end is refused.** Its session was checked against the database when it identified, possibly before the end committed.

## Catch-up

- **Events from before the user's own changes in the window are judged as they stood then.** The database the connection was read from may already show those changes. See [Catch-up](catch-up.md#judging-a-catch-up-by-its-moment).
- **Events from before a role change are left out.** Which roles the user held before is not known. When that hides what the database's reading would have shown, `resumed: false` makes the client read its state again rather than miss it.
- **Stream counts are kept in the process.** A server that stops leaves no count behind.

## Presence

- **Presence is pulled, not pushed.** Nothing announces a change. See [Presence](presence.md).
- **A bot is never away.** It uses Aspen through the API rather than as a person does.
- **A chosen status is kept on the `user` row, with a copy in Valkey.** Statuses are read for whole member lists at once; a third key in the same `MGET` costs nothing more, where reading the row would add a query to every read. The row is what push and rings decide by, since do not disturb holds while the user is offline too, when nothing renews a Valkey key.
- **Do not disturb hides unread marks and counts, not just alerts.** It is for not being drawn back in: a badge draws as a chime does. Read positions are untouched, so nothing is lost when it ends.
- **A DM call does not ring someone in do not disturb at all.** A silent ring would still show others they are being rung; with none, the call shows in the DM like any other, for them to join if they look.
- **Presence answers `offline` to strangers and the blocked.** It does not tell them when a person is about.
- **Online counts are made on the server.** No client knows every member of a large community.
- **A sorted set of listed members per community.** Reading every member's keys would cost as much as the community is large; the set makes a count cost as much as the number connected.
- **`user:{uuid}:listed` throttles the listing.** The fan-out stays at one write per community per half lifetime.
- **Counts are cached ten seconds per server and shared.** Every viewer sees the same count. The trade-off is that a blocked caller's count can be one off for those ten seconds.
