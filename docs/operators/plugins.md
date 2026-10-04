# Plugins

A plugin changes what your deployment does without changing Aspen: it can filter or rewrite
what people post, note things beside messages and on profiles, answer commands through an
account of its own, keep data, and answer requests at routes of its own. Anyone may write one,
so a plugin runs in a sandbox and can do only what you grant it when you install it.
`spec/plugins.md` is the design; this page is how to run them.

A plugin is two files: a WebAssembly component (`.wasm`) and its manifest
(`aspen-plugin.json`), which says what it is, what it asks to do, and what it keeps. Both are
kept in the database once installed, so every API server runs the same plugin and picks up a
change within moments, without a restart. Nothing needs copying to each server.

## Installing

```
aspen-chat-server plugins install path/to/aspen-plugin.json
```

It shows what the plugin asks for and asks you to accept:

- **What it may do**: read messages where it runs, change or refuse them before they are
  saved, note things beside messages or on profiles, read attachments, keep data, answer
  requests, send events to people's apps, and act through an account of its own.
- **Which hosts it may call**, when it sends what it sees elsewhere (an AI-image checker calling
  its service, say). Read this carefully: whatever it is shown may leave your deployment for
  those hosts.
- **What it keeps, and for how long**, in its author's words. Deleting a message cannot reach
  a copy a plugin sent elsewhere.
- **Its account**, and which community permissions it asks communities for.

A plugin that asks to run in DMs does not, unless you install it with `--grant-dms`. While any
plugin runs in DMs, every DM tells the people in it so, naming the plugin.

A new plugin is installed **off**. Configure it, then turn it on:

```
aspen-chat-server plugins settings org.example.filter '{"words": ["…"]}'
aspen-chat-server plugins enable org.example.filter
```

Installing again with a newer manifest of the same id upgrades it, keeping its settings, data,
mode, and whether it is on; it asks again when the new version asks for more. Add `--yes` to
accept without being asked (in scripts), and `--mode everywhere` to install a plugin that runs
in every community (below).

## Where it runs

- **`optIn`** (the default): it runs in the communities that turn it on. A member with the
  community permission **Manage plugins** (in the Admin template) turns it on under the
  community's settings, configures it there, and grants its account the permissions it asks
  for. They may turn it off again.
- **`everywhere`**: it runs in every community, which may configure it but not turn it off.
  For what the whole deployment must hold, such as a filter you are required to run.

`aspen-chat-server plugins mode <id> everywhere|optIn` changes it. Plugins that decide messages
before they are saved run in the order `plugins order <id> <id> …` sets, each seeing what the
one before left.

## Every command

| Command | |
| --- | --- |
| `plugins install <manifest> [--mode optIn\|everywhere] [--grant-dms] [--yes]` | Install or upgrade. |
| `plugins list` | Every plugin, with its order, version, mode, and whether it is on. |
| `plugins show <id>` | What it asks for, what it was granted, its settings (secrets hidden), and the storage it uses. |
| `plugins settings <id> '<json>'` | Change its settings: a JSON object by setting name; `null` restores a setting's default. |
| `plugins mode <id> <mode>` | Where it runs. |
| `plugins order <id>…` | The order plugins decide messages in. |
| `plugins enable <id>` / `disable <id>` | Turn it on or off everywhere. Turning it on needs every required setting. |
| `plugins remove <id> [--yes]` | Stop it: it no longer runs, its notes beside messages and on profiles go, and its account leaves every community. What it kept stays. |
| `plugins purge <id> [--yes]` | Delete everything a removed plugin kept. Its account stays as a bot no one owns, which a holder of Manage bots may delete. |

Holders of the deployment permission **Manage plugins** can turn plugins on and off, change
their mode, order, and settings from the Administration Dashboard. Installing, upgrading,
removing, and granting permissions are the terminal's alone.

## Limits

`[plugins]` in `aspen.toml` (see [Configuration](configuration.md#plugins)) sets how long each
call may run and how much memory it may use. A plugin that decides messages before they are
saved makes their authors wait, so its time is short. When a call fails or runs out of time, a
plugin that decides messages says in its manifest whether the message goes through anyway
(most) or is refused (filters that must hold). Each failure is logged with the plugin's id.

## Backups

Plugins, their settings, and what they keep are in the database, so backing it up backs them
up. Keep the plugin's files too if you may need to install the same version again.
