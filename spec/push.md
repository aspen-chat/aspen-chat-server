# The Aspen push protocol

**Draft for review.** Nothing here is built yet.

How an Aspen deployment wakes a phone that is not running Aspen, so that it can show a
notification, without the deployment holding anything that could reach a phone that did not ask
it to, and without anything but the phone and the deployment learning what the notification is
about. It has three parties besides Apple and Google:

- **The deployment**: an Aspen API server. It decides who should be notified of what.
- **The relay**: a service holding the credentials of one published build of the Aspen app,
  through which alone that build's phones can be reached: Apple's push service (APNs) for iOS,
  Google's (FCM) for Android builds from the Play Store. The Aspen Foundation runs the relay for
  the app it publishes; anyone who publishes their own build runs their own. A relay is the only
  holder of those credentials because they cannot be shared: an APNs key sends to every user of
  every app of its team, and an FCM service account to every user of its project.
- **The app**: the Aspen app on one phone, signed in to one or more deployments.

A phone on Android without Google's services receives push through **UnifiedPush** instead: a
distributor app of the user's choosing, and its server, stand where the relay does, and no relay
is involved.

## The shape of it

Deployments speak one protocol to every relay and every UnifiedPush server: **Web Push**, the
IETF standard ([RFC 8030](https://www.rfc-editor.org/rfc/rfc8030)), with messages encrypted to
the phone ([RFC 8291](https://www.rfc-editor.org/rfc/rfc8291)) and each deployment signing its
requests with a key of its own ([RFC 8292](https://www.rfc-editor.org/rfc/rfc8292), VAPID).
UnifiedPush requires exactly this, so a deployment need not know which kind of phone it is
waking. (This is the protocol browsers use for their push, but nothing here involves a browser.)

```
 app ──(1) subscribe: device token, deployment's key──▶ relay ──▶ endpoint URL
 app ──(2) endpoint URL, app's public key────────────▶ deployment
 deployment ──(3) POST endpoint: ciphertext, signed───▶ relay ──(4)──▶ APNs / FCM ──▶ phone
 phone: decrypt ──(5) fetch what the pointer names───▶ deployment ──▶ show the notification
```

1. The app asks the relay for a **subscription**: an endpoint URL that reaches this phone, and
   only accepts messages signed by one deployment's key.
2. The app gives the deployment the endpoint and a public key of the phone's.
3. The deployment encrypts a small **pointer** (which account, which channel, which message) to
   the phone's key, signs the request, and POSTs it to the endpoint.
4. The relay forwards the ciphertext through APNs or FCM without being able to read it.
5. On the phone, Aspen decrypts the pointer, fetches the message from the deployment with the
   account's own session, and shows it.

## What each party learns

| Party | Learns | Never learns |
| --- | --- | --- |
| Apple, Google | That a phone was woken, when, and roughly how many bytes. | Anything in the pointer; which deployment. |
| The relay | Which phones subscribe to which deployment keys (and the deployment's `sub` claim, if it gives one), and when each is woken. | The pointer, the account, the channel, any content. |
| The deployment | The endpoint URL and the phone's public key. | The phone's APNs or FCM token, or which other deployments the phone uses. |
| The phone | Everything, from the deployment, over the account's own session. | |

## Keys

**The deployment's push key**: a P-256 key pair (as RFC 8292 requires), made by the first API
server to need it and kept in the database beside the federation key, so every API server signs
with the same one. It is the deployment's identity to relays: subscriptions are bound to it,
and relays keep quotas by it. Deployments without federation have one too. It is advertised in
`GET /api/v1/auth/methods` as `push.applicationServerKey` (the uncompressed point, base64url, as
RFC 8292 writes it). Replacing it ends every subscription made with the old one: apps that see
the advertised key change subscribe again with the new one.

**The phone's keys**: for each subscription, a P-256 key pair and a 16-byte authentication
secret (RFC 8291), made by the app and kept in storage its notification extension can read (on
iOS, a keychain access group shared with the extension). The private key never leaves the
phone.

## The relay's API

Everything under `https://{relay}/v1`. Errors are RFC 9457 Problem Details with a `code`, as the
Aspen API's are, and each `detail` says what to do.

### Devices

`POST /v1/devices` registers one install of the app:

```json
{ "platform": "apns", "app": "org.aspen.chat", "environment": "production", "token": "…" }
```

`platform` is `apns` or `fcm`; `app` is the bundle id (APNs topic) or FCM project, which must be
one the relay holds credentials for; `environment` (APNs only) is `production` or `sandbox`. It
answers `201` with `{ "device": "<id>", "secret": "<bearer secret>" }`. The secret authenticates
every later call about this device, and is known only to the app.

`PATCH /v1/devices/{device}` with `{ "token": "…" }` replaces the token when the platform
issues a new one, keeping every subscription. `DELETE /v1/devices/{device}` ends the device and
all its subscriptions, as signing out of every account on the phone does.

### Subscriptions

`POST /v1/devices/{device}/subscriptions` with `{ "applicationServerKey": "…" }`, the
deployment's push key, answers `201` with:

```json
{ "subscription": "<id>", "endpoint": "https://{relay}/v1/push/<unguessable id>" }
```

A device has one subscription per deployment account it is signed in to. `DELETE
/v1/devices/{device}/subscriptions/{subscription}` ends one.

### Pushing

`POST /v1/push/{id}` is an RFC 8030 push: the body is the RFC 8291 (`aes128gcm`) ciphertext, at
most 4096 bytes, with the headers

- `Content-Encoding: aes128gcm`.
- `Authorization: vapid t=<JWT>, k=<push key>`: the JWT, signed ES256 with the push key the
  subscription was made for, has `aud` the relay's origin, `exp` within 24 hours, and `sub` the
  deployment's contact (`https://{domain}`, or `mailto:`), which is optional on the free tier.
- `TTL`: seconds the message is worth delivering; `0` to drop it if the phone cannot be
  reached now.
- `Urgency`: `high` for what a person should see now (a message for them), `normal` otherwise.
- `Topic`: optional; a later message with the same topic replaces an undelivered earlier one.
  Deployments set it to a hash of the channel id, so a burst in one channel wakes the phone once.

Answers:

| Status | Meaning | The deployment |
| --- | --- | --- |
| `201` | Accepted for delivery. | |
| `400` | Malformed. | Logs it; a bug. |
| `403` `wrongKey` | Not signed by the key this subscription was made for. | Deletes the subscription. |
| `404`, `410` | No such subscription, or the platform says the phone is gone. | Deletes the subscription. |
| `413` | Over 4096 bytes. | Logs it; a bug. |
| `429` `quotaExhausted` / `rateLimited` | Over the key's quota for the period, or too fast for this phone; `Retry-After` says when. | Retries after it; tells its administrators when it is the quota. |

### What reaches the phone

**iOS**: an alert push (`apns-push-type: alert`) with `mutable-content: 1` and a placeholder
the app localizes on the phone (`"loc-key": "notification.placeholder"`, "New activity in
Aspen"), carrying the ciphertext under the key `aspen`. Aspen's notification service extension
decrypts it, fetches, and replaces the placeholder with the real notification, all within the
thirty seconds iOS allows it; if it fails, the placeholder shows, which is why the placeholder
must read sensibly on its own. `Topic` becomes `apns-collapse-id`, `TTL` `apns-expiration`, and
`Urgency: high` priority 10 (otherwise 5).

A pointer that should show nothing (a `read` or `deleted`, below) needs Apple's notification
filtering entitlement, which lets the extension drop a notification; the Foundation must request
it for the published app. Until Apple grants it, deployments send no pointer that shows nothing
on iOS.

**Android (FCM)**: a high-priority data message carrying the ciphertext; the app decrypts,
fetches, and posts the notification itself. Android lowers the priority of apps whose
high-priority messages show nothing, so pointers that show nothing are sent `Urgency: normal`.

**Android (UnifiedPush)**: the distributor hands the app the RFC 8291 ciphertext, and the app
does the same as for FCM.

## The pointer

What the deployment encrypts: a JSON object of at most 1024 bytes, so its ciphertext fits in an
APNs payload once encoded.

```json
{
  "v": 1,
  "origin": "https://chat.example.org",
  "account": "<the user's id on this deployment>",
  "kind": "message",
  "channel": "<channel id>",
  "message": "<message id>",
  "badge": 3
}
```

- `origin`: the deployment's API origin, as the app signed in to it, so the app finds the
  account's session.
- `account`: which account on that deployment, for a phone signed in to several.
- `kind`, and what it needs:
  - `message` (`channel`, `message`): a message the person should be told of. The app fetches
    it (`GET /api/v1/messages/{message}?include=authors,channels`) and shows who wrote it,
    where, and what it says, grouped by channel.
  - `read` (`channel`, `message`): the person read the channel up to `message` on another
    device; the app removes that channel's notifications up to it. It shows nothing.
  - `deleted` (`channel`, `message`): the message is gone; the app removes its notification. It
    shows nothing.
- `badge`: how many unread messages tag the person on this deployment (the sum of their
  `ReadState.mentions`). The app keeps each account's latest and shows their sum.

An app ignores a kind it does not know, and fields it does not know, as the federation
protocol's readers do; kinds and fields are only ever added (`spec/federation.md`, How the
protocol changes). No pointer names a person, a channel name, or any text: those come from the
fetch.

## The deployment's API

- `GET /api/v1/auth/methods` gains `push: { applicationServerKey }`, absent where push is off.
- `POST /api/v1/users/@me/push-subscriptions` with `{ endpoint, p256dh, auth }` registers the
  calling session's phone, answering `201`. A subscription belongs to the session that made it:
  signing out, or the session ending any other way, deletes it, and the deployment tells the
  relay (`DELETE` on the endpoint, signed as a push is). One session holds at most one.
- `DELETE /api/v1/users/@me/push-subscriptions/{subscription}`.

The deployment POSTs only to HTTPS endpoints at public addresses (`app::outbound`), since an
endpoint is a URL a client chose.

## Who is woken, and for what

The deployment decides; the relay and the phone only carry it out. A person is sent a `message`
pointer when a message is posted that they may read and that is either in a DM they are in, or
tags them (by name, a role they hold, or everyone), unless:

- they wrote it, or they blocked its author on this deployment (a block made elsewhere is the
  app's to apply: it drops the notification of anyone it holds blocked);
- the channel is muted for them (a thread's messages count as its parent's);
- they are using Aspen right now on any device (their `active` presence key is set), since that
  device shows it.

Their own notification settings, per community and channel ("all messages", "only tags",
"nothing"), are part of notifications in general, not of this protocol, and narrow this further.
When they read a channel that had woken their phone, the deployment sends `read`; when such a
message is deleted, `deleted`.

## Quotas and abuse

- A subscription reaches only the phone that made it, and accepts only the deployment key it
  was made for, so no deployment can wake a phone that did not sign in to it, and a stolen
  endpoint is useless without that deployment's key.
- Relays rate-limit each subscription (a phone can be woken at most so often, whoever asks) and
  each deployment key, and keep a quota per key per month. Over the free tier, a key gets
  `quotaExhausted` until the month turns or the deployment's operator takes a paid tier, which
  a relay ties to the key and, optionally, to a domain proven through the `sub` claim and the
  federation document.
- A phone's platform token is known only to the app and its relay, never to deployments, so
  nobody can subscribe a phone they do not hold.

## Open questions

1. **Where the relay lives**: a crate in this workspace (sharing `aspen_limits`, metrics, and
   the Problem types), or a repository of its own, since the Foundation runs it and deployments
   never do. Leaning towards its own repository.
2. **The free tier**: how many pushes a month per key, and whether it needs any registration.
   Leaning towards no registration at all, as Web Push has none, with a quota generous enough
   for a community of a few hundred.
3. **Calls**: ringing a phone for a DM call needs Apple's VoIP push and CallKit, with rules of
   their own (every VoIP push must show a call at once). Leaning towards a later kind, `call`,
   once calls ring anywhere.
4. **Proving the app is genuine** to the relay (App Attest, Play Integrity), which would stop a
   modified app from registering devices. Leaning towards not at first: a registered device
   only receives what deployments send it.
5. **The desktop apps** keep their event stream open, and need none of this.
