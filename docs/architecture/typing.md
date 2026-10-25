# Typing

Who is typing where is told as it happens and kept nowhere: no table, no JetStream. Design notes:
[typing-design-notes.md](typing-design-notes.md).

| Part                           | Where                                                                                                                           |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| Server logic                   | `app::typing`                                                                                                                   |
| Per-connection task            | `Typist`                                                                                                                        |
| Permission check               | `app::message::may_type`                                                                                                        |
| NATS subject                   | `aspen.typing` (core NATS)                                                                                                      |
| Routing                        | `event_feed::relayed_events`                                                                                                    |
| Blocked readers left out       | `typing::blocked_in`                                                                                                            |
| Channels a connection has open | `Subscription::viewing`, at most `event_feed::MAX_VIEWING`                                                                      |
| Client                         | `AspenSync`, `RecordStore.typers`, `useTypers`, `AspenSync.watchTyping` (see `client/docs/architecture/composer-and-drafts.md`) |
| Account preference             | `privacy.typingNotices`                                                                                                         |

## Constants

| Constant                  | Value     | Meaning                                                            |
| ------------------------- | --------- | ------------------------------------------------------------------ |
| `TYPING_REFRESH_SECONDS`  | 3         | How often a client repeats `typing` while its user goes on         |
| `TYPING_EXPIRY_SECONDS`   | 8         | How long someone shows as typing after the last word that they are |
| Repeat window             | 2 seconds | A `typing` for a channel told of less than this before is dropped  |
| Channels shown            | 4         | A `typing` for a fifth channel is dropped                          |
| Budget                    | 4, 2/s    | Checks and publishes one connection may make in a burst, and after |
| `event_feed::MAX_VIEWING` | 8         | Most channels one `viewing` frame may name                         |
| `viewing` lists taken     | 1/s       | One sent sooner waits, replaced by any later one                   |

## Client frames

A client says its user is typing in a channel with a `typing` frame on its event stream:

```json
{"type":"typing","channelId":…}
```

- It sends it again every `TYPING_REFRESH_SECONDS` (three) while they go on.
- It sends `stoppedTyping` when they send, empty the box, or leave it.
- Turning off the account preference `privacy.typingNotices` stops the reference client sending
  either. Nothing else changes, so the user still sees others typing.

A client also says which channels it has open for typing, with a `viewing` frame:

```json
{"type":"viewing","channelIds":[…]}
```

- It names at most `event_feed::MAX_VIEWING` (eight) channels.
- The client sends one on every `ready`, and whenever the channels showing typing change.
- Each replaces the last. The connection's shard keeps the set (`Subscription::viewing`).
- A connection takes at most one a second (`event_stream::VIEWING_EVERY`). One sent sooner waits
  until then, replaced by any sent after it, so the last always arrives.
- The reference client shows typing only beside a message box (`useTypers`,
  `AspenSync.watchTyping`), and names those channels.

## Checking and publishing

Each connection hands its frames to a task of its own (`Typist`), which takes them in order. It
remembers, for each channel it was told of in the last `TYPING_EXPIRY_SECONDS`, when it last checked
it and what the check found, through stops and refusals alike, so no frame has a channel checked
more than once in two seconds.

1. A `typing` for a channel it told of less than two seconds before is dropped.
2. A `typing` for a channel refused less than two seconds before is dropped.
3. A `typing` for a fifth channel while four are shown is dropped.
4. Every other takes a token of the connection's budget (four, earned back at two a second, the same
   for every channel), and is dropped when there is none.
5. One for a channel checked less than two seconds before is published to whom that check found.
6. Any other is checked as posting text is (`app::message::may_type`, which reads as `may_post` does
   but locks no row): a channel that holds messages, and Send messages, or Send in threads in a
   thread. This also refuses a blocked one-to-one DM and the system account's.
7. One refused is remembered as refused. What that connection showed there runs out by the expiry
   rather than being withdrawn, since the audience it was told to was read at least a refresh ago.
8. One allowed is published.

A `stoppedTyping` for a channel shown is said to the audience the last check found when that check
is under two seconds old. Otherwise the channel is checked again, taking a token, and the stop goes
to whom that finds; with no token, or refused, nothing is said and what was shown runs out by the
expiry. So a stop never reaches someone read into the audience longer ago than that, such as a
person just taken out of a group DM.

So once its burst is spent one connection makes at most two checks a second, and sends at most four
relays a second to every API server.

### Typing while invisible

Typing tells the channel the typist is about, whatever their presence says. Someone invisible (see
[Presence](event-routing/presence.md)) who types is shown typing to those who may view the channel,
as on other chat services; turning off `privacy.typingNotices` keeps that from happening.

