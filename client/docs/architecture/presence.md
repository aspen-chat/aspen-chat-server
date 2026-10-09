# Presence

Presence is pulled. The server pushes no status events.

## Where it lives

| Part | Where |
| --- | --- |
| Polling statuses | `AspenSync`, `RecordStore.presenceCandidates()`, `applyStatuses` |
| Reporting activity | `AspenSync.noteActivity`, `src/api/activity.ts`, `SyncProvider` |
| A channel's online count | `OnlineCount` in `ChannelHeader`, `useChannelOnline`, `AspenSync.watchChannelPresence` |
| Drawing a status | `PresenceMark` (`src/features/users/PresenceMark.tsx`), `StatusDot` in `MemberList` |

## Reading statuses

`AspenSync` asks `GET /users/statuses` for `RecordStore.presenceCandidates()`: the members shown
for every community, and everyone in a call. It asks in batches of `PRESENCE_BATCH`:

1. when the sync goes live;
2. every `PRESENCE_POLL_MS` while the page is visible;
3. when the page becomes visible again.

`applyStatuses` is the only way a user's `onlineStatus` changes after the bootstrap.

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
| Offline | A ring |

- Over a member's picture it is named for assistive technology (`StatusDot` in `MemberList`).
- Beside text that says the same, it is hidden from assistive technology.
