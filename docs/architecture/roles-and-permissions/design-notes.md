# Roles and permissions: design notes

The reasons behind the choices described in [Roles and permissions](index.md).

## Permissions

- **Community and channel permissions take separate bit ranges (0 to 31, 32 to 62).** Each group has room to grow without moving the other. See [Bits](permissions.md#bits).
- **An edit that only takes away needs no send permission.** Someone who may no longer post can still withdraw what they said. See [Editing a message](permissions.md#editing-a-message).

## Overrides and categories

### Managing takes viewing

- **Managing a channel takes viewing it.** A channel hidden from someone is not found to them, so they cannot undo what hides it. See [Managing channels and categories](overrides-and-categories.md#managing-channels-and-categories).
- **Filing a channel in a category takes viewing that category.** The channel takes the category's overrides.
- **Managing a category takes viewing it.** Changing its overrides or deleting it would uncover its channels.

### Category deletion is one event

- **A deleted category's overrides are inferred from the deletion, not announced one by one.** The last override's going would otherwise show the category to everyone. See [Deleting a category](overrides-and-categories.md#deleting-a-category).
- **Deleting a category moves its channels and clears its overrides in one transaction.** Nothing a deleted category decided outlives it.

### Other choices

- **Deleting a role announces only the role's deletion.** The request deletes its overrides and a job takes it from its holders; the event feed and clients take it from both themselves, however many there are.
- **A channel created with overrides writes them in its transaction and announces them before the channel.** A channel made private is never open to more people than they allow, and its creation reaches only those who may view it.

## Roles and ranking

- **Giving a role takes holding every permission it allows.** Assign roles never hands on more than its holder has.
- **Handing on is judged by community roles alone, even for deployment moderators.** A deployment moderator who also holds Manage or Assign roles reaches no further with them than their roles do.
- **A deleted role is marked and stripped in the request, and a job does the rest.** It grants nothing and ranks nobody at once, while taking it off its holders a thousand at a time means a role held by a million members is deleted as quickly as one held by none. See [Deleting a role](roles-and-ranking.md#deleting-a-role).
- **Renumbering is announced with `publish_events`.** Every copy goes to NATS in order and is acknowledged together, so a renumbering costs one round trip and the stream still holds the updates in the order the database made them.
- **Each change to a member's roles locks their `community_user` row and announces their whole list.** Concurrent changes are announced in commit order, each with every earlier one in it, so the event feed's view of the roles they hold is the database's.

## Owners and limits

- **Caps count inside the transaction with the community's row held.** Additions at once take turns.
- **`add_member` holds the community's row until the membership commits.** A deletion beside it waits.

## Role colours

- **The server stores and sends only the hue.** Clients choose how light and how strong to draw it, so that every hue reads on every ground.
- **Message reads sideload `include=memberships`.** A client can colour an author outside the member sample without reading them one by one. Whoever may read a channel's messages may see their authors' roles, as `read_community_member` already allows.

## Members

- **Any member may read one member's roles.** Whoever reads someone's messages may see what roles they hold.
- **Searching a community larger than the sample is limited to its owner, deployment moderators, and holders of a few managing permissions.** No ordinary member can list a large community whole.

## Everyone mention limit

- **A large community loses Mention everyone from its everyone role.** A tag of everyone would then reach more people than it likely means to.
- **Channel and category overrides are left as they are.** They are deliberate choices for their channels.
- **The community's row is locked and its members counted after a join commits, acting only if no one has.** Concurrent joins that cross the limit act once, and a join that finds it crossed acts even when the one that crossed it could not.
