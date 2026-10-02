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