### What is published

Published means one message on the core NATS subject `aspen.typing`, over the connection each server
already holds. It lies outside `aspen.events.>`, so the stream neither keeps nor replays it.

The message carries:

- the `EphemeralEvent`: `typing`, with the channel, the user, and `typing: true` or `false`;
- who it reaches: a community and the channel governing the one typed in, a thread's parent, or a
  DM's people, read when the word was checked;
- who it does not reach: the typist, and those they block whom the audience holds (members of the
  community, or the DM's people; `typing::blocked_in`). A block keeps them from these as it keeps
  their presence (see [Blocking](blocking.md)).

### When a connection ends

- For whatever reason a connection ends, its task says `typing: false` for every channel it showed
  in the last `TYPING_EXPIRY_SECONDS` (eight), as a `stoppedTyping` would. So a lost connection
  stops showing as soon as the server notices it.
- A server that stops altogether says nothing, and its users run out by the expiry.

## Routing

Every API server's event feed subscribes to the subject and routes each word as it routes an event
of the same scope (`event_feed::relayed_events`; see [Event routing](event-routing/index.md)). A
channel's mana level, on `aspen.mana`, is a relay of the same shape routed the same way (see
[Channel mana](channel-mana/index.md)).

| Where       | Delivered to                                                                                                                                  |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| A community | Through the dispatcher's model of it as it stands: members who may view the governing channel, and deployment moderators, who view everything |
| A DM        | Each of its people who reads here                                                                                                             |

In either case it reaches only connections whose `viewing` set holds the channel typed in.

It is delivered as an `ephemeral` frame, which:

- has no sequence;
- is never retained for a catch-up;
- is skipped, rather than dropping the connection, when the connection's queue is full.

The dispatcher hands relays to the shards without waiting (`event_feed::route_relays`): a shard
whose queue is full goes without one, so relays never hold up the stream's events.

## Reference client

The reference client (`AspenSync`, `RecordStore.typers`):

- shows someone typing for `TYPING_EXPIRY_SECONDS` after the last word that they are;
- takes them away at `typing: false`, or when a message of theirs arrives in the channel;
- never shows its own user, or anyone they block on any deployment (`silenced`);
- forgets everyone shown when its connection drops, since nobody can then tell it they stopped.

See `client/docs/architecture/composer-and-drafts.md`.

## Across deployments

Typing works the same on every deployment a user uses.

- Signed in abroad, they are a user there like any other, with an event stream of their own.
- So they type in that deployment's communities, and in the DMs it hosts, as they do at home. DMs
  across deployments live on one host, whose people all have accounts there (see
  [Federation](federation/index.md)).
- Their client tells each deployment over its own stream.
- Whether they tell anyone is the one account preference at home, which every deployment's sync
  reads.
- Nothing passes between deployments for it.

## When access is given or taken away

1. **Who can observe it, and by which routes?** Only the event stream, as `ephemeral` frames: those
   who may view the channel (its community's members as the routing model allows, or a DM's people)
   and have it open, less the typist and those they block. It is never read over REST, sideloaded,
   pushed, searched, or kept by the client past its expiry.
2. **What decides it, and where is that checked?** Sending: `may_type` in the typist's task, on
   every word it publishes, or within two seconds before it. Receiving: the event feed's routing, by
   the model attached as the word is dispatched (`may_read`, as for any event naming
   `Aspen-Channel`), or a DM's people as read when the word was checked.
3. **When the deciding permission is lost, what happens to what is already open?**
   - Losing Send messages, or a block, refuses the typist's next refresh (within three seconds).
     What reached others before runs out by the expiry.
   - A block in a community, or lifting it, reaches the typist's words within two seconds: a word
     within two seconds of the last check goes to whom that check found, its blocks among it.
   - Losing View channel (an override, a role edited, taken, or deleted, a move, a category deleted,
     a removal, a ban, leaving) is followed by the routing model as for every event. The next word
     does not reach them, and their client drops what it showed with the channel or after the
     expiry.
   - A sign-out, a password change, a ban from the deployment, or an account deleted closes the
     typist's streams, which say they stopped.
   - Nothing is cached on the server besides the task's last check of each channel, which is used
     for two seconds at most, to publish or to say a stop.
4. **When it is gained, how does a client already open find out without a reload?** The next
   refresh, within three seconds, reaches them by the model as it then stands.
5. **Does every path that changes it announce it?** Only an event stream connection changes it, and
   its end announces the stop. The changes to access it follows are the events the routing model
   already follows.
6. **Is it published inside the transaction that makes the change?** There is no transaction:
   nothing is stored, and a lost word is said again at the next refresh or runs out.
