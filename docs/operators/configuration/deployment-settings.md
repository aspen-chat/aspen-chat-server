# Deployment settings

These are kept in the database, not in `aspen.toml`. Every server follows them, and a change
takes effect at once without a restart.

## Changing them

- In the Administration Dashboard: holders of Manage deployment settings change them under
  **Settings**, and holders of Manage federation change the gates under **Federation**.
- From the terminal:
  - `aspen-chat-server settings show` lists every one.
  - `aspen-chat-server settings set` changes those it is given, such as
    `settings set --registration-invite-required true --bots-max-per-user 5`.
  - The terminal needs NATS, which tells every server at once.
  - An invite-only deployment is made one this way before anyone has an account.

## Accounts and registration

| Setting | Default | |
| --- | --- | --- |
| `display-name` | none | What the deployment calls itself. See [Display name](#display-name). |
| `registration-invite-required` | `false` | Creating an account takes a registration invite, made in the dashboard or with `aspen-chat-server invites create`. See [Dual invites](#dual-invites). |
| `require-two-factor` | `false` | Every account must have an authenticator app or a passkey. An account without one can do nothing but add one or sign out; turned on, the apps of those without one are asked to at once. |

### Display name

- It is shown on the deployment's sign-in screens.
- It names the account the deployment's notices come from.
- It is what users' authenticator apps and passkey prompts call the deployment. Without one, they
  say "Aspen".
- The dashboard also sets an icon beside it.
- An authenticator keeps the name it was given when it was added.

### Dual invites

An invite made in the dashboard may also name a community.

- Each account it makes joins that community as it is made.
- Someone who already has an account just joins.
- Revoking a dual invite revokes its community invite too.
- Revoking that community invite from the community leaves a plain registration invite.

## Bots, communities, and uploads

| Setting | Default | |
| --- | --- | --- |
| `bots-enabled` | `true` | Whether people may make bots. Bots already made keep working either way. |
| `bots-max-per-user` | `25` | The most bots one person may own. |
| `everyone-mention-limit` | `200` | How many members a community gains before its everyone role loses Mention everyone. `0` never turns it off. See [below](#everyone-mention-limit). |
| `custom-emoji-limit` | `1000` | The most custom emoji one community may hold. Each is a small picture (PNG, JPEG, WebP, or GIF, at most 256 KiB) in the object storage. |
| `upload-quota-gib` | `25` | How many GiB one person may upload, attachments and pictures together, in any 24 hours. `0` sets no limit. See [below](#upload-quota). |
| `evidence-retention-days` | `365` | How many days files kept as evidence (those of deleted messages, and attachments taken off theirs) are kept for reviewing reports, unless a report about the message is open or was closed within as long. `0` keeps them for good. See [Backups](../backups.md#evidence). |
| `file-transfers` | `true` | Whether people may offer files to one another in calls. See [below](#file-transfers). |

### Everyone mention limit

- It happens once per community.
- The owner is told why by the deployment's own account, which cannot be signed in to, messaged,
  or blocked.
- The owner may turn Mention everyone back on.

### Upload quota

- Each upload counts with the size it was started for, whether or not it is then sent or
  deleted.
- Past the limit, an upload is refused, with when there will be room.

### File transfers

- Transfers go between people's devices, or through a voice server's relay (`[transfer]` in
  `voice_server.toml`; see [Voice server](voice-server.md#transfer)).
- Either way, this deployment keeps a record of each offer and transfer for its moderators, never
  the file.
- `false` turns the whole feature off: no one can offer a file, whatever the channels'
  permissions, and calls in progress follow at once.

## Email

All of these need [`[email]`](email.md).

| Setting | Default | |
| --- | --- | --- |
| `email-required` | `false` | Creating an account takes an email address. It binds registration only: accounts made before it was turned on are not asked for one. |
| `email-verification-required` | `false` | An account that has an email address must verify it, by the code mailed to it, before it can use the deployment. See [below](#email-verification). |
| `newsletter-enabled` | `false` | The deployment has a newsletter. See [below](#newsletter). |

### Email verification

- Until verified, an account can only verify, change, or resend its address, or sign out.
- Turned on, the apps of those with an unverified address are asked at once.
- Accounts without an address are not affected, unless `email-required` is on too, and then only
  once they give one.

### Newsletter

- People may subscribe at registration (unticked by default) and in their account settings.
- Holders of Send newsletters write and send posts under **Newsletter** in the dashboard.
- Only verified addresses receive it.
- Each piece carries an unsubscribe link.

## Federation gates

See [Federation](../federation/index.md).

| Setting | Default | |
| --- | --- | --- |
| `users-emigration`, `bots-emigration` | `closed` | Whether this deployment's accounts may use other deployments: `closed`, `open`, `allowList`, or `blockList`. |
| `users-immigration`, `bots-immigration` | `closed` | Whether other deployments' accounts may use this one, the same way. Closing or narrowing it signs out at once those it no longer admits. |
| `users-shared-list`, `bots-shared-list` | `false` | Both directions read one list; both gates must then be the same kind of list. |
| `users-immigration-invite-required`, `bots-immigration-invite-required` | `false` | An account of another deployment arriving for the first time needs a registration invite. |
