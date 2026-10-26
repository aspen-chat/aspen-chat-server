# Roles and permissions

Who may do what in a community is decided by roles. A role grants permissions, members hold roles,
and channel and category overrides adjust channel permissions where they apply.

## Key files

| Part                  | Where                                                                  |
| --------------------- | ---------------------------------------------------------------------- |
| Resolving permissions | `app::permissions` (`channel_access`, `in_category`, `require_member`) |
| Managing roles        | `app::role`                                                            |
| HTTP                  | `api::role`, `api::ban`                                                |
| Bans                  | `app::ban`, table `community_ban`                                      |
| Visibility of lists   | `app::visibility::Visibility`                                          |
| Client resolver       | `client/packages/protocol/src/permissions.ts`                          |
| Shared test cases     | `spec/permission_vectors.json`                                         |

Both resolvers run the cases in `spec/permission_vectors.json`. Change the three together.

## Pages

- [Permissions](permissions.md): every permission, its bit, who holds what in threads and DMs, and
  editing messages.
- [Overrides and categories](overrides-and-categories.md): how overrides resolve, managing channels
  and categories, and hidden categories.
- [Roles and ranking](roles-and-ranking.md): the everyone role, templates, positions, ranking, and
  giving and taking roles.
- [Owners and limits](owners-and-limits.md): the owner, deleted communities, and the caps on roles,
  channels, and invites.
- [Role colours](role-colours.md): a role's hue and showing it apart.
- [Bans](bans.md): banning, lifting, and deleting recent messages.
- [Large communities](large-communities.md): what changes when a community grows past the
  deployment's threshold: Mention everyone, its channels' default notifications, and the message
  announcing both.
- [Members](members.md): the member sample and member search.
- [Access checks](access-checks.md): how reads, events, and calls apply permissions, and how the
  client follows changes.
- [Nicknames](nicknames.md): per-community nicknames.

The reasons behind these choices are in the [design notes](design-notes.md).
