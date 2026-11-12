# Push

Phones not running Aspen are woken through their app's push relay or a UnifiedPush distributor, as
`spec/push.md` describes. The relay, which alone holds a published app's Apple and Google
credentials, is its own repository (`aspen-push-relay`). Design notes:
[push-design-notes.md](push-design-notes.md).

## Where it lives

| Part                   | Where                                                               |
| ---------------------- | ------------------------------------------------------------------- |
| What a push says       | `app::push::Pointer`                                                |
| Encryption and signing | `app::push::webpush` (the `aspen_webpush` crate)                    |
| HTTP client            | `app::push::client`                                                 |
| Dispatcher             | `app::push::spawn_dispatcher`, one per API server                   |
| Who to wake            | `RECIPIENTS_SQL`, `visibility::viewers`                             |
| Send queue             | `app::push::queue`                                                  |
| Who has phones         | `phones_of`                                                         |
| Badge count            | `push::badge_of`, kept for `BADGE_KEPT`                             |
| Address check          | `reaches_inside`                                                    |
| Table of subscriptions | `push_subscription`                                                 |
| Table of the push key  | `push_key`                                                          |
| Metrics                | `aspen_pushes_total` (by outcome), `aspen_pushes_waiting` (queued)  |
| A notification's text  | `app::message::notification`, read with `include=notificationTexts` |
| Development check      | `scripts/dev_push.py`, standing in for relay and phone              |

## What a push is

Every push is Web Push.

- The payload is a pointer (`app::push::Pointer`): `message`, `read`, or `deleted`, naming a channel
  and a message, and a `badge` count.
