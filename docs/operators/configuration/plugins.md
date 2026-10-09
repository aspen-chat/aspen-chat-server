# Plugin limits

How plugins run on this server. Which plugins are installed, and their settings, are in the
database; see [Plugins](../plugins.md).

## `[plugins]`

| Setting | Default | |
| --- | --- | --- |
| `intercept_millis` | `25` | How long a plugin has to decide a message about to be saved. Its author waits for it, so keep it short. |
| `observe_millis` | `10000` | How long a plugin has to handle something that happened, such as checking a new message's pictures with another service. |
| `route_millis` | `3000` | How long a plugin has to answer a request to one of its routes. |
| `memory_mib` | `64` | The most memory one call of a plugin may use, all its memories together. |
| `concurrency` | two per logical CPU | The most calls of plugins this server runs at once. See [Concurrency](#concurrency). |
| `concurrency_per_plugin` | one per logical CPU | The most calls of any one plugin this server runs at once. |
| `notify_per_minute` | `10` | The most notices one plugin may give one person in a minute, counted across every server. |
| `notify_per_day` | `100` | The same, in a day. |
| `storage_total_gib` | `16` | The most one plugin may keep, every community's, DM's, and person's share together, in GiB. Each share is also held to the plugin's own quota. `0` sets no limit beyond the shares'. |

### Concurrency

- A call waits for a place within its own time limit, and counts as failed when none comes.
- So a refusing filter (`failure: closed`) refuses messages while the server is this busy.
- A quarter of the `concurrency` places (at least one, from two up) are kept for deciding
  messages. Routes and observers cannot take them.
- A quarter of `concurrency_per_plugin` is likewise kept for deciding messages.
- With `memory_mib`, `concurrency` bounds what plugins can take of the server's memory.
