# Plugins

Plugins are WebAssembly components the operator installs, built against `spec/plugin.wit` and described by a manifest. They intercept and observe messages, answer routes, keep storage, and can add kinds of channel with views of their own.

`spec/plugins.md` is the design, and [`docs/operators/plugins.md`](../../operators/plugins.md) is the operator's guide.

## Key files

| Part | Where |
| --- | --- |
| Business logic | `app::plugin` |
| HTTP | `api::plugin` |
| Terminal commands | `operator::plugins` |
| Manifest | `app::plugin::manifest`, schema `spec/plugin_manifest.schema.json` |
| Interface | `spec/plugin.wit` |
| Engine, bindings, call limits | `aspen_plugin_runtime` (`server/plugin_runtime`) |
| Registry of running plugins | `app::plugin::registry::Plugins` |
| Host calls | `host::Call` |

## Example plugins

Each is built for `wasm32-wasip2` in a workspace of its own.

| Plugin | Exercises |
| --- | --- |
| `plugins/word_filter` | Intercepting, observing, and most host calls. |
| `plugins/forum` | A kind of channel and its view. |
| `plugins/calendar` | Timers, notices, cards, and a capability URL. |
| `plugins/blackjack` | A game whose table and players' chips are written by several servers and its timer at once, through `storage-swap`. |

## Pages

- [Installing](installing.md): installing, upgrading, removing, purging, and announcing changes.
- [Running](running.md): where plugins run, admission, and the sandbox of each call.
- [Intercepting](intercepting.md): deciding messages, edits, commands, and polls before they are saved.
- [Observing](observing.md): the durable consumer and what each plugin is told.
- [Host calls](host-calls.md): what the host answers and as whom, attachments, `fetch`, counters, and actions.
- [Storage](storage.md): keys, `storage-swap`, scopes, quotas, and forgetting.
- [Annotations and events](annotations-and-events.md): message and user annotations, `pluginEvent`, and `communityPlugin`.
- [Routes](routes.md): plugin routes, their limits and headers, and the host's reserved paths.
- [Channel types and views](channel-types-and-views.md): kinds of channel and their sandboxed pages.
- [Timers](timers.md): timers as jobs, setting, and firing them.
- [Notices and cards](notices-and-cards.md): telling someone of something, and messages with cards.
- [Capability URLs](capability-urls.md): per-person URLs and their revocation.
- [Catalogue and federation](catalogue.md): `GET /plugins` and plugins as capabilities.
- [When access is given or taken away](access.md): the revocation checklist for plugins.

The reasons behind these choices are in the [design notes](design-notes.md).