- It is encrypted to the phone (RFC 8291, `app::push::webpush`, the `aspen_webpush` crate, checked
  against RFC 8291's own example).
- It is signed with the deployment's push key (RFC 8292).
- The badge counts tags only in channels the person may view (see [The badge](#the-badge)).

### The push key

- A P-256 key in `push_key`, made by the first server to start, like the federation key.
- Advertised as `push.applicationServerKey` in `GET /auth/methods`.
- Each audience's token is reused until half its life is left.
- At most 1024 tokens are kept (`webpush::MAX_TOKENS`), letting go of those nearest expiry.

### The badge

- It counts tags only in channels the person may view.
- Once counted (`push::badge_of`, a few queries), it is kept in Valkey for five minutes
  (`BADGE_KEPT`).
- While kept, it grows by one for each new message tagging the person in a community.
- It is forgotten when the person reads, when they mark a channel unread, when a DM of theirs
  speaks, or when a message that woke them is deleted.
- A change no push follows (a role or a channel taken from them) shows on the badge within those
  five minutes.
- A `read` push to someone in do not disturb carries 0 (`shown_badge`, which reads their presence
  override's Valkey copy). Pushes that tell of something new never reach them then.

## What a notification says

A pointer carries no text. The phone fetches the message, and with `include=notificationTexts` the
read answers what its notification says, written out (`app::message::notification`,
`included.notificationTexts`), so the phones' notification code parses nothing. The web and desktop
client reads the same for the system notification of a message that arrives while it is open
(`AspenSync.notificationText`), so every notification of a message reads alike.

- A message's text has its tokens (`app::message::tokens`) written as names outside code: `@name`
  for a person or role, `:name:` for an emoji. Its Markdown stays as written.
- A spoiler is written as a word for one (`notificationSpoiler`, in the request's language) in place
  of its markers and all it holds, code and tags included, and nothing in it is looked up. Text that
  is all spoilers is still the message's text, so it never reads "New message". An attachment's
  spoiler mark changes nothing here: a message of files alone says what kind they are and shows none
  of them.
- A spoiler is whatever the client draws as one. `notification::spoilers` reads pulldown-cmark's
  events into the tree the client's renderer has and looks through it as `remarkSpoilers.ts` does
  (the client's
  [Markdown](../../client/docs/architecture/message-rendering/markdown.md#what-makes-a-spoiler) has
  the grammar in full):
  - markers pair within one parent's text, across markup and code but never from inside code;
  - a marker is two characters written as themselves: one written with a backslash (`\|`) or as a
    character reference (`&#124;`) is text, but for a table cell's `\|`, the only way a cell writes
    a pipe, so `\|\|text\|\|` is a cell's spoiler. pulldown-cmark starts a new text after each
    escape's backslash and gives a reference as text that differs from its source, which is how
    `Builder::text` tells them;
  - openers are tried from left to right, each closing at the nearest closer after it (`opening`).
    After markers holding only blank text (`|| ||`) the search goes on from the end of the closer,
    and after markers holding nothing (`||||`) from the second, which may open; an opener nothing
    closes in its own text closes in a later text of the same parent, or not at all;
  - a named link, a picture, and a definition are the text they were written as;
  - a quote starting `!` and holding `!<`, each written as itself, is a `>!` spoiler;
  - a message nested deeper than the client draws (`MAX_NESTING`) is read with no Markdown, or, when
    its lines open too many quotes and lists, with no spoilers.

  `spec/spoiler_vectors.json` holds texts both read alike, which the tests of each run; change the
  two together.

- Where pulldown-cmark and the client's parser build different trees the two can differ. What
  pulldown-cmark reports nothing of is not read: a definition of a label already defined, and a
  table cell past its table's width, both of which the client shows. A table too large for the
  client to draw is code to it and a table here. In each the server may show a spoiler as written; a
  spoiler is a courtesy of its author's, not a secret, and the message itself is readable by whoever
  is told.
- The email digest's excerpts are written out by the same function (`notification::written_out`), so
  they hide the same spoilers (see [the daily digest](email/digest.md#what-a-digest-holds)).
- A person or role is named only when the message tags it (`Message.mentions`, what its author was
  allowed to tag) or is its author, and an emoji only when it is live and of the channel's own
  community. Anything else reads `@…` or `:…:`, so a notification names nothing its reader could not
  otherwise see.
- A message with no text says what its kind records (`recorded`), in the request's language. For a
  kind that records nothing in words it says what the message holds (`held`): one picture, several,
  files that are not all pictures, or nothing. A file is a picture by the type it was uploaded as
  (`attachment::pictured`), read for the whole read in one query.
- The names of a whole read are loaded in one query each for people, roles, and emoji
  (`read_notification_texts`), however many messages it holds.

A phone reading a deployment of a version without the include writes the text out itself, as
`spec/push.md` says.

## Subscriptions

`POST /users/@me/push-subscriptions` registers the calling sign-in's phone.

- One `push_subscription` per refresh token, which cascades, so signing out stops it.
- At most ten per account (`MAX_SUBSCRIPTIONS_PER_USER`).
- Registrations are made one at a time per account, under a lock on its `user` row. Each lets go of
  those of expired sign-ins and of the oldest beyond ten.
- Only phones of live sign-ins, of accounts neither deleted nor banned from the deployment, are
  woken (`phones_of`). So a sign-in revoked by a password change or a ban stops its phone too.

### When a subscription is dropped

- Its endpoint answers `403`, `404`, or `410`.
- It was made for a key since replaced.
- Its endpoint names an address that `reaches_inside` refuses (checked before a push).

## Endpoints and addresses

Pushes go to HTTPS endpoints at public addresses only, through a client of their own
(`app::push::client`), built like the one for calls to other deployments.

- It resolves names to public addresses alone, whatever `[federation.development]` says.
- The exception is loopback addresses when `[federation.development]` allows private ones, which
  only a deployment at `localhost` may give (for `scripts/dev_push.py`).
- An endpoint that names an address rather than a host is checked itself (`reaches_inside`): refused
  when registered, and its subscription dropped before a push.
- Only the first 4 KiB of a refusal is read.

## Deciding who to wake

One task per API server (`app::push::spawn_dispatcher`) reads the event stream through the durable
JetStream consumer `aspen_push`, which they share, so each event is handled once. Each DM copy is
deduplicated in Valkey.

A new message wakes, as far as each one's level for its channel asks (see
[Notifications](notifications.md)):

- the other people of its DM; or
- the members of a small community, whose channels tell of every message by default; or
- those whose setting asks for every message; or
- those it tags (by name, a role, or everyone); or
- those who follow the thread it is a reply in;

provided they may view its channel. An echo, a call's record, and a bot command wake no one. It
never wakes:

- its author;
- anyone who blocked the author here;
- anyone who muted the channel, or its category without keeping the channel unmuted (a thread
  counting as its parent; `app::channel_mute::muted_sql!`);
- anyone in do not disturb (see [Presence](event-routing/presence.md#choosing-a-status));
- anyone active on Aspen at that moment.

### Finding them

1. One query (`RECIPIENTS_SQL`) finds the candidates from indexed branches:
   - the members tagged by name;
   - the holders of tagged roles;
   - every member, when everyone is tagged, or when the default level asks for every message (a
     small community's, bounded by the threshold; see
     [Large communities](roles-and-permissions/large-communities.md));
   - those whose setting for the channel, its category, or its community asks for every message;
   - a DM's people;
   - the thread's followers, and a DM's thread's only while they are still its people.
2. The same query resolves each one's level from the channel's setting, then its category's, then
   the community's, then the default. A follower is told whatever theirs.
3. It leaves out those who blocked the author, muted the channel or its category, are in do not
   disturb, or have no phone.
4. Who may view the channel is decided once for the candidates (`visibility::viewers`).
5. Who is active is asked of Valkey, `ACTIVE_BATCH` (1000) at a time.

Each is sent `message` with `Urgency: high`. Up to `FAN_OUT` people are prepared at once, their
phones read in one query.

### Failure and redelivery

Handling fails only before it wakes anyone. A failure lets go of its Valkey claim, and the event is
delivered again, up to three times in all.

### Waiting for the commit

Events are published before their transaction commits (see [Event routing](event-routing/index.md)).

1. The dispatcher waits for the message, or the read position, to be there before it reads anything
   else.
2. It looks again after 50 ms, then at doubling waits of up to a second.
3. It gives up after ten seconds (`COMMIT_WAIT`). A message not there by then was rolled back.

A message deleted meanwhile wakes no one.

## Sending

The dispatcher never waits for a push service. Once it has decided an event's pushes, it queues them
(`app::push::queue`) and acknowledges the event.

| Limit                                | Value                                                                      |
| ------------------------------------ | -------------------------------------------------------------------------- |
| Concurrent sends per endpoint origin | 256 slots                                                                  |
| Concurrent sends per server          | 1024 slots                                                                 |
| Time to answer                       | Three seconds (`SEND_TIMEOUT`)                                             |
| Pushes waiting on one server         | 200,000; beyond that, queueing holds up the dispatcher until there is room |
| Pushes waiting for one origin        | 50,000; beyond that, that origin's pushes are dropped                      |

Each push is a task of its own. It waits for a slot of its endpoint's origin, then one of the
server's, so a slow or unreachable push service delays only its own pushes.

### Suspended subscriptions

- A subscription whose push service has gone unanswered (no answer in time, or no connection) five
  times in a row is suspended.
- It is skipped until an hour has passed since the last failure.
- The count is in Valkey (`push:failing:<subscription>`), so every server skips it.
- Any answer clears the count.

## `read` and `deleted`

- Whom a message woke is remembered in Valkey for a week.
- Reading that channel elsewhere sends `read`, and deleting the message sends `deleted`, both
  `Urgency: low`.
- Marking a channel unread (`channelMarkedUnread`) sends `read` at the position it moved back to,
  once that is committed, with the badge counted afresh. It takes down nothing the position had not
  already passed. A channel still read once marked sends nothing.
- Relays whose iOS app cannot hide a notification leave these undelivered.
