# DMs and group DMs

DMs and group DMs are channels of type `dm` and `groupDm` that belong to no community (`app::dm`).

## Their people

- Their people are rows of `dm_recipient`.
- They ride on the channel record as `recipients`, empty for every other channel.

## Making and changing them

| Endpoint                                    | Does                                                                                                        |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `POST /users/@me/dms` with one other person | Returns their one-to-one DM, made on first use: `201`, else `200`                                           |
| `POST /users/@me/dms` with more people      | Always makes a new group DM, at most `app::dm::MAX_RECIPIENTS` (ten) people including the caller            |
| `PUT /channels/{channel}/recipients/{user}` | Adds someone to a group DM                                                                                  |
| `DELETE /channels/{channel}/recipients/@me` | The caller leaves a group DM                                                                                |
| `GET /users/@me/dms`                        | Lists the caller's DMs, the most recently active first, a page at a time. See [Listing them](#listing-them) |

- `channel.dm_key` holds a one-to-one pair's sorted ids under a unique index, so concurrent first
  messages make one DM.
- Adding and leaving are only for someone in the group. A deployment moderator who reads it is
  answered `404`, as by `channel_access`.
- A one-to-one DM can neither gain nor lose people.

## Listing them

`GET /users/@me/dms` lists the caller's DMs, the most recently active first, a page at a time.

| Parameter                                | Does                                                                                          |
| ---------------------------------------- | --------------------------------------------------------------------------------------------- |
| `cursor`                                 | The `nextCursor` of the page before                                                           |
| `limit`                                  | 50 by default, at most `MAX_DM_PAGE` (100)                                                    |
| `include=users`                          | Sideloads their people                                                                        |
| `include=voice`                          | Sideloads their calls, and the rings in force (see [Calls in DMs](dm-calls.md))               |
| `include=mutes`, `include=notifications` | Sideloads the caller's mutes and notification settings of every DM they are in, listed or not |

How recently a DM was active is `dm_recipient.active_at`, one for each of its people:

1. It is set when they join the DM.
2. Every message posted in the DM moves it on. The trigger `message_inserted`, which every insert of
   a message runs, does this, and also fills `message.home_channel`.
3. The list is read through the index `dm_recipient_by_activity`, however many DMs someone has.

The answer is `{data, included, nextCursor}`:

- `nextCursor` is the `cursor` of the page after, and `null` on the last. The read asks for one DM
  more than the page holds and gives a cursor only when it gets it, so a reader goes by `nextCursor`
  and never by how many a page holds.
- A cursor (`app::cursor`) is the place of the page's last DM, when it was last active for the
  caller and its channel, the columns the list is ordered by. It is compared against the index as it
  stands, so the page after it is read whether or not the caller is still in that DM.
- A DM that became active since the page before has moved above the cursor and is on no later page;
  the event that made it active tells its reader of it.
- Text that is not such a cursor, or is a cursor of another list (each names its own), is refused
  with `validation`.

`GET /admin/users/{user}/dms` pages the same way.

## Who may be in one

Everyone in a DM must share a community with whoever started it or added them.

A holder of the deployment permission Message any user may:

- Start a DM with anyone.
- Add anyone.
- Write in a one-to-one DM past a block, either way.

Blocks (see [Blocking](../blocking.md)):

- A block between the two people of a one-to-one DM leaves it readable and nothing more.
- No group DM may bring two people with a block between them together, Message any user or not.

## Who may read one

Only recipients may read or write a DM or anything in it, its threads included. Everyone else is
answered `404` by every channel, message, reaction, poll, and thread endpoint
(`app::permissions::channel_access`). No role reaches into a DM.

### Moderate any community

The one deployment power that reaches into a DM is Moderate any community (see
[Administration](../administration/index.md)). It reaches it only where a path asks for it by name:

| Function                    | For                                                                                                                                                             | Logging                                                                                                                   |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `channel_access_reading`    | Reading what is in it: its messages, one message, its pins, a poll, who reacted, an attachment, and messages sideloaded by `include=echoes` or `include=linked` | Writes the reading to the moderation log before anything is read, once per DM however many of its messages a read returns |
| `channel_access_moderating` | Taking something out of it, or reading its record                                                                                                               | Each such action logs its own action                                                                                      |

Every other path, search included, finds no DM a moderator is not in.

**Why:** a new path cannot read a DM unlogged by forgetting to log. See
[design notes](design-notes.md#dm-access).
