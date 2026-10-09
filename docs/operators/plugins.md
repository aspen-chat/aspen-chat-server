# Plugins

A plugin changes what your deployment does without changing Aspen. It can:

- filter or rewrite what people post;
- note things beside messages and on profiles;
- answer commands through an account of its own;
- keep data;
- answer requests at routes of its own.

Anyone may write one, so a plugin runs in a sandbox and can do only what you grant it when you
install it. `spec/plugins.md` is the design; this page is how to run them.

## What a plugin is

A plugin is two files:

- a WebAssembly component (`.wasm`);
- its manifest (`aspen-plugin.json`), which says what it is, what it asks to do, and what it
  keeps.

Both are kept in the database once installed. Every API server runs the same plugin, and picks up
a change within moments, without a restart. Nothing needs copying to each server.

## Installing

```
aspen-chat-server plugins install path/to/aspen-plugin.json
```

It shows what the plugin asks for, and asks you to accept:

| It asks for | What to check |
| --- | --- |
| What it may do | Read messages where it runs, change or refuse them before they are saved, note things beside messages or on profiles, read attachments, keep data, answer requests, send events to people's apps, and act through an account of its own. |
| Which hosts it may call | When it sends what it sees elsewhere (an AI-image checker calling its service, say). **Read this carefully: whatever it is shown may leave your deployment for those hosts.** |
| What it keeps, and for how long | In its author's words. Deleting a message cannot reach a copy a plugin sent elsewhere. |
| Its account | And which community permissions it asks communities for. |

### DMs

A plugin that asks to run in DMs does not, unless you install it with `--grant-dms`. While any
plugin runs in DMs, every DM tells the people in it so, naming the plugin.

### Turning it on

A new plugin is installed **off**. Configure it, then turn it on:

```
aspen-chat-server plugins settings org.example.filter '{"words": ["…"]}'
aspen-chat-server plugins enable org.example.filter
```

### Upgrading

Install again with a newer manifest of the same id. The upgrade keeps its settings, data, mode,
and whether it is on. It asks again when the new version asks for more.

### Install options

| Option | What it does |
| --- | --- |
| `--yes` | Accept without being asked (in scripts). |
| `--mode everywhere` | Install a plugin that runs in every community (see [Where it runs](#where-it-runs)). |
| `--grant-dms` | Let it run in DMs, if it asks to. |

### Views and channel kinds

A plugin may also add pages of its own (its views), and kinds of channel shown by them, such as a
forum's boards or a calendar.

- Their files are installed with it, from the directory its manifest names.
- Your API servers serve them to people's apps, sandboxed so a page reaches nothing of the app's.
- There is nothing more to set up.

Someone who may manage channels makes a channel of such a kind like any other, where the plugin
runs.

## Where it runs

| Mode | Where it runs |
| --- | --- |
| `optIn` (the default) | In the communities that turn it on. A member with the community permission **Manage plugins** (in the Admin template) turns it on under the community's settings, configures it there, and grants its account the permissions it asks for. They may turn it off again. |
| `everywhere` | In every community, which may configure it but not turn it off. For what the whole deployment must hold, such as a filter you are required to run. |

`aspen-chat-server plugins mode <id> everywhere|optIn` changes it.

Plugins that decide messages before they are saved run in the order `plugins order <id> <id> …`
sets. Each sees what the one before left.

## Every command

| Command | What it does |
| --- | --- |
| `plugins install <manifest> [--mode optIn\|everywhere] [--grant-dms] [--yes]` | Install or upgrade. |
| `plugins list` | Every plugin, with its order, version, mode, and whether it is on. |
| `plugins show <id>` | What it asks for, what it was granted, its settings (secrets hidden), and the storage it uses. |
| `plugins settings <id> '<json>'` | Change its settings: a JSON object by setting name. `null` restores a setting's default. |
| `plugins mode <id> <mode>` | Where it runs. |
| `plugins order <id>…` | The order plugins decide messages in. |
| `plugins enable <id>` / `disable <id>` | Turn it on or off everywhere. Turning it on needs every required setting. |
| `plugins remove <id> [--yes]` | Stop it: it no longer runs, its notes beside messages and on profiles go, and its account leaves every community. What it kept stays. |
| `plugins purge <id> [--yes]` | Delete everything a removed plugin kept: its data, and its settings, yours and every community's. `plugins list` still shows it as removed, and its account stays, unused, for if you install it again. |

### From the dashboard

Holders of the deployment permission **Manage plugins** can, from the Administration Dashboard:

- turn plugins on and off,
- change their mode, order, and settings.

Installing, upgrading, removing, and granting permissions are the terminal's alone.

## Limits

`[plugins]` in `aspen.toml` (see [`[plugins]`](configuration/plugins.md#plugins)) sets how long each call
may run and how much memory it may use.

- A plugin that decides messages before they are saved makes their authors wait, so its time is
  short.
- When a call fails or runs out of time, a plugin that decides messages says in its manifest
  whether the message goes through anyway (most) or is refused (filters that must hold).
- Each failure is logged with the plugin's id.

See [Troubleshooting: Plugins](troubleshooting/plugins.md) for the errors people see.

## Backups

Plugins, their settings, and what they keep are in the database, so backing it up backs them up.
Keep the plugin's files too, if you may need to install the same version again.
