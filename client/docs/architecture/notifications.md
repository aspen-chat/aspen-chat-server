# Notifications

Each user's notification settings decide which arriving messages tell of themselves. The app plays a chime and, where allowed, shows a system notification.

## Where it lives

| Part | Code |
| --- | --- |
| Settings and levels | `RecordStore.notificationLevel` (topic `notifications`), `useNotificationLevel` |
| Thread follows | `follows` (topic `follow:<threadId>`) |
| Deciding | `notifies(message)` |
| Announcing | `AspenSync.onNotify`, `onPluginNotice` |
| Chime and system notifications | `NotifyOnMessages`, `src/features/notifications` |
| Sounds | `sounds.ts`, `sounds/`; `PlaySounds` |

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

- plays the chime (`chime.wav`, see [Sounds](#sounds), through the notification speaker)
- shows the system's notification (`describe`), which opens the message, when `DESKTOP_NOTIFICATIONS` is on and the browser allows

It tells of plugins' notices (`onPluginNotice`) the same way (see [Plugins](plugins/index.md)).

Exceptions:

- It does nothing for the conversation in view.
- It does nothing at all in do not disturb, by the home's answer (`RecordStore.doNotDisturb`; see
  [Presence](presence.md#do-not-disturb)).
- It shows no system notification in the mobile app, whose phone push wakes.

## Sounds

Every sound the app plays is a recording in `src/features/notifications/sounds/`, played by `playSound`, or `loopSound` until stopped (`sounds.ts`):

| File | Plays when | Through |
| --- | --- | --- |
| `chime.wav` | A message or plugin notice tells of itself (`NotifyOnMessages`) | The notification sound's speaker |
| `ringtone.wav` | Looped while a call rings the user (`IncomingCall`; see [DM calls and rings](voice/dm-calls-and-rings.md)) | The notification sound's speaker |
| `dial-tone.wav` | Looped while the user's DM call rings someone (`DmCall`) | The voice chat's speaker |
| `call-joined.wav` | The user's call connects, or connects again after a rejoin; someone else joins it | The voice chat's speaker |
| `call-left.wav` | The user leaves their call; someone else leaves it | The voice chat's speaker |
| `disconnected.wav` | The user's call is lost under them (a lost server or socket, a failed rejoin, or an end the call bar explains), or a deployment's event stream stays down for `DISCONNECTED_GRACE_MS` | The call's speaker, or the notification sound's for a deployment |

`PlaySounds` decides when the last three play, for every deployment the user uses: `watchCallSounds` (`src/features/voice/callSounds.ts`) and `watchConnectionSounds` (`connectionSounds.ts`).

- Someone else joining or leaving is not played while the user is deafened.
- Moving to another call plays only the joining.
- Who is in a call when it connects is taken as found, silently.
- `playSound` (`sounds.ts`) plays a sound again only after its `REPEAT_GAP_MS`, so several people joining at once are heard once, and a network failure that drops a call and its deployment together is heard once.

The files are WAV (16-bit PCM, mono, 48 kHz), which every browser and web view the apps run in plays; Ogg Opus would be smaller but Safari before 18.4 cannot play it. `sounds.ts` imports them as `data:` URLs (`import.meta.glob` with `?inline`), since the desktop app's page, loaded from a file, may play no media from its own origin. Each is a chunk of its own, loaded the first time it plays, so none is part of the page's first load. The ringtone and the dial tone loop whole, so each file's length is its cycle: the ringtone 2.4 seconds, the dial tone 1.2 seconds of tone in every 4. To replace a sound, overwrite its file under the same name.

## Where the user sets it

| Setting | Where |
| --- | --- |
| A channel's level | The channel's menu, "Notify me about" |
| A community's level | Community settings (`CommunityNotifications`) |
| The two device preferences | Settings (`NotificationsSection`) |
