# Push

- Push (`spec/push.md`): on the mobile app, `WakeThisPhone` (`src/api/push.tsx`) asks to notify,
  registers with `@capacitor/push-notifications`, and keeps a relay subscription with every
  deployment of `useSources` through `syncPush` (`packages/protocol/src/push.ts`, which also
  decrypts a push, RFC 8291, as the native code must), saving the `PushState` through the native
  plugin `AspenPush` (`src/api/pushBridge.ts`), which says which platform, app, and relay the
  build is. A build without that plugin simply has no push. Signing out forgets every account.
  Tapping a notification opens its message (a notice about no message, its channel). On Android (`packages/mobile/android`, the one
  native project kept in git), `AspenPushPlugin` holds the state in the app's private storage,
  and `AspenMessagingService` takes the place of the push plugin's FCM service (the manifest
  removes that one), handing it anything that is not a relay push: `PushHandler` decrypts
  (`WebPush`), fetches with the account's session, refreshing it on `401`, and posts the
  notification, tagged `channel/message` so `read` and `deleted` take it down; a `notice`
  pointer is fetched from `GET /users/@me/plugin-notices/{notice}` and shown with the plugin's
  name and what it says, tagged `channel/notice` (notice ids are UUIDv7 too, so reading the
  channel takes down the notices before it), and opens its message, or its channel when it is
  about none. A build pushes
  only with a relay in the `aspen_push_relay` string and a `google-services.json` from its
  publisher's Firebase project. On iOS (`packages/mobile/ios/App`), `AspenPushPlugin.swift` is the
  same plugin: `describe` names APNs, the bundle id, the sandbox for a debug build, and the relay
  `AspenPushRelay` in `Info.plist` names (the build setting `ASPEN_PUSH_RELAY`, given on the
  `xcodebuild` command line or in a publisher's project; empty, like Android's string, the build
  has no push), and the state is a keychain item in the access group `AspenKeychainAccessGroup`
  (`$(AppIdentifierPrefix)` and the bundle id with `.push`, in both `Info.plist` files and both
  entitlements files), which the notification service extension `AspenNotificationService`
  shares (`PushState.swift`). The extension (`NotificationService.swift`) decrypts the push's
  ciphertext with the account's keys (`WebPush.swift`, CryptoKit; `ios/scripts/check_webpush.sh`
  runs it against RFC 8291's example with `swiftc`, no target needed), fetches the message
  with the session (`include=authors,channels,mentions`, so tags show as names; renewed once
  on `401`, the new token written back), and shows the author,
  the channel, and the text with tags as names, carrying where the message is for a tap, and
  fetches a `notice` pointer's notice, showing its plugin's name and text; a
  `read` or `deleted` pointer, and any failure, leaves the placeholder, since the build holds no
  filtering entitlement. The `AppDelegate` posts APNs registration to the Capacitor push plugin.
  `xcrun simctl push <device> org.aspenchat.client payload.json` delivers a push to the
  simulator without APNs or the relay, with `aspen.s` and `aspen.c` as the relay would send
  them, but shows only the placeholder: the simulator's bridge adds the notification as a
  request directly, never through the path that launches service extensions, so the extension
  runs only on a device. `ios/scripts/check_notification.sh state.json payload.json` runs the
  extension's work on a Mac instead, with its own sources, on a kept state and such a payload,
  reaching the deployment the state names, and prints what the notification would show. `pnpm --filter @aspen/mobile test:android` runs the JVM tests
  and, on a connected device or emulator, the handler end to end against a stand-in deployment
  (debug builds may use plain HTTP to the device itself for it). It needs JDK 21 and the Android
  SDK; CI does not run it yet.
