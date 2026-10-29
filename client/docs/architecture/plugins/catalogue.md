# The catalogue and plugins' accounts

## The catalogue

`GET /plugins` is read:

- with the bootstrap (`AspenSync`);
- at every `ready` of the event stream: a change while this client was not connected was told to no
  one;
- on the ephemeral frame `pluginCatalogueChanged`, which every API server writes to every open
  stream when a plugin is installed, upgraded, turned on or off, reordered, or removed;
- again (`loadPlugins`, one read shared by every ask) whenever an event names a plugin the store
  does not know, or a kind of channel no plugin in it adds: an annotation's plugin, a card's, a
  plugin's own event's (`pluginEvent`) or notice's (`pluginNotice`), one in a message's `alteredBy`,
  or a new plugin channel's `pluginType` (`RecordStore.pluginKind`).

A read for word that the catalogue changed (the frame, `ready`) is sent after any read already in
flight, which may have been answered before the change; every such ask while one is in flight shares
the read that follows it.

A deployment that does not answer runs none.

### In the store

`RecordStore.plugins` (topic `plugins`, hooks `usePlugins` and `usePlugin`) holds for each plugin:

- its name and description;
- whether it reads DMs;
- its account, and the permissions that account asks for;
- its community settings;
- its `messages`, already in the reader's language. The server makes the pseudo-locales.

Installing a catalogue (`setPlugins`) drops what was held of a plugin it no longer lists: its
annotations of messages and of people, and each community's use of it (`communityPlugins`). Nothing
but the catalogue says a plugin was removed, since its rows go without events.

### A plugin's text

`pluginText` and `pluginKey` (`packages/protocol/src/plugins.ts`) render a plugin's text, filling
its `%{name}` placeholders.

## A plugin's account

A user whose record names a `plugin` is that plugin's principal: a bot, marked as one. Its card
names its plugin (`PluginAccount`) rather than an owner.

## DMs

While any plugin of the deployment reads DMs, every DM shows `DmPluginNotice` above its messages,
naming them.

## Permissions

`managePlugins` is both a community permission (in the Community group of the role editor) and a
deployment permission. It is named in `permissionNames` and `deploymentPermissionNames`.
