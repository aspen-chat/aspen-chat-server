# Visibility

A shard decides whether each reader may see each of a community's events, from a model of the community's permissions attached to the event.

## Headers

| Header | Carried by | Decides |
| --- | --- | --- |
| `Aspen-Channel` | A channel's events, and a community channel's own events (its creation, moves, deletion, and overrides) | The channel whose View channel permission decides who gets them. For a thread, its parent. |
| `Aspen-Category` | A category's own events and its overrides' | The category whose overrides decide who gets them (below). |
| `Aspen-Requires` | Events that need a community permission besides membership | The permission. |
| `Aspen-Creator` | With `Aspen-Requires` | One member who receives the event anyway. |
| `Aspen-Event-Id` | Every copy of an event about a user | See [Publishing](publishing.md#events-about-a-user). |

## The community model

The dispatcher holds an `app::visibility::CommunityModel` of each community a local connection reads.

- It holds the owner, role permissions, each channel's category, and the overrides.
- It is loaded with the first connection that needs it.
- The dispatcher applies the events that change it, in stream order.
- It attaches the model as it then stands to each of the community's events.
- A shard decides each reader's view from that model and the roles the reader holds. It follows those roles from the reader's own `userCommunity` events.

## Events that change who may view a channel

An override set or cleared, a move, a creation, or a deletion also carries the model as it stood before (`FeedEvent::before`). It reaches whoever may view the channel by either model:

- those losing view learn they have;
- those gaining view learn they may look it up;
- nobody else learns the channel exists.

## Categories

A category's events reach those whose roles its own overrides leave View channel there (`CommunityModel::can_view_category`).

| Event | Also reaches |
| --- | --- |
| An override's change | Those who could view it before (`FeedEvent::before`). |
| Its deletion (`ModelChange::CategoryDeleted`, which drops its overrides from the model) | Only those who could view it before. |

A category the model holds no overrides of is decided by the community's permissions alone (see [Roles and permissions](../roles-and-permissions/index.md)).

## Edge cases

- A channel the model does not know (one not yet made, or deleted) nobody views. So a new channel's overrides, published ahead of it, reach only those they let view it.
- A model adopted for a community whose events were retained before the dispatcher held it attaches to each of those the model as it stood there. So a catch-up judges them by the permissions of their moment.
- A permission name the model does not know (from a newer server) is passed over, rather than costing the whole change.

## Required permissions

The shard checks `Aspen-Requires` against the same attached model.

| Event | Requires | `Aspen-Creator` |
| --- | --- | --- |
| Every invite event (it carries the invite's code) | Manage invites | Whoever made the invite |
| `communityPlugin` (a community's use of a plugin, whose settings may be its moderators' alone) | Manage plugins | |

A plugin's `pluginEvent` names a channel, a community, or neither. `expected_kind` routes it as an event of that scope (see [Plugins](../plugins/index.md)).

## Membership list position

- A membership event's copy for the community (`events::for_community`) leaves out the member's own list position, `sortIndex`. Only they receive it.
- A reorder reaches only them.
- Reads likewise give `sortIndex` for the caller's own membership alone.

## REST reads

REST list reads leave out hidden channels with `app::visibility::Visibility`, the same resolver.
