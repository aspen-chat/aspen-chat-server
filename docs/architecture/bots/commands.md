# Commands

A bot advertises the commands it answers, which people invoke by typing `/name`.

| Part                   | Where                                                                 |
| ---------------------- | --------------------------------------------------------------------- |
| Lists and invocations  | `app::bot_command`, `api::bot_command`                                |
| Stored lists           | `bot_command_list`                                                    |
| Schema                 | `spec/bot_commands.schema.json`                                       |
| Checking an invocation | `app::bot_command::check`, called from `app::message::create_message` |

## Publishing a list

A bot publishes one list with `PUT /bots/{bot}/commands`, by the bot or its owner. The list is the
same everywhere. A bot of another deployment publishes its list on each deployment it uses.

The list follows `spec/bot_commands.schema.json`. A test holds the schema to the Rust types;
`ASPEN_WRITE_SPEC=1 cargo test` writes it afresh.

A new list is announced as `botCommandsChanged` wherever the bot's profile would be.

## Rules for a list

The list is checked whole before it is kept in `bot_command_list`. A refusal names what is wrong and
where.

| Rule                   | Limit                                                                                                        |
| ---------------------- | ------------------------------------------------------------------------------------------------------------ |
| Commands               | At most `MAX_COMMANDS` (100)                                                                                 |
| Parameters per command | At most `MAX_PARAMETERS` (25)                                                                                |
| Names                  | 1 to 32 printable characters of any script, but no whitespace or emoji. Unique within the bot, ignoring case |
| Descriptions           | 1 to 100 characters, with optional translations by language tag                                              |
| Optional parameters    | Last                                                                                                         |
| `pattern`              | On `regex` parameters alone                                                                                  |

### Patterns

A `pattern` is a regular expression a value must match whole. The server matches with Rust's `regex`
crate; Aspen's clients check a typed value with a `u`-mode `RegExp` before sending it. A pattern is
of the subset the two can be made to read alike, and is at most `MAX_PATTERN_CHARS` (256) characters
and small once compiled (`PATTERN_SIZE_LIMIT`, 2 MiB, of which each `\w` or `\p{L}` written, or
repeated by a count, takes about 50 KiB).

| Part                  | Where                                                                      |
| --------------------- | -------------------------------------------------------------------------- |
| The server's check    | `app::bot_command::pattern::check`, when a list is published               |
| The client's `RegExp` | `compilePattern` in `client/packages/app/src/features/commands/pattern.ts` |
| Cases both pass       | `spec/bot_command_pattern_vectors.json`                                    |
| The subset in words   | The `pattern` property of `spec/bot_commands.schema.json`                  |

The server parses the pattern (`regex-syntax`) and walks what it parsed, so what a construct is
never depends on the text around it: `\(?` is an optional parenthesis and `\\A` a backslash and an
`A`. Each node is one of three kinds.

**Read alike by both.** Characters and their escapes (`\n`, `\x41`, `\u00e9`, `\u{1F600}`), classes
and ranges, `\p{…}` with the listed names, groups without names or flags, `|`, `^`, `$`, and
repetition of a character, a class, or a group.

**Unicode's on the server, rewritten by the client.** JavaScript's `\d`, `\w`, and `\b` know only
ASCII, its `\s` takes U+FEFF and not U+0085, and its `.` stops at U+000D, U+2028, and U+2029 as well
as U+000A. The server's meaning stands, and the client writes each as the class that says the same:

| Written    | Means on the server                                         | The client compiles                                 |
| ---------- | ----------------------------------------------------------- | --------------------------------------------------- |
| `\d`, `\D` | `Nd` and all but it                                         | `\p{Nd}`, `\P{Nd}`                                  |
| `\w`, `\W` | `Alphabetic`, `M`, `Nd`, `Pc`, and `Join_Control`; the rest | `[\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\p{Join_Control}]` |
| `\s`, `\S` | `White_Space` and all but it                                | `\p{White_Space}`, `\P{White_Space}`                |
| `\b`, `\B` | A `\w` character on exactly one side; on both or neither    | Look-behind and look-ahead for the `\w` class       |
| `.`        | Any character but U+000A                                    | `[^\n]`                                             |

