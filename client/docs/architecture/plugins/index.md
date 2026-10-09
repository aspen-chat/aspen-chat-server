# Plugins

The deployment's plugins run on the server (see the server's
[plugins](../../../../docs/architecture/plugins/index.md)). The app draws what they contribute from
records and from each plugin's catalogue, so a plugin it has never heard of shows as well as one it
has. The one code of a plugin's it shows is a view's page, in a sandboxed frame that reaches the app
only by the bridge.

## Pages

- [The catalogue and plugins' accounts](catalogue.md): `GET /plugins`, the store, a plugin's text,
  its account, the DM notice, and the `managePlugins` permission.
- [Annotations](annotations.md): notes on messages and people, and the "changed by" mark.
- [Settings](settings.md): the community's Plugins tab, `SettingsForm`, and the dashboard.
- [Channels and views](channels-and-views.md): channels of a plugin's kind, `PluginView`, and the
  bridge.
- [Cards and notices](cards-and-notices.md): a message's card and its buttons, and plugin notices.
- [Tests](tests.md): what `e2e/plugins.spec.ts` checks.
- [Design notes](design-notes.md): why the app handles plugins this way.

## Key files

| Part | Where |
| --- | --- |
| Plugin text | `packages/protocol/src/plugins.ts` (`pluginText`, `pluginKey`) |
| Annotations | `src/features/plugins/Annotations.tsx` |
| Settings form | `src/features/plugins/SettingsForm.tsx` |
| Dashboard tab | `src/features/admin/Plugins.tsx` |
| Plugin channels and views | `src/features/plugins/PluginChannel.tsx` |
| Cards | `src/features/plugins/PluginCard.tsx` |
| Bridge protocol | `spec/plugins.md` |
