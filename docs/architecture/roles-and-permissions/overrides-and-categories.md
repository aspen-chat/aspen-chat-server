# Overrides and categories

## Resolving a member's permissions

Tables: `community_member_role`, `channel_override`, `category_override`.

1. A member's community permissions are the union of their roles' permissions. Every member holds the everyone role and whichever others they are given.
2. In a channel, its category's overrides apply first, then the channel's own.
3. Within each layer, the everyone role's override applies first. Then the member's other roles' overrides apply together, denials before allowances.

So an allowance for any of their roles beats a denial for everyone. Overrides never touch community permissions and cannot name them.

## Managing channels and categories

| Action | Takes |
| --- | --- |
| Renaming, moving, or deleting a channel, or setting or clearing its overrides | Manage channels, and viewing the channel (`app::channel::managed_channel`, through `channel_access`). |
| Filing a channel in a category, as it is made or moved | Viewing that category (`app::channel::ensure_category_of`, through `in_category`). |
| Renaming, moving, or deleting a category, or its overrides | Manage categories, and viewing the category (`app::category::managed_category`). |

A channel hidden from someone is not found to them, so they cannot undo what hides it. See the [design notes](design-notes.md#managing-takes-viewing).

## Creating a channel with overrides

A channel may be created with its overrides (`overrides` on `POST /channels`, `app::role::check_initial_overrides`).

- Each override is checked as setting it afterwards would be.
- They are written in the same transaction as the channel.
- They are announced before the channel itself.

A channel made private is never open to more people than its overrides allow, and its creation reaches only those who may view it.

## Who learns of a category

A member learns of a category only while its own overrides, applied to what they may do across the community, leave them View channel there. The overrides apply as above: everyone's override, then their roles' together.

| Use | Check |
| --- | --- |
| One category | `app::category::viewed_category` with `app::permissions::in_category` |
| Lists | `Visibility::can_view_category` |
| The event stream | `CommunityModel::can_view_category` |

- The owner and deployment moderators learn of every category.
- A deployment moderator views every channel, so this takes none of their powers.
- A category is made with no overrides, so it is shown to every member until its overrides are set.
- What its channels' own overrides allow does not reveal it. A channel someone may view in a category they may not is listed with its `parentCategory`, a category they do not know. The client files it at the top level and counts the unknown category as denying View channel.

### What a hidden category answers

- `GET /categories/{category}` and its channels answer `404`.
- Its collapse (`PUT`/`DELETE /categories/{category}/collapses/@me`) answers `404`.
- Managing it answers `404`.
- Community reads leave it out of `include=categories`, its overrides out of `categoryOverrides`, and it out of `collapses`.

### Category events

Its events (`category`, and its overrides' `categoryOverride`) carry the `Aspen-Category` header. They reach those it lets view it (see [Event routing](../event-routing/index.md)).

| Change | Who receives it |
| --- | --- |
| An override's change | Those who could view the category before or after. |
| The category's deletion | Those who could view it before. |

- Someone losing the category hears of the change and lets it go. The client's store drops it and its overrides once they no longer give View channel.
- Someone gaining it hears of a category they do not have and reads the community again.

### What decides it

Every change that decides who views a category is already an event the client widens or narrows on as for channels:

- a role's permissions
- a role given, taken, or deleted
- the owner changed
- a deployment role
- a category override

Removal, a ban, leaving, and sign-outs end what the member reads, as for channels.

## Deleting a role

Deleting a role deletes its overrides at once, and a job (`purgeRole`) takes it from its holders. Only the role's deletion is announced. The event feed and clients take it from holders and overrides themselves. See [Deleting a role](roles-and-ranking.md#deleting-a-role) for what the request and its job do.

## Deleting a category

In one transaction:

1. Its channels move out of it, each move announced.
2. Its overrides are cleared.

Nothing a deleted category decided outlives it. A deleted category's overrides apply to nothing either way.

The cleared overrides are not announced one by one. They are inferred from the deletion (`ModelChange::CategoryDeleted`, and the client's store dropping a deleted category's overrides). **Why:** see the [design notes](design-notes.md#category-deletion-is-one-event).
