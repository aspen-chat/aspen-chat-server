# The iOS app

The iOS project (`packages/mobile/ios`) is kept, as Android's is. CI builds it, and signs it for the
App Store, as `apple-builds.md` describes. Its UI tests drive the app with real taps and finger
drags in WebKit, against a real deployment. They check what the browser specs cannot reach: the iOS
web view itself, the system's keyboard, photo picker, and notifications, links opened from outside
the app, and the app being suspended and resumed. The app also links the mobile call engine and the
Swift layer under it, which the second half of this page describes.

## Where it lives

| Part           | Location                                                  |
| -------------- | --------------------------------------------------------- |
| UI tests       | `ios/App/AppUITests`                                      |
| Unit tests     | `ios/App/AppTests`, run in the app's process              |
| The call layer | `ios/App/App/Call`                                        |
| Scheme         | `App`                                                     |
| Wrapper        | `ios/scripts/ui_tests.py`                                 |
| Relay stand-in | `scripts/push_relay_stand_in.py` at the repository's root |

## Running them

`ui_tests.py` runs them on a Mac with Xcode and a simulator:

```
scripts/push_relay_stand_in.py --app-host <host>                 # beside the deployment
client/packages/mobile/ios/scripts/ui_tests.py --server https://<deployment> \
    --relay https://<host>:7788 --trust target/dev-ca/ca.pem --build
```

The tests run a release build of the web app, which reaches everything over HTTPS (see
`message-rendering/address-safety.md`, Transport): the deployment, its storage, its voice servers,
and the stand-in, whose `--app-host` serves its app side over HTTPS with a certificate from the
development authority (`scripts/dev_ca.py`). `--server` is the deployment as the simulator reaches
it, and `--relay` the stand-in's app side. `--trust` has the simulator, and the wrapper, trust an
authority, such as the development one. `--api` is the deployment as the Mac reaches it, when that
differs (its plain port behind a TLS proxy): the extension's code run there trusts the system's
authorities alone. The deployment needs these settings:

| Setting                                                                    | Why                                                                                         |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `[federation.development] extra_root_certificates`, `target/dev-ca/ca.pem` | The stand-in's push side has a certificate from that authority                              |
| `[federation.development] allow_private_addresses = true`                  | The stand-in's push side is on loopback, as `push.localhost`                                |
| `[presence] away_after_seconds`, the wrapper's `--away-after` (15)         | A user who has just used Aspen is not woken; the test waits that long after leaving the app |
| `[media.s3] public_endpoint` and `public_base_url` over `https`            | The photo test uploads its picture and looks for it                                         |
| Its voice servers' `url` over `https`                                      | The call test joins one                                                                     |

The wrapper does the rest:

1. With `--build`, it builds the web app with `VITE_SCROLL_DEBUG=1` and the tests for the simulator,
   the app naming the stand-in as its relay. Install the client's dependencies first.
2. It seeds a fixture under `target/ios-ui-tests/` on first use, and reuses it while the deployment
   still has it: a reader and a sender, the reader's community with 260 messages in `general`,
   `probe-voice`, and `scratch`, and the sender's community the reader is invited to.
3. It installs the app afresh, so no session from an earlier run is kept, and adds a picture to the
   photo library.
4. It answers the tests while they run (`ASPEN_TEST_HELPER`, below).
5. It fails unless every test ran and passed. A skipped test fails the run.

### What the tests read

The tests read these variables, which `xcodebuild test` passes on with the `TEST_RUNNER_` prefix.
The wrapper gives them all. Without them, the tests skip.

| Variable                      | Use                                                                                                             |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `ASPEN_TEST_SERVER`           | The deployment, entered when the app asks for one                                                               |
| `ASPEN_TEST_USER`             | The user to sign in as                                                                                          |
| `ASPEN_TEST_PASSWORD`         | That user's password                                                                                            |
| `ASPEN_TEST_CHANNEL`          | The text channel read back through and measured, with enough history to read back through; "general" when unset |
| `ASPEN_TEST_VOICE_CHANNEL`    | The voice channel whose call is joined; "probe-voice" when unset                                                |
| `ASPEN_TEST_SCRATCH_CHANNEL`  | The text channel the tests write in; "scratch" when unset                                                       |
| `ASPEN_TEST_INVITE`           | An `aspen://app/invite/…` link to a community the user is not in                                                |
| `ASPEN_TEST_INVITE_COMMUNITY` | That community's name                                                                                           |
| `ASPEN_TEST_HELPER`           | The wrapper's helper, on loopback                                                                               |

