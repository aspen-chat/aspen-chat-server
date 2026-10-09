# Bots

A bot is a user with `bot` set that signs in only with its token. Everything a person may do through the API a bot may do too, under the same permissions and limits.

## Pages

- [Bot accounts](bot-accounts.md): tokens, owners, the cap, bans, the bot endpoints, ownerless bots, and plugins' bots.
- [Handing a bot over](handing-over.md): offering a bot to another person, accepting, and the revocation checklist.
- [Adding a bot to a community](adding-to-communities.md): the bot's link and the role made for it.
- [Commands](commands.md): publishing a command list, its rules, and invoking a command.

Why things are the way they are: [design notes](design-notes.md).

## Key files

| Part | Where |
| --- | --- |
| Bots, tokens, transfers | `app::bot`, `api::bot`, tables `bot_token`, `bot_transfer` |
| Shutting out banned bots | `app::user_ban::shut_out` |
| A bot's role | `app::role::insert_role`, `app::role::delete_bot_role` |
| Commands | `app::bot_command`, `api::bot_command`, table `bot_command_list`, `spec/bot_commands.schema.json` |
