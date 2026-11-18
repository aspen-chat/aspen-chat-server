# Push

How the mobile app is woken by pushes ([spec/push.md](../../../spec/push.md)). The design notes are
in [push-design-notes.md](push-design-notes.md).

## Where it lives

| Part                               | Where                                                                                                                              |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Asking, registering, subscribing   | `WakeThisPhone` (`src/api/push.tsx`)                                                                                               |
| The person's answer                | `pushChoice.ts`, the prompt `PushPrompt`, and the switch in Settings' `NotificationsSection`                                       |
| Saying the app is in front of them | `reportActivity` (`src/api/activity.ts`), `AspenSync.noteAttending`                                                                |
| Relay subscriptions and decryption | `syncPush` (`packages/protocol/src/push.ts`)                                                                                       |
| Native plugin bridge               | `AspenPush` (`src/api/pushBridge.ts`)                                                                                              |
| Android                            | `packages/mobile/android`: `AspenPushPlugin`, `AspenMessagingService`, `PushHandler`, `WebPush`                                    |
| iOS                                | `packages/mobile/ios/App`: `AspenPushPlugin.swift`, `PushState.swift`, `NotificationService.swift`, `WebPush.swift`, `AppDelegate` |

## In the app

On the mobile app, `WakeThisPhone`:

1. asks whether to wake this device (`PushPrompt`), and only on Accept asks the system to notify;
2. registers with `@capacitor/push-notifications`;
3. keeps a relay subscription with every deployment of `useSources` through `syncPush`;
4. saves the `PushState` through the native plugin `AspenPush`.

### Asking

The answer is this install's, kept in `localStorage` as `aspen.push`, not the account's: `on`,
`off`, or not given.

| Answer    | System says   | The app                                                      |
| --------- | ------------- | ------------------------------------------------------------ |
| Not given | Not asked yet | Shows the prompt                                             |
| Not given | Allowed       | Takes it as `on`, asking nothing (an install from before it) |
| Not given | Refused       | Asks nothing; Settings says to allow it in the system's      |
| `on`      | Not asked yet | Asks the system, then registers if allowed                   |
| `on`      | Allowed       | Registers                                                    |
| `on`      | Refused       | Registers nothing; checks again each time the app comes back |
| `off`     | Anything      | Registers nothing, and ends what was registered (`stopPush`) |

- Accept answers `on`, Decline `off`. The X, Escape, or a tap beside the prompt leaves it unanswered
  until the next start.
- Settings' Notifications section, in the mobile app only, turns it on and off. Turning it off ends
  the platform's token, each deployment's subscription, and the device at the relay. Turning it on
  after the system refused looks again, since only the system's own settings can allow it then.
- Android before 13 never asks; its system answers allowed, so it never shows the prompt.

### While the app is open

