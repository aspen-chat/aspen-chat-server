# Installing

Only the terminal installs (`app::plugin::install::install`).

## Install steps

1. Check the manifest (`Manifest::check`):
   - a reversed-domain id;
   - semver;
   - interface version 1;
   - every message key the manifest names is present in its default language;
   - settings fields are well formed;
   - permissions and what they need go together.
2. Compile the component against the host's linker, to prove it imports nothing the host lacks (`Plugins::check_component`).
3. Show what it asks for.
4. Write the row of `plugin`: manifest, component, granted permissions, `mode`, `position`, and settings. It starts off.

`dms` is granted only with `--grant-dms`.

## The principal

A manifest with a `principal` makes its account:

- a `user` row with `bot` set, no owner, and `plugin` naming it;
- `app::bot::delete` refuses to delete it;
- its commands are published as any bot's are.

## Upgrading

An upgrade replaces the manifest and component. It keeps the mode, the order, whether it is on, and the settings it still has. It asks again when the plugin asks for more.

## Remove and purge

| Command | Does | Keeps |
| --- | --- | --- |
| `remove` | Turns it off and drops the component at once. Saves a job (`retirePlugin`) that deletes its annotations a batch at a time and takes its principal out of every community fifty at a time. | Everything else. |
| `purge` | Deletes its settings at once. Saves a job (`purgePlugin`) deleting its storage, usage counts, timers, capability URLs, and communities' rows a batch at a time. | The row (the record that it was installed) and its principal, which installing it again takes up. |

Both jobs are described in [Jobs](../jobs/index.md).

Annotations deleted after `remove` go without events. No read shows a removed plugin's annotations while the job runs, and clients draw none for a plugin `GET /plugins` does not list.

## The dashboard

Settings, mode, order, and on/off are also the dashboard's, under the deployment permission Manage plugins (`/admin/plugins`, `/admin/plugin-order`).

## Announcing changes

1. Every change touches `updated_at`.
2. It is announced on the core NATS subject `aspen.plugins.changed` (`registry::announce`).
3. Every API server reloads on it (`Plugins::reload`), compiling a component again only when its SHA-256 changed.
