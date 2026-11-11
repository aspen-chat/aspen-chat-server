# Bans

Code: `app::ban`, `api::ban`, table `community_ban`.

A ban removes a member as removal does and refuses them every way back in, until its `until` passes
or it is lifted. Every join, invite, and bot addition passes through `app::community::add_member`,
which refuses a banned user with `banned`. Its detail carries the reason they were given.

## Endpoints

All take Ban members.

| Endpoint                                      | Does                                                            |
| --------------------------------------------- | --------------------------------------------------------------- |
| `PUT /communities/{community}/bans/{user}`    | Bans. `201`, or `200` replacing a ban that stood.               |
| `DELETE /communities/{community}/bans/{user}` | Lifts.                                                          |
| `GET /communities/{community}/bans`           | Lists the standing bans, newest first. See [Listing](#listing). |

### Ban request fields

| Field                   | Meaning                                                |
| ----------------------- | ------------------------------------------------------ |
| `reason`                | Optional, at most `REASON_MAX_CHARS`.                  |
| `durationSeconds`       | At least a minute. Absent, the ban lasts until lifted. |
| `deleteMessagesSeconds` | 3600 or 86400. See below.                              |

### Listing

`GET /communities/{community}/bans` returns a page of at most `LIST_PAGE` (100) bans at a time,
newest first (index `community_ban_listed`), as `{data, nextCursor}`:

- `nextCursor` is the `cursor` of the page after, and `null` on the last. The read asks for one ban
  more than the page holds and gives a cursor only when it gets it, so a reader goes by `nextCursor`
  and never by how many a page holds. The client offers the next page under the list while the last
  answer gave a cursor.
- A cursor (`app::cursor`) is the place of the page's last ban, when it was made and whom it bans,
  the columns the list is ordered by. It is compared against the index as it stands, so the page
  after it is read whether or not that ban has been lifted since.
- Text that is not such a cursor, or is a cursor of another list (each names its own), is refused
  with `validation`.

### Deleting recent messages

With `deleteMessagesSeconds`, the person's messages from the last hour or day go with the ban.

- Only in the community's channels the banner may view, and their threads, as deleting them one by
  one would allow.
- This takes Manage messages besides Ban members.

## Expiry

Every read of bans leaves out those past their `until`. The recurring job `sweepExpired` deletes
them (see [job kinds](../jobs/kinds.md)).

## Ranking

Banning ranks as removal does: only someone below the banner's highest role, never the owner.
Someone who has already left may be banned by their id.

## Deployment moderators

A deployment moderator's actions are logged:

| Action                   | Log entry              |
| ------------------------ | ---------------------- |
| Ban                      | `banMember`            |
| Lift                     | `liftBan`              |
| Deleting recent messages | `deleteRecentMessages` |

## Events

`communityBan` events are on the community's subject and reach holders of Ban members. A ban that
stood is announced as its deletion and then the new one.
