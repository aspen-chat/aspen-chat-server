# Plugins: design notes

Rationale behind the [plugins](index.md) pages.

## Drawing from records and the catalogue

- **The app draws plugins' contributions from records and each plugin's catalogue.** A plugin the app
  has never heard of shows as well as one it has. The one code of a plugin's the app shows is a
  view's page, in a sandboxed frame.
- **The catalogue is read again whenever an event names an unknown plugin.** One read is shared by
  every ask (`loadPlugins`). See [the catalogue](catalogue.md).
- **Annotations and cards of plugins the catalogue lacks are passed over.** A plugin turned off or
  removed takes its notes with it. See [annotations](annotations.md).
- **Pressing an annotation opens a popover.** Nothing depends on hover.

## Views and the bridge

See [channels and views](channels-and-views.md).

- **The view's frame has no same origin.** It holds no session and reads nothing of the app's.
- **Theme colours are resolved through a probe element.** `light-dark()` pairs arrive settled.
- **Fonts are handed over as addresses (`view-fonts.css` and the user's own faces).** The view's
  origin is its own, so it can reach neither the app's stylesheets nor the font library.
- **`pluginRoute` refuses `.` and `..` segments and unknown methods, unsent.** A page cannot reach the
  rest of the API.
- **A route's answer comes back as it came.** The plugin's status codes are its own: its `401`
  refreshes no session, its `403` flags no enrollment, and its `429` is not waited out.
- **The port belongs to the page it was handed to.** A page the frame goes to after (a link, a
  redirect) cannot use it, and nothing posted to the frame's window is answered.

## Cards

See [cards and notices](cards-and-notices.md).

- **A card button's press is fetched as a plugin's route is.** The answer is the plugin's to choose,
  so it never refreshes or flags the session, and every failure reads the same, naming the plugin.
