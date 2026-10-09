# Administration: design notes

Why administration works as it does. The how is in the [Administration pages](index.md).

## Deployment roles

### Community roles never reach the dashboard

- **Only deployment roles open `/admin/*`.** A dashboard anyone could open would let anyone mint the invites an invite-only deployment is closed by. See [Deployment roles](deployment-roles.md#the-admin-api).

### Only own people hold deployment roles

- **Bots and foreign users are refused deployment roles.** A bot acts for its owner, and a foreign user's account is their home deployment's to control. Either would carry the powers past the person this deployment answers for. See [Deployment roles](deployment-roles.md#only-this-deployments-own-people).
- **`deployment_access` reads no role of either.** So no stray row could give them powers, even if one were written.

### Permissions are whole and stepwise

- **Each step of a task takes the permission for what that step does.** So no role holds a power it cannot use.

## Moderation

- **A moderator posts nowhere and joins no DM.** Moderate any community gives oversight, not participation: adding someone to a group DM, or leaving one, takes being in it. See [Moderation](moderation.md).
- **Reading a DM one is not in goes through one function, and is logged.** `channel_access_reading` is the only way, so every such read lands in the moderation log.
- **The log resolves names as it is read.** So the dashboard names things rather than showing ids, at one query per kind of record per page.

## The dashboard

### Directories page by offset

- **The directories use `offset` and `limit`, not keyset parameters.** A page must follow whatever column sorts it, which the keyset parameters the rest of the API uses cannot. See [The dashboard](dashboard.md#directories).

### Fleet heartbeats

- **Servers report health through `aspen_fleet` rather than being scraped.** The heartbeat is how the dashboard sees every server without reaching their loopback metrics endpoints. Entries expire, so a server that stops drops out on its own.

### Powers kept at the terminal

- **Suspending rate limits and similar stay terminal commands.** They are too strong for even a privileged web API.

## Deployment settings

- **Policy lives in the database, not `aspen.toml`.** Every API server follows one answer and a change needs no restart. `aspen.toml` keeps only what a server needs to start and what is bound to its domain. See [Deployment settings](deployment-settings.md).
- **The revision goes to NATS before the row commits.** Each server reads the row until it holds that revision for up to ten seconds. If it never does, the change must have rolled back and the row stands.
- **No event carries the settings.** The signed-out screens and the dashboard read them when they open.
- **`federation_domain` never changes once pinned.** Other deployments pin this deployment's key at that name.
- **`pin_domain` runs from `settings set` too.** So gates can open before any server has run.
- **Lowered limits are not applied retroactively.** Bots owned, emoji held, and communities past the everyone mention limit stay as they are. Turning bots off leaves existing bots working.
- **A 4403 close makes the client ask for a second factor.** It then behaves as it would at sign-in.

## Registration invites

- **An invite is redeemed under a row lock in the account's transaction.** So concurrent registrations cannot overdraw it.
- **A dual invite's community invite is made by the person who makes it.** So it needs Create invites there like any invite.
- **Joining passes over a dead community invite rather than refusing the account.**

## Bans from the deployment

- **A ban rides through the owner's bots.** The bots they own act for them, so each is sent `accountBanned` and its tokens resolve to no one while the ban stands. The bot keeps its tokens for when the ban is lifted.
- **Banned people stay in their communities.** So their name and what they posted still show.
- **A ban of someone who outranks the caller is not theirs to lift.** Banning, replacing, and lifting all rank by deployment roles.
- **A password sign-in learns of a ban before any second factor is asked for.**
