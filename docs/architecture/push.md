# Push

Phones not running Aspen are woken through their app's push relay or a UnifiedPush distributor, as `spec/push.md` describes. The relay, which alone holds a published app's Apple and Google credentials, is its own repository (`aspen-push-relay`). Design notes: [push-design-notes.md](push-design-notes.md).

## Where it lives

| Part | Where |
| --- | --- |
| What a push says | `app::push::Pointer` |
| Encryption and signing | `app::push::webpush` (the `aspen_webpush` crate) |
| HTTP client | `app::push::client` |
| Dispatcher | `app::push::spawn_dispatcher`, one per API server |
| Who to wake | `RECIPIENTS_SQL`, `visibility::viewers` |
| Send queue | `app::push::queue` |
| Who has phones | `phones_of` |
| Badge count | `push::badge_of`, kept for `BADGE_KEPT` |
| Address check | `reaches_inside` |
| Table of subscriptions | `push_subscription` |
| Table of the push key | `push_key` |
| Metrics | `aspen_pushes_total` (by outcome), `aspen_pushes_waiting` (queued) |
| Development check | `scripts/dev_push.py`, standing in for relay and phone |

## What a push is

Every push is Web Push.

- The payload is a pointer (`app::push::Pointer`): `message`, `read`, or `deleted`, naming a channel and a message, and a `badge` count.
- It is encrypted to the phone (RFC 8291, `app::push::webpush`, the `aspen_webpush` crate, checked against RFC 8291's own example).
- It is signed with the deployment's push key (RFC 8292).
- The badge counts tags only in channels the person may view (see [The badge](#the-badge)).

### The push key

- A P-256 key in `push_key`, made by the first server to start, like the federation key.
- Advertised as `push.applicationServerKey` in `GET /auth/methods`.
- Each audience's token is reused until half its life is left.
- At most 1024 tokens are kept (`webpush::MAX_TOKENS`), letting go of those nearest expiry.

### The badge

- It counts tags only in channels the person may view.
- Once counted (`push::badge_of`, a few queries), it is kept in Valkey for five minutes (`BADGE_KEPT`).
- While kept, it grows by one for each new message tagging the person in a community.
- It is forgotten when the person reads, when a DM of theirs speaks, or when a message that woke them is deleted.
- A change no push follows (a role or a channel taken from them) shows on the badge within those five minutes.

## Subscriptions

`POST /users/@me/push-subscriptions` registers the calling sign-in's phone.

- One `push_subscription` per refresh token, which cascades, so signing out stops it.
- At most ten per account (`MAX_SUBSCRIPTIONS_PER_USER`).
- Registrations are made one at a time per account, under a lock on its `user` row. Each lets go of those of expired sign-ins and of the oldest beyond ten.
- Only phones of live sign-ins, of accounts neither deleted nor banned from the deployment, are woken (`phones_of`). So a sign-in revoked by a password change or a ban stops its phone too.

### When a subscription is dropped

- Its endpoint answers `403`, `404`, or `410`.
- It was made for a key since replaced.
- Its endpoint names an address that `reaches_inside` refuses (checked before a push).

## Endpoints and addresses

Pushes go to HTTPS endpoints at public addresses only, through a client of their own (`app::push::client`), built like the one for calls to other deployments.

- It resolves names to public addresses alone, whatever `[federation.development]` says.
- The exception is loopback addresses when `[federation.development]` allows private ones, which only a deployment at `localhost` may give (for `scripts/dev_push.py`).
- An endpoint that names an address rather than a host is checked itself (`reaches_inside`): refused when registered, and its subscription dropped before a push.
- Only the first 4 KiB of a refusal is read.

## Deciding who to wake

One task per API server (`app::push::spawn_dispatcher`) reads the event stream through the durable JetStream consumer `aspen_push`, which they share, so each event is handled once. Each DM copy is deduplicated in Valkey.

A new message wakes:

- the other people of its DM; or
- those it tags (by name, a role, or everyone); or
- those who follow the thread it is a reply in;

provided they may view its channel. It never wakes:

- its author;
- anyone who blocked the author here;
- anyone who muted the channel (a thread counting as its parent);
- anyone active on Aspen at that moment.

### Finding them

1. One query (`RECIPIENTS_SQL`) finds the candidates from indexed branches:
   - the members tagged by name;
   - the holders of tagged roles;
   - every member, when everyone is tagged;
   - those whose setting asks for every message;
   - a DM's people;
   - the thread's followers, and a DM's thread's only while they are still its people.
2. The same query resolves each one's level from the channel's setting, then the community's, then the default. A follower is told whatever theirs.
3. It leaves out those who blocked the author, muted the channel, or have no phone.
4. Who may view the channel is decided once for the candidates (`visibility::viewers`).
5. Who is active is asked of Valkey, `ACTIVE_BATCH` (1000) at a time.

Each is sent `message` with `Urgency: high`. Up to `FAN_OUT` people are prepared at once, their phones read in one query.

### Failure and redelivery

Handling fails only before it wakes anyone. A failure lets go of its Valkey claim, and the event is delivered again, up to three times in all.

### Waiting for the commit

Events are published before their transaction commits (see [Event routing](event-routing/index.md)).

1. The dispatcher waits for the message, or the read position, to be there before it reads anything else.
2. It looks again after 50 ms, then at doubling waits of up to a second.
3. It gives up after ten seconds (`COMMIT_WAIT`). A message not there by then was rolled back.

A message deleted meanwhile wakes no one.

## Sending

The dispatcher never waits for a push service. Once it has decided an event's pushes, it queues them (`app::push::queue`) and acknowledges the event.

| Limit | Value |
| --- | --- |
| Concurrent sends per endpoint origin | 256 slots |
| Concurrent sends per server | 1024 slots |
| Time to answer | Three seconds (`SEND_TIMEOUT`) |
| Pushes waiting on one server | 200,000; beyond that, queueing holds up the dispatcher until there is room |
| Pushes waiting for one origin | 50,000; beyond that, that origin's pushes are dropped |

Each push is a task of its own. It waits for a slot of its endpoint's origin, then one of the server's, so a slow or unreachable push service delays only its own pushes.

### Suspended subscriptions

- A subscription whose push service has gone unanswered (no answer in time, or no connection) five times in a row is suspended.
- It is skipped until an hour has passed since the last failure.
- The count is in Valkey (`push:failing:<subscription>`), so every server skips it.
- Any answer clears the count.

## `read` and `deleted`

- Whom a message woke is remembered in Valkey for a week.
- Reading that channel elsewhere sends `read`, and deleting the message sends `deleted`, both `Urgency: low`.
- Relays whose iOS app cannot hide a notification leave these undelivered.
