# Channel types and views

A plugin holding `channelTypes` declares kinds of channel (`channelTypes` in its manifest). Each kind is shown by a page among its `assets`.

## Assets

Installing reads the assets from the manifest's directory (`asset::read_dir`) into `plugin_asset`. Every server holds them in memory with the plugin.

| Rule | Limit |
| --- | --- |
| Kinds of file | Those a page uses. |
| One file | 2 MiB. |
| All files | 20 MiB together. |
| Views | Every kind's view must be among them. |

## Channels of a kind

- A channel of a kind has `ty` `plugin` and `plugin_type` naming it (`org.example.forums:board`).
- `POST /channels` makes one where a plugin that declares the kind runs (`channel_type::check`), with Manage channels as any channel.
- Posting a message to one is refused (`pluginChannelHasNoMessages`).
- Its contents are the plugin's storage scoped to the channel. View channel, overrides, and its deletion govern them as they govern a channel's messages.

## Serving the view

The files are served anonymously at `/api/v1/plugins/{id}/assets/{*path}` (`api::plugin::asset`) with `asset::VIEW_POLICY`:

- a CSP sandbox without `allow-same-origin`, which gives a page an opaque origin however it is opened;
- scripts, styles, pictures, and fonts of its own or inline;
- no connection anywhere.

The app shows the page in a frame sandboxed the same way. The bridge described in `spec/plugins.md` is its only way to the plugin, whose routes the app calls as the person.

## In the catalogue

`GET /plugins` gives each kind's `pluginType`, name, glyph, and view.
