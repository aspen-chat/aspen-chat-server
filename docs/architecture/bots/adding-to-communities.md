# Adding a bot to a community

## The bot's link

A bot's link is the client page `/bots/{bot}/add?permissions=…`, which suggests permissions by name.

It calls `PUT /communities/{community}/members/{bot}` with the permissions chosen. This takes:

- The community permission Add bots (in the Moderator and Admin templates).
- A bot that is public, or the caller's own.

## The bot's role

Permissions given go on a role made for the bot (`community_role.bot`, `app::role::insert_role`).

- Making it takes Manage roles and Assign roles.
- It holds only permissions the caller holds.
- The role is the bot's alone. It cannot be given to anyone else or deleted.
- It goes when the bot leaves (`app::role::delete_bot_role`, from `end_membership`).

See [Roles and permissions](../roles-and-permissions/index.md).
