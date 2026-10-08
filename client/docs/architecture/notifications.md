# Notifications

- Notifications: each user's settings (`notificationSettings`, from the community and DM reads
  and `notificationSettingChanged` events) are store state, `RecordStore.notificationLevel`
  (topic `notifications`, `useNotificationLevel`) resolving a channel's level from its own
  setting, its community's, and the default, and `notifies(message)` deciding whether a message
  tells of itself, every reply in a thread the caller follows (`follows`, topic
  `follow:<threadId>`, from `GET /users/@me/thread-follows` at bootstrap and
  `threadFollowChanged`) telling whatever the level, unless its parent is muted. `AspenSync.onNotify` announces such messages as they arrive live, never the
  replay a connection starts with. `NotifyOnMessages` (`src/features/notifications`), for every
  deployment the user uses, plays the chime (`chime.ts`, a two-note sound made in code, through
  the notification speaker) and, when `DESKTOP_NOTIFICATIONS` is on and the browser allows, shows
  the system's notification (`describe`), which opens the message, and tells of plugins' notices
  (`onPluginNotice`) the same way (see Plugins); nothing for the conversation
  in view, and no system notification in the mobile app, whose phone push wakes. A channel's menu
  sets its level ("Notify me about"), community settings the community's
  (`CommunityNotifications`), and Settings the two device preferences (`NotificationsSection`).
