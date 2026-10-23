# Notifications

Each user's notification settings decide which arriving messages tell of themselves. The app plays a
chime and, where allowed, shows a system notification.

## Where it lives

| Part                           | Code                                                                            |
| ------------------------------ | ------------------------------------------------------------------------------- |
| Settings and levels            | `RecordStore.notificationLevel` (topic `notifications`), `useNotificationLevel` |
| Thread follows                 | `follows` (topic `follow:<threadId>`)                                           |
| Deciding                       | `notifies(message)`                                                             |
| Announcing                     | `AspenSync.onNotify`, `onPluginNotice`                                          |
| Chime and system notifications | `NotifyOnMessages`, `src/features/notifications`                                |
| Sounds                         | `sounds.ts`, `sounds/`, `speakers.ts`; `PlaySounds`                             |

## State

- Settings: `notificationSettings`, from the community and DM reads and `notificationSettingChanged`
  events.
- Thread follows: from `GET /users/@me/thread-follows` at bootstrap and `threadFollowChanged`
  events.

## Deciding whether a message tells

`RecordStore.notificationLevel` resolves a channel's level from, in order:

1. the channel's own setting
2. its category's
3. its community's
4. the default

`notifies(message)` decides whether a message tells of itself:

- By the channel's level.
- Every reply in a thread the caller follows tells, whatever the level, unless its parent is muted.

## Announcing

`AspenSync.onNotify` announces such messages as they arrive live. It never announces the replay a
connection starts with.

`NotifyOnMessages` runs for every deployment the user uses. For each message, it:

