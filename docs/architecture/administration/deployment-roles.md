# Deployment roles and permissions

What anyone may do across the deployment comes from deployment roles. They are ranked by position like a community's roles, and each holds deployment permissions.

## Where it lives

| Part | Code |
| --- | --- |
| Access computed for a user | `app::deployment` (`deployment_access`, `require_outranks`) |
| The roles themselves | `app::deployment_role`, tables `deployment_role` and `user_deployment_role` |
| Dashboard role endpoints | `api::deployment` |
| Other dashboard endpoints | `api::admin`, `app::admin` (`/admin/*`) |
| Terminal | `aspen-chat-server admin grant\|revoke\|list\|allow\|deny` |

## Permissions

| Permission | What it allows |
| --- | --- |
| `viewDashboard` | The totals, growth, fleet, directories, and moderation log |
| `manageRegistrationInvites` | Making and revoking [registration invites](registration-invites.md) |
| `manageVoiceServers` | The `/voice-servers` registry |
| `manageDeploymentRoles` | Creating, editing, ordering, giving, and taking deployment roles |
| `moderateCommunities` | Moderate any community; see [Moderation](moderation.md) |
| `manageFederation` | The gates and the directory of other deployments; see [Federation](../federation/index.md) |
| `reviewReports` | Reading report cases, dismissing them, and resolving them with a warning; see [Reports](../reports/index.md) |
| `removeContent` | Deleting a reported message, clearing a reported nickname, resetting a reported profile, and deleting a banned person's recent messages. It acts only on what a case or a ban names |
| `manageReportCategories` | See [Reports](../reports/index.md) |
| `banUsers` | [Bans from the deployment](deployment-bans.md) |
| `messageAnyUser` | A DM with anyone, past shared communities and blocks; see [Threads and DMs](../threads-and-dms/index.md) |
| `manageDeploymentSettings` | The deployment's profile and policies ([Deployment settings](deployment-settings.md)), and deleting bots whose owners are gone; see [Bots](../bots/index.md) |
| `managePlugins` | Installed plugins' settings, mode, order, and whether they run. Installing them is the terminal's; see [Plugins](../plugins/index.md) |
| `sendNewsletters` | See [Email](../email/index.md) |
| `viewJobs` | The preview of background jobs (`GET /admin/jobs`); see [Jobs](../jobs/index.md). The migration that made it gave it to every role holding Manage deployment settings |

Rules:

- Each permission does something whole on its own.
- Each step of a task takes the permission for what that step does, so no role holds a power it cannot use.
- Where one permission is part of another, holding the greater gives the lesser (`DeploymentPermission::includes`). Moderate any community includes Remove content.
- The access computed for a user, `GET /users/@me/admin`, and `deploymentAccessChanged` all carry what is included.
- `GET /users/@me/admin` lists the inclusions (`inclusions`), so the dashboard's role editor shows an included permission as held and fixed.

### Permission groups

| Constant | Members | Used for |
| --- | --- | --- |
| `DeploymentPermissions::DIRECTORIES` | View dashboard, and every moderation permission but Message any user | Opens the user and community directories |
| `DeploymentPermissions::MODERATION` | Moderate any community, Review reports, Remove content, Ban users, Message any user | Left out of `admin grant`'s Administrator role |

## The admin API

- Every `/admin/*` handler takes `AdminUser`.
- `AdminUser` refuses anyone without a deployment permission with `403` `adminRequired`, and hands the handler what they may do.
- Each handler requires its own permission, refusing with `forbidden` without it.
- `GET /users/@me/admin` tells a client what the caller may do and which roles give it.
- A change reaches them as the `deploymentAccessChanged` event.
- Community roles never reach any of this. **Why:** see [design notes](design-notes.md#community-roles-never-reach-the-dashboard).

## Managing roles from the dashboard

A holder of Manage deployment roles works at `/admin/roles`, `/admin/role-order`, and `/admin/users/{user}/roles/{role}`.

- They create, edit, reorder, delete, give, and take only roles below their own highest.
- They give only permissions they hold, counting what is included.
- Giving a role takes holding everything it allows. Taking it away takes rank alone.

## The top role and the terminal

Only the terminal reaches the top role.

| Command | What it does |
| --- | --- |
| `admin grant <username>` | Gives the top role. When there is none, it makes an Administrator role with every permission but `DeploymentPermissions::MODERATION` |
| `admin revoke` | Takes every role |
| `admin list` | Shows who holds what |
| `admin allow\|deny <permission>` | Changes what the top role allows. It refuses to deny a permission that one it allows includes. This is how moderation is first granted |

Each command announces the change to those it touches as the dashboard does (`deploymentAccessChanged`) and rechecks their calls. So the terminal needs NATS, as the servers do.

## Only this deployment's own people

Bots and users of other deployments never hold deployment roles.

- Giving one to either, from the dashboard or `admin grant`, is refused by `app::deployment_role::ensure_may_hold` with `deploymentRoleNotForBots` or `deploymentRoleNotForForeignUsers`.
- `app::deployment::deployment_access` reads no role of either, so no stray row could give them powers.
- The migration `deployment_roles_own_people_only` took away any they held.
- `AdminUser` refuses a bot's token outright with `adminRequired`.

**Why:** see [design notes](design-notes.md#only-own-people-hold-deployment-roles).

## Role hue and `nameHue`

A deployment role may have a hue: `hue`, 0 to 359 around HSV's wheel. It is set on `POST /admin/roles` and `PATCH /admin/roles/{role}`, where `null` takes it away. The names of its holders are drawn in it everywhere, over any community role's (see [Roles and permissions](../roles-and-permissions/index.md)).

- Each user's `nameHue`, on the user record everyone reads, is the hue of the highest deployment role they hold that has one (`user.name_hue`).
- Every change that can move it recomputes it for those it touches in the same transaction: a hue edited, roles reordered or deleted, a role given or taken from the dashboard or the terminal.
- Each change is announced as an update of the user to everyone who sees them (`app::deployment_role::refresh_name_hues`).
- Only the hue leaves the dashboard. Which roles someone holds stays the dashboard's.
- A user of another deployment carries this deployment's hue here, never their home's.
