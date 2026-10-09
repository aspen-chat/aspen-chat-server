# Roles and ranking

Code: `app::role`, `api::role`.

## The everyone role

Every member holds the community's everyone role (`everyone`, position 0). It cannot be renamed, moved, or deleted. Members hold whichever other roles they are given (`community_member_role`, listed as `roles` on their `userCommunity` record).

## Templates

A new community gets three roles, and its creator as owner, holding Admin too.

| Role | Permissions |
| --- | --- |
| everyone (member template) | Every channel permission but `mentionRoles` and `mentionEveryone`; `createInvites`; `changeNickname`. |
| Moderator | The member template, plus `mentionRoles`, `mentionEveryone`, `manageInvites`, `removeMembers`, `manageMessages`, `pinMessages`, `manageCalls`, `addBots`, `manageCustomEmoji`, `banMembers`, and `manageNicknames`. |
| Admin | Every permission. |

## Positions

Making, reordering, or deleting a role takes turns with every other such change in the community: the community's row is held (`hold_for_count`). It gives every role a dense position again in one statement (`app::role::renumber`).

- Each moved role's `role` update is announced with `app::events::publish_events`.
- `publish_events` hands every copy to NATS in order, then waits for the stream to acknowledge them together.
- A renumbering costs one round trip, and the stream still holds the updates in the order the database made them.

## Deleting a role

`app::role::retire_role` does this in the request:

1. marks the role deleted (`community_role.deleted_at`);
2. takes its permissions, hue, and showing apart away;
3. puts it below every role;
4. deletes its overrides;
5. announces its `delete`.

Every read of roles leaves deleted ones out. So from that moment the role grants nothing and ranks nobody.

A job (`purgeRole`; see [Jobs](../jobs/index.md)) then takes it off its holders and out of the tags of it, a thousand at a time, and deletes the row. So a role held by a million members is deleted as quickly as one held by none.

A bot's role goes the same way as the bot leaves.

## Ranking

Roles are ranked by position. The owner is above every role. A member may:

- manage, assign, or take away only roles below their highest;
- remove only members whose highest role is below it, never the owner;
- server-mute or remove from a call only such members;
- give a role or override only permissions they hold.

### Giving and taking roles

`app::role::set_member_role`:

- Giving someone a role takes holding every permission it allows, as making it would. So Assign roles never hands on more than its holder has.
- Taking a role away takes rank alone.
- The client offers to give only roles the caller could give.

### Deployment moderators

Handing on is judged by what the member is in the community alone.

- Managing, giving, and taking roles, reordering them, and setting overrides rank by the member's highest role (`CommunityAccess::require_role_above`, `role_rank`).
- They give only what their roles allow (`require_holds` reads `member_permissions`).
- So a deployment moderator who also holds Manage or Assign roles there reaches no further with them than their roles do.
- Moderate any community ranks them above every role only for what it takes away.

## Concurrent changes to one member

Each change to a member's roles:

1. locks the member's `community_user` row before it is made;
2. announces their whole list of roles as it then stands.

Concurrent changes to one member's roles are announced in the order they commit, each with every earlier one in it. The event feed's view of the roles they hold is the database's.
