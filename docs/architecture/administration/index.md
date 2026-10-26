# Administration

The Administration Dashboard is the deployment's console. Deployment roles decide who may use it,
and the deployment settings hold what its administrators decide.

## Pages

- [Deployment roles and permissions](deployment-roles.md): the permissions, ranking, the admin API,
  the terminal's `admin` commands, and role hues.
- [Moderation and the moderation log](moderation.md): what Moderate any community allows and what is
  logged.
- [The dashboard and fleet health](dashboard.md): totals, growth, directories, and server
  heartbeats.
- [Registration invites and dual invites](registration-invites.md): invite-only registration, and
  invites that also join a community.
- [Deployment settings](deployment-settings.md): the profile, the policies, `federation_domain`, and
  how changes reach every server.
- [The Privacy Policy and Terms of Use](deployment-documents.md): what the deployment publishes for
  anyone to read, and who may change it.
- [Bans from the deployment](deployment-bans.md): banning anyone from the whole deployment.
- [Community takedowns](community-takedowns.md): deleting a community from the dashboard, hiding or
  erasing what it held, and banning its members.
- [Design notes](design-notes.md): why it works this way.

## Key files

| Part                 | Code                                                         |
| -------------------- | ------------------------------------------------------------ |
| Access and roles     | `app::deployment`, `app::deployment_role`, `api::deployment` |
| Dashboard API        | `api::admin`, `app::admin` (`/admin/*`)                      |
| Moderation log       | `app::moderation_log`                                        |
| Fleet health         | `app::fleet`                                                 |
| Registration invites | `app::registration_invite`                                   |
| Deployment settings  | `app::deployment_settings`, `api::deployment_settings`       |
| Documents            | `app::deployment_document`, `api::deployment_document`       |
| Bans                 | `app::user_ban`                                              |
| Community takedowns  | `app::community_takedown`                                    |
| Terminal             | `operator` (`admin`, `invites`, `settings`, `documents`)     |
