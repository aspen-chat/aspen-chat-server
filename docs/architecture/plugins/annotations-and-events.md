# Annotations and events

## Annotations

| Record | Published with | Read with |
| --- | --- | --- |
| `messageAnnotation` | `EventScope::Message` (the message's channel, and its `Aspen-Channel`) | `include=annotations` on message reads, as `included.messageAnnotations`, read only for messages the read already allowed. |
| `userAnnotation` | `EventScope::UserEverywhere`, as a profile is | `GET /users/{user}/annotations`. |

- There is one of each kind per plugin and subject.
- Setting one again is an `update`.

## `pluginEvent`

`pluginEvent` carries the plugin's kind and JSON payload.

- The payload is 16 KiB at most, measured both as given and as written out again for publishing.
- It goes to a channel's viewers, a community, or one user.
- `expected_kind` routes it by which of `channel` and `community` it names.

## `communityPlugin`

A `communityPlugin` event announces a community turning a plugin on or off, or configuring it. It carries `Aspen-Requires: managePlugins`, so only the community's managers receive its settings.