Inside a class `\d`, `\w`, `\s`, `\D`, and `\S` become members in place. A `u`-mode class cannot
hold the complement that `\W` is, so a class holding `\W` becomes the class beside `[^…]` of the
word characters. `\b` inside a class is refused: Rust refuses it, and JavaScript reads a backspace.
The client reads the pattern token by token to do this, and a pattern it cannot read as the subset,
or that its engine cannot compile, it leaves to the server.

**Refused**, each by name with the spelling to use (`PatternError`):

| Construct                                                            | Why                                                           |
| -------------------------------------------------------------------- | ------------------------------------------------------------- |
| Group flags, `(?i)` and `(?i:…)`                                     | JavaScript has no `(?i)`, and only its newer engines `(?i:…)` |
| Named groups                                                         | `(?P<n>…)` is Rust's alone, and nothing reads a group's name  |
| `\A`, `\z`                                                           | Not JavaScript's; `^` and `$` say the same                    |
| `[:alpha:]`, a class inside a class, `&&`, `--`, `~~` in a class     | JavaScript reads the characters                               |
| `\pL`, `\p{Greek}`, `\p{sc:Greek}`, a name loosely spelled           | JavaScript takes exact names and `name=value`                 |
| `\<`, `\>`, `\b{start}` and its kin                                  | Not JavaScript's                                              |
| `\x{41}`, `\U0001F600`, `\a`, a backslash before any other character | Not escapes in `u` mode                                       |
| An unescaped `]` or `}`; `-` between two members of a class          | JavaScript refuses it, or reads a range                       |
| `{ 2 }`, `{,n}`                                                      | JavaScript refuses the spaces; Rust has no `{,n}`             |
| A repetition of a repetition (`a**`) or of an assertion (`\b+`)      | JavaScript refuses it                                         |
| Look-around, backreferences, anything else that does not parse       | Rust has none                                                 |

The `\p{…}` names are the general categories and binary properties both engines know, and the
scripts for `Script` and `Script_Extensions`, kept as lists in `app::bot_command::pattern` and in
the vector file, which a test holds equal. Which characters a property holds follows each reader's
version of Unicode, so the two can differ on a character one of them does not yet know.

A list already kept is not checked again. An invocation compiles its parameter's pattern with
`pattern::published`, which applies only the size limit, so a command whose pattern was kept under a
wider subset takes the values it took until its bot publishes again; the client leaves such a
pattern to the server.

### Parameter types

| Type             | An argument must be                               |
| ---------------- | ------------------------------------------------- |
| `userId`         | A person who exists                               |
| `channelId`      | A channel the caller may read                     |
| `messageId`      | A message the caller may read                     |
| `communityId`    | A community the caller belongs to                 |
| `roleId`         | A role of this community                          |
| `attachmentId`   | A file sent with the invocation and uploaded      |
| `deploymentHost` | A deployment's domain                             |
| `react`          | One emoji, which the bot receives fully qualified |
| `any`            | Text of at most `MAX_ARGUMENT_CHARS`              |
| `regex`          | A match for the pattern                           |

## Reading lists

| Endpoint                           | Reads                                                                                                                                                       |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /bots/{bot}/commands`         | A bot's list                                                                                                                                                |
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
   - Each is what its parameter's type says, as the caller may reach it (see
     [Parameter types](#parameter-types)).
   - A refusal names the parameter and why.
3. The command becomes the caller's message of kind `command`:
   - Its content is the command as sent (`invocation_text`): people and roles as the tags a message
     names them by, anything else quoted where it holds whitespace.
   - `commandBot` names its bot.
   - It tags no one, fetches no link preview, wakes no phone, and cannot be edited.
4. The bot alone hears of it as `botCommandInvoked`, published to its user subject in the same
   transaction. It carries the message's id, the channel and its community, who invoked it, and the
   checked, typed arguments.

The bot answers as it answers anything, with messages.
