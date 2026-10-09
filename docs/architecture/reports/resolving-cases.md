# Resolving cases

`POST /admin/reports/{case}/resolution` resolves an open case with one or more actions. Each action takes its own permission.

## Actions

| Action | Applies to | Permission | Moderation log |
| --- | --- | --- | --- |
| `warn` | Any case | Review reports | `warnUser`, naming the reviewer |
| `ban` | Any case | Ban users | (see [Administration](../administration/index.md)) |
| `deleteMessage` | Message cases | Remove content | `deleteMessage` |
| `clearNickname` | Nickname cases | Remove content | `clearNickname` |
| `reset` | Profile cases of this deployment's users | Remove content | `resetProfile` |

### `warn`

The reviewer's words, sent by the system account (`app::system_account::post`) in its DM with the subject:

- Sent for the deployment's moderators, without the reviewer's name.
- A message of kind `warning`. Its `warning` names the subject and what the warning is about:

  | Case | The warning keeps |
  | --- | --- |
  | Message | The reported message |
  | Profile | The profile as the latest report found it, with every aspect the reports named |
  | Nickname | The nickname as the latest report found it, with its community and that community's name then (`NicknameSnapshot`) |

  So it reads the same after they leave or it is deleted.
- Review reports allows it, so every reviewer can act on a case.
- The system account reaches anyone, and nobody answers or blocks it (see [The system account](../threads-and-dms/system-account.md)).

Clients label it as a moderator's warning and show what it is about, the reported message even once deleted. Message reads sideload it with `include=warnings` as `included.warnedMessages`. Only the people of the warning's DM read the warning.

### `ban`

A ban from the deployment, as `PUT /admin/users/{user}/ban` takes it (see [Administration](../administration/index.md)).

### `deleteMessage`

Deletes the reported message wherever it is (`app::message::soft_delete`), without the reviewer reaching its channel. See [Evidence](evidence.md) for what deletion keeps.

### `clearNickname`

Clears the member's nickname in its community, whatever it is now (`app::community::erase_nickname`).

- Announced as their own clearing is.
- Nothing happens if they have left or cleared it.
- The community's ranks do not apply, since the reviewer's deployment rank was checked. An owner's nickname can be cleared this way.
- It reaches foreign users too, since the nickname is this deployment's.

### `reset`

Clears the named aspects of a profile of this deployment's user (`app::user::apply_profile_update`), announced like the owner's own change.

- A username becomes `user-` and eight random letters and digits.
- The system account tells them to choose another.

## Order of a resolution

1. The case is marked resolved first, under its row lock, so two reviewers cannot both act.
2. The actions follow, each recorded in the case's `resolution`.
3. If one fails, the case is opened again and the error returned. The actions before it stand.

## Dismissing and restoring

- `PUT /admin/reports/{case}/dismissal` dismisses an open case.
- `DELETE /admin/reports/{case}/dismissal` restores a dismissed one.

Nothing deletes a report or a case through the API.
