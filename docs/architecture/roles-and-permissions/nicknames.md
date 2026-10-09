# Nicknames

A member may go by a nickname in a community (`community_user.nickname`, `nickname` on their `userCommunity` record). It is shown there in place of their display name: in its channels, threads, calls, member list, and tags.

## Setting and clearing

| Request | Takes | Code |
| --- | --- | --- |
| `PATCH /communities/{community}/members/@me` with a nickname (trimmed, 1 to `DISPLAY_NAME_MAX_CHARS` characters) | Change nickname | `app::community::update_membership` |
| The same with `null`, clearing one's own | Nothing | `app::community::update_membership` |
| `DELETE /communities/{community}/members/{user}/nickname` for oneself | Nothing | `app::role::clear_nickname` |
| The same for someone else | Manage nicknames, for a member ranked below the caller, never the owner, as removal ranks | `app::role::clear_nickname` |

- A deployment moderator may clear one too, since Moderate any community includes Manage nicknames. It is logged as `clearNickname` when the community's own permissions would not have allowed it.
- Nobody sets another's nickname.
- Losing Change nickname keeps the nickname a member has. They may still clear it, and moderators may.

## Announcing

A nickname is announced as the membership's update on the community's subject, which every member and deployment moderator receives. The list position, theirs alone, is left out (`for_community`).

## Reads

Every read of a membership carries it:

- the member sample;
- member search, which matches nicknames and orders by the name the community shows;
- one member's read;
- message reads' `include=memberships`.

So whoever may see someone's roles in a community sees their nickname there, and nobody else does.

## Lifetime

It goes with the membership. Leaving, removal, or a ban takes it, and the client's store drops it with the community.

A nickname may be reported (see [Reports](../reports/index.md)), and a review may clear it.
