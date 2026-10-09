# Blocking

A user may block anyone, for themself alone.

| Part | Where |
| --- | --- |
| Logic | `app::block` |
| Table | `user_block`: the blocker and the blocked, both cascading |
| Event | Custom `userBlockChanged`, on the blocker's own subject |

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /users/@me/blocks/{user}` | Blocks (`201`, or `200` when they were blocked already) |
| `DELETE /users/@me/blocks/{user}` | Unblocks |
| `GET /users/@me/blocks` | Lists them, most recent first, with `include=users` |

A person may block at most 10,000 others (`block::MAX_BLOCKS`), counted with the blocker's row held. One more is refused with a `validation` detail.

Each change is published to the blocker's own subject as `userBlockChanged`. The blocked user is never told.

## What the client does

Most of what a block does is the blocker's client's to do:

- it collapses the blocked user's messages into a row the blocker may open;
- it silences them and hides their screens in calls.

## What the server does

The server does what the client cannot.

### DMs

- No one-to-one DM is started or written in while a block stands, either way.
- `channel_access` leaves both people only View channel there. Every write (messages, edits, reactions, threads, polls, votes, pins, calls) answers `403` `blocked`, which does not say who blocked whom.
- The history stays readable.
- `POST /users/@me/dms` still returns a DM made before the block.
- No group DM is started with, or joined by, two people with a block between them. One already holding both stays open to both.

### Counts and lists

| Effect | Where |
| --- | --- |
| A blocked user's messages never make a channel unread for the blocker | `app::read_state` |
| Their reactions are left out of the blocker's summaries and reactor lists, so the blocker's counts may differ from everyone else's | `app::react` |
| Poll votes still count, since they decide the poll | |

### Presence

- The blocker's presence reads as `offline` to the blocked user everywhere it is given (`app::user_status::presence_visible`).
- A block, and lifting it, tells a blocked user whose stream watches the blocker of the change within a second (`app::presence_feed`).
- The blocker is left out of every channel's count of who is online for the blocked user (`app::channel_presence`).
- So a block keeps the blocked user from watching when the blocker is about.
- The blocker still sees the blocked user's presence, which their client may hide.
