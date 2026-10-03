# Presence

- Presence is pulled. The server pushes no status events; `AspenSync` asks
  `GET /users/statuses` for `RecordStore.presenceCandidates()` (the members shown for every
  community and everyone in a call), in batches of `PRESENCE_BATCH`, when the sync goes live,
  every `PRESENCE_POLL_MS` while the page is visible, and when the page becomes visible again;
  `applyStatuses` is the only way a user's `onlineStatus` changes after the bootstrap.
  What makes the server show the user as online rather than away is `AspenSync.noteActivity`,
  which `SyncProvider` calls on pointer, keyboard, wheel, and touch input and when the window
  gains focus or comes into view (`src/api/activity.ts`); it sends an `activity` frame at most
  every `ACTIVITY_INTERVAL_MS`, and on reconnecting only if the user was active within that
  interval. Nothing else may call it: background work is not the user using the app.
- A channel's header shows how many people who may view it are online (not away), behind the
  online status's green dot (`OnlineCount` in `ChannelHeader`). No client knows every member of
  a large community, so the server counts (`GET /channels/{channel}/presence`). `useChannelOnline`
  asks `AspenSync.watchChannelPresence` to keep the count current while the header is shown: it
  is read at once, then with every presence poll, into `RecordStore.channelOnline`.
