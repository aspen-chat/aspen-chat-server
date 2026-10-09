# Push

How the mobile app is woken by pushes ([spec/push.md](../../../spec/push.md)). The design notes
are in [push-design-notes.md](push-design-notes.md).

## Where it lives

| Part | Where |
| --- | --- |
| Asking, registering, subscribing | `WakeThisPhone` (`src/api/push.tsx`) |
| Relay subscriptions and decryption | `syncPush` (`packages/protocol/src/push.ts`) |
| Native plugin bridge | `AspenPush` (`src/api/pushBridge.ts`) |
| Android | `packages/mobile/android`: `AspenPushPlugin`, `AspenMessagingService`, `PushHandler`, `WebPush` |
| iOS | `packages/mobile/ios/App`: `AspenPushPlugin.swift`, `PushState.swift`, `NotificationService.swift`, `WebPush.swift`, `AppDelegate` |

## In the app

On the mobile app, `WakeThisPhone`:

1. asks to notify;
2. registers with `@capacitor/push-notifications`;
3. keeps a relay subscription with every deployment of `useSources` through `syncPush`;
4. saves the `PushState` through the native plugin `AspenPush`.

- `syncPush` also decrypts a push (RFC 8291), as the native code must.
- `AspenPush` says which platform, app, and relay the build is.
- A build without that plugin simply has no push.
- Signing out forgets every account.
- Tapping a notification opens its message. A notice about no message opens its channel.

## Android

The project is `packages/mobile/android`.

### Parts

- `AspenPushPlugin` holds the state in the app's private storage.
- `AspenMessagingService` takes the place of the push plugin's FCM service; the manifest removes
  that one. It hands that service anything that is not a relay push.

### Handling a push

`PushHandler`:

1. decrypts (`WebPush`);
2. fetches with the account's session, refreshing it on `401`;
3. posts the notification, tagged `channel/message`, so `read` and `deleted` take it down.

A `notice` pointer:

- is fetched from `GET /users/@me/plugin-notices/{notice}`;
- is shown with the plugin's name and what it says;
- is tagged `channel/notice`. Notice ids are UUIDv7 too, so reading the channel takes down the
  notices before it;
- opens its message, or its channel when it is about none.

### Build requirements

A build pushes only with both:

- a relay in the `aspen_push_relay` string;
- a `google-services.json` from its publisher's Firebase project.

A build naming no relay registers for nothing. `describe` refuses there, before the app asks.

## iOS

`packages/mobile/ios/App`. `AspenPushPlugin.swift` is the same plugin as Android's.

### What `describe` names

- APNs;
- the bundle id;
- the sandbox, for a debug build;
- the relay that `AspenPushRelay` in `Info.plist` names. It comes from the build setting
  `ASPEN_PUSH_RELAY`, given on the `xcodebuild` command line or in a publisher's project. Empty,
  like Android's string, the build has no push.

### State

- The state is a keychain item in the access group `AspenKeychainAccessGroup`:
  `$(AppIdentifierPrefix)` and the bundle id with `.push`.
- The access group is in both `Info.plist` files and both entitlements files.
- The notification service extension `AspenNotificationService` shares it (`PushState.swift`).

### The notification service extension

`NotificationService.swift`:

1. decrypts the push's ciphertext with the account's keys (`WebPush.swift`, CryptoKit);
2. fetches the message with the session, with `include=authors,channels,mentions,memberships`
   so tags show as names and the author by their nickname in a community. It renews the session
   once on `401` and writes the new token back;
3. shows the author, the channel, and the text with tags as names, carrying where the message is
   for a tap.

- A `notice` pointer: it fetches the notice and shows its plugin's name and text.
- A `read` or `deleted` pointer, and any failure, leaves the placeholder. The build holds no
  filtering entitlement.

The `AppDelegate` posts APNs registration to the Capacitor push plugin.

## Testing

| Check | What it does |
| --- | --- |
| `ios/scripts/check_webpush.sh` | Runs `WebPush.swift` against RFC 8291's example with `swiftc`; no target needed |
| `xcrun simctl push <device> org.aspenchat.client payload.json` | Delivers a push to the simulator without APNs or the relay, with `aspen.s` and `aspen.c` as the relay would send them. Shows only the placeholder |
| `ios/scripts/check_notification.sh state.json payload.json` | Runs the extension's work on a Mac with its own sources, on a kept state and such a payload. Reaches the deployment the state names and prints what the notification would show |
| `pnpm --filter @aspen/mobile test:android` | Runs the JVM tests and, on a connected device or emulator, the handler end to end against a stand-in deployment |

- The simulator's bridge adds the notification as a request directly, never through the path
  that launches service extensions. The extension runs only on a device.
- For the Android end-to-end test, debug builds may use plain HTTP to the device itself.
- The Android tests need JDK 21 and the Android SDK. CI's `android` job runs the JVM tests and,
  on an emulator, the instrumented tests, the push handler's among them.

## Backups

**Nothing of the app's sessions or push keys leaves the phone in a backup.** The app's storage
holds the web view's sessions on each deployment and the push state's keys. The person signs in
again on a new phone.

| Platform | How |
| --- | --- |
| Android | The manifest sets `android:allowBackup="false"`. `res/xml/data_extraction_rules.xml` excludes every domain from cloud backup and from moving to a new device, which Android 12 and later do even without backups |
| iOS | The push state's keychain item is `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`. `PushStateStore` moves an item kept otherwise to it on every read. The app marks the web view's data directory, `Library/WebKit`, excluded from backup at every launch (`AppDelegate`) |
