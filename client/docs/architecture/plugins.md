# Plugins

The deployment's plugins run on the server (`docs/architecture/plugins.md` at the repository's
root); the app runs no code of theirs. It draws what they contribute from records and from each
plugin's catalogue, so a plugin it has never heard of shows as well as one it has.

- **The catalogue.** `GET /plugins` is read with the bootstrap (`AspenSync`), and again
  (`loadPlugins`, one read shared by every ask) whenever an event names a plugin the store does
  not know: an annotation's, or one in a message's `alteredBy`. `RecordStore.plugins` (topic
  `plugins`, `usePlugins` and `usePlugin`) holds each plugin's name, description, whether it
  reads DMs, its account, the permissions that account asks for, its community settings, and its
  `messages`, already in the reader's language (the server makes the pseudo-locales). A
  deployment that does not answer runs none. `pluginText` and `pluginKey`
  (`packages/protocol/src/plugins.ts`) render a plugin's text, filling its `%{name}`
  placeholders.
- **Annotations.** Message reads ask for `annotations`, and `setAnnotations` gives each message
  read (and each echoed reply the read names) exactly the annotations listed for it; the
  `messageAnnotation` events add, patch, and remove them, an `update` or `delete` finding its
  message by `RecordStore`'s note of which message each annotation is about. A message's
  annotations go with it when it is deleted or leaves its window. `RecordStore.annotations`
  (topic `annotations:<messageId>`, `useAnnotations`) passes over those of plugins the catalogue
  lacks, so a plugin turned off or removed takes its notes with it. `MessageAnnotations`
  (`src/features/plugins/Annotations.tsx`) draws them under a message as chips coloured by
  severity; pressing one opens a popover saying which plugin said it, with its detail and link,
  so nothing depends on hover. A person's annotations are read when their card first shows
  them (`useUserAnnotations`, `AspenSync.loadUserAnnotations`, topic `userAnnotations:<userId>`),
  kept current by `userAnnotation` events, and drawn by `UserAnnotations` on `ProfileCard`.
- **Changed by a plugin.** `MessageBody` marks a message whose `alteredBy` names plugins with
  `AlteredBy`, "(changed by …)", beside the edited mark.
- **A plugin's account.** A user whose record names a `plugin` is that plugin's principal: a bot,
  marked as one, whose card names its plugin (`PluginAccount`) rather than an owner.
- **DMs.** While any plugin of the deployment reads DMs, every DM shows `DmPluginNotice` above
  its messages, naming them.
- **Community settings.** Holders of Manage plugins have a Plugins tab (`PluginsPanel`): each
  plugin with whether it runs there, read with `useCommunityPlugins` (`GET
/communities/{community}/plugins`, topic `communityPlugins:<communityId>`) and kept current by
  `communityPlugin` events, which only they receive. One that runs where it is turned on offers
  Turn on, with the permissions its account asks for ticked where the caller holds them
  (`AspenSync.enableCommunityPlugin`), and Turn off (`disableCommunityPlugin`); one that runs
  everywhere offers to bring its account in. Its settings are a `SettingsForm`.
- **`SettingsForm`** (`src/features/plugins/SettingsForm.tsx`) draws any plugin's settings from
  the fields it declares, labelled from its catalogue: a checkbox, a number, a line or lines of
  text (a list one entry a line), a choice, or the community's roles or text channels. A secret
  is never shown; typing replaces it. Saving sends only the fields that changed, `null` restoring
  a default, as a JSON Merge Patch.
- **The dashboard.** The Plugins tab (`PluginsSection`, `src/features/admin/Plugins.tsx`), for
  holders of View dashboard or Manage plugins, lists the installed plugins in the order they
  decide messages in, with what each was granted, the hosts it may call, what it keeps, and the
  storage it uses (`AdminApi.plugins`). Holders of Manage plugins turn each on or off, choose
  where it runs, move it earlier or later (`orderPlugins`), and change its settings
  (`updatePlugin`). Installing, upgrading, and removing are the server's terminal's alone, which
  the tab says.
- **Permissions.** `managePlugins` is a community permission (in the Community group of the role
  editor) and a deployment permission, each named in `permissionNames` and
  `deploymentPermissionNames`.

`e2e/plugins.spec.ts` checks an annotation arriving and its popover, a changed message's mark,
the DM notice, and turning a plugin on with its account's permissions; the world answers `GET
/plugins` and people's annotations with nothing.
