# Read positions

How far each user has read each channel is a row of `read_state`: the user, the channel (both
cascading on delete), and a message id.

| Part     | Where                                                                     |
| -------- | ------------------------------------------------------------------------- |
| Logic    | `app::read_state`                                                         |
| Table    | `read_state`                                                              |
| Events   | Custom `channelRead` and `channelMarkedUnread`, on the user's own subject |
| Record   | `ReadState`: `lastRead`, `lastMessage`, `mentions`                        |
| Sideload | `include=readStates` on community reads, the DM list, and message reads   |

## The position

The id is a position, not a reference.

- Message ids are UUIDv7 and ordered by time, so "read up to this id" stays exact after that message
  is deleted.
- Deleting a message never writes here.

## Moving it

- `PUT /channels/{channel}/read-states/@me` with `lastRead`, answered `204`.
- Reading only moves a position forward. An older report is ignored. Marking unread is the one way
  back (see below).
- Each move is published to the user's own subject as `channelRead`, so their other devices follow.
- Posting a message or a poll moves the poster's position to it, in the same transaction.

### Many at once

`PUT /users/@me/read-states` moves several positions in one request
(`app::read_state::mark_many_read`), as an app does when its user marks everything read.

- Its body lists `positions`, each a `channel` and a `lastRead`, at most `MAX_MARKED` (100). More
  are refused with `validation`.
- Each moves as one does alone, and each move is published as `channelRead`.
- A position is passed over, not refused, when its channel is not one the caller may read or its
  message is not of that channel. What is marked read together may have lost a channel since it was
  listed.
- Access is decided for all of them together: the channels' communities through `Visibility`, and
  the DMs through `dm_recipient`, a thread through its parent.
- It has a rate limit of its own.

## Marking unread

The same `PUT /channels/{channel}/read-states/@me` marks a channel, DM, or thread unread
(`app::read_state::mark_unread`). Its body gives exactly one of `lastRead`, `unreadFrom`, and
`unreadNewest`; anything else is refused with `validation`.

| Field                | Marks unread from                                                                                                                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `unreadFrom`         | That message, of the channel and not deleted; otherwise not found                                                                                    |
| `unreadNewest: true` | The newest message by neither the reader nor anyone they blocked, among the channel's newest `NEWEST_LOOKED_AT` (100); with none there, `validation` |

- The position is set just before that message, back or forward: its id less one, as a 128-bit
  number, which names no message but orders exactly.
- Joining still bounds what is unread, so nothing from before it becomes unread.
- The new position and the read state it leaves (`lastMessage`, `mentions`, computed in the same
  transaction) are published as `channelMarkedUnread`, so the reader's devices take it whole.
- Their devices stop marking the channel read as they show it until it is opened again or the reader
  posts there (see the client's
  [Unread and muting](../../client/docs/architecture/unread-and-muting.md)).
- Phones are sent the new badge (see [Push](push.md)).
- It shares the endpoint's rate limit.

### Who may

Marking unread shows nothing new: it moves the caller's own position, and what it publishes goes to
the caller alone.

- Access is checked through `channel_access`, as reading the position is. A channel the caller may
  not view is not found, and nothing is published.
- A channel lost later leaves its position as it was. Community and DM reads leave the channel out,
  as they do any channel the caller may not view.
- `scripts/check_permissions.py` (`marking_unread`) checks both.

## Unread

- Nothing from before a member joined is unread to them. `community_user.joined_at` (and
  `dm_recipient.joined_at` for DMs) stands in for the position until they have read past it.
- A channel is unread while it holds a message by someone else after the position.
- The reader's own messages, and those of anyone they blocked, never make it unread.
- `ReadState.mentions` counts the unread messages that tag the reader (see [Tagging](tagging.md)),
  up to `MAX_COUNTED_MENTIONS` (100, which the apps show as "99+").

## Reading it

The `ReadState` record carries:

| Field         | Holds                                                                                 |
| ------------- | ------------------------------------------------------------------------------------- |
| `lastRead`    | The position                                                                          |
| `lastMessage` | The newest message by anyone else after the position, `null` when the channel is read |
| `mentions`    | Unread messages that tag the reader, up to `MAX_COUNTED_MENTIONS` (100)               |

These are computed in one query for every channel asked about. That query reads only from the
position on, so its cost grows with what is unread rather than with the channel's history:

1. The later of `lastRead` and the joining moment becomes one UUIDv7 bound (`aspen_uuid_floor`).
2. The newest message is found backwards through `message (channel, id)`, down to that bound.
3. The tags are counted through `mention`'s indexes on whom they tag: one branch each for the
   reader, everyone, and each role the reader holds.

- Community reads and the DM list sideload them with `include=readStates`.
- `GET /channels/{channel}/read-states/@me` reads one. A channel the caller may not view is answered
  as not found, as community reads leave such channels out.

## Threads

- A thread keeps a position too, moved by reading it and by posting in it.
- It is read only when asked for by name: `GET /channels/{thread}/read-states/@me`, or a message
  read's `include=readStates`.
- Community and DM reads leave threads out.
- Until its reader has a position there, it is the moment they joined where its parent belongs.
