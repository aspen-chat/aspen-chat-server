# Plugins

See also [Plugins](../plugins.md) for installing and running them.

## `pluginRefused`

**Means:** a plugin refused a message before it was saved. The detail is the plugin's reason, in
the reader's language.

**What to do:** nothing, unless the plugin is wrong. Its community settings (or yours,
`plugins show <id>`) decide what it refuses.

## `pluginUnavailable`

**Means:** one of these:

- a plugin that must decide messages could not (it failed, or ran out of time), and its manifest
  says to refuse them then;
- a plugin's route could not answer.

The detail names the plugin.

**What to do:**

- The log has a line for each failure, with the plugin's id.
- If it only runs out of time, raise `[plugins] intercept_millis`.
- `plugins disable <id>` stops it meanwhile.
