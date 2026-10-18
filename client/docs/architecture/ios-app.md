# The iOS app's UI tests

The iOS project (`packages/mobile/ios`) is kept, as Android's is. Its UI tests drive the app with real taps and finger drags in WebKit, against a real deployment. They check what the browser specs cannot reach: the iOS web view itself, the system's keyboard, photo picker, and notifications, links opened from outside the app, and the app being suspended and resumed.

## Where it lives

| Part | Location |
| --- | --- |
| UI tests | `ios/App/AppUITests` |
| Scheme | `App` |
| Wrapper | `ios/scripts/ui_tests.py` |
| Relay stand-in | `scripts/push_relay_stand_in.py` at the repository's root |

## Running them

`ui_tests.py` runs them on a Mac with Xcode and a simulator:

```
scripts/push_relay_stand_in.py --app-host <host>                 # beside the deployment
client/packages/mobile/ios/scripts/ui_tests.py --server https://<deployment> \
    --relay https://<host>:7788 --trust target/dev-federation/ca.pem --build
```

The tests run a release build of the web app, which reaches everything over HTTPS (see `message-rendering/address-safety.md`, Transport): the deployment, its storage, its voice servers, and the stand-in, whose `--app-host` serves its app side over HTTPS with a certificate from the development authority. `--server` is the deployment as the simulator reaches it, and `--relay` the stand-in's app side. `--trust` has the simulator, and the wrapper, trust an authority, such as the development one. `--api` is the deployment as the Mac reaches it, when that differs (its plain port behind a TLS proxy): the extension's code run there trusts the system's authorities alone. The deployment needs these settings:

| Setting | Why |
| --- | --- |
| `[federation.development] extra_root_certificates`, `target/dev-federation/ca.pem` | The stand-in's push side has a certificate from that authority |
| `[federation.development] allow_private_addresses = true` | The stand-in's push side is on loopback, as `push.localhost` |
| `[presence] away_after_seconds`, the wrapper's `--away-after` (15) | A user who has just used Aspen is not woken; the test waits that long after leaving the app |
| `[media.s3] public_endpoint` and `public_base_url` over `https` | The photo test uploads its picture and looks for it |
| Its voice servers' `url` over `https` | The call test joins one |

The wrapper does the rest:

1. With `--build`, it builds the web app with `VITE_SCROLL_DEBUG=1` and the tests for the simulator, the app naming the stand-in as its relay. Install the client's dependencies first.
2. It seeds a fixture under `target/ios-ui-tests/` on first use, and reuses it while the deployment still has it: a reader and a sender, the reader's community with 260 messages in `general`, `probe-voice`, and `scratch`, and the sender's community the reader is invited to.
3. It installs the app afresh, so no session from an earlier run is kept, and adds a picture to the photo library.
4. It answers the tests while they run (`ASPEN_TEST_HELPER`, below).
5. It fails unless every test ran and passed. A skipped test fails the run.

### What the tests read

The tests read these variables, which `xcodebuild test` passes on with the `TEST_RUNNER_` prefix. The wrapper gives them all. Without them, the tests skip.

| Variable | Use |
| --- | --- |
| `ASPEN_TEST_SERVER` | The deployment, entered when the app asks for one |
| `ASPEN_TEST_USER` | The user to sign in as |
| `ASPEN_TEST_PASSWORD` | That user's password |
| `ASPEN_TEST_CHANNEL` | The text channel read back through and measured, with enough history to read back through; "general" when unset |
| `ASPEN_TEST_VOICE_CHANNEL` | The voice channel whose call is joined; "probe-voice" when unset |
| `ASPEN_TEST_SCRATCH_CHANNEL` | The text channel the tests write in; "scratch" when unset |
| `ASPEN_TEST_INVITE` | An `aspen://app/invite/…` link to a community the user is not in |
| `ASPEN_TEST_INVITE_COMMUNITY` | That community's name |
| `ASPEN_TEST_HELPER` | The wrapper's helper, on loopback |

Signing in allows the system's notification alert, so the phone registers for push as a person's would.

### The helper

| Request | What the wrapper does |
| --- | --- |
| `POST /say` | Has the sender write in the scratch channel |
| `POST /notify` | Has the sender message the reader until a push wakes them. It checks that the push reached the app's own subscription at the stand-in, then shows the notification as the service extension would |

Both answer with the text written.

**The simulator never runs the service extension** (see `push.md`), and the app's push keys stay in the simulator's keychain. So the wrapper keeps a second subscription of the reader's on the Mac, with keys of its own. The extension's own code (`check_notification.sh`) reads the push to that subscription, and `xcrun simctl push` delivers the notification it makes. The tap the test makes then reaches the app as a tap on the extension's notification would.

## The tests

| Test | What it checks |
| --- | --- |
| `testSignsIn` | Names the deployment and signs in |
| `testReadingBackMovesOnlyWithTheFinger` | Safari's own answer to `historyScroll.spec.ts` |
| `testReadingBackQuicklyNeverJumps` | Reads back at a reader's pace, with flicks; the list counts its own jumps, so the web build needs `VITE_SCROLL_DEBUG=1` |
| `testJoinsAndLeavesACall` | Joins a call, with the microphone the simulator takes from the Mac |
| `testTextFollowsLargerText` | Draws a channel's name over twice as tall at Larger Text's Accessibility Extra Large, with Settings still on the screen |
| `testOpensATappedNotification` | Opens a message from its notification, tapped as its banner comes down, with the push woken through the stand-in |
| `testSendsAMessageFromTheKeyboard` | Keeps the message box above the keyboard, and sends what is typed there |
| `testCatchesUpAfterTheBackground` | Shows what was written while the app was suspended, once it is back, without a reload |
| `testOpensAnInviteLink` | Opens an `aspen://app/invite/…` link from outside the app on its invite |
| `testReactsFromALongPress` | Opens a message's actions with a held finger, reacts from the sheet, and takes the reaction back |
| `testSendsAPictureFromThePhotoLibrary` | Attaches a picture through the system's photo picker, describes it, and sends it |

Only a device shows the extension launched for a push, and its replacement of the placeholder.
