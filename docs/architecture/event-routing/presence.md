# Presence

Presence is kept in Valkey (`app::user_status`). Clients read it for the users they show, and an event stream connection that watches users is told of changes to them as they happen, gathered for up to a second (`app::presence_feed`; see [Telling of changes](#telling-of-changes)).

## States

| State | When |
| --- | --- |
| `online` | They have a connection and have used Aspen recently. |
| `away` | Connected but idle, or connected and they chose away. |
| `doNotDisturb` | Connected, and they chose do not disturb. |
| `invisible` | Connected, and they chose invisible. Told only to themself; everyone else is told `offline`. |
| `offline` | No connection is left. |

A chosen status holds only while they are connected: offline is offline whatever they chose.

## Valkey keys

| Key | Set by | Lifetime |
| --- | --- | --- |
| `user:{uuid}:online` | Connecting the event stream, or any authenticated request. Refreshed by the stream's pings. Each server marks one user at most every fifteen seconds (`user_status::MARK_EVERY`, a quarter of the key's minute). | Refreshed while connected |
| `user:{uuid}:active` | Each `activity` frame a client sends on its event stream while its user interacts with it. At most once a minute; the stream ignores closer ones. | `[presence] away_after_seconds` (ten minutes) |

- Finding `online` not set is the user coming online. Their arrival writes `user.last_seen_at`, copies their override (below), and only then sets `online`, so until it is done they show as offline to everyone. A mark that finds the key set renews it (`SET … XX`), and one that finds it not set leaves it to the arrival.
- What follows from either key being set is done in batches by one task per server (`app::presence_upkeep`), at most 100 people to a statement, on one database connection at a time: coming online's writes and override copy (below), and listing them in their communities. A person waits there at most once, so what waits is bounded by those this server marked online. Rows are locked in key order, since another server may write some of the same people's at once.
- Both keys belong to the user rather than a connection. Any active device keeps them online, and any connected one keeps them from going offline.
- A bot is never away. Whatever marks it connected sets its `active` key too, for as long.

## Choosing a status

A user may show invisible, away, or do not disturb in place of what their connections say, for a while or until they change it (`app::presence_override`).

| Part | Where |
| --- | --- |
| Logic | `app::presence_override` |
| Columns | `user.presence_override` (`invisible`, `away`, or `doNotDisturb`) and `presence_override_until` (`NULL` for one that lasts until changed) |
| Valkey copy | `user:{uuid}:override`, expiring at `presence_override_until` |
| Event | Custom `presenceOverrideChanged`, on the user's own subject |

| Endpoint | Does |
| --- | --- |
| `GET /users/@me/presence-override` | The override in force, or `null` fields for none |
| `PUT /users/@me/presence-override` | Sets one. `durationSeconds` is absent or `null` for until changed, at most thirty days (`MAX_OVERRIDE_SECONDS`). `201` when none was in force, `200` when it replaced one |
| `DELETE /users/@me/presence-override` | Ends it |

- An override that runs out ends on each device by its own clock, with no event. Valkey lets its copy go at the same moment, and the row's `until` has passed, so neither is read back.
- The copy is written while the transaction that changes the row holds it locked, so concurrent changes reach Valkey in the order they commit. A transaction that fails after writing it puts back what is committed.
- Coming online copies the row again (the same statement that writes `last_seen_at` reads it back), so a Valkey that lost the copy has it again, and sets `online` only after the copy. Wherever `online` exists the copy does too, so someone who chose invisible is never shown online, even right after Valkey lost its keys. An arrival that cannot read the row, or write the copy, leaves `online` unset: the user shows offline, and the next mark, at most fifteen seconds later while they stay connected, tries again.
- Deleting an account clears both.
- Clients set it on every deployment they use alike, since it is the person's, not one deployment's.

### What do not disturb does

Connected or not:

- No phone is woken for them, by a message or a plugin's notice (see [Push](../push.md)), and the `read` pushes sent them meanwhile carry a badge of 0.
- No DM call rings them (see [Calls in DMs](../threads-and-dms/dm-calls.md)).
- Their clients tell of nothing and show no unread marks or tag counts. What arrives is still unread once it ends.

### Who learns of it

- What they chose, and until when, reaches only them: the endpoints are `@me`'s, and the event their own subject's.
- Others learn only the status it makes, by the same rules as any presence (below): invisible is `offline` to them, and is neither counted online nor sorted among the connected in a member sample.
- An override changes nobody's access, so nothing open needs to be rechecked when it is set or ends.

## Reading presence

- Clients ask `GET /users/statuses?ids=…` (at most 100 ids) for the users they show.
- The reference client does so when it connects, once the server says it has taken up whom the connection watches (`presenceWatching`), and at most every two minutes after, since its event stream tells it of changes.
- Every record that carries `onlineStatus` fills it the same way for the caller: `GET /users/{user}`, the `users` sideloaded with messages, reactors, DMs, blocks, and report cases, members, and bots.

## Telling of changes

| Part | Where |
| --- | --- |
| Logic | `app::presence_feed` |
| Hints | Core NATS subject `aspen.presence` (`PRESENCE_SUBJECT`), a JSON array of user ids |
| Watching | The client frame `watchPresence`, at most `MAX_WATCHED_PRESENCE` (500) users |
| Telling | `ephemeral` frames of type `presence`, each a list of `{id, onlineStatus}`, and `presenceWatching` for each watch list taken up |
| Window | `PRESENCE_WINDOW_MILLIS` (one second) |

### Hints

A hint says only that someone's presence may have changed. It is made:

| By | When |
| --- | --- |
| `user_status::mark_user_online_id` | Their `online` key was not set: they came online |
| `user_status::mark_active` | Their `active` key was not set: they were away and used Aspen again |
| `presence_override` | They chose a status or ended one, once it is committed |
| `block` | They blocked someone or lifted a block, once it is committed |
| A timer at each expiry | Their `online` key, `active` key, or timed override ran out |

- The timers are kept by the server that set what runs out (`PresenceFeed::expires`), one per user and kind, the latest replacing the last, a second after the expiry. A key renewed by another server meanwhile makes a hint that changes nothing.
- One task per server gathers the hints made meanwhile into one message, at most 4096 users.
- A user's coming online is hinted once their override is copied and their `online` key set (`app::presence_upkeep`).

### Watching and telling

1. A connection names the users it shows with `watchPresence`. Each replaces the last; the router takes up at most one per window.
2. One router per API server keeps who watches whom. It reads every hint and keeps those for users watched here.
3. From the first hint or watch list kept, it waits one window, then:
   1. reads the presence of every user to tell of in batched `MGET`s (`user_status::raw_statuses`);
   2. decides in one query per 5000 pairs which watchers may learn each (`user_status::presence_visible_pairs`, the same rule as below);
   3. sends each connection one frame holding only what differs from what it was last told of each.
4. A user that a connection's watch list adds is told to it whatever they are, except the users of its first list, which its client reads whole over REST once the list is taken up (see [Design notes](design-notes.md#presence)). From the take-up on, every change is told.
5. Each list taken up is answered with a `presenceWatching` frame, at the telling that takes it up, or the next one when the connection's queue is full. A list dropped from a full queue is never answered, and its client sends it again.
6. Someone a watcher may no longer learn the presence of is told to them as `offline`.

A lost hint (a full queue, a server that stopped with timers pending, a core NATS message dropped) leaves a watcher behind until the next change or the client's next whole read.

### Cost

- Nothing is done for a change no connection here watches but reading its hint.
- A telling costs one batched read and one query per 5,000 pairs, per window per server.
- One telling decides at most `MAX_PAIRS_PER_TELLING` (50,000) connection and user pairs, so at most ten queries, changes first; the rest wait for the next window.
- Memory is one entry per connection per user watched, at most 500 per connection, and one timer per user and kind marked on this server.

## Who may see it

Someone's presence is told only (`app::user_status::presence_visible`, through `statuses_for`):

- to themself;
- to those who share a community or a DM with them;
- to those who own them as a bot;
- never to anyone they have blocked.

To everyone else it answers `offline`. The member sample orders those who blocked the caller as if they were not connected.

## Online count of a channel

`GET /channels/{channel}/presence` (`app::channel_presence`) counts how many people are online in a channel. It counts those whose status is online or do not disturb, not away or invisible:

- among the community's members who may view the channel (`app::visibility::viewers`); or
- among a DM's recipients.

A thread counts as its parent.

### The listing set

Each community has a Valkey sorted set, `community:{uuid}:online`, of members who may be online. Each is scored with the Unix time their listing runs out.

- Setting either of someone's presence keys lists them in each of their communities, for the longer of the two keys' lifetimes and half as long again.
- `user:{uuid}:listed`, living that half, keeps the fan-out to one write per community per half lifetime, however often they are active or connected.
- Joining a community lists the joiner there at once.
- So the set holds everyone connected, away or not.

### Counting

1. Read the listings that have not run out, dropping those that have.
2. Confirm each by its presence keys.
3. Keep those who may view the channel.

It costs as much as the number connected.

- Each server keeps a channel's count for ten seconds (`app::recent`). Requests that arrive while one is being worked out wait for it rather than starting their own.
- Nobody is counted for someone who may not learn their presence (`presence_visible`):
  - a DM's count keeps only those the caller may;
  - a community channel's shared count has the caller's blockers who are online and may view it now taken off.
- For the up to ten seconds the shared count lags a blocker coming online or leaving, the blocked caller's count is one off.
- The reference client asks for the open channel's count with each presence poll.

### The member sample

The member sample (see [Roles and permissions](../roles-and-permissions/index.md)) reads the same set. It keeps those whose keys say they are online, away, or in do not disturb, but not the invisible. Each server keeps a community's for ten seconds the same way.


## When access is given or taken away

1. **Who can observe it, and by which routes?** Those who may learn it (above), by REST (`GET /users/statuses`, and every record carrying `onlineStatus`) and by `presence` frames on an event stream that watches them. It is never pushed to a phone, searched, or carried by a stored event.
2. **What decides it, and where is that checked?** `presence_visible`, for REST reads in `statuses_for`, and for frames by `presence_visible_pairs` at each telling, never when a connection names whom it watches.
3. **When the deciding permission is lost, what happens to what is already open?** A block hints the blocker, so the blocked watcher is told `offline` within a window. Leaving a community, a removal, a ban, or a DM ending takes effect at the next change to that person, which the watcher is told as `offline`; meanwhile the watcher's client drops the member with the community's events, and so stops watching them. A sign-out, a password change, a ban from the deployment, or an account deleted closes the watcher's streams, which ends their watches.
4. **When it is gained, how does a client already open find out without a reload?** It watches whom it now shows, and is told each of them at the next telling, as a user its watch list adds is; on a new connection, it reads them whole.
5. **Does every path that changes it announce it?** Every change to a key or override hints, at once or by a timer at its expiry. A change to who may learn it hints only for a block; the rest are caught at the next change, or the client's next whole read.
6. **Is it published inside the transaction that makes the change?** Hints from overrides and blocks are made once the transaction commits; the rest change Valkey, which has no transaction. A hint is published on core NATS and lost with no harm beyond lateness.

[Design notes](design-notes.md#presence)
