# Subjects and the feed

## Subjects

Every event is published on a NATS subject that names whose it is (`app::events`).

| Subject                                   | What is published there                                                                     |
| ----------------------------------------- | ------------------------------------------------------------------------------------------- |
| `aspen.events.c.{community}.ch.{channel}` | What happens in a channel: messages, reactions, polls, calls.                               |
| `aspen.events.c.{community}.all`          | The community itself and what is defined in it: channels, categories, invites, memberships. |
| `aspen.events.u.{user}`                   | What is the user's alone: preferences, and their own membership changes.                    |
| `aspen.events.p.{user}`                   | A copy of what is about the user (their profile), for the connections following them.       |

- A DM or group DM belongs to no community. What happens in it, and the DM itself, are published to
  each recipient's user subject. A recipient who just left gets the update that removes them too.
- An event about a user (`EventScope::UserEverywhere`) goes to each of their communities, to their
  own subject, and to their profile subject, one copy each; one for some of their communities alone
  (`UserIn`, a plugin's note, or a bot's making and deletion for its owner with no communities at
  all) goes to those and to their own subject; see [Publishing](publishing.md#events-about-a-user).
- A thread belongs wherever its parent does.
- Where a channel belongs is its `ChannelHome`. It never changes, and is cached while there is room
  (`events::ChannelHomes`, the 100,000 most used channels).

## The event feed

Each API server reads the stream once (`app::event_feed`):

1. One ordered JetStream consumer reads every event subject.
2. One dispatcher task hands each event, in order, to the routing tasks.
3. There are `event_feed_shards` routing tasks (one per logical CPU by default).
4. Each connection belongs to one shard. The shard delivers to it the events whose subject's owner
   (`events::subject_owner`) is its user, one of their communities, or a profile it follows.

[Why one consumer per server](design-notes.md#one-consumer-per-server)

## What a connection receives

- Its user's subject, plus each community they belong to.
- The community list is computed from the database when it connects, never from the client.
- Their own `userCommunity` events change it as the dispatcher routes them, in stream order. A join
  brings the community's events from that point, and a leave ends them.
- A connection that joins a community no local connection reads is dropped and resumes. It loads the
  community's model as it registers again.

Which of a community's events a connection receives depends on its view of the channel or category;
see [Visibility](visibility.md).

## Profiles of DM partners

A DM partner who shares no community with the user would otherwise never hear of their new name,
their new avatar, or that their account is gone. Each connection follows the profile subjects of its
user's DM partners (`app::event_feed::Following`), with no visibility check: the copy on
`aspen.events.p.{user}` reaches every connection following that profile.

- At registration it follows the people of the user's most recently active DMs
  (`dm_recipient.active_at`), as many DMs as keep the partners within
  `[limits] max_followed_dm_partners` (100).
- A DM `channel` create or `recipients` update reaching the connection on its user's own subject
  follows that DM's people as its most recent, letting go of the oldest DMs' past the cap; one whose
  `recipients` no longer hold the user is let go of. A partner in another followed DM stays followed
  through it.
- The user's own connections do not follow their own profile: they receive the copy on their own
  subject.
- The index of who follows whom (`Routes::by_profile`) holds one entry per followed partner per
  connection, so it is bounded by the cap times the connections.
- A partner from an older DM than the cap reaches is shown as the client last read them until it
  reads them again.

## Notices to every connection

A server tells all its connections of something at once through the feed too
(`EventFeed::tell_everyone`): the installed plugins changed (`pluginCatalogueChanged`, see
[Plugins](../plugins/installing.md#announcing-changes)). The event is serialized once and handed to
each shard, which queues it for every connection it holds as an `ephemeral` frame; a connection
whose queue is full goes without it, as it would without a typing frame.

## Typing frames

The dispatcher also subscribes to the core NATS subject `aspen.typing` (see [Typing](../typing.md)),
outside what the stream keeps.

- It routes each word on it as an event of its scope:
  - a community's, through the model as it stands, naming the channel that decides who views it; or
  - each of a DM's people.
- It leaves out the readers the word names.
- It reaches only connections whose `viewing` set (`Subscription::viewing`) holds the channel typed
  in.
- It reaches its connections as an `ephemeral` frame with no sequence.
- It is never retained.
- It is skipped, rather than dropping the connection, when a connection's queue is full.