Signing in allows the system's notification alert, so the phone registers for push as a person's
would.

### The helper

| Request        | What the wrapper does                                                                                                                                                                             |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /say`    | Has the sender write in the scratch channel                                                                                                                                                       |
| `POST /notify` | Has the sender message the reader until a push wakes them. It checks that the push reached the app's own subscription at the stand-in, then shows the notification as the service extension would |

Both answer with the text written.

**The simulator never runs the service extension** (see `push.md`), and the app's push keys stay in
the simulator's keychain. So the wrapper keeps a second subscription of the reader's on the Mac,
with keys of its own. The extension's own code (`check_notification.sh`) reads the push to that
subscription, and `xcrun simctl push` delivers the notification it makes. The tap the test makes
then reaches the app as a tap on the extension's notification would.

## The tests

| Test                                      | What it checks                                                                                                                                                              |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `testSignsIn`                             | Names the deployment and signs in                                                                                                                                           |
| `testShowsOnlyTheNoticeToAnUnderageUser`  | Shows only the age gate's notice when the simulator is told the user is underage (`-AspenAgeCheck underage`)                                                                |
| `testReadingBackMovesOnlyWithTheFinger`   | Safari's own answer to `historyScroll.spec.ts`                                                                                                                              |
| `testReadingBackQuicklyNeverJumps`        | Reads back at a reader's pace, with flicks; the list counts its own jumps, so the web build needs `VITE_SCROLL_DEBUG=1`                                                     |
| `testJoinsAndLeavesACall`                 | Joins a call, with the microphone the simulator takes from the Mac                                                                                                          |
| `testTextFollowsLargerText`               | Draws a channel's name over twice as tall at Larger Text's Accessibility Extra Large, with Settings still on the screen                                                     |
| `testOpensATappedNotification`            | Opens a message from its notification, tapped as its banner comes down, with the push woken through the stand-in                                                            |
| `testSendsAMessageFromTheKeyboard`        | Keeps the message box above the keyboard, and sends what is typed there                                                                                                     |
| `testEmojiKeyboardTakesTheKeyboardsPlace` | Swaps the keyboard for the emoji keyboard: the web view ends at the keyboard, and the emoji keyboard takes its height, edge to edge, without the box moving                 |
| `testKeyboardsLeaveTheMessagesInPlace`    | Brings the keyboard up, swaps it for the emoji keyboard and back, and puts it away, at the newest messages and read back: the messages stay where they belong at every step |
| `testCatchesUpAfterTheBackground`         | Shows what was written while the app was suspended, once it is back, without a reload                                                                                       |
| `testOpensAnInviteLink`                   | Opens an `aspen://app/invite/…` link from outside the app on its invite                                                                                                     |
| `testReactsFromALongPress`                | Opens a message's actions with a held finger, reacts from the sheet, and takes the reaction back                                                                            |
| `testSendsAPictureFromThePhotoLibrary`    | Attaches a picture through the system's photo picker, describes it with the description dialog's Save above the keyboard, and sends it                                      |

Only a device shows the extension launched for a push, and its replacement of the placeholder.

## The call engine

The mobile apps' calls run natively, in the call engine (`voice_client/` at the repository's root, a
Rust crate: `voice_client/src/lib.rs` describes it), which drives the platform's WebRTC, HTTP, and
WebSockets through interfaces each platform implements. The iOS implementation is
`ios/App/App/Call`, compiled into the App target with the engine's generated Swift module: the
engine's platform, the system's call integration (CallKit), and the audio routes. Nothing in the web
view uses it yet: the Capacitor plugin that will command it and show its state, the call's sounds,
rings (PushKit), and the camera are later pieces.

### What the App target links, and where it comes from

