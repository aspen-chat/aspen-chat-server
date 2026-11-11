# Observing

Code: `observe::run`, `observe::deliver`.

## The consumer

Each plugin that observes anything has a durable JetStream consumer of its own, named from its id.
Every API server pulls from it with explicit acknowledgement.

An event the plugin's call fails on is delivered again:

- a second later;
- three times at most;
- only while the stream retains it (`MAX_EVENT_AGE`).

## DM events

A DM's events come once per recipient's subject. `first_copy` keeps, in Valkey, which subject's copy
the plugin handles. So the plugin is told once, and a redelivery of that copy is still handled.

Events on a community's subjects come once, and need no such note.

## Where the plugin runs

Every observer reads every event, so where the plugin runs is decided first, from the event's
subject alone:

- a community whose plugins are cached (`Plugins::runs_in`);
- a DM, which takes `dms`.

An event elsewhere costs nothing more.

## What a plugin is told

| Event                                | Told as                                                                    | When                                                       |
| ------------------------------------ | -------------------------------------------------------------------------- | ---------------------------------------------------------- |
| A message's `update`                 | `message-edited`                                                           | When its text, its files, or a file's description changed. |
| Message events                       | The message as saved, read after its transaction commits, retrying briefly | Only where the plugin runs, and with `messages.read`.      |
| `botCommandInvoked`                  | `command-invoked`                                                          | Reaches the plugin whose principal it names.               |
| A `communityPlugin` event of its own | `plugin-enabled` or `plugin-disabled`                                      | When the community turned it on or off (see below).        |

Events are published before their transaction commits, so the message is read with a short wait
(`observe::message_row`): a new message until its row is there, an edited one until its row holds
the edit the event names (`editedAt`) or a later one, and one an attachment was taken off, whose
update names no edit, until it holds no attachment but those the event says it keeps (`attachments`;
a posted message only loses them). A change that has not shown in that time fails the call, so the
event is delivered again rather than handled with what the message held before it.

A plugin is told a community turned it on or off only when it did (`observe::toggled`): by an
`update` that carries `enabled`, which `community::enable` sets only when the plugin was off there,
and by the `create` of an `optIn` plugin's first record there when it says on. Saving a community's
settings for a plugin, turning on one that is already on, and the first record of an `everywhere`
plugin tell it nothing.

Timers are delivered through `observe::deliver` too (see [Timers](timers.md)).
