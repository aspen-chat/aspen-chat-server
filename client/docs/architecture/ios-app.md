# The iOS app's UI tests

- The iOS project (`packages/mobile/ios`) is kept, as Android's is. Its UI tests
  (`ios/App/AppUITests`, scheme `App`) drive the app with real taps and finger drags in WebKit
  against the server the web build names, signing in as `ASPEN_TEST_USER`/`ASPEN_TEST_PASSWORD`
  (given to `xcodebuild test` as `TEST_RUNNER_`-prefixed variables; without them they skip):
  `testReadingBackMovesOnlyWithTheFinger` is Safari's own answer to `historyScroll.spec.ts`;
  signing in allows the system's notification alert, so the phone registers for push as a
  person's would; `testJoinsAndLeavesACall` joins a call with the microphone the simulator
  takes from the Mac; and `testOpensATappedNotification` (given `ASPEN_TEST_NOTIFICATION`,
  text of a notification delivered first) opens a tapped notification's message, which needs
  one the service extension replaced, so a device.