| Piece                                                                                 | Where the project reads it                                                                                 | Made by                                                                                              |
| ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Google's WebRTC, `WebRTC.xcframework`                                                 | `target/webrtc/`, linked and embedded                                                                      | `scripts/webrtc_build.py build` on a Mac, once (`docs/development/webrtc-build.md`)                  |
| The engine, `libvoice_client.a`                                                       | `target/aarch64-apple-ios-sim/release/` for the simulator, `target/aarch64-apple-ios/release/` for devices | `ios/scripts/call_engine.py` (`cargo rustc --crate-type staticlib` for each target, against its SDK) |
| Its Swift module: `VoiceClient.swift`, `VoiceClientFFI.h`, `VoiceClientFFI.modulemap` | `ios/VoiceClient/`, gitignored, compiled into the App target                                               | `scripts/voice_client_bindings.py --language swift` (`voice_client/README.md`)                       |

`ios/scripts/call_engine.py` builds the engine for both targets (or `--simulator-only`), writes the
bindings, and says when the framework is missing. The paths are the project's own build settings
(library search paths by SDK, `-lvoice_client`, and the module map given to Swift with
`-fmodule-map-file`), relative to the repository, so cargo must write to its default target
directory. Both pieces have arm64 slices alone, so simulator builds exclude x86_64
(`EXCLUDED_ARCHS`), and an Intel Mac cannot build the app. The App and AppTests targets both take
the module map, since the test imports the app's module, whose interface names the FFI module;
AppTests links the framework for the same reason and loads the engine's symbols from the app.

### The layer

Each file under `Call/` implements one of the engine's interfaces over the platform, as
`voice_client/src/bindings.rs` declares them:

| File                       | Implements                                                                   | Over                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| -------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CallEngine.swift`         | The assembly                                                                 | Starts an `Engine` for one deployment with the parts below; one WebRTC factory and one track registry for the process                                                                                                                                                                                                                                                                                                                                                                 |
| `CallSnapshots.swift`      | `CallObserver`                                                               | Keeps the latest `CallState` and calls each listener on the main queue                                                                                                                                                                                                                                                                                                                                                                                                                |
| `CallHttp.swift`           | `Http`                                                                       | URLSession: no cookies, no redirects followed, every status answered as a response                                                                                                                                                                                                                                                                                                                                                                                                    |
| `CallSockets.swift`        | `SocketFactory`, `Socket`                                                    | `URLSessionWebSocketTask`, which answers pings itself; `connect` waits for the upgrade, so a refusal is a `TransportError`                                                                                                                                                                                                                                                                                                                                                            |
| `CallCredentials.swift`    | `Credentials`                                                                | The account at the deployment among those `PushStateStore` keeps (the keychain item the web view writes on sign-in): its session token for each request, renewed through `TokenRefresh` (shared with the notification service extension) and written back when the API server refuses it                                                                                                                                                                                              |
| `CallTracks.swift`         | The track ids the engine passes around                                       | WebRTC tracks by their own `trackId`: the microphone, and each receiver's track                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallPeerConnection.swift` | `PeerConnectionFactory`, `PeerConnection`, `PeerConnectionObserver`'s caller | `RTCPeerConnection`, unified plan, `max-bundle`, RTCP muxed. Transceivers are numbered in the order first seen and told apart by their sender's id, since `transceivers` wraps the native ones anew on every read; a sender's parameters object is kept between `getParameters` and `setParameters`; a receiver's volume is its audio track's source's; a statistics report is written as the W3C array with `id`, `type`, and `timestamp` in milliseconds beside each entry's values |
| `CallAudio.swift`          | The audio session and the microphone                                         | `RTCAudioSession` in manual mode (`useManualAudio`): the app configures the session for a call (play and record, voice chat, Bluetooth HFP and A2DP) and never activates it; `activated(_:)` and `deactivated(_:)` are what CallKit's `didActivate` and `didDeactivate` call, and WebRTC's audio units run between them (`isAudioEnabled`). The microphone is a track from WebRTC's audio device module                                                                               |
| `CallSystem.swift`         | The call as the system knows it                                              | One `CXProvider` for the process and the one call it holds at a time, whichever deployment's engine runs it; see The system's call below                                                                                                                                                                                                                                                                                                                                              |
| `CallRoutes.swift`         | What the call plays through                                                  | `AVAudioSession`'s current route and available inputs as typed entries, the choice held for the call, and the proximity sensor; see Routes below                                                                                                                                                                                                                                                                                                                                      |

`CallEngine.join`, `leave`, and `setMuted` go through `CallSystem`, so that the system's own
controls and the engine agree; the engine's other commands are called directly.

