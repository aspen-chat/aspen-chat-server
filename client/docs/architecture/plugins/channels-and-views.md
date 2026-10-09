# Channels and views

## Channels of a plugin's kind

A plugin's `channelTypes` each have a `pluginType`, a name in the reader's language, a `glyph`, and
its `view`'s path.

- They are offered in the add dialog beside text and voice (`AddDialog`, `CreateChannelForm` with
  `ty="plugin"`, `AspenSync.createChannel`'s `pluginType`).
- `usePluginKind` finds the plugin and kind of a channel.
- The channel list draws it with its kind's glyph (`PluginGlyph`: board, calendar, list, chat, or a
  puzzle piece for a kind no plugin declares).
- It opens like a text channel, and has a text channel's menu, muting and notification level
  included. The plugin's notices obey them.
- `PluginChannelScreen` (`src/features/plugins/PluginChannel.tsx`) shows its header and the
  plugin's view, or says it needs a plugin that does not run here.

## The view's frame

`PluginView` shows the view's page from the deployment (`apiBase` and the kind's `view`) in an
`iframe`, sandboxed as the server serves it:

```
allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox
```

There is no same origin, so the page holds no session and reads nothing of the app's.

## The bridge

The page speaks `spec/plugins.md`'s bridge over a `MessagePort`.

| Message | What it does |
| --- | --- |
| `hello` | Posted to the frame once its page has loaded. Hands the page one end of a `MessageChannel`, with the plugin, view, channel and its name, community, the person, locale and direction, the plugin's catalogue, `apiBase`, and the theme. |
| `theme` | Sent when the palette, mode, fonts, or the system's scheme change. See [the theme](#the-theme). |
| `request` | Made as the person through `AspenSync.pluginRoute`. See [requests](#requests). |
| `users` | People from the store or read (`AspenSync.loadUsers`), at most 100. |
| Plugin events | The plugin's `pluginEvent`s for the channel, its community, or the person (`AspenSync.onPluginEvent`). |
| `open` | Opens a channel the store holds. |

### The theme

- `readTheme` resolves each colour token through a probe element, so `light-dark()` pairs arrive
  settled.
- `useBridgeTheme` adds the address of `view-fonts.css` on the view's deployment, which `viewFonts.ts`
  builds from every face the app bundles.
- It also adds the files of the user's own faces drawn now (`fontLibrary.facesUnder`).
- The view registers these with `FontFace`. Its origin is its own, so it can reach neither the app's
  stylesheets nor the library.

### Requests

`AspenClient.pluginRoute` builds the URL beneath the plugin's `routes/`.

- It refuses `.` and `..` segments, and methods a route never takes, with 400, unsent. So a page
  cannot reach the rest of the API.
- `status` is 0 when the deployment cannot be reached.
- The plugin's answer comes back as it came, since its status codes are its own:
  - its `401` refreshes no session;
  - its `403` flags no enrollment;
  - its `429` is not waited out.

### Who the bridge answers

**The bridge speaks only to the page the app loaded.**

- The port belongs to the page it was handed to. A page the frame goes to after (a link, a redirect)
  cannot use it.
- Nothing posted to the frame's window is answered.
- An answer goes back over the port it was asked on, and only while that port is still the frame's.
- A later load of the same frame closes the port and puts "Load the view again" in the frame's place.
  That makes a new frame with a new port.
