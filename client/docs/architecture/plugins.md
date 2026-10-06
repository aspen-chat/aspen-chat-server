# Plugins

The deployment's plugins run on the server (`docs/architecture/plugins.md` at the repository's
root). The app draws what they contribute from records and from each plugin's catalogue, so a
plugin it has never heard of shows as well as one it has; the one code of a plugin's it shows is
a view's page, in a sandboxed frame that reaches the app only by the bridge (below).

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
- **Channels of a plugin's kind.** A plugin's `channelTypes` (each a `pluginType`, a name in the
  reader's language, a `glyph`, and its `view`'s path) are offered in the add dialog beside text
  and voice (`AddDialog`, `CreateChannelForm` with `ty="plugin"`, `AspenSync.createChannel`'s
  `pluginType`). `usePluginKind` finds the plugin and kind of a channel; the channel list draws
  it with its kind's glyph (`PluginGlyph`: board, calendar, list, chat, or a puzzle piece for a
  kind no plugin declares), opens it like a text channel, and gives it a text channel's menu,
  muting and notification level included, which the plugin's notices obey.
  `PluginChannelScreen` (`src/features/plugins/PluginChannel.tsx`) shows its header and the
  plugin's view, or says it needs a plugin that does not run here.
- **Views and the bridge.** `PluginView` shows the view's page from the deployment
  (`apiBase` and the kind's `view`) in an `iframe` sandboxed as the server serves it
  (`allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox`, no same origin), so
  it holds no session and reads nothing of the app's. It speaks `spec/plugins.md`'s bridge over
  `postMessage`, hearing only its own frame: `hello` on load and on `ready` (the plugin, view,
  channel and its name, community, the person, locale and direction, the plugin's catalogue,
  `apiBase`, and the theme), `theme` when the palette, mode, fonts, or the system's scheme
  change (`readTheme` resolves each colour token through a probe element, so `light-dark()`
  pairs arrive settled, and leaves the emoji family out of the font stacks: a view cannot load
  the app's faces, and the system's Noto Color Emoji would draw its digits and spaces),
  `request` made as the person through `AspenSync.pluginRoute`
  (`AspenClient.pluginRoute` builds the URL beneath the plugin's `routes/`, refusing `.` and
  `..` segments and methods a route never takes with 400, unsent, so a page cannot reach the
  rest of the API; `status` 0 when the deployment cannot be reached), `users` from the store or
  read (`AspenSync.loadUsers`, at most 100), the plugin's `pluginEvent`s for the channel, its
  community, or the person (`AspenSync.onPluginEvent`), and `open` for a channel the store
  holds.
- **Cards.** `PluginCard` (`src/features/plugins/PluginCard.tsx`), drawn by `MessageBody`, shows
  a message's `card` from its plugin's catalogue: a title, fields (a time in the reader's zone
  and language, a count, a person by `PersonName`, a link), and buttons styled as the plugin
  asks, each pressed through `AspenSync.pressCardButton`, disabled while on its way and saying
  why it failed. The card's change arrives as the message's update. A card of a plugin the
  catalogue lacks is not drawn, nor its buttons where the message is shown still.
- **Notices.** `AspenSync.onPluginNotice` hears `pluginNotice`, which the server sends only
  where the person's mute and level for the channel would tell of a message that tags them;
  `NotifyOnMessages` tells of it as of such a message, its title the plugin's name and its body
  the notice's text, opening its message, thread, or channel when clicked. Phones are woken with
  the `notice` pointer (see Push).
- **Permissions.** `managePlugins` is a community permission (in the Community group of the role
  editor) and a deployment permission, each named in `permissionNames` and
  `deploymentPermissionNames`.

`e2e/plugins.spec.ts` checks an annotation arriving and its popover, a changed message's mark,
the DM notice, turning a plugin on with its account's permissions, a channel of a plugin's kind
showing a stand-in page that talks over the bridge (its hello, a route, a path beyond the routes
refused, a person named, and only its own events) and passes axe, a kind no plugin declares, a
card's fields and button, and a notice's system notification; the world answers `GET /plugins`
and people's annotations with nothing.
