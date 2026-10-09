# Permissions

A permission is one bit of `community_role.permissions`, and each has exactly one meaning. Its wire name is the camelCase `Permission` enum. A role's record lists names, never bits.

## Community permissions

Community permissions hold across the community. Overrides never touch them and cannot name them.

| Permission | Allows |
| --- | --- |
| `manageCommunity` | Rename the community and change its icon. |
| `manageChannels` | Create, edit, arrange, and delete channels, and set their overrides. |
| `manageCategories` | The same for categories. |
| `createInvites` | Create invites. |
| `manageInvites` | Edit or revoke anyone's invites, and see them all. Everyone may manage their own. |
| `manageRoles` | Manage roles. |
| `assignRoles` | Give and take roles. |
| `removeMembers` | Remove members. |
| `manageMessages` | Delete anyone's messages and take down write-ins. |
| `pinMessages` | Pin messages. |
| `manageCalls` | Server mute, and remove from a call. |
| `addBots` | Add a bot through its link (see [Bots](../bots/index.md)). |
| `manageCustomEmoji` | Add, rename, and remove the community's emoji (see [Custom emoji](../custom-emoji.md)). |
| `banMembers` | Ban and unban (see [Bans](bans.md)). |
| `changeNickname` | Choose one's own nickname (see [Nicknames](nicknames.md)). |
| `manageNicknames` | Clear others' nicknames. |
| `managePlugins` | Turn the deployment's plugins on and off in the community, configure them, and read their settings there (see [Plugins](../plugins/index.md)). |

## Channel permissions

Channel permissions can also be allowed or denied per channel and per category.

| Permission | Allows |
| --- | --- |
| `viewChannel` | See the channel. |
| `sendMessages` | Post messages. |
| `attachFiles` | Attach files. |
| `addReactions` | React. |
| `startThreads` | Start threads. |
| `sendInThreads` | Post in threads. |
| `createPolls` | Create polls. |
| `joinVoice` | Join the call. |
| `speak` | Speak in the call. |
| `shareScreen` | Share a screen. |
| `useCamera` | Use a camera in calls. |
| `mentionMembers` | Tag members (see [Tagging](../tagging.md)). |
| `mentionRoles` | Tag roles. |
| `mentionEveryone` | Tag everyone. |
| `transferFiles` | Transfer files in calls (see [File transfers](../file-transfers.md)). |

## Bits

- Community permissions take bits 0 to 31 (`Permissions::COMMUNITY`).
- Channel permissions take bits 32 to 62 (`Permissions::CHANNEL`).
- The sign bit is never used.

### Adding a permission

1. Take the next free bit of its group.
2. Give it a name in `Permission`.
3. Add a localized `permission<Name>` string.
4. Give it a place in the templates (see [Roles and ranking](roles-and-ranking.md#templates)).
5. Change the client's resolver (`client/packages/protocol/src/permissions.ts`) and `spec/permission_vectors.json` with the server's.

## Editing a message

Editing a message is its author's alone (`app::message::update_message`). Adding to it is posting:

| Edit | Takes |
| --- | --- |
| New text (`content` given and not empty) | The channel's send permission: `sendMessages`, or `sendInThreads` in a thread (`ChannelAccess::send_permission`). |
| A new attachment (`attachments` naming one the message lacks) | The send permission, and `attachFiles` besides. |
| Only taking away (clearing the text, removing attachments, or both) | Nothing. |

So someone who may no longer post can still withdraw what they said.

## Threads and DMs

- A thread takes its parent channel's permissions.
- Posting in a thread takes `sendInThreads`. An echo also takes `sendMessages` in the parent.
- Every recipient of a DM holds every channel permission there and no community permission, except that any recipient may pin.
