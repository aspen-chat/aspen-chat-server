# Community takedowns

A takedown is how a moderator of the deployment removes a whole community from the dashboard's
community directory (its **Ban** button). The community is deleted at once, and the moderator
chooses what becomes of everything it held, and whether its members are banned from the deployment.

## Where it lives

| Part                  | Code                                                                                 |
| --------------------- | ------------------------------------------------------------------------------------ |
| Counting whom it hits | `app::community_takedown::reach`, `GET /admin/communities/{community}/reach`         |
| Asking for it         | `app::community_takedown::take_down`, `POST /admin/communities/{community}/takedown` |
| Banning and erasing   | the job `takeDownCommunity` (`app::community_takedown::step`)                        |
| Each ban              | `app::user_ban::ban_one`, as a ban of one person                                     |
| The dialog            | `CommunityBanDialog` in the client's `features/admin`                                |

## Who may

| Part                            | Permission                             |
| ------------------------------- | -------------------------------------- |
| Counting, deleting, and erasing | Moderate any community                 |
| Banning the members             | Ban users besides                      |
| The bans' deletion window       | Remove content besides, as for one ban |

## The request

`POST /admin/communities/{community}/takedown` takes:

| Field        | Meaning                                                                                                                                       |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `erase`      | Required. `true` erases what the community held; `false` only hides it, as its owner's deletion does                                          |
| `banMembers` | Optional: a ban's `reason`, `durationSeconds`, and `deleteMessagesSeconds`, as `PUT /admin/users/{user}/ban` takes them, given to each member |

In one transaction, with the community's row held as every way in holds it:

1. it counts whom it reaches, which the answer gives (`members`, `bannedMembers`);
2. writes `deleteCommunity`, and `eraseCommunity` when erasing, to the moderation log;
3. deletes the community as its owner's deletion does (`community::mark_deleted`): marked deleted,
   plugins' values forgotten, and `community` `delete` announced, which takes everyone out of its
   calls once it commits;
4. saves the job `takeDownCommunity`, keyed by the community, when there are bans or an erasure to
   do.

A community already deleted is not found. The dialog makes the moderator choose between hiding and
erasing, with neither chosen for them, and confirms with the count from `GET .../reach`, saying it
cannot be undone.

## Whom the bans reach

Every member whose account stands, except:

- the moderator;
- the system account;
- anyone banned already, whose ban stands as it was;
- anyone of the deployment's own people (not a bot, not from elsewhere) holding a deployment role at
  or above the moderator's highest, as a ban of one person ranks.

`GET .../reach` counts the members (`members`) and those (`bannable`) in one statement over the
community's memberships.

## The job

`takeDownCommunity` (class normal) does, in order:

1. **Bans**, fifty members a step, in order of id (`community_user_by_community_user`). Each is
   banned as a ban of one person bans them (`user_ban::ban_one`): signed out, streams closed, calls
   left, bots shut out, logged as `banUser`, and their deletion window queued. Each is in a
   savepoint of its own, so someone whose rank or account changed since the step read them is passed
   over. The moderator's access is read again each step; once they can no longer ban, nobody more is
   banned, and the job goes on to erase if it was asked to.
2. **Erasure**, when asked for, described below.

The bans go first because they read the memberships the erasure deletes.

## What erasing deletes

Erasing deletes rows outright, where every other deletion in Aspen hides them. Its stages, each a
step at a time:

| Stage      | What goes                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Messages   | Each channel's messages, threads' included, fifty a step in order of id: their reactions, saves, and tags (a thousand rows a step), every reply, echo, and thread start naming them, pins, attachments no other message holds, link previews, and the messages. The files they let go of (attachments, their previews, link previews' pictures, at every key they may be at) are deleted from storage in the next step |
| Channels   | Threads first, then the rest, one a step: read positions, tags, follows, mutes, notification levels, held messages, plugins' notices, polls and their votes (a thousand rows a step), the file transfer record's and the moderation log's references cleared, pins, its call, and the channel                                                                                                                          |
| Categories | One a step, with what each kept per person                                                                                                                                                                                                                                                                                                                                                                             |
| Members    | Memberships with their roles, what members set for it, its bans, and its call mutes, a thousand a step                                                                                                                                                                                                                                                                                                                 |
| Roles      | Tags of its roles, then the roles                                                                                                                                                                                                                                                                                                                                                                                      |
| Invites    | Its invites, revoked and expired ones included; registration invites naming one stay, naming none                                                                                                                                                                                                                                                                                                                      |
| Emoji      | Marked deleted, each with its `purgeCustomEmoji` job, which takes its reactions off everywhere and deletes its picture                                                                                                                                                                                                                                                                                                 |
| Finish     | Plugins' settings for it, and its icon, which `forgetIcon` then deletes                                                                                                                                                                                                                                                                                                                                                |

Kept:

- the community's own row, so the moderation log and report cases still name it;
- every message a report case is about, deleted as any message is (`message::soft_delete_many`), so
  its files are kept as [evidence](../reports/evidence.md) and purged after the retention period;
- the channels holding those messages, deleted, out of their category.

The migration `community_takedown_keys` gave each table the erasure deletes from, or whose foreign
key reaches what it deletes, an index leading with that column, so no statement reads a whole table.

## When access is given or taken away

1. **Who can observe it, and by which routes?** The count and the request: holders of Moderate any
   community, over REST. The community itself: nobody once it is deleted (see
   [Deleted communities](../roles-and-permissions/owners-and-limits.md#deleted-communities)); what
   erasing keeps, reviewers of report cases alone.
2. **What decides it, and where is that checked?** `DeploymentAccess` in `take_down` and `reach`,
   and again in each step of the bans. Each ban ranks as a ban of one person does (`require_above`),
   and the statement choosing whom to ban applies the same rule.
3. **When the deciding permission is lost, what happens to what is already open?** The deletion is
   done when the request answers. A moderator who loses Ban users (or Remove content, for a deletion
   window) while the job runs bans nobody more; the erasure, asked for under Moderate any community,
   goes on. Each ban closes the banned person's streams and takes them out of calls, as a ban of one
   person does. Members' clients prune the community on its `delete` event.
4. **When it is gained, how does a client already open find out without a reload?** Nothing is
   granted.
5. **Does every path that changes it announce it?** The deletion is announced to the community; each
   ban to its person. Erasing announces nothing, since nobody reads a deleted community; the
   messages kept for report cases are announced deleted as any deletion is.
6. **Is it published inside the transaction that makes the change?** Yes: the deletion inside the
   request's, each ban inside its step's.