- plays the chime (`chime.wav`, see [Sounds](#sounds), through the notification speaker)
- shows the system's notification (`describe`), which opens the message, when
  `DESKTOP_NOTIFICATIONS` is on and the browser allows

It tells of plugins' notices (`onPluginNotice`) the same way (see [Plugins](plugins/index.md)).

Exceptions:

- It does nothing for the conversation in view.
- It does nothing at all in do not disturb, by the home's answer (`RecordStore.doNotDisturb`; see
  [Presence](presence.md#do-not-disturb)).
- It shows no system notification in the mobile app, whose phone push wakes.

## Sounds

Every sound the app plays is a recording in `src/features/notifications/sounds/`, played by
`playSound`, or `loopSound` until stopped (`sounds.ts`):

| File                    | Plays when                                                                                                                                                                            | Through                                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `chime.wav`             | A message or plugin notice tells of itself (`NotifyOnMessages`)                                                                                                                       | The notification sound's speaker                                 |
| `ringtone.wav`          | Looped while a call rings the user (`IncomingCall`; see [DM calls and rings](voice/dm-calls-and-rings.md))                                                                            | The notification sound's speaker                                 |
| `dial-tone.wav`         | Looped while the user's DM call rings someone (`DmCall`)                                                                                                                              | The voice chat's speaker                                         |
| `call-joined.wav`       | The user's call connects; someone else joins it                                                                                                                                       | The voice chat's speaker                                         |
| `call-left.wav`         | The user leaves their call; someone else leaves it                                                                                                                                    | The voice chat's speaker                                         |
| `disconnected.wav`      | The user's call is lost under them (a lost server or socket, a failed rejoin, or an end the call bar explains), or a deployment's event stream stays down for `DISCONNECTED_GRACE_MS` | The call's speaker, or the notification sound's for a deployment |
| `reconnected.wav`       | A call lost under the user rejoins, or a deployment's event stream is live again, after `disconnected.wav` was played for it                                                          | The call's speaker, or the notification sound's for a deployment |
| `stream-started.wav`    | The user, or someone else in their call, starts sharing a screen, window, or game                                                                                                     | The voice chat's speaker                                         |
| `transfer-accepted.wav` | Any file transfer in the user's call begins, someone having accepted an offer: everyone in the call hears it, whoever its two sides are                                               | The voice chat's speaker                                         |
| `transfer-complete.wav` | A file the user sent or received in a call has arrived whole                                                                                                                          | The voice chat's speaker                                         |

Every file but `ringtone.wav` and `dial-tone.wav` is in `sounds/aspen-sound-effects-pack/`: the
Aspen Sound Effects Pack, © 2026 by Kayla Wardle, unmodified and under CC BY-SA 4.0 rather than the
app's license, with its `NOTICE` and `LICENSE` beside it, which the attributions page lists (see
[About](about.md)). A sound added from the pack goes there too.

`PlaySounds` decides when the call's and the connection's sounds play, for every deployment the user
uses: `watchCallSounds` (`src/features/voice/callSounds.ts`) and `watchConnectionSounds`
(`connectionSounds.ts`).

- Someone else joining, leaving, or starting a share, and a transfer the user is not a side of
  beginning, are not played while the user is deafened.
- Moving to another call plays only the joining.
- Who is in a call when it connects, what they share, and the transfers under way are taken as
  found, silently.
- A transfer begins when the voice server tells everyone in the call of its link
  (`transferLinkChanged`), as the receiver accepts, before its bytes move.
- `reconnected.wav` answers only a `disconnected.wav` that was heard, so a blip within the grace
  makes no sound either way, and a call that rejoins plays it rather than `call-joined.wav`.
- `playSound` (`sounds.ts`) plays a sound again only after its `REPEAT_GAP_MS`, so several people
  joining at once, or several accepting one offer, are heard once, and a network failure that drops
  a call and its deployment together is heard once, and so is their return.
- `playSound` and `loopSound` play at most `MAX_PLAYING_PER_SOUND` (two) of one sound at once, each
  sound counted on its own; one asked for past that is not heard, rather than waiting its turn, so a
  burst of messages cannot queue a run of chimes. A sound gives its place back when it ends, fails,
  or the browser refuses it, and a loop when it is stopped.

The files are WAV (16-bit PCM, mono, 48 kHz), which every browser and web view the apps run in
decodes; Ogg Opus would be smaller but Safari before 18.4 cannot decode it. `sounds.ts` imports them
as `data:` URLs (`import.meta.glob` with `?inline`) and decodes them itself, so playing one makes no
request, which the desktop app's page could not make to its own files. Each is a chunk of its own,
loaded the first time it plays, so none is part of the page's first load, and decoded once. The
ringtone and the dial tone loop whole, so each file's length is its cycle: the ringtone 2.4 seconds,
the dial tone 1.2 seconds of tone in every 4. To replace a sound, overwrite its file under the same
name, or move it and change its path in `SOUND_FILES`.

### Speakers

Sounds play through Web Audio, one context for each device they play through (`speakers.ts`). An
output device that sleeps while idle (PipeWire suspends one after five seconds, a sound card's codec
powers down, a wireless headset stops sending) drops what it is given while it wakes, and every
recording starts sounding within its first few milliseconds, so a sound started as the device wakes
loses its start. A speaker therefore:

- opens when a sound asks for it, and plays `WAKE_SECONDS` (0.15) of silence before the first sound.
  Where the context plays to the device itself, that is counted in output the device has taken, so
  the sound waits however long the device takes to start, and then that much more;
- stays open, playing silence, for `KEEP_AWAKE_MS` (ten seconds) after its last sound ends, so the
  device stays awake and the sounds in that time start at once. Then it closes, and the device may
  sleep;
- is given up when the browser has not started it within `START_LIMIT_MS` (two seconds), as before
  the page has been interacted with: the sound is not heard, rather than heard late;
- wakes as a new one does when the system suspends it (Safari, during a phone call).

How a speaker reaches the chosen device depends on the browser:

| Browser                                           | How                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Chromium (the desktop app, Android, Chrome, Edge) | The context is opened on the device (`AudioContext`'s `sinkId`)                                                                                                                                                                                                                                 |
| Firefox                                           | The context plays into a stream that an `<audio>` element plays through the device (`setSinkId`), since Firefox cannot open a context on a device. Its clock does not follow the device, so `WAKE_SECONDS` is all the wait there is, and a device slower to wake can still lose a sound's start |
| Safari                                            | Neither can choose a device; every sound plays through the system's default                                                                                                                                                                                                                     |

Firefox and Chromium name a device to a page only once it may capture sound, so a device chosen
before that plays through the default.

## Where the user sets it

| Setting                    | Where                                                                        |
| -------------------------- | ---------------------------------------------------------------------------- |
| A channel's level          | The channel's menu, "Notify me about"                                        |
| A category's level         | The category's heading menu, "Notify me about" (`categoryNotificationLevel`) |
| A community's level        | Community settings (`CommunityNotifications`)                                |
| The two device preferences | Settings (`NotificationsSection`)                                            |
