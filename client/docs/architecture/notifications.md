# Notifications

Each user's notification settings decide which arriving messages tell of themselves. The app plays a chime and, where allowed, shows a system notification.

## Where it lives

| Part | Code |
| --- | --- |
| Settings and levels | `RecordStore.notificationLevel` (topic `notifications`), `useNotificationLevel` |
| Thread follows | `follows` (topic `follow:<threadId>`) |
| Deciding | `notifies(message)` |
| Announcing | `AspenSync.onNotify`, `onPluginNotice` |
| Chime and system notifications | `NotifyOnMessages`, `src/features/notifications`; `chime.ts` |

## State

- Settings: `notificationSettings`, from the community and DM reads and `notificationSettingChanged` events.
- Thread follows: from `GET /users/@me/thread-follows` at bootstrap and `threadFollowChanged` events.

## Deciding whether a message tells

`RecordStore.notificationLevel` resolves a channel's level from, in order:

1. the channel's own setting
2. its community's
3. the default

`notifies(message)` decides whether a message tells of itself:

- By the channel's level.
- Every reply in a thread the caller follows tells, whatever the level, unless its parent is muted.

## Announcing

`AspenSync.onNotify` announces such messages as they arrive live. It never announces the replay a connection starts with.

`NotifyOnMessages` runs for every deployment the user uses. For each message, it:

- plays the chime (`chime.ts`, a two-note sound made in code, through the notification speaker)
- shows the system's notification (`describe`), which opens the message, when `DESKTOP_NOTIFICATIONS` is on and the browser allows

It tells of plugins' notices (`onPluginNotice`) the same way (see [Plugins](plugins/index.md)).

Exceptions:

- It does nothing for the conversation in view.
- It shows no system notification in the mobile app, whose phone push wakes.

## Where the user sets it

| Setting | Where |
| --- | --- |
| A channel's level | The channel's menu, "Notify me about" |
| A community's level | Community settings (`CommunityNotifications`) |
| The two device preferences | Settings (`NotificationsSection`) |
