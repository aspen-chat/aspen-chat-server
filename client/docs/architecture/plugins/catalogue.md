# The catalogue and plugins' accounts

## The catalogue

`GET /plugins` is read:

- with the bootstrap (`AspenSync`);
- again (`loadPlugins`, one read shared by every ask) whenever an event names a plugin the store does
  not know: an annotation's, or one in a message's `alteredBy`.

A deployment that does not answer runs none.

### In the store

`RecordStore.plugins` (topic `plugins`, hooks `usePlugins` and `usePlugin`) holds for each plugin:

- its name and description;
- whether it reads DMs;
- its account, and the permissions that account asks for;
- its community settings;
- its `messages`, already in the reader's language. The server makes the pseudo-locales.

### A plugin's text

`pluginText` and `pluginKey` (`packages/protocol/src/plugins.ts`) render a plugin's text, filling its
`%{name}` placeholders.

## A plugin's account

A user whose record names a `plugin` is that plugin's principal: a bot, marked as one. Its card names
its plugin (`PluginAccount`) rather than an owner.

## DMs

While any plugin of the deployment reads DMs, every DM shows `DmPluginNotice` above its messages,
naming them.

## Permissions

`managePlugins` is both a community permission (in the Community group of the role editor) and a
deployment permission. It is named in `permissionNames` and `deploymentPermissionNames`.
