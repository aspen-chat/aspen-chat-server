# Running

## The registry

`app::plugin::registry::Plugins` holds the plugins that are on, compiled (`PluginPre`), in order.

- It caches each community's rows of `community_plugin` for at most a minute.
- A community's cache is forgotten at once when the community's use changes. The change is announced on `aspen.plugins.changed`, naming the community.

## Where a plugin runs

| Function | Plugins |
| --- | --- |
| `running_in_community` | Every `everywhere` plugin, and every `optIn` plugin the community turned on, each with the community's settings (defaults filled in). |
| `running_in_dms` | Those granted `dms`. |

A thread runs where its parent does.

## Admission

Each call waits for places before it runs (`Plugins::admit`):

1. one of its plugin's places (`[plugins] concurrency_per_plugin`);
2. then one of every plugin's places (`concurrency`).

It waits within its own deadline, and fails as any call does when none comes.

### Places kept for intercepting

A quarter of each set of places is kept for intercepting calls (`registry::Places`). That is at least one, where there are two or more. A call that does not intercept also needs a place among the rest. **Why:** see the [design notes](design-notes.md#places-kept-for-intercepting).

## Each call's sandbox

Each call is a fresh instance in a store of its own (`host::Instance`).

| Limit | How |
| --- | --- |
| Memory | `[plugins] memory_mib` for all its memories together. |
| Instances, tables, table elements | Bounded by `aspen_plugin_runtime::CallLimits`. |
| Time | A deadline kept by the engine's epoch. |

### The deadline

- A thread of its own ticks the epoch every millisecond (`aspen_plugin_runtime::start_ticker`).
- At each tick, a call within its deadline yields to Tokio, and one past it traps.
- The whole call, host calls included, is cut off at the deadline (`host::within`).

The crate `aspen_plugin_runtime` (`server/plugin_runtime`) also holds the bindings `spec/plugin.wit` generates, and the engine.

### WASI

WASI is linked with nothing behind it: no preopens, sockets, environment, or arguments.

Host calls live on `host::Call`, apart from the store's WASI state, since what they hold across an await must be shareable between threads.
