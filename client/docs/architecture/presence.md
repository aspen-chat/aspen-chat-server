# Presence

Presence is pulled. The server pushes no status events.

## Where it lives

| Part | Where |
| --- | --- |
| Polling statuses | `AspenSync`, `RecordStore.presenceCandidates()`, `applyStatuses` |
| Reporting activity | `AspenSync.noteActivity`, `src/api/activity.ts`, `SyncProvider` |
| A channel's online count | `OnlineCount` in `ChannelHeader`, `useChannelOnline`, `AspenSync.watchChannelPresence` |
| Drawing a status | `PresenceMark` and `StatusDot` (`src/features/users/PresenceMark.tsx`), `knownStatus` (`presenceStatus.ts`) |
| Choosing a status | `PresenceMenu` (`src/features/users/PresenceMenu.tsx`), `AspenSync.setChosenPresence` |
| The chosen status | `RecordStore.chosenPresence`, `doNotDisturb` (topic `presence`), `useChosenPresence`, `useDoNotDisturb` |

## Reading statuses

`AspenSync` asks `GET /users/statuses` for `RecordStore.presenceCandidates()`: the user, whose own
status the user bar shows, the members shown for every community, and everyone in a call. It asks in batches of `PRESENCE_BATCH`:

1. when the sync goes live;
2. every `PRESENCE_POLL_MS` while the page is visible;
3. when the page becomes visible again.

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
- The count is read at once, then with every presence poll, into `RecordStore.channelOnline`.

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
