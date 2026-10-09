# The iOS app's UI tests

The iOS project (`packages/mobile/ios`) is kept, as Android's is. Its UI tests drive the app with real taps and finger drags in WebKit, against a real deployment.

## Where it lives

| Part | Location |
| --- | --- |
| UI tests | `ios/App/AppUITests` |
| Scheme | `App` |

## Running them

The tests read these variables. Give them to `xcodebuild test` with the `TEST_RUNNER_` prefix. Without them, the tests skip.

| Variable | Use |
| --- | --- |
| `ASPEN_TEST_SERVER` | The deployment, entered when the app asks for one |
| `ASPEN_TEST_USER` | The user to sign in as |
| `ASPEN_TEST_PASSWORD` | That user's password |
| `ASPEN_TEST_NOTIFICATION` | Text of a notification delivered first; needed by `testOpensATappedNotification` |

Signing in allows the system's notification alert, so the phone registers for push as a person's would.

## The tests

| Test | What it checks |
| --- | --- |
| `testReadingBackMovesOnlyWithTheFinger` | Safari's own answer to `historyScroll.spec.ts` |
| `testJoinsAndLeavesACall` | Joins a call, with the microphone the simulator takes from the Mac |
| `testOpensATappedNotification` | Opens a tapped notification's message. This needs a notification the service extension replaced, so it needs a device. |
