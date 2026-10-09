# Deployment settings

The deployment's settings are the one row of `deployment_settings`. They hold how the deployment presents itself, the policies its administrators set, and the federation gates (see [Gates and lists](../federation/gates-and-lists.md)). Each has its default in the column.

`aspen.toml` keeps what a server needs to start and what is bound to its domain. What administrators decide is here, so every API server follows one answer and a change needs no restart.

## Where it lives

| Part | Code |
| --- | --- |
| Settings and changes | `app::deployment_settings` (`update_as`, `after_change`, `pin_domain`, `spawn_watcher`) |
| Endpoints | `api::deployment_settings` |
| Each server's copy | `SettingsCache`, `GlobalServerContext::settings` |
| Terminal | `aspen-chat-server settings show\|set` (`operator::settings`) |

The row is inserted by a migration. Its key, `singleton`, can hold nothing but true.

## The profile

The profile is a display name and an icon.

- The sign-in screens welcome people with them: "Welcome to {name}, an Aspen Chat instance.", or "Welcome to our Aspen Chat instance." without a name. It shows under the icon, or Aspen's without one.
- The name, or "Aspen" without one (`DeploymentSettings::name`), is also what the system account is called (see [Threads and DMs](../threads-and-dms/index.md)).
- It is what authenticator apps and passkey prompts call the deployment: `otpauth` URIs and the relying party's name, each built as it is used.
- The icon goes to `NULL` when its icon row is deleted.

## Endpoints

| Endpoint | What it does | Permission |
| --- | --- | --- |
| `GET /deployment` | The profile, to anyone, signed in or not. The icon is its record with `downloadUrl` included, since a signed-out client cannot read `/icons/{icon}` | None |
| `PATCH /deployment` | Changes the profile | Manage deployment settings |
| `GET`, `PATCH /admin/settings` | Reads and changes the policies | Manage deployment settings |
| `PATCH /admin/federation` | Changes the gates | Manage federation |

`update_as` checks each part of a change against its permission.

## Policies

| Setting | See |
| --- | --- |
| `registration_invite_required` | [Registration invites](registration-invites.md) |
| `require_two_factor` | [Sign-in and security](../sign-in-and-security/index.md) |
| `bots_enabled`, `bots_max_per_user` | [Bots](../bots/index.md) |
| `everyone_mention_limit` | [Roles and permissions](../roles-and-permissions/index.md) |
| `custom_emoji_limit` | [Custom emoji](../custom-emoji.md) |
| `upload_quota_gib` | [Attachment previews](../attachment-previews/index.md) |
| `evidence_retention_days` | [Reports](../reports/evidence.md#purging-evidence) |
| `file_transfers` | [File transfers](../file-transfers.md) |
| `email_required`, `email_verification_required`, `newsletter_enabled` | [Email](../email/index.md) |

The email settings turn on only where `[email]` is configured. `emailAvailable` in the policies and `email` in the profile say whether it is.

## Validation

- A display name is trimmed and from 1 to 64 characters, in any script.
- An icon must be one whose upload is confirmed, or the change is refused with `deploymentIconMissing`.
- A count is at most what `INTEGER` holds.

## The terminal

`aspen-chat-server settings show|set` changes any setting. It is how an invite-only deployment is made one before anyone has an account.

## `federation_domain`

`federation_domain` is the domain this deployment is known by.

- It is pinned the first time a server starts with an `https` `public_url`, whose host it is (`pin_domain`).
- The terminal's `settings set` runs `pin_domain` too, so gates can open before any server has run.
- **A server started with another domain, or with none once one is pinned, refuses to start.** Other deployments pin its key at that name.
- A gate opens only where the domain is set. A check on the row backs that.

## How a change reaches every server

Every API server keeps a copy of the settings (`SettingsCache`), read as it starts. Requests read the copy instead of the row.

A change:

1. locks the row;
2. checks the settings as it would leave them;
3. writes them with `revision` one higher;
4. renames the system account if the name changed;
5. before committing, writes the revision to the NATS key-value bucket `aspen_settings`.

Each server watches the bucket (`spawn_watcher`):

- On a new revision, it reads the row until it holds that revision, every twenty milliseconds for up to ten seconds.
- After ten seconds the change must have rolled back, and the row stands.
- A server that loses the watch reads the row afresh as it follows it again.
- The server that made a change takes its copy at once.

## Who can observe the settings

| What | Who | Route |
| --- | --- | --- |
| The profile | Anyone | `GET /deployment` |
| Whether registering takes an invite, and whether two factors are required | Anyone | `GET /auth/methods` |
| The policies and gates | Holders of their permissions | `/admin/settings`, `/admin/federation` |

No event carries them. The signed-out screens and the dashboard read them when they open.

## What a change does to what is already open

Whoever made the change brings what is open in line once it commits (`after_change`).

| Change | Effect |
| --- | --- |
| File transfers turned on or off | Every call is rechecked (`Recheck::Everyone`), and participants' grants follow |
| An immigration gate or its shared list | Users of other deployments the gates no longer admit are signed out and taken out of their calls (`standing::shut_out`), as the standing check would |
| `require_two_factor` turned on | Each server's open event streams follow its copy. A stream whose account lacked a second factor when it identified closes with 4403 (`api::event_stream`), and the client asks for one as at sign-in |
| Bots turned off | Bots already made keep working |
| A lower limit | Not applied to what already exceeds it: bots owned, emoji held, or communities already past the everyone mention limit |
| The gates, for CORS | CORS follows the gates as each request arrives |

## Checks

- `scripts/check_permissions.py` checks file transfers reaching a call, two factors closing a stream from the terminal, and the permission.
- `scripts/dev_federation_check.py` checks a gate closed from the dashboard ending a visitor's session.
