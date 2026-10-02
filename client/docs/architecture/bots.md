# Bots and their commands

- Bots are users with `bot` set, marked by `BotBadge` wherever they are named (messages, the
  member list, their card, which also says who made them, the admin users directory). Nothing
  in the app signs in as a bot; bots use the API with their tokens. Developer mode is the
  account preference `DEVELOPER_MODE`, turned on in Settings. With it comes the ID wizard
  (`ID_WIZARD`, on unless turned off; `useIdWizard`): everything with an id offers to copy it
  through `CopyIdButton`, or `CopyIdMenuItem` in a menu (`src/features/layout/CopyId.tsx`),
  always last in whatever shows it, so the everyday controls keep their places whether it is
  on or off, and each dashboard table gains a last ID column (`Directory`'s `idThing`). A new
  place that shows something with an id gives it one. Developer mode also offers `BotsDialog`
  (`src/features/bots`): the caller's bots (`RecordStore.ownedBots`, topic `bots`, derived from
  the cached users whose `botOwner` is the caller and filled by `AspenSync.loadBots`), making
  one, whose token is shown once as it arrives, its picture (`IconPicker`, written with
  `AspenSync.updateBotProfile`), public or private, the link that adds it with
  the permissions it suggests (`botAddLink`: the page's address on the web, the path in the
  shells), a new token, handing it to someone (`PeoplePicker`), and deleting it. The writes are
  `AspenSync.createBot`, `rotateBotToken`, `setBotPublic`, `transferBot`, and `deleteBot`,
  applied from their answers, since a bot's own events reach only those who share a community
  with it. The link opens `BotAddScreen` (`/bots/$botId/add?permissions=…`), which offers the
  communities where the caller holds `addBots` and the suggested permissions the caller may
  give (Manage roles and Assign roles, and each permission held), and calls `AspenSync.addBot`.
  A role made for a bot says whose it is in the role editor and is offered for neither
  deletion nor assigning. With `manageBots`, the admin users directory deletes ownerless bots.
- Bots' commands (`src/features/commands`) are offered in the message box by `useCommandLine`,
  which the Composer runs beside `useTagging` and which turns tagging off while the draft
  begins with `/`. The channel's commands are store state (`RecordStore.commands`, topic
  `commands:<channelId>`, read by `AspenSync.loadCommands` through `useCommands`), dropped
  whenever something may change which bots see the channel or what they answer
  (`botCommandsChanged`, roles, overrides, a bot's membership, a DM's recipients), and read
  again where shown. Typing the name offers every bot's commands with the bot's picture and
  name and the command's description in the reader's language (`describe`); picking one that
  another bot answers too writes its bot's tag after the name (`/kick @modbot `), which is how
  a line says which it means. While an argument is typed, the command's form shows above the
  box with the parameter at the caret marked, what it takes, and a warning when a `regex`
  value does not match; people, roles, channels, communities, and deployments are suggested by
  name, and a pick is remembered so that sending turns its text into the id. `commandLine.ts`
  holds the reading, the server's quoting included (`tokenize`, `quoteArgument`), the last
  `any` parameter taking the rest of the line, files filling `attachmentId` parameters in
  order. Sending a line that names a command here invokes it (`AspenSync.invokeCommand`), one
  two bots answer untagged is refused in the box naming both, and any other line beginning
  with `/` goes as a message. A command's message says who it was sent to beside its author
  (`sentTo`), and its text names people and roles by the tags that tag no one, so they read by
  name.
