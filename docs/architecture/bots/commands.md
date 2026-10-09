# Commands

A bot advertises the commands it answers, which people invoke by typing `/name`.

| Part | Where |
| --- | --- |
| Lists and invocations | `app::bot_command`, `api::bot_command` |
| Stored lists | `bot_command_list` |
| Schema | `spec/bot_commands.schema.json` |
| Checking an invocation | `app::bot_command::check`, called from `app::message::create_message` |

## Publishing a list

A bot publishes one list with `PUT /bots/{bot}/commands`, by the bot or its owner. The list is the same everywhere. A bot of another deployment publishes its list on each deployment it uses.

The list follows `spec/bot_commands.schema.json`. A test holds the schema to the Rust types; `ASPEN_WRITE_SPEC=1 cargo test` writes it afresh.

A new list is announced as `botCommandsChanged` wherever the bot's profile would be.

## Rules for a list

The list is checked whole before it is kept in `bot_command_list`. A refusal names what is wrong and where.

| Rule | Limit |
| --- | --- |
| Commands | At most `MAX_COMMANDS` (100) |
| Parameters per command | At most `MAX_PARAMETERS` (25) |
| Names | 1 to 32 printable characters of any script, but no whitespace or emoji. Unique within the bot, ignoring case |
| Descriptions | 1 to 100 characters, with optional translations by language tag |
| Optional parameters | Last |
| `pattern` | On `regex` parameters alone |

A `pattern` is of the subset every client reads alike (`compile_pattern`):

- No group flags or named groups.
- No `\A`, `\z`, or POSIX classes.
- Small once compiled.

A value must match it whole.

### Parameter types

| Type | An argument must be |
| --- | --- |
| `userId` | A person who exists |
| `channelId` | A channel the caller may read |
| `messageId` | A message the caller may read |
| `communityId` | A community the caller belongs to |
| `roleId` | A role of this community |
| `attachmentId` | A file sent with the invocation and uploaded |
| `deploymentHost` | A deployment's domain |
| `react` | One emoji, which the bot receives fully qualified |
| `any` | Text of at most `MAX_ARGUMENT_CHARS` |
| `regex` | A match for the pattern |

## Reading lists

| Endpoint | Reads |
| --- | --- |
| `GET /bots/{bot}/commands` | A bot's list |
| `GET /channels/{channel}/commands` | What a channel offers: the lists of the bots that can view it. The community's bots in a community channel, the DM's in a DM. A thread counts as its parent |

## Invoking a command

`POST /channels/{channel}/commands` takes:

- `bot`.
- `name`.
- `arguments`, in order.
- `attachments`: the files its `attachmentId` arguments name, uploaded first.

It takes what posting a message there takes.

1. It goes through `app::message::create_message` with the invocation.
2. `app::bot_command::check` checks it in the same transaction:
   - The bot can see the channel and answers the command (names compared ignoring case).
   - The arguments are as many as it takes.
   - Each is what its parameter's type says, as the caller may reach it (see [Parameter types](#parameter-types)).
   - A refusal names the parameter and why.
3. The command becomes the caller's message of kind `command`:
   - Its content is the command as sent (`invocation_text`): people and roles as the tags a message names them by, anything else quoted where it holds whitespace.
   - `commandBot` names its bot.
   - It tags no one, fetches no link preview, wakes no phone, and cannot be edited.
4. The bot alone hears of it as `botCommandInvoked`, published to its user subject in the same transaction. It carries the message's id, the channel and its community, who invoked it, and the checked, typed arguments.

The bot answers as it answers anything, with messages.
