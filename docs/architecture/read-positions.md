# Read positions

How far each user has read each channel is a row of `read_state`: the user, the channel (both cascading on delete), and a message id.

| Part | Where |
| --- | --- |
| Logic | `app::read_state` |
| Table | `read_state` |
| Event | Custom `channelRead`, on the user's own subject |
| Record | `ReadState`: `lastRead`, `lastMessage`, `mentions` |
| Sideload | `include=readStates` on community reads, the DM list, and message reads |

## The position

The id is a position, not a reference.

- Message ids are UUIDv7 and ordered by time, so "read up to this id" stays exact after that message is deleted.
- Deleting a message never writes here.

## Moving it

- `PUT /channels/{channel}/read-states/@me` with `lastRead`, answered `204`.
- A position only moves forward. An older report is ignored.
- Each move is published to the user's own subject as `channelRead`, so their other devices follow.
- Posting a message or a poll moves the poster's position to it, in the same transaction.

## Unread

- Nothing from before a member joined is unread to them. `community_user.joined_at` (and `dm_recipient.joined_at` for DMs) stands in for the position until they have read past it.
- A channel is unread while it holds a message by someone else after the position.
- The reader's own messages, and those of anyone they blocked, never make it unread.
- `ReadState.mentions` counts the unread messages that tag the reader (see [Tagging](tagging.md)), up to `MAX_COUNTED_MENTIONS` (100, which the apps show as "99+").

## Reading it

The `ReadState` record carries:

| Field | Holds |
| --- | --- |
| `lastRead` | The position |
| `lastMessage` | The newest message by anyone else after the position, `null` when the channel is read |
| `mentions` | Unread messages that tag the reader, up to `MAX_COUNTED_MENTIONS` (100) |

These are computed in one query for every channel asked about. That query reads only from the position on, so its cost grows with what is unread rather than with the channel's history:

1. The later of `lastRead` and the joining moment becomes one UUIDv7 bound (`aspen_uuid_floor`).
2. The newest message is found backwards through `message (channel, id)`, down to that bound.
3. The tags are counted through `mention`'s indexes on whom they tag: one branch each for the reader, everyone, and each role the reader holds.

- Community reads and the DM list sideload them with `include=readStates`.
- `GET /channels/{channel}/read-states/@me` reads one. A channel the caller may not view is answered as not found, as community reads leave such channels out.

## Threads

- A thread keeps a position too, moved by reading it and by posting in it.
- It is read only when asked for by name: `GET /channels/{thread}/read-states/@me`, or a message read's `include=readStates`.
- Community and DM reads leave threads out.
- Until its reader has a position there, it is the moment they joined where its parent belongs.
