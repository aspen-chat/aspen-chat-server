# Role colours

## Fields

| Field | Meaning |
| --- | --- |
| `hue` | 0 to 359 around HSV's wheel. |
| `hoist` | The role is shown apart. |

- Both are set with Manage roles on roles below the caller, like any other field (`POST /communities/{community}/roles`, `PATCH /roles/{role}`).
- Both are announced as the role's update.
- Everyone's role has neither (`everyoneRolePlain`, and a check constraint).

## Drawing a name

- A member's name is drawn in the hue of the highest role they hold that has one.
- That holds only within that community: its channels, threads, calls, and member list.
- A deployment role's hue (see [Administration](../administration/index.md)) shows everywhere and is drawn over any community role's.
- The server stores and sends only the hue. Clients choose how light and how strong to draw it (the reference client's `theme/nameColors.ts`).
- Each install may turn name colours off.

## Authors outside the member sample

A message read sideloads `include=memberships`: its authors' memberships of the communities the messages were posted in (`app::community::read_authors_memberships`). A client can colour an author outside the member sample without reading them one by one. Whoever may read a channel's messages may see their authors' roles, as `read_community_member` already allows.

## Shown apart

Online holders of a role shown apart are listed under it in the member list, each under their highest. They come first in the sample (see [Members](members.md)).

## Access

Neither field grants anything.

- Who sees a community role's hue is who sees its roles: its members and deployment moderators, through the reads and events in [Access checks](access-checks.md).
- A member who leaves or is removed takes the community, its roles with it, out of their store.