### The system's call

A call is represented to CallKit as an outgoing call: iOS has no model of a room people sit in for
hours, and an outgoing call started from the app is the nearest; whether it suits the lock screen,
cars, and a call held for hours is on the device checklist below. The provider is configured once
(`supportsVideo`, calls kept out of Recents, one call group of one call, generic handles; the
ringtone is the rings piece's). The handle is the channel's id, and the name the system shows is
whatever `join` was given as `name`. In order, on joining:

1. `CallEngine.join` asks `CXCallController` for a `CXStartCallAction`, and subscribes to the
   engine's snapshots for the call. A refusal (another app's call holds the one group, no CallKit)
   joins nothing, since there would then be no one to activate the audio session; `Status.refusal`
   carries the system's words.
2. In `perform(CXStartCallAction)` the audio session is configured (`CallAudio.configureForCall`),
   the engine is told to join, the call is reported as connecting, its name is reported, and the
   action is fulfilled.
3. CallKit activates the session and calls `didActivate`: `CallAudio.activated` tells WebRTC
   (`audioSessionDidActivate`, `isAudioEnabled = true`), and the standing route choice is applied,
   since an override set before activation is not kept.
4. When a snapshot shows the engine connected, the call is reported connected.

On leaving: `CallEngine.leave` asks for a `CXEndCallAction`; `perform` leaves the engine, lets the
call go, resets the routes, and fulfils; CallKit deactivates the session and `didDeactivate` stops
WebRTC's audio (`CallAudio.deactivated`). The system's own End does the same through the same
action. When the engine's state says the call is over without that (`idle` after it was in the call,
or `failed`), the call is reported ended with the reason: `failed` for a failure, otherwise
`remoteEnded` (a moderator's kick, access lost, the API server ending an idle call, a session the
API server ended). Joining another channel while the system holds this engine's call is a move the
engine makes itself, with the call's handle updated; a call of another deployment's engine is left
and reported ended first.

The system's mute action is the engine's `setMuted`; the app's own mute goes through a
`CXSetMutedCallAction` so the system's controls show the same. Hold is what the engine can do of it:
`CXSetHeldCallAction` mutes and deafens (the microphone's track leaves its sender, the server pauses
every audio consumer), and the hold's end puts back the mute and deafen the state showed at the
hold, which includes a moderator's mute. The engine has no held state of its own, so the interface
shows the user muted and deafened meanwhile; the peer connection stays up and the microphone track
stays open.

Interruptions follow CallKit's rules: the app never activates the session. Before iOS 27 it watches
`interruptionNotification`; from iOS 27, `didBecomeInactiveNotification` with the system as source
and `resumptionRecommendationNotification`, which replace it. On an interruption's start WebRTC's
audio is stopped (`isAudioEnabled = false`) and `Status.interrupted` set; a phone call answered on
top of ours also holds it through `CXSetHeldCallAction` and deactivates the session through
`didDeactivate`, and when it ends CallKit activates the session again and `didActivate` resumes the
call. An interruption that ends with the session still active in the system's eyes (no
`didDeactivate` came: a short one, Siri's) has WebRTC's audio started again on it. WebRTC's own
`RTCAudioSession` also reactivates the session itself on an interruption's end while its activation
count is above zero, which is the library's doing and not the app's.

`Info.plist` declares the `audio` and `voip` background modes, so a call goes on with the app in the
background and, later, VoIP pushes reach it. `providerDidReset` leaves the engine and stops WebRTC's
audio, since CallKit has forgotten the call.

### Routes

