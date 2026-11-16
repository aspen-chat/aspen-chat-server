# The Aspen push protocol

The deployment's side is `app::push`; the relay is its own repository, `aspen-push-relay`.

How an Aspen deployment wakes a phone that is not running Aspen, so that it can show a notification,
without the deployment holding anything that could reach a phone that did not ask it to, and without
anything but the phone and the deployment learning what the notification is about. It has three
parties besides Apple and Google:

- **The deployment**: an Aspen API server. It decides who should be notified of what.
- **The relay**: a service holding the credentials of one published build of the Aspen app, through
  which alone that build's phones can be reached: Apple's push service (APNs) for iOS, Google's
  (FCM) for Android builds from the Play Store. Bitcrafters LLC runs the relay for the app it
  publishes; anyone who publishes their own build runs their own. A relay is the only holder of
  those credentials because they cannot be shared: an APNs key sends to every user of every app of
  its team, and an FCM service account to every user of its project.
- **The app**: the Aspen app on one phone, signed in to one or more deployments.

A phone on Android without Google's services receives push through **UnifiedPush** instead: a
distributor app of the user's choosing, and its server, stand where the relay does, and no relay is
involved.

## The shape of it

Deployments speak one protocol to every relay and every UnifiedPush server: **Web Push**, the IETF
standard ([RFC 8030](https://www.rfc-editor.org/rfc/rfc8030)), with messages encrypted to the phone
([RFC 8291](https://www.rfc-editor.org/rfc/rfc8291)) and each deployment signing its requests with a
key of its own ([RFC 8292](https://www.rfc-editor.org/rfc/rfc8292), VAPID). UnifiedPush requires
exactly this, so a deployment need not know which kind of phone it is waking. (This is the protocol
browsers use for their push, but nothing here involves a browser.)

```
 app ──(1) subscribe: device token, deployment's key──▶ relay ──▶ endpoint URL
 app ──(2) endpoint URL, app's public key────────────▶ deployment
 deployment ──(3) POST endpoint: ciphertext, signed───▶ relay ──(4)──▶ APNs / FCM ──▶ phone
 phone: decrypt ──(5) fetch what the pointer names───▶ deployment ──▶ show the notification
```

1. The app asks the relay for a **subscription**: an endpoint URL that reaches this phone, and only
   accepts messages signed by one deployment's key.
2. The app gives the deployment the endpoint and a public key of the phone's.
3. The deployment encrypts a small **pointer** (which channel, which message) to the phone's key,
   signs the request, and POSTs it to the endpoint.
4. The relay forwards the ciphertext through APNs or FCM without being able to read it, with the id
   of the subscription it came through, by which the phone knows which account it is for and which
   key decrypts it.
5. On the phone, Aspen decrypts the pointer, fetches the message from the deployment with the
   account's own session, and shows it.

## What each party learns

| Party          | Learns                                                                                                                       | Never learns                                                              |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Apple, Google  | That a phone was woken, when, and roughly how many bytes.                                                                    | Anything in the pointer; which deployment.                                |
| The relay      | Which phones subscribe to which deployment keys (and the deployment's `sub` claim, if it gives one), and when each is woken. | The pointer, the account, the channel, any content.                       |
| The deployment | The endpoint URL and the phone's public key.                                                                                 | The phone's APNs or FCM token, or which other deployments the phone uses. |
| The phone      | Everything, from the deployment, over the account's own session.                                                             |                                                                           |

## Keys

**The deployment's push key**: a P-256 key pair (as RFC 8292 requires), made by the first API server
to need it and kept in the database beside the federation key, so every API server signs with the
same one. It is the deployment's identity to relays: subscriptions are bound to it, and relays keep
quotas by it. Deployments without federation have one too. It is advertised in
`GET /api/v1/auth/methods` as `push.applicationServerKey` (the uncompressed point, base64url, as RFC
8292 writes it). Replacing it ends every subscription made with the old one: apps that see the
advertised key change subscribe again with the new one.

**The phone's keys**: for each subscription, a P-256 key pair and a 16-byte authentication secret
(RFC 8291), made by the app and kept in storage its notification extension can read (on iOS, a
keychain access group shared with the extension). The private key never leaves the phone.

## The relay's API

Everything under `https://{relay}/v1`. Errors are RFC 9457 Problem Details with a `code`, as the
Aspen API's are, and each `detail` says what to do.

### Devices

`POST /v1/devices` registers one install of the app:

```json
{ "platform": "apns", "app": "com.aspen-chat.client", "environment": "production", "token": "…" }
```

`platform` is `apns` or `fcm`; `app` is the bundle id (APNs topic) or FCM project, which must be one
the relay holds credentials for; `environment` (APNs only) is `production` or `sandbox`. An `apns`
device may also give `voipToken`, the token PushKit gave it for VoIP pushes, which is how a call
reaches it (Calls, below); without one, calls are not sent to it. It answers `201` with
`{ "device": "<id>", "secret": "<bearer secret>" }`. The secret authenticates every later call about
this device, and is known only to the app.

`PATCH /v1/devices/{device}` with `{ "token": "…" }`, `{ "voipToken": "…" }`, or both replaces the
tokens the platform issued anew, keeping every subscription; `voipToken` is refused for an `fcm`
device. `DELETE /v1/devices/{device}` ends the device and all its subscriptions, as signing out of
every account on the phone does.

### Subscriptions

`POST /v1/devices/{device}/subscriptions` with `{ "applicationServerKey": "…" }`, the deployment's
push key, answers `201` with:

```json
{ "subscription": "<id>", "endpoint": "https://{relay}/v1/push/<unguessable id>" }
```

A device has one subscription per deployment account it is signed in to.
`DELETE /v1/devices/{device}/subscriptions/{subscription}` ends one.

### Pushing

`POST /v1/push/{id}` is an RFC 8030 push: the body is the RFC 8291 (`aes128gcm`) ciphertext, at most
2048 bytes (what fits an APNs payload once encoded; UnifiedPush servers take 4096), with the headers

- `Content-Encoding: aes128gcm`.
- `Authorization: vapid t=<JWT>, k=<push key>`: the JWT, signed ES256 with the push key the
  subscription was made for, has `aud` the relay's origin, `exp` within 24 hours, and `sub` the
  deployment's contact (`https://{domain}`, or `mailto:`), which is optional on the free tier.
- `TTL`: seconds the message is worth delivering; `0` to drop it if the phone cannot be reached now.
- `Urgency`: `high` for what a person should see now (a message for them); `low` for a pointer that
  shows nothing (`read`, `deleted`).
- `Topic`: optional; a later message with the same topic replaces an undelivered earlier one.
  Deployments set it to the channel id, so a burst in one channel wakes the phone once, and a call's
  to its session id, so a ring and a message never replace each other.
- `Aspen-Push-Kind: call`: the push is a call ringing the phone (Calls, below), which the relay must
  send as the platform's call push and cannot read from the ciphertext. Sent on a `call` pointer
  alone; no other kind is named to the relay. A relay that does not know the header sends the push
  as any other, and the phone shows the ring as a notification at most.

Answers:

| Status                                 | Meaning                                                                                   | The deployment                                                   |
| -------------------------------------- | ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `201`                                  | Accepted for delivery.                                                                    |                                                                  |
| `400`                                  | Malformed.                                                                                | Logs it; a bug.                                                  |
| `403` `wrongKey`                       | Not signed by the key this subscription was made for.                                     | Deletes the subscription.                                        |
| `404`, `410`                           | No such subscription, or the platform says the phone is gone.                             | Deletes the subscription.                                        |
| `413`                                  | Over 2048 bytes.                                                                          | Logs it; a bug.                                                  |
| `429` `quotaExhausted` / `rateLimited` | Over the key's quota for the period, or too fast for this phone; `Retry-After` says when. | Retries after it; tells its administrators when it is the quota. |

### What reaches the phone

**iOS**: an alert push (`apns-push-type: alert`) with `mutable-content: 1` and a placeholder the app
localizes on the phone (`"loc-key": "notification.placeholder"`, "New activity in Aspen"), carrying
`"aspen": { "s": "<subscription id>", "c": "<ciphertext, base64url>" }`. Aspen's notification
service extension decrypts it, fetches, and replaces the placeholder with the real notification, all
within the thirty seconds iOS allows it; if it fails, the placeholder shows, which is why the
placeholder must read sensibly on its own. `Topic` becomes `apns-collapse-id`, `TTL`
`apns-expiration`, and `Urgency: high` priority 10 (otherwise 5).

A pointer that should show nothing (a `read` or `deleted`, below) needs Apple's notification
filtering entitlement, which lets the extension drop a notification; Bitcrafters must request it for
the published app. A relay whose iOS app lacks it answers `201` to a `low` push for an iOS phone and
delivers nothing, so deployments need not know which phones are which.

**Android (FCM)**: a data message carrying the same `s` and `c`, high priority for `Urgency: high`
and normal otherwise; the app decrypts, fetches, and posts the notification itself. Android lowers
the priority of apps whose high-priority messages show nothing, which is why pointers that show
nothing are `low`.

**Android (UnifiedPush)**: the distributor hands the app the RFC 8291 ciphertext, and the app does
the same as for FCM.

### Calls

A push marked `Aspen-Push-Kind: call` rings the phone, so it must wake the app at once and let it
show a call rather than a notification:

**iOS**: a VoIP push (`apns-push-type: voip`, topic `<bundle id>.voip`, priority 10,
`apns-expiration` from `TTL`, which a deployment sets to what is left of the ring) to the device's
`voipToken`, carrying `"aspen": { "s": …, "c": … }` and no `aps`. PushKit wakes the app, whose
handler (not the extension) decrypts and reports the call to CallKit before it returns, as Apple
requires. A device that registered no `voipToken` is answered `201` and sent nothing: an app without
one is of a version that could not show the call. A `voipToken` Apple says is gone is forgotten, and
the device kept, since its alert token may still stand. Nothing is pushed when a ring ends, which
Apple says not to do: the app learns of it through its own connection, and at the ring's `until` by
the clock.

**Android (FCM)**: a data message of high priority whatever `Urgency` says, with `"k": "call"`
beside `s` and `c`, so the app's messaging service takes the call path before it has decrypted
anything; the pointer's `kind` is what decides, and `k` only hurries it. The app adds the call to
Telecom and posts its call notification, with a full-screen intent, within the five seconds a
service started by a push has.

**Android (UnifiedPush)**: the ciphertext as for any push; the app decides by the pointer's `kind`.

## The pointer

What the deployment encrypts: a JSON object of at most 1024 bytes, so its ciphertext fits in an APNs
payload once encoded.

```json
{
  "v": 1,
  "kind": "message",
  "channel": "<channel id>",
  "message": "<message id>",
  "badge": 3
}
```

The pointer names no deployment or account: the app knows both from the subscription the push came
through (the relay's `s`, or on UnifiedPush the registration it arrived for), and keeps each
subscription's keys beside the session it registered them with.

- `kind`, and what it needs:
  - `message` (`channel`, `message`): a message the person should be told of. The app fetches it
    (`GET /api/v1/messages/{message}?include=authors,channels,mentions,memberships,notificationTexts`)
    and shows who wrote it (by the nickname their membership carries, in a community), where, and
    what it says (The notification's text, below), grouped by channel.
  - `read` (`channel`, `message`): the person read the channel up to `message` on another device;
    the app removes that channel's notifications up to it. It shows nothing.
  - `deleted` (`channel`, `message`): the message is gone; the app removes its notification. It
    shows nothing.
  - `notice` (`channel`, `notice`): one of the deployment's plugins tells the person of something in
    the channel (`spec/plugins.md`, Notices). The app fetches it
    (`GET /api/v1/users/@me/plugin-notices/{notice}`, which answers the plugin's name as `title`,
    its text as `body`, both in the person's language, and the channel's `community` and
    `parentChannel`, and the `message` it is about, if any) and shows the title and body.
  - `call` (`channel`, `session`, `caller`, `callerName`, `until`): the call of `session`, in the DM
    `channel`, started by `caller`, is ringing the person until `until` (RFC 3339, on the
    deployment's clock). The app shows an incoming call from `callerName` at once, before it reaches
    the network (The app, below); it fetches nothing to show it. The deployment is the one the
    subscription names, as for every pointer.
- `badge`: how many unread messages tag the person on this deployment (the sum of their
  `ReadState.mentions`), and how many of their DMs are unread; absent when a message wakes so many
  people that working it out for each would cost too much. The app keeps each account's latest and
  shows their sum.

An app ignores a kind it does not know, and fields it does not know, as the federation protocol's
readers do; kinds and fields are only ever added (`spec/federation.md`, How the protocol changes).
No pointer but `call` names a person, a channel name, or any text: those come from the fetch. A
`call` names its caller, since a phone must show who is calling before it can reach the deployment,
and that name alone (the caller's display name, or else their username; the channel's name, if a
group DM has one, is not carried).

## The deployment's API

- `GET /api/v1/auth/methods` gains `push: { applicationServerKey }`, absent where push is off.
- `POST /api/v1/users/@me/push-subscriptions` with `{ endpoint, p256dh, auth }` registers the
  calling sign-in's phone, answering `201`. A subscription belongs to the sign-in that made it (its
  refresh token): signing out, or the sign-in ending any other way, deletes it. One sign-in holds at
  most one; registering again replaces it. An account holds at most ten: a registration beyond them
  deletes the oldest, and those of sign-ins that have expired. The app ends its relay subscription
  itself when it signs out; one whose sign-in ended some other way is simply never pushed to again.
- `DELETE /api/v1/users/@me/push-subscriptions/{subscription}`.

The deployment POSTs only to HTTPS endpoints at public addresses (`app::outbound`, as for calls to
other deployments), since an endpoint is a URL a client chose: a name must resolve to one, and an
endpoint naming an address inside a network is refused when registered and dropped before a push. It
drops a subscription whose endpoint answers `403`, `404`, or `410`, and one made for a push key
since replaced. A push service has three seconds to answer a push; one that leaves a subscription's
pushes unanswered five times in a row has that subscription skipped for an hour.

## The app

The app registers the phone with its relay, and each account with its deployment, whenever it
starts, gets a new platform token, or signs in or out anywhere (`syncPush` in
`client/packages/protocol/src/push.ts`). A subscription is made again when the deployment's key or
the sign-in changes; one no longer needed is ended at the relay.

What the notification code needs while the app is closed is one JSON object, `PushState`, which the
app keeps through its native plugin `AspenPush` (`saveState`, `loadState`) where that code can read
it: on iOS in a keychain access group shared with the notification service extension, on Android in
the app's private storage.

```json
{
  "version": 1,
  "device": { "id": "…", "secret": "…", "token": "…" },
  "accounts": [
    {
      "subscription": "<the relay's subscription id, a push's s>",
      "origin": "https://chat.example.org",
      "userId": "…",
      "refreshToken": "…",
      "sessionToken": "…",
      "keys": {
        "privateKey": { "kty": "EC", "crv": "P-256", "x": "…", "y": "…", "d": "…" },
        "publicKey": "…",
        "auth": "…"
      },
      "applicationServerKey": "…",
      "deploymentSubscription": "…"
    }
  ]
}
```

For a push, the notification code:

1. finds the account whose `subscription` is the push's `s`, and decrypts `c` with its keys;
2. for `message`, fetches
   `GET {origin}/api/v1/messages/{message}?include=authors,channels,mentions,memberships,notificationTexts`
   with `sessionToken` (on `401`, gets another with `POST /api/v1/auth/token-refresh` and
   `refreshToken`, and keeps it), and shows who wrote it (by their nickname in a community), where,
   and what it says (The notification's text, below), grouped by channel, with the message id as the
   notification's identifier;
3. for `read`, removes that channel's notifications up to `message`; for `deleted`, that one; either
   shows nothing; for `notice`, fetches it as the message is fetched and shows its title and body,
   with the notice id as the notification's identifier;
4. sets the badge to the sum of every account's latest `badge`;
5. for `call`, shows an incoming call from `callerName` through the platform's call system (CallKit,
   Telecom) at once, fetching nothing, and then watches the ring (Rings, below).

### Rings

A `call` pointer reaches the phone with no one on the line yet: the ring must end on this phone when
it ends anywhere, and answering must join the call. The app, having shown the call:

1. Opens the deployment's event stream with the account's session (`GET /api/v1/events`, identifying
   as any client does; on `401`, with a token renewed from the refresh token) and watches it for the
   ring's end, resuming the stream from the last sequence it saw if it drops, until the ring ends:
   - a `voiceParticipant` `create` in `channel` for the account's own user: answered on another
     device;
   - a `voiceRing` `delete` for `session` and the user: declined elsewhere (or answered, when the
     participant's `create` was not seen first);
   - a `voiceSession` `delete` for `session`, or `voiceSessionEnded` naming it: the call ended;
   - `until`, judged against the deployment's clock from `ready`'s `serverTime`: unanswered. Nothing
     is pushed to say any of this.
2. On **answer**, joins the call as the account (`POST /api/v1/channels/{channel}/voice/join`, then
   the voice server), which ends the ring at the deployment. On **decline**, ends the ring
   (`DELETE /api/v1/channels/{channel}/voice/rings/@me`), so the person's other devices stop too.
3. If the stream, resumed, answers `resumed: false`, reads `GET /api/v1/channels/{channel}/voice`
   once: a call that is gone, or another session's, ended the ring; one the user is in was answered
   elsewhere; otherwise the ring stands and the watch goes on.

On iOS 26.4 and later PushKit says with each VoIP push whether the app must report a call for it
(`PKVoIPPushMetadata.mustReport`): it is `false` when the app is in the foreground, already in a
call, or the push was delayed too long to matter, and the app then completes the push and reports
nothing. On earlier versions every VoIP push is reported. A VoIP push the app cannot read (no
account for its subscription, a ciphertext that does not decrypt) is reported as a call and ended at
once with `failed`, since an app that reports nothing for a VoIP push is terminated.

The ringtone is the platform's own default on both phones: Aspen has no ringtone file of its own
yet.

### The notification's text

The deployment writes out what a message's notification says, so the notification code interprets
nothing of a message's text. Whatever shows a notification with nothing but the message to go on
reads it: the phones' notification code here, and Aspen's web and desktop client for the system's
notification of a message that arrives while it is open
(`GET /api/v1/messages/{message}?include=notificationTexts`), which words the message itself only
when the read fails or is slow. `include=notificationTexts` on the fetch answers
`included.notificationTexts`, a list of `{ "message": "<message id>", "text": "…" }` with one entry
per message of the read. `text` is plain text, shown as given, in the language the fetch asks for
with `Accept-Language`:

- for a message with text of its own, that text with each person and role it tags written as
  `@name`, each emoji of its community (`<:id>`) as `:name:`, and `@…` or `:…:` where the message
  names something its reader could not be shown: a person or role it does not tag, an emoji of
  another community or since deleted. Code (a code span or block) is left as written, and so is the
  text's Markdown. A spoiler (`||text||` or `>!text!<`, where Aspen's client would draw one) is
  written as a word for one, "[spoiler]" in English, in place of its markers and everything between
  them, code included, so a notification never shows what the message hides. Text that is nothing
  but spoilers is still text;
- for a message with no text, what its kind records: a poll, a poll's closing, a join, or a
  community become large (with what its `growth` changed);
- for a message with no text whose kind records nothing in words, what it holds: "Sent an image" for
  one picture, "Sent images" for several, "Sent an attachment" for files that are not all pictures
  (by the type each was uploaded as), and "New message" for none.

The notification code writes the text out in words of its own when `included` has no
`notificationTexts`, which is how a deployment of a version without it answers, an `include` name it
does not know being ignored: tags of the people in `included.users` as `@name`, any other tag as
`@…`, and by its `kind` a message with no text. Text that holds a spoiler's marker anywhere (`||`,
`>!`, `!<`, or `\|\|`, a table cell's spelling of the first) it does not show at all, and words the
message as one with no text. Which markers make a spoiler is the client's Markdown to say
(`spec/spoiler_vectors.json` holds texts the client and the deployment read alike) and the
notification code reads no Markdown, so it shows nothing a spoiler hides at the cost of hiding some
texts that only look as if they held one, a table or a shell pipeline among them.

A notification it posts carries `{ origin, userId, channel, message, community, parentChannel }`,
the channel's community (`null` in a DM) and, for a message in a thread, the channel the thread is
in, so that tapping it opens the message.

`AspenPush.describe()` says what the build is: `platform` (`apns` or `fcm`), `app` (the bundle id or
Firebase project), `environment`, and `relay`, the URL of the relay of whoever published the build,
which alone can wake it.

## Who is woken, and for what

The deployment decides; the relay and the phone only carry it out. A person is sent a `message`
pointer when a message is posted that they may read and that their notification settings ask for,
per community, category, and channel ("all messages", "only tags", "nothing"; without any, every
message of a DM or of a small community, and in a large community those that tag them by name, a
role they hold, or everyone), unless:

- they wrote it, or they blocked its author on this deployment (a block made elsewhere is the app's
  to apply: it drops the notification of anyone it holds blocked);
- the channel is muted for them, by its own mute or its category's (a thread's messages count as its
  parent's);
- they are in front of Aspen right now on any device: one of their event stream connections has said
  `attending` within the last minute and not `stoppedAttending` since, since that device shows it.
- they are in do not disturb on this deployment, which also gives the `read` pointers sent them
  meanwhile a `badge` of 0.

A person is sent a `call` pointer when a DM's call starts ringing them (`app::voice::ring`), to
every phone they have, whether or not they are using Aspen elsewhere, as a phone call rings every
handset. Who is rung is decided before the ring exists: not those in do not disturb, and no one a
block stands between. Nothing is sent when the ring ends.

Which messages those settings ask for is part of notifications in general, not of this protocol. A
message with no text of its own (a poll's result, a join, a community becoming large) is described
by its `kind`, and a `communityGrew` message by its `growth` (The notification's text). When they
read a channel that had woken their phone, the deployment sends `read`; when such a message is
deleted, `deleted`. When they mark a channel unread, it sends `read` at the position the channel
moved back to, which takes down nothing still shown but carries the new `badge`.

## Quotas and abuse

- A subscription reaches only the phone that made it, and accepts only the deployment key it was
  made for, so no deployment can wake a phone that did not sign in to it, and a stolen endpoint is
  useless without that deployment's key.
- Relays rate-limit each subscription (a phone can be woken at most so often, whoever asks) and each
  deployment key, and keep a quota per key per month. Over the free tier, a key gets
  `quotaExhausted` until the month turns or the deployment's operator takes a paid tier, which a
  relay ties to the key and, optionally, to a domain proven through the `sub` claim and the
  federation document.
- A phone's platform token is known only to the app and its relay, never to deployments, so nobody
  can subscribe a phone they do not hold.

## Decisions

1. **The relay** is its own repository (`aspen-push-relay`), run by Bitcrafters LLC for the apps it
   publishes; deployments never run one.
2. **The free tier** needs no registration: each deployment key may send a fixed number of pushes a
   month (200,000 at Bitcrafters' relay, which costs it little more than bandwidth), and a relay's
   operators give a key a tier of its own when it needs more.
3. **Calls** ring phones with the `call` kind, through Apple's VoIP push and CallKit on iOS and FCM
   and Telecom on Android, which have rules of their own (Calls, Rings): the relay is told that a
   push is a call and nothing else, a ring's end is never pushed, and a push that cannot be read is
   still reported as a call.
4. **The relay trusts any app that registers**, without App Attest or Play Integrity: a registered
   device only receives what deployments it subscribed to send it.
5. **The desktop apps** keep their event stream open, and need none of this.
