# Presence

The client reads presence for the users it shows, and its event stream tells it of changes to
those it watches as they happen.

## Where it lives

| Part | Where |
| --- | --- |
| Reading statuses | `AspenSync`, `RecordStore.presenceCandidates()`, `applyStatuses` |
| Watching statuses | `AspenSync`'s `#tellWatching`, `RecordStore.presenceWatchList`, `RecordStore.onChange`, `EventStream.sendWatchPresence` |
| Reporting activity | `AspenSync.noteActivity`, `src/api/activity.ts`, `SyncProvider` |
| A channel's online count | `OnlineCount` in `ChannelHeader`, `useChannelOnline`, `AspenSync.watchChannelPresence` |
| Drawing a status | `PresenceMark` and `StatusDot` (`src/features/users/PresenceMark.tsx`), `knownStatus` (`presenceStatus.ts`) |
| Choosing a status | `PresenceMenu` (`src/features/users/PresenceMenu.tsx`), `AspenSync.setChosenPresence` |
| The chosen status | `RecordStore.chosenPresence`, `doNotDisturb` (topic `presence`), `useChosenPresence`, `useDoNotDisturb` |

## Reading statuses

### Watching

`AspenSync` tells its event stream whose presence it shows with a `watchPresence` frame
(`RecordStore.presenceWatchList`), at most `MAX_WATCHED_PRESENCE` (500), the most wanted first:

1. the user, whose own status the user bar shows;
2. everyone in a call;
3. the members shown for the communities with a channel on screen (`watchChannelPresence`);
4. the members shown for every other community.

- It sends it on every `ready`, since a new connection watches nobody.
- It sends it again, `WATCH_SETTLE_MS` after the members shown, the calls, the communities, or the
  user change (`RecordStore.onChange`, which hears every topic a change touched), when the list
  differs from the last sent.
- The server answers with `presence` frames (`ephemeral`): each change to a watched user, gathered
  for up to a second, and each user a later list adds as they are now. The users of a
  connection's first list are not told as they are: the whole read below covers them, so a crowd
  reconnecting at once puts that work on requests, which a busy server can refuse, rather than on
  its presence router. `applyStatuses` takes them.
- The server answers each list it takes up with a `presenceWatching` frame (`#onWatching`).

### Reading whole

`AspenSync` also asks `GET /users/statuses` for `RecordStore.presenceCandidates()` (the user, the
members shown for every community, and everyone in a call) in batches of `PRESENCE_BATCH`:

1. on every connection, once `presenceWatching` says the server took up its first
   `watchPresence`, or when the sync goes live if later: the server tells changes from the
   take-up, so a change before it is in the read and one after it is told. With no answer within
   `WATCH_ACK_TIMEOUT_MS` (ten seconds), the list was lost to a busy server: it is sent again,
   and everyone shown is read anyway;
2. then at most every `PRESENCE_READ_MS` (two minutes), checked at each `PRESENCE_POLL_MS` while
   the page is visible, and when the page becomes visible again.

It catches what the stream could not tell: a change lost on the way, and those beyond the 500
watched.

`applyStatuses` is the only way someone else's `onlineStatus` changes after the bootstrap. The
user's own also follows what they choose at once (below).

## Reporting activity

`AspenSync.noteActivity` is what makes the server show the user as online rather than away.

- `SyncProvider` calls it on pointer, keyboard, wheel, and touch input, and when the window gains
  focus or comes into view (`src/api/activity.ts`).
- It sends an `activity` frame at most every `ACTIVITY_INTERVAL_MS`.
- On reconnecting it sends one only if the user was active within that interval.
- **Nothing else may call it.** Background work is not the user using the app.

## A channel's online count

A channel's header shows how many people who may view it are online (not away), behind the
online status's mark (`OnlineCount` in `ChannelHeader`).

- No client knows every member of a large community, so the server counts
  (`GET /channels/{channel}/presence`).
- `useChannelOnline` asks `AspenSync.watchChannelPresence` to keep the count current while the
  header is shown.
- The count is read at once, then every `PRESENCE_POLL_MS` (thirty seconds), into
  `RecordStore.channelOnline`.

## Drawing a status

`PresenceMark` draws a status as a shape as well as a colour. It reads without telling green from
yellow, in greyscale, and under forced colours.

| Status | Shape |
| --- | --- |
| Online | A full dot |
| Away | A crescent |
| Do not disturb | A dot with a bar cut through it, in `busy` (each palette's danger colour) |
| Offline | A ring |
| Invisible | A ring, since that is how everyone else sees it |

- Over a picture it is named for assistive technology (`StatusDot`, on a disc of the ground the
  picture sits on): in the member list, and over the user's own in the user bar.
- Beside text that says the same, it is hidden from assistive technology.
- A status the client does not know, from a newer deployment, is drawn and named as offline
  (`knownStatus`). The member list puts the invisible and the offline together (`showsConnected`).

## Choosing a status

Pressing the user's picture and name in the user bar (`SidebarFooter`) opens `PresenceMenu`:

- **Online** ends any chosen status, so their connections say it again.
- **Away**, **Do not disturb**, and **Invisible** each open how long to keep it: until they change
  it, or for 15 minutes, 1 hour, 3 hours, 8 hours, 1 day, or 3 days. The server takes up to thirty
  days.
- The one in force is ticked, and says until when instead of what it does.

`AspenSync.setChosenPresence` sets it with `PUT /users/@me/presence-override` (or ends it with
`DELETE`), on every deployment the user uses, since it is theirs rather than one deployment's. A
deployment that refuses says so in a toast naming it.

The store holds it (`RecordStore.chosenPresence`) from `GET /users/@me/presence-override` at
bootstrap, the answer to a change, and `presenceOverrideChanged` events from the user's other
devices. Holding one shows the user's own status by it at once. A timed one ends by this device's
clock, as a mute does (`AspenSync`'s `#scheduleChosenPresenceEnd`).

### Do not disturb

Do not disturb is the user's own, so the home's answer (`useDoNotDisturb`, which reads the home
store wherever it is used) holds beside every deployment's lists alike. While it lasts:

- nothing notifies (see [Notifications](notifications.md));
- no call rings (see [Rings](voice/dm-calls-and-rings.md));
- nothing is drawn unread and no tag is counted (see [Unread and muting](unread-and-muting.md)).
