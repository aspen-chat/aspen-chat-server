# Annotations and events

## Annotations

| Record              | Published with                                                          | Read with                                                                                                                  |
| ------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `messageAnnotation` | `EventScope::Message` (the message's channel, and its `Aspen-Channel`)  | `include=annotations` on message reads, as `included.messageAnnotations`, read only for messages the read already allowed. |
| `userAnnotation`    | `EventScope::UserIn`: the person, and the communities where it is shown | `GET /users/{user}/annotations`, which the client reads again each time a view of the person opens.                        |

- There is one of each kind per plugin and subject.
- Setting one again is an `update`.
- A person's annotations are shown only in the communities of theirs where the plugin runs
  (`annotation::shown_in`: installed, and on everywhere or turned on there). They reach the person
  and whoever shares one of those communities with them, and no one else.
- `annotate-user` answers `notFound` when the plugin runs in none of the person's communities, or
  when it does not reach them (`host::Call::reaches`). In a route the caller must share one of those
  communities with the person, unless they are the person.

## `pluginEvent`

`pluginEvent` carries the plugin's kind and JSON payload.

- The payload is 16 KiB at most, measured both as given and as written out again for publishing.
- It goes to a channel's viewers, a community, or one user. One user is, in a route, only its
  caller, and otherwise someone the plugin reaches (`host::Call::reaches`).
- `expected_kind` routes it by which of `channel` and `community` it names.

## `communityPlugin`

A `communityPlugin` event announces a community turning a plugin on or off, or configuring it. It
carries `Aspen-Requires: managePlugins`, so only the community's managers receive its settings.
