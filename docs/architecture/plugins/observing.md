# Observing

Code: `observe::run`, `observe::deliver`.

## The consumer

Each plugin that observes anything has a durable JetStream consumer of its own, named from its id. Every API server pulls from it with explicit acknowledgement.

An event the plugin's call fails on is delivered again:

- a second later;
- three times at most;
- only while the stream retains it (`MAX_EVENT_AGE`).

## DM events

A DM's events come once per recipient's subject. `first_copy` keeps, in Valkey, which subject's copy the plugin handles. So the plugin is told once, and a redelivery of that copy is still handled.

Events on a community's subjects come once, and need no such note.

## Where the plugin runs

Every observer reads every event, so where the plugin runs is decided first, from the event's subject alone:

- a community whose plugins are cached (`Plugins::runs_in`);
- a DM, which takes `dms`.

An event elsewhere costs nothing more.

## What a plugin is told

| Event | Told as | When |
| --- | --- | --- |
| A message's `update` | `message-edited` | When its text or its attachments changed. |
| Message events | The message as saved, read after its transaction commits, retrying briefly | Only where the plugin runs, and with `messages.read`. |
| `botCommandInvoked` | `command-invoked` | Reaches the plugin whose principal it names. |
| A `communityPlugin` event of its own | `plugin-enabled` or `plugin-disabled` | Always. |

Timers are delivered through `observe::deliver` too (see [Timers](timers.md)).