Every deployment's event stream says when the app is in front of the user (`attending`, at most
every 15 seconds as they use it) and when it goes to the background or loses focus
(`stoppedAttending`), which no phone of theirs is woken while (see the server's
[Push](../../../docs/architecture/push.md#attention)). A window losing focus to a frame of its own,
such as a plugin's view, still counts.

### The device at the relay

The kept `PushState` records what the device was registered as: its platform, app, and environment.
When `describe` gives any of them otherwise, `syncPush` ends the device (`stopPush`: each
deployment's subscription, then the device and its subscriptions at the relay) and registers it
anew, subscribing every account again. A build signed otherwise installed over the app, a
development build over a TestFlight one or the reverse, changes the environment its tokens belong
to, and the relay would otherwise go on sending to the other APNs. A state kept without them is
registered anew once.

### Logs on a device

`console` output reaches the system's log (os_log on iOS, logcat on Android) in debug builds only,
unless the native projects are synced with `ASPEN_DEVICE_LOGS=1` (`cap sync`, which writes
`capacitor.config.ts`'s `loggingBehavior` into them). Builds for the developers' own devices set it;
CI and published builds never do. Every place where waking the phone can stop logs a `push:` warning
or error with its cause: the build has no push, the system did not allow notifications (and which
answer it gave), the platform refused to register, and each deployment's failing step in `syncPush`.

### What else

- `syncPush` also decrypts a push (RFC 8291), as the native code must.
- `AspenPush` says which platform, app, and relay the build is.
- A build without that plugin simply has no push.
- Signing an account out forgets it and the deployments it signed in to from there; others stay.
- A tapped notification names the account it was for (`userId`), which comes to the front.
- Tapping a notification opens its message. A notice about no message opens its channel.
- The notification code on both phones reads only what it knows the meaning of: a `PushState` whose
  `version` is 1 and a pointer whose `v` is 1, as the app's own code does (`pushBridge.ts`,
  `protocol/src/push.ts`). For any other, Android shows nothing and iOS leaves the placeholder.

## Android

The project is `packages/mobile/android`.

### Parts

- `AspenPushPlugin` holds the state sealed under an Android Keystore key (`PushStore`,
  `SecretStore`).
- `AspenMessagingService` takes the place of the push plugin's FCM service; the manifest removes
  that one. It hands that service anything that is not a relay push.

### Handling a push

`PushHandler`:

1. decrypts (`WebPush`);
2. fetches with the account's session, refreshing it on `401`, asking for the phone's languages
   (`Accept-Language`);
3. posts the notification, tagged `subscription/channel/message`, so `read` and `deleted` take down
   that account's alone. Its text is the one the deployment wrote out
   (`included.notificationTexts`), shown as given; from a deployment of a version that writes none,
   `readable` and `recorded` make it. Text holding a spoiler's marker is not shown
   (`SpoilerMarkers`; see The phones' own wording).

A `notice` pointer:

- is fetched from `GET /users/@me/plugin-notices/{notice}`;
- is shown with the plugin's name and what it says;
- is tagged `subscription/channel/notice`. Notice ids are UUIDv7 too, so reading the channel takes
  down the notices before it;
- opens its message, or its channel when it is about none.

### Build requirements

A build pushes only with both:

- a relay in the `aspen_push_relay` string, `https://notify.aspen-chat.com` (Bitcrafters LLC's)
  unless a publisher changes it;
- a `google-services.json` from its publisher's Firebase project, in `android/app/`. The repository
  carries Bitcrafters LLC's (project `aspen-chat-713fa`), which identifies the project and holds no
  secret: only the relay's service account can send to it.

A build naming no relay registers for nothing. `describe` refuses there, before the app asks.

## iOS

`packages/mobile/ios/App`. `AspenPushPlugin.swift` is the same plugin as Android's.

### What `describe` names

- APNs;
- the bundle id;
- the APNs environment the build's tokens belong to: the `aps-environment` entitlement of the
  provisioning profile in its bundle (`ApsEnvironment`), so a Release build signed for development
  says the sandbox; with no profile, as the App Store's and TestFlight's builds have, production,
  and on the simulator the sandbox;
- the relay that `AspenPushRelay` in `Info.plist` names. It comes from the build setting
  `ASPEN_PUSH_RELAY`, which the app target sets to `https://notify.aspen-chat.com` and the
  `xcodebuild` command line or a publisher's project may replace. Empty, like Android's string, the
  build has no push.

`describe` refuses, and the build has no push, when the relay is empty or when the profile grants no
`aps-environment` or cannot be read.

### State

- The state is a keychain item in the access group `AspenKeychainAccessGroup`:
  `$(AppIdentifierPrefix)` and the bundle id with `.push`.
- The access group is in both `Info.plist` files and both entitlements files.
- The notification service extension `AspenNotificationService` shares it (`PushState.swift`).

### The notification service extension

`NotificationService.swift`:

1. decrypts the push's ciphertext with the account's keys (`WebPush.swift`, CryptoKit);
2. fetches the message with the session, with
   `include=authors,channels,mentions,memberships,notificationTexts` so the author shows by their
   nickname in a community and the text as the deployment wrote it out. It renews the session once
   on `401` and writes the new token back;
3. shows the author, the channel, and the text the deployment wrote out
   (`included.notificationTexts`), carrying where the message is for a tap. From a deployment of a
   version that writes none, it shows the text with tags as names, or what the message holds instead
   of text, which is also what it shows of text holding a spoiler's marker (see The phones' own
   wording).

- A `notice` pointer: it fetches the notice and shows its plugin's name and text.
- A `read` or `deleted` pointer, and any failure, leaves the placeholder. The build holds no
  filtering entitlement.

The `AppDelegate` posts APNs registration to the Capacitor push plugin.

## The phones' own wording

A deployment of a version that answers no `notificationTexts` leaves the notification code to word a
message itself. Neither phone reads Markdown, and which markers make a spoiler is the renderer's to
say ([Markdown](message-rendering/markdown.md#what-makes-a-spoiler)), so neither tries:

- Text holding a spoiler's marker anywhere is not shown at all: `||`, `>!`, `!<`, or `\|\|`, which
  is how a table's cell writes the first (`SpoilerMarkers.present` on Android; the same four in
  `MessageFetcher.body` on iOS). No spoiler is written without one of them.
- Such a message reads as one with no text of its own: what its kind records, or "New message" (on
  iOS, "Sent an attachment" when it holds files).
- **The trade-off:** nothing a spoiler hides is ever shown, and some texts that only look as if they
  held one are hidden too: a table, a shell pipeline (`ls | wc || true`), code holding `||`. A
  second and third copy of the spoiler grammar, in Java and Swift, would have to be kept in step
  with the renderer and the server for the sake of deployments too old to write the text out.

## Testing

| Check                                                                    | What it does                                                                                                                                                                                                                                                          |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ios/scripts/check_webpush.sh`                                           | Runs `WebPush.swift` against RFC 8291's example with `swiftc`; no target needed                                                                                                                                                                                       |
| `xcrun simctl push <device> com.aspen-chat.client payload.json`          | Delivers a push to the simulator without APNs or the relay, with `aspen.s` and `aspen.c` as the relay would send them. Shows only the placeholder                                                                                                                     |
| `ios/scripts/check_notification.sh state.json payload.json [shown.json]` | Runs the extension's work on a Mac with its own sources, on a kept state and such a payload. Reaches the deployment the state names and prints what the notification would show; given `shown.json`, writes there the payload of that notification, for `simctl push` |
| `ios/scripts/ui_tests.py`                                                | Runs the iOS UI tests, among them a tapped notification's, end to end through `scripts/push_relay_stand_in.py` (see `ios-app.md`)                                                                                                                                     |
| `pnpm --filter @aspen/mobile test:android`                               | Runs the JVM tests and, on a connected device or emulator, the handler end to end against a stand-in deployment                                                                                                                                                       |

- The simulator's bridge adds the notification as a request directly, never through the path that
  launches service extensions. The extension runs only on a device.
- For the Android end-to-end test, debug builds may use plain HTTP to the device itself.
- The Android tests need JDK 21 and the Android SDK. CI's `android` job runs the JVM tests and, on
  an emulator, the instrumented tests, the push handler's among them.

## Backups

**Nothing of the app's sessions or push keys leaves the phone in a backup.** The app's storage holds
the web view's sessions on each deployment and the push state's keys. The person signs in again on a
new phone.

| Platform | How                                                                                                                                                                                                                                                                          |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Android  | The manifest sets `android:allowBackup="false"`. `res/xml/data_extraction_rules.xml` excludes every domain from cloud backup and from moving to a new device, which Android 12 and later do even without backups                                                             |
| iOS      | The push state's keychain item is `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`. `PushStateStore` moves an item kept otherwise to it on every read. The app marks the web view's data directory, `Library/WebKit`, excluded from backup at every launch (`AppDelegate`) |
