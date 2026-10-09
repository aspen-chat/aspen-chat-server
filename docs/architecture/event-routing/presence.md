# Presence

Presence is pulled, not pushed (`app::user_status`). Nothing announces a change; clients ask.

## States

| State | When |
| --- | --- |
| `online` | They have a connection and have used Aspen recently. |
| `away` | Connected but idle. |
| `offline` | No connection is left. |

## Valkey keys

| Key | Set by | Lifetime |
| --- | --- | --- |
| `user:{uuid}:online` | Connecting the event stream, or any authenticated request. Refreshed by the stream's pings. Each server marks one user at most every fifteen seconds (`user_status::MARK_EVERY`, a quarter of the key's minute). | Refreshed while connected |
| `user:{uuid}:active` | Each `activity` frame a client sends on its event stream while its user interacts with it. At most once a minute; the stream ignores closer ones. | `[presence] away_after_seconds` (ten minutes) |

- Setting `online` when it was not set is the user coming online, and writes `user.last_seen_at`.
- Both keys belong to the user rather than a connection. Any active device keeps them online, and any connected one keeps them from going offline.
- A bot is never away. Whatever marks it connected sets its `active` key too, for as long.

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

`GET /channels/{channel}/presence` (`app::channel_presence`) counts how many people are online in a channel. It counts those whose status is online, not away:

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

The member sample (see [Roles and permissions](../roles-and-permissions/index.md)) reads the same set. It keeps those whose keys say they are online or away. Each server keeps a community's for ten seconds the same way.

[Design notes](design-notes.md#presence)
