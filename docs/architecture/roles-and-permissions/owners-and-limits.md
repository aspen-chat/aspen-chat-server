# Owners and limits

## The owner

The community's `owner` holds every permission and is above every role.

- Only the owner may delete the community or hand it on (`PUT /communities/{community}/owner`).
- The owner cannot leave until they have handed it on.

### Naming an owner from the terminal

`aspen-chat-server communities unowned|set-owner <community> <username>` lists the communities with no owner and names one. A community has none when its owner's account was deleted with nobody left to take it, or when the `community_owners` migration found no member whose account still existed. It is announced as a handover is (`app::role::set_owner`).

### When an owner's account is deleted

`app::user::retire` passes each community they own to the member whose highest role ranks highest, the earliest to join among equals.

- Left out: bots, the system account, and anyone deleted or banned from the deployment.
- The new owner is told so by the system account.
- A community with nobody left is left without an owner.

## Deleted communities

A deleted community keeps its rows but counts for nothing wherever memberships are read. `app::community::live` is the subquery those reads keep to. For a deleted community:

- its members find nothing of it in search (`Visibility` drops it);
- it shares nothing for DMs or presence;
- it does not count toward the membership cap;
- event streams no longer read it (`app::events::memberships`);
- its invites read as not found and join nobody;
- `app::community::add_member` refuses every way in. It holds the community's row until the membership commits, so a deletion beside it waits.

## Caps

| Cap | Limit | Constant |
| --- | --- | --- |
| Roles, everyone's and bots' included | 250 | `app::community::MAX_ROLES` |
| Channels and categories together, threads aside | 500 | `MAX_CHANNELS_AND_CATEGORIES` |
| Invites that are neither revoked nor expired | 1000 | `app::invite::MAX_ACTIVE_INVITES` |

Each addition counts inside its transaction with the community's row held (`app::community::hold_for_count`). Additions at once take turns. One past the cap is refused with a `validation` detail naming it.

A channel keeps at most 250 pinned messages (`app::message::MAX_PINS`), counted with the channel's row held.
