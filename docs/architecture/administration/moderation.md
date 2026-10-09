# Moderation and the moderation log

Moderate any community (`moderateCommunities`) is the deployment's power over what its users post.

## Where it lives

| Part | Code |
| --- | --- |
| The permission's effect in communities | `app::permissions::MODERATION` |
| Ranking against deployment roles | `app::deployment::require_outranks` |
| Listing someone's DMs | `app::dm::list_dms_moderating`, `GET /admin/users/{user}/dms` |
| Reading a DM one is not in | `app::permissions::channel_access_reading` |
| The log | `app::moderation_log`, table `moderation_log`, `GET /admin/moderation-log` |

## What a moderator may do

Its holder:

- reaches every community and DM without belonging;
- views every channel whatever the overrides;
- ranks above every role but the owner;
- may delete messages, attachments, reactions, and poll write-ins;
- may remove members, never the owner;
- may rename and delete channels and communities;
- may list anyone's DMs (`GET /admin/users/{user}/dms`).

It does nothing else. A moderator posts nowhere they could not already, and joins and reshapes no DM. Adding someone to a group DM, or leaving one, takes being in it.

### Outranking

What moderation takes away from a person never reaches anyone holding a deployment role at or above the moderator's own (`app::deployment::require_outranks`, ranked as [bans from the deployment](deployment-bans.md) are). That covers their messages, attachments, reactions, polls and write-ins, nickname, membership, or a community ban.

### Reading DMs

`app::permissions::channel_access_reading` is the only way to read a DM by someone not in it (see [Threads and DMs](../threads-and-dms/index.md)).

## What is logged

Each of these is written to `moderation_log` (`app::moderation_log`) and to the server's log:

- each use of the permission that the community's own permissions would not have allowed;
- every listing of someone's DMs (`listDms`, naming the person, `app::dm::list_dms_moderating`);
- every reading of a DM by someone not in it.

Bans and lifts are also logged (`banUser`, `liftUserBan`); see [Bans from the deployment](deployment-bans.md).

## Reading the log

The dashboard shows the log (`GET /admin/moderation-log`) to holders of View dashboard.

- Each entry carries what its ids name, read as the log is read (`details`), so the dashboard names things rather than showing ids.
- `details` takes one query per kind of record for a page: the community, the channel or DM and its people, the message and its author, the person acted on, and the emoji, file name, write-in, or new name.

## A moderator's client

- A moderator's event stream follows the communities they belong to, where it shows every channel.
- Elsewhere they read over REST, and the client applies their writes to its cache itself.

## Related dashboard reads

- The record of files offered in calls is `/admin/file-transfers`, which takes Moderate any community (see [File transfers](../file-transfers.md)).
- The directories, totals, and growth are on [The dashboard and fleet health](dashboard.md).
