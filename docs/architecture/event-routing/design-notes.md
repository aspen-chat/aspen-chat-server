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

## Reconnect storms

A deployment's people can all lose their connections at once and come back at once: a server restarts, or an ISP's customers come back from an outage in the middle of the evening. Past a minute away, each of them reads its whole state again, ten reads at once. See [Catch-up](catch-up.md#taking-on-streams).

- **New streams are taken on at the pace the server serves them.** A stream is cheap to keep and dear to start, since its client reloads right after. Taking every one as it comes would queue their reads for the database ahead of the requests of those already connected, until each waited out the pool and was refused, and tried again. So a server identifies a bounded number at once, and turns new ones away while requests already queue for the pool.
- **A stream turned away is told when to come back, and its client spreads its return.** A fixed wait would bring everyone it turned away back together. Each client waits what the server said and anywhere up to as long again.
- **Clients spread every retry over a window that grows.** A fixed interval, however long, keeps a crowd as synchronized as it arrived, and keeps offering the same load however overloaded the server is. A random wait in a doubling window ("full jitter") spreads a crowd out, and spreads it further the longer the server cannot take it.
- **A failed reload is tried again by itself.** Left to the person, everyone who saw it fail presses Retry, or reloads the page, at about the same moment. Tried again by the client, it is spread out like a reconnect and follows `Retry-After`.
- **Address limits count only those who have not signed in.** Many people may share one address, a carrier-grade NAT the most of all, and an outage at their ISP brings them all back together. Counted by address, they would be held to one share between them, and some locked out for as long as the rest stay. Once signed in on a connection or an event stream, it counts toward its user's share, which holds an account with many connections as the address did. The address limits then bound only what has not said who it is.
- **A signed-in connection whose user has no room stays on its address's share.** Signing in never closes a connection, so a request that presents a session is never refused for it.

## Presence

- **Presence is told only to those who watch it.** Every member being told of every member's changes costs as the square of a community; a connection names the few hundred users it shows, and only changes to those reach it. See [Presence](presence.md#telling-of-changes).
- **A hint names a user, not a status.** Hints from several servers can arrive out of order; the router reads each user's presence when it tells, so what it tells is what is true then, and a hint that changes nothing tells nobody.
- **Changes are gathered for a second.** A burst (a server restarting, a community's evening) is told in a few reads, queries, and frames rather than one each, and a second is too short to notice.
- **Who may learn it is decided when telling, not when watching.** A block or a community left takes effect at the next telling, which tells the watcher `offline`.
- **Expiries are hinted by timers on the server that set the key.** Valkey's notices of expired keys are lazy and cost every server a subscription to every expiry; a timer costs nothing beyond the one entry, and a key renewed elsewhere only makes a hint that changes nothing.
- **Clients still read presence whole now and then.** It catches what a lost hint or the 500-user cap left behind.
- **A telling decides at most 50,000 pairs, changes first.** After a crowd reconnects, every connection names hundreds of users to watch at once; deciding all of them in one go would hold the router, and a database connection, for minutes, while hints and watch lists queued behind it. The rest wait for the next window. Changes go first, since newly watched users' clients read them whole when they connect.
- **`online` is set only after the chosen status is copied.** The copy in Valkey is what hides an invisible user, and Valkey can lose it. Setting `online` first and copying after would show them online to anyone reading in between, however briefly, and a crowd coming online at once (after Valkey itself restarted, say) makes "briefly" long. Setting it after makes them offline in between instead, which is what they chose to show. A mark renews `online` only where it exists, in the same one call, so only an arrival ever creates it.
- **Coming online is written in batches, by one task per server.** Each arrival writes a row for every membership and copies the override to Valkey. A task of its own for each, in a crowd, would make as many tasks queueing for the pool ahead of everyone's requests. Batched, a hundred people share a statement, the task holds one connection, and a crowd makes the writes late rather than the requests. Rows are locked in key order, since another server may write some of the same people's at once.
- **A bot is never away.** It uses Aspen through the API rather than as a person does.
- **A chosen status is kept on the `user` row, with a copy in Valkey.** Statuses are read for whole member lists at once; a third key in the same `MGET` costs nothing more, where reading the row would add a query to every read. The row is what push and rings decide by, since do not disturb holds while the user is offline too, when nothing renews a Valkey key.
- **Do not disturb hides unread marks and counts, not just alerts.** It is for not being drawn back in: a badge draws as a chime does. Read positions are untouched, so nothing is lost when it ends.
- **A DM call does not ring someone in do not disturb at all.** A silent ring would still show others they are being rung; with none, the call shows in the DM like any other, for them to join if they look.
- **Presence answers `offline` to strangers and the blocked.** It does not tell them when a person is about.
- **Online counts are made on the server.** No client knows every member of a large community.
- **A sorted set of listed members per community.** Reading every member's keys would cost as much as the community is large; the set makes a count cost as much as the number connected.
- **`user:{uuid}:listed` throttles the listing.** The fan-out stays at one write per community per half lifetime.
- **Counts are cached ten seconds per server and shared.** Every viewer sees the same count. The trade-off is that a blocked caller's count can be one off for those ten seconds.
