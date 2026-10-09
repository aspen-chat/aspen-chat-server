# Presence

Presence is pulled, not pushed (`app::user_status`). Nothing announces a change; clients ask.

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

- Setting `online` when it was not set is the user coming online, and writes `user.last_seen_at`.
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
- Coming online copies the row again (the same statement that writes `last_seen_at` reads it back), so a Valkey that lost the copy has it again before anyone could be shown it.
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
- The reference client does so every thirty seconds while the page is visible.
- Every record that carries `onlineStatus` fills it the same way for the caller: `GET /users/{user}`, the `users` sideloaded with messages, reactors, DMs, blocks, and report cases, members, and bots.

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

[Design notes](design-notes.md#presence)