`CallRoutes` lists what the call can play through, each entry a kind from the port's type (receiver,
loudspeaker, Bluetooth for HFP, A2DP, and LE, wired headset for headphones, a headset's microphone,
and line out, car, other) with the system's name beside it for display alone, which is in use, and
whether the device has a receiver (an iPhone's idiom; an iPad has none). The list is the receiver
(on a phone) and the loudspeaker, every available input but the built-in microphone, and, when none
of those plays, the output that does (Bluetooth headphones without a microphone, AirPlay). The
receiver and loudspeaker are chosen by overriding the output port (preferring the built-in
microphone first, which brings the call back from a headset), a headset or car by preferring its
input. The choice holds for the call: an override lasts only until the next route change, so it is
applied again on every `routeChangeNotification` and when CallKit activates the session; a new
device arriving (`newDeviceAvailable`) takes the call, as the Phone app's does, and ends the choice;
a device going away leaves it. The proximity sensor is on while the route in use is the receiver and
off otherwise, and the choice is let go when the call ends. Listeners hear every change on the main
queue.

### What only a device can show

The simulator's CallKit cannot hold an outgoing call: its daemon opens `facetime://?showInCallUI=1`
for every outgoing call, the simulator has no such app, and it ends a call whose UI it could not
open about half a second after the start action is fulfilled, with an end action the provider
performs (`callservicesd`'s log: `Failed to open URL facetime://?showInCallUI=1`, then
`disconnect reason: 55`). So the simulator shows the start action performed and the end action
handled, and nothing after; and it plays through the Mac, with no receiver, headset, or cellular
radio. These stand to be checked on an iPhone:

- [ ] An outgoing call stands: started from the app in the foreground, it shows no system in-call UI
      over the app, is reported connected, and ends when the user leaves.
- [ ] CallKit activates the session for the call and `didActivate` starts WebRTC's built-in audio
      device module (the simulator runs on `ToneAudioDevice`): both sides hear each other, with the
      app in the foreground, in the background, and locked.
- [ ] The system's mute and hold actions (`CXSetMutedCallAction`, `CXSetHeldCallAction`) reach the
      engine, and the hold's end puts the mute and deafen back; the app's own mute shows in the
      system's controls.
- [ ] The receiver is listed, chosen, and heard at the ear; the proximity sensor turns the screen
      off there and back on when the call moves to the loudspeaker.
- [ ] A Bluetooth headset is listed by kind with its name, chosen, and the call moves to it;
      choosing the loudspeaker brings it back; connecting a headset during a call takes the call and
      ends the choice; disconnecting it leaves the call on the choice made before.
- [ ] A wired headset does the same, and a car over Bluetooth is listed as `car`.
- [ ] A phone call answered on top of ours holds it (muted and deafened), and when it ends ours
      resumes through `didActivate`, with the same route; a phone call declined leaves ours running.
- [ ] Siri, an alarm, and a timer interrupt and give the session back.
- [ ] The system's call UI on the lock screen ends, mutes, and holds the call, and shows the name
      given to `join`.
- [ ] A call held for an hour with the screen locked stays connected.

### The proof

`AppTests/CallEngineTests` runs the engine with this layer against a real deployment, in the app's
process on the simulator, WebRTC on a microphone and speaker in software (`ToneAudioDevice`, an
`RTCAudioDevice` recording a warble of 440 and 660 Hz and dropping what it plays out), since the
simulator's own audio unit aborts the process when a test initializes it: it joins a voice channel's
call through the system and watches the start action join the engine, the call listed as outgoing
(`CXCallObserver`), and the simulator's daemon end it through the provider, which leaves the engine;
then, driving the engine directly with the test standing in for `didActivate`, it joins, has a
second participant brought in from a browser, reads both sides' statistics for the other's sound
(the browser's as the loudest sample off its received tracks, `loudness` in `calls/page.ts`, since
Chromium's own level statistics read zero behind the muted element it plays each track in), mutes
and unmutes, cuts its socket and watches the engine take its place back with `resume`, and leaves.
`AppTests/CallRoutesTests` reads the simulator's routes (its loudspeaker in use, a receiver listed
for the iPhone idiom), the kind of each port type, and the proximity rule, and holds and resets a
choice. `scripts/check_mobile_call.py --platform ios` at the repository's root runs them end to end
from Linux, with the same deployment and browser the Android check uses (`android-builds.md`, The
call engine): a deployment of its own serving the call page (`client/packages/protocol/calls/`), SSH
reverse tunnels so the simulator reaches the API server and the voice server at the Mac's own
loopback (plain HTTP there, as a development build allows; media goes to the voice server's
announced address over the network), Chromium as the second participant with a tone WAV as its
microphone (`calls/second.ts`), and `xcodebuild test-without-building` on the Mac, with `--build`
first building the engine, the bindings, and the app for the simulator there. The test reads its
deployment, channel, user, refresh token, and the script's helper from the environment
(`TEST_RUNNER_ASPEN_CALL_*`; the test's comment lists them) and skips without them.
