# Bots and their commands

Bots are users with `bot` set. Nothing in the app signs in as a bot; bots use the API with their tokens. The app lets people make and manage their bots, add bots to communities, and invoke bots' commands from the message box.

## Where it lives

| Part | Code |
| --- | --- |
| Bot badge | `BotBadge` |
| ID wizard | `CopyIdButton`, `CopyIdMenuItem` (`src/features/layout/CopyId.tsx`), `useIdWizard` |
| Managing bots | `BotsDialog`, `src/features/bots` |
| Adding a bot | `BotAddScreen` |
| Commands | `src/features/commands`, `useCommandLine`, `commandLine.ts` |

## How bots are shown

`BotBadge` marks a bot wherever it is named:

- messages
- the member list
- the admin users directory
- their card, which also says who made them. For a plugin's own account, it says which plugin it belongs to (see [Plugins](plugins/index.md)).

## Developer mode and the ID wizard

Developer mode is the account preference `DEVELOPER_MODE`, turned on in Settings. It brings the ID wizard and `BotsDialog`.

The ID wizard (`ID_WIZARD`, on unless turned off; `useIdWizard`):

- Everything with an id offers to copy it, through `CopyIdButton`, or `CopyIdMenuItem` in a menu.
- The copy control is always last in whatever shows it, so the everyday controls keep their places whether it is on or off.
- Each dashboard table gains a last ID column (`Directory`'s `idThing`).

A new place that shows something with an id gives it a copy control.

## Managing bots: `BotsDialog`

The dialog lists the caller's bots: `RecordStore.ownedBots` (topic `bots`). It is derived from the cached users whose `botOwner` is the caller, and filled by `AspenSync.loadBots`.

| Action | How |
| --- | --- |
| Make a bot | `AspenSync.createBot`. The token is shown once, as it arrives. |
| Set its picture | `IconPicker`, written with `AspenSync.updateBotProfile` |
| Make it public or private | `AspenSync.setBotPublic` |
| Copy the link that adds it with the permissions it suggests | `botAddLink`: the page's address on the web, the path in the shells |
| Make a new token | `AspenSync.rotateBotToken` |
| Hand it to someone | `PeoplePicker`, `AspenSync.transferBot` |
| Delete it | `AspenSync.deleteBot` |

These writes are applied from their answers. **Why:** a bot's own events reach only those who share a community with it.

## Adding a bot to a community

The link opens `BotAddScreen` (`/bots/$botId/add?permissions=…`). It offers:

- the communities where the caller holds `addBots`
- the suggested permissions the caller may give: Manage roles and Assign roles, and each permission held

It calls `AspenSync.addBot`.

A role made for a bot says whose it is in the role editor. It is offered for neither deletion nor assigning.

## Ownerless bots

With `manageDeploymentSettings`, the admin users directory deletes ownerless bots.

## Commands

### Offering commands

`useCommandLine` offers bots' commands in the message box. The Composer runs it beside `useTagging`, and it turns tagging off while the draft begins with `/`.

The channel's commands are store state:

- `RecordStore.commands`, topic `commands:<channelId>`
- read by `AspenSync.loadCommands` through `useCommands`
- dropped whenever something may change which bots see the channel or what they answer: `botCommandsChanged`, roles, overrides, a bot's membership, a DM's recipients
- read again where shown

### Typing a command

- Typing the name offers every bot's commands, with the bot's picture and name and the command's description in the reader's language (`describe`).
- Picking a command that another bot answers too writes its bot's tag after the name (`/kick @modbot `). That is how a line says which bot it means.
- While an argument is typed, the command's form shows above the box:
  - the parameter at the caret is marked, with what it takes
  - a warning shows when a `regex` value does not match
- People, roles, channels, communities, and deployments are suggested by name. A pick is remembered, so sending turns its text into the id.

### Reading the line

`commandLine.ts` holds the reading:

- the server's quoting (`tokenize`, `quoteArgument`)
- the last `any` parameter takes the rest of the line
- files fill `attachmentId` parameters in order

### Sending

| The line | What happens |
| --- | --- |
| Names a command here | Invoked (`AspenSync.invokeCommand`) |
| Names a command two bots answer, untagged | Refused in the box, naming both |
| Any other line beginning with `/` | Sent as a message |

### A command's message

- It says who it was sent to, beside its author (`sentTo`).
- Its text names people and roles by the tags that tag no one, so they read by name.
