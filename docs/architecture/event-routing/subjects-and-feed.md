# Subjects and the feed

## Subjects

Every event is published on a NATS subject that names whose it is (`app::events`).

| Subject | What is published there |
| --- | --- |
| `aspen.events.c.{community}.ch.{channel}` | What happens in a channel: messages, reactions, polls, calls. |
| `aspen.events.c.{community}.all` | The community itself and what is defined in it: channels, categories, invites, memberships. |
| `aspen.events.u.{user}` | What is the user's alone: preferences, and their own membership changes. |

- A DM or group DM belongs to no community. What happens in it, and the DM itself, are published to each recipient's user subject. A recipient who just left gets the update that removes them too.
- A thread belongs wherever its parent does.
- Where a channel belongs is its `ChannelHome`. It never changes, and is cached while there is room (`events::ChannelHomes`, the 100,000 most used channels).

## The event feed

Each API server reads the stream once (`app::event_feed`):

1. One ordered JetStream consumer reads every event subject.
2. One dispatcher task hands each event, in order, to the routing tasks.
3. There are `event_feed_shards` routing tasks (one per logical CPU by default).
4. Each connection belongs to one shard. The shard delivers to it the events whose subject's owner (`events::subject_owner`) is its user or one of their communities.

[Why one consumer per server](design-notes.md#one-consumer-per-server)

## What a connection receives

- Its user's subject, plus each community they belong to.
- The community list is computed from the database when it connects, never from the client.
- Their own `userCommunity` events change it as the dispatcher routes them, in stream order. A join brings the community's events from that point, and a leave ends them.
- A connection that joins a community no local connection reads is dropped and resumes. It loads the community's model as it registers again.

Which of a community's events a connection receives depends on its view of the channel or category; see [Visibility](visibility.md).

## Typing frames

The dispatcher also subscribes to the core NATS subject `aspen.typing` (see [Typing](../typing.md)), outside what the stream keeps.

- It routes each word on it as an event of its scope:
  - a community's, through the model as it stands, naming the channel that decides who views it; or
  - each of a DM's people.
- It leaves out the readers the word names.
- It reaches only connections whose `viewing` set (`Subscription::viewing`) holds the channel typed in.
- It reaches its connections as an `ephemeral` frame with no sequence.
- It is never retained.
- It is skipped, rather than dropping the connection, when a connection's queue is full.
