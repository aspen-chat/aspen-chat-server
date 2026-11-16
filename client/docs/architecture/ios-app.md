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
engine's platform, the system's call integration (CallKit), the audio routes, the Capacitor plugin
the web view commands it through and reads its state from (`AspenCallPlugin`;
`voice/native-calls.md` is the page's side), the call's video: the camera, the pictures drawn
beneath the web view, and the picture in picture (Video below), the call's sounds, played on the
call's session (Sounds below), and a DM's call ringing the phone (PushKit, see Rings below).

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

| File                       | Implements                                                                   | Over                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| -------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CallEngine.swift`         | The assembly                                                                 | Starts an `Engine` for one deployment with the parts below; one WebRTC factory and one track registry for the process                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `CallSnapshots.swift`      | `CallObserver`                                                               | Keeps the latest `CallState` and calls each listener on the main queue; hands each sound the engine cues (`playSound`) to `CallSounds`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `CallHttp.swift`           | `Http`                                                                       | URLSession: no cookies, no redirects followed, every status answered as a response                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `CallSockets.swift`        | `SocketFactory`, `Socket`                                                    | `URLSessionWebSocketTask`, which answers pings itself and offers `permessage-deflate` on every socket of its own accord (no switch; seen in the upgrade request CFNetwork sends), so the voice server's offers arrive compressed (`negotiatedExtensions` is the server's answer, `permessage-deflate; server_no_context_takeover; client_no_context_takeover` in the proof); `connect` waits for the upgrade, so a refusal is a `TransportError`                                                                                                                                                                                                                                      |
| `CallCredentials.swift`    | `Credentials`                                                                | The account at the deployment among those `PushStateStore` keeps (the keychain item the web view writes on sign-in): its session token for each request, renewed through `TokenRefresh` (shared with the notification service extension) and written back when the API server refuses it                                                                                                                                                                                                                                                                                                                                                                                              |
| `CallTracks.swift`         | The track ids the engine passes around                                       | WebRTC tracks by their own `trackId`: the microphone, and each receiver's track                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallPeerConnection.swift` | `PeerConnectionFactory`, `PeerConnection`, `PeerConnectionObserver`'s caller | `RTCPeerConnection`, unified plan, `max-bundle`, RTCP muxed. Transceivers are numbered in the order first seen and told apart by their sender's id, since `transceivers` wraps the native ones anew on every read; a sender's parameters object is kept between `getParameters` and `setParameters`; a receiver's volume is its audio track's source's; a statistics report is written as the W3C array with `id`, `type`, and `timestamp` in milliseconds beside each entry's values                                                                                                                                                                                                 |
| `CallAudio.swift`          | The audio session, the microphone, and the sounds' turn                      | `RTCAudioSession` in manual mode (`useManualAudio`): the app configures the session for a call (play and record, voice chat, Bluetooth HFP and A2DP) and never activates it; `activated(_:)` and `deactivated(_:)` are what CallKit's `didActivate` and `didDeactivate` call, and WebRTC's audio units run between them (`isAudioEnabled`), and not through an interruption (`interrupted`, `resumed`); `CallSounds` is told the same, and plays only within them. The microphone is a track from WebRTC's audio device module                                                                                                                                                        |
| `CallSounds.swift`         | The call's sounds                                                            | One `AVAudioPlayer` per cue (two while one plays) on the recordings bundled as the folder `aspen-sound-effects-pack`, played on the call's session only while the system has it active; see Sounds below                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `CallSystem.swift`         | The call as the system knows it                                              | One `CXProvider` for the process and the one call it holds at a time, whichever deployment's engine runs it; see The system's call below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `CallRoutes.swift`         | What the call plays through                                                  | `AVAudioSession`'s current route and available inputs as typed entries, the choice held for the call, and the proximity sensor; see Routes below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `CallVideo.swift`          | Where video is drawn                                                         | A view beneath the web view with one `AVSampleBufferDisplayLayer` renderer per hole the page reports, each drawing its surface's track, and the system's picture in picture on one of them; see Video below                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `CallCamera.swift`         | The camera                                                                   | `RTCCameraVideoCapturer` into one `RTCVideoSource` and track for the process, listed with each camera's position, moved between cameras with the track kept; see Video below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `AspenCallPlugin.swift`    | The web view's way in                                                        | `AspenCall`: each deployment's engine from `CallEngines`, followed for the page from the first time it names the origin; `join` asks the system for the microphone and opens it (`CallEngine.askForMicrophone`, the one way a join is given its microphone, which an answered ring takes too) and joins through `CallSystem`; each snapshot's JSON (`CallSnapshots.listenJSON`) is a `state` event and `state()`'s answer, untouched; the routes are `routes` events, `setRoute` and `clearRoute`; the cameras, `startCamera`, `switchCamera`, `stopCamera`, `clearCameraError`, `setWatched`, and `setTiles`, with `shape` and `pip` events back; registered in `MainViewController` |
| `CallEngines.swift`        | The engines of the process, one per deployment                               | `CallEngines.shared.forOrigin(origin)` makes a `CallEngine` for a deployment on first use and keeps it, so an incoming call answered (`CallSystem`) and the plugin the web view commands drive the same engine                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `CallRings.swift`          | A DM's call ringing the phone                                                | PushKit's registry and VoIP token, registered with the relay beside the alert token, the VoIP push handler that reports the call to CallKit before returning, and declining at the deployment; see Rings below                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |

`CallEngine.join`, `leave`, and `setMuted` go through `CallSystem`, so that the system's own
controls and the engine agree; the engine's other commands are called directly. `CallSnapshots`
keeps each snapshot as the record and as the JSON the engine writes beside it
(`CallObserver.stateChanged(state:json:)`), which the plugin alone reads.

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
`CXSetMutedCallAction` so the system's controls show the same. The system's hold is the engine's:
`CXSetHeldCallAction` calls `setHeld`, and so does an interruption, so the engine's call is on hold
exactly while `Status.held` or `Status.interrupted` stands (`applyHold`, which asks the engine only
when that changes). The voice server then pauses everything the user sends and sends them nothing,
keeps their own mute and deafen for the hold's end, and tells the others, who see the user on hold
rather than muted (`CallState.held` is the user's own, each participant's `held` the others'; the
engine shows the hold at once and clears it only on the server's word). A mute asked while held is
the user's own, kept beside the hold. The peer connection stays up and the microphone track stays
open through a hold; the audio session's handling is unchanged, `didActivate` resuming the audio.

Interruptions follow CallKit's rules: the app never activates the session. Before iOS 27 it watches
`interruptionNotification`; from iOS 27, `didBecomeInactiveNotification` with the system as source
and `resumptionRecommendationNotification`, which replace it. On an interruption's start WebRTC's
audio is stopped (`isAudioEnabled = false`), `Status.interrupted` set, and the engine's call put on
hold, since the user can neither hear nor be heard; a phone call answered on top of ours also holds
it through `CXSetHeldCallAction` and deactivates the session through `didDeactivate`, and when it
ends CallKit activates the session again and `didActivate` resumes the call, the hold ending with
the interruption unless the system still holds the call. An interruption that ends with the session
still active in the system's eyes (no `didDeactivate` came: a short one, Siri's) has WebRTC's audio
started again on it, and the hold ends with it. WebRTC's own `RTCAudioSession` also reactivates the
session itself on an interruption's end while its activation count is above zero, which is the
library's doing and not the app's.

`Info.plist` declares the `audio` and `voip` background modes, so a call goes on with the app in the
background and, later, VoIP pushes reach it. `providerDidReset` leaves the engine and stops WebRTC's
audio, since CallKit has forgotten the call.

### Sounds

The call's sounds are the engine's cues (`CallObserver.playSound`, derived in `voice_client::sounds`
from the call's own transitions; `voice/native-calls.md`, Sounds), played by `CallSounds`: each cue
is a recording of the Aspen Sound Effects Pack, the web client's own files, which the App target
bundles as a folder reference to
`client/packages/app/src/features/notifications/sounds/aspen-sound-effects-pack` (so the two never
differ, and the pack's `LICENSE` and `NOTICE` ride along in the bundle), played by an
`AVAudioPlayer`, which renders into the shared session as CallKit configured and activated it for
the call (play and record, voice chat). So a sound takes the route the call plays on (the receiver,
the loudspeaker, a headset), the call's volume, and plays with the app in the background, mixed with
the voices.

Nothing in it activates the session: `AVAudioPlayer.play` would, were the session inactive, and
CallKit's audio is CallKit's to activate. A sound plays only between `CallAudio.activated` and
`deactivated`, and not while another session interrupts ours (`interrupted`, `resumed`); a cue that
arrives before the activation (the engine connecting before CallKit has the session up) waits for it
up to `activationWait` (2 s, the web client's `START_LIMIT_MS`, past which a held sound is stale)
and is dropped past that, and one arriving while the session is inactive is dropped too; a
deactivation stops a sound under way. At most two of one cue play at once (`maxPlayingPerCue`, the
web client's `MAX_PLAYING_PER_SOUND`), a third not being heard; the engine spaces repeats of a cue
itself. `play` takes the cue on the engine's thread and hops to the main queue, where the activation
arrives too. The player is behind `CallSoundPlayer` (`CallSounds.makePlayer`), which the simulator's
proof replaces with a player in software (`AppTests/SoftwareSoundPlayer.swift`): the simulator's
audio server starts no output for a test process (`HALC_ProxyIOContext::_StartIO` fails after the
HAL's 30-second waits, each `AVAudioPlayer.play` blocking the main queue for them), so what the
simulator proves is which cue plays which bundled file, only on an active session, and at most two
at once; the sound itself is the device's to show.

### Rings

A DM's call ringing the phone (`spec/push.md`, Calls and Rings) arrives as a VoIP push through
PushKit, which wakes the app itself, not the notification service extension, and requires the call
reported to CallKit before the handler returns; an app that reports nothing is terminated. In order:

1. `CallRings.shared.start()` at launch asks PushKit for VoIP pushes. The VoIP token PushKit gives
   is kept in `UserDefaults` and registered with the relay this build names (`AspenPushRelay`) as
   `voipToken` of the device the web view registered (`PushState.device`, which `PushStateStore` now
   types), by `PATCH /v1/devices/{device}`, once per token and device: when the token arrives, and
   again whenever the web view keeps a new `PushState` (`AspenPushPlugin.saveState`), whose device
   may be new. The web view's own registration path is untouched.
2. A VoIP push is handed to `CallRings.handle`, on the main queue, by the delegate method of the iOS
   at hand: the one of iOS 26.4 and later carries `PKVoIPPushMetadata.mustReport`, false when the
   app is in the foreground, already in a call, or the push came too late, and the handler then
   completes the push and shows nothing (the web view's own event stream shows the ring where it
   matters); before 26.4 every VoIP push must be reported. `CallRings.read` decrypts the push's
   `aspen` (`s`, `c`) with the keys of the account `PushStateStore` keeps for the subscription, as
   the extension decrypts an alert push, and reads the `call` pointer: the channel, the session, the
   caller's name, and `until`.
3. `CallSystem.reportIncoming` reports the call synchronously (`reportNewIncomingCall`, the caller's
   name as `localizedCallerName`, the channel as the handle), and starts the engine's ring watch
   (`RingWatch`, `voice_client/src/ring.rs`) over `CallHttp`, `CallSockets`, and `CallCredentials`
   for the ring's deployment. One incoming call rings at a time; a second, or one the system refuses
   (Do Not Disturb, another app's call), is reported and ended at once. A push that cannot be read
   (`CallRings.Unreadable`) is reported as a call with no name and ended at once with `failed`
   (`reportUnreadable`), as Apple requires.
4. The watch's end reaches `CallSystem.ringEnded` on the main queue and the call is reported ended
   with the reason: `answeredElsewhere`, `declinedElsewhere` for a decline on another device,
   `remoteEnded` for the call ending, `unanswered` at the ring's `until` on the deployment's clock,
   `failed` when the account has no session. Nothing is pushed to end a ring.
5. **Answer** (`CXAnswerCallAction`): the watch stops, `CallEngines.shared.forOrigin(origin)` gives
   the deployment's engine, the one the web view's plugin commands too, so the page shows the
   answered call as any call; the audio session is configured as for a start and the action is
   fulfilled; CallKit then activates the session and `didActivate` starts the audio as for any call.
   No interface commands the engine at a ring answered from the lock screen, so the answer asks the
   system for the microphone as the plugin's `join` does (`CallEngine.askForMicrophone`, the one
   path both take): granted, the microphone is opened and set on the engine; refused, the engine is
   given none and the join fails with `errorKind` `microphone`, which ends the system's call as
   `failed`. The engine joins the ring's channel (which ends the ring at the deployment) once the
   system has answered, which is at once when it was asked before and otherwise when the app is next
   in front, past the time the system allows an action, which is why the action is fulfilled first;
   a call the answer made that is gone by then (the system ended it, or a start on another engine
   left it) joins nothing. The call is held as `Current` with `outgoing` false, so it is never
   reported connecting or connected (its answer is that); everything else, mute, hold, end, and the
   engine's end, is as for an outgoing call. A call of another engine the system held is left first.
6. **Decline** (`CXEndCallAction` on the ringing call): the watch stops and
   `CallRings.shared.decline` sends `DELETE /channels/{channel}/voice/rings/@me` as the account
   (renewing its session once on `401`), so the person's other devices stop ringing too.

`Status.incoming` is the ringing call's id, for the interface, and `Status.ringEnd` why the last
incoming call stopped ringing without this phone answering or declining it (`nil` while one rings,
and when this phone ended it). The ringtone is CallKit's default: Aspen has no ringtone file of its
own yet. No entitlement beyond push (`aps-environment`) is needed for VoIP pushes; the `voip`
background mode in `Info.plist` is.

What only a device can show, since the simulator receives no pushes and PushKit gives it no token:

- [ ] PushKit gives a VoIP token on launch and the relay's `PATCH` is sent for it, once, and again
      after signing in to another deployment registers a new device.
- [ ] A real VoIP push from the relay wakes the app from not running, locked, and the incoming call
      shows on the lock screen with the caller's name, ringing with the system's ringtone, within a
      second or two of the ring at the deployment.
- [ ] Answer from the lock screen joins the DM's call with audio both ways through `didActivate`;
      Decline ends the ring on every other device; a ring answered or declined on another device
      ends the lock screen's ring with the matching reason in Recents' absence (calls are kept out
      of Recents).
- [ ] The ring ends by itself at thirty seconds when no one answers, and when the caller hangs up.
- [ ] On iOS 26.4 or later, a push arriving while the app is in the foreground or in a call is
      delivered with `mustReport` false and shows nothing native.
- [ ] A push the app cannot read (signed out meanwhile) is reported and ended at once, and the app
      is not terminated.
- [ ] A phone whose clock runs a minute ahead of the deployment's still rings for the whole ring
      (the watch gives the deployment's clock the say once `ready` arrives).

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
queue. The app's Speaker button reaches it through the plugin's `setRoute` and `clearRoute`, and
reads the list through `routes` (`voice/native-calls.md`, Where the call plays); the web view's own
audio has no route of its own.

### Video

The page draws no picture itself: every picture is a surface the phone draws beneath a transparent
web view, where the page leaves a hole (`voice/native-calls.md`, Video). Capacitor's view controller
has no container under its web view, so the window's root is `RootViewController`
(`App/RootViewController.swift`), which holds `CallEngine.video`'s view across the whole window and
`MainViewController` as a child over it; `MainViewController` makes its web view transparent
(`isOpaque` off, clear backgrounds on the web view, its scroll view, and its own view) as it loads.
The video view is black, so a hole shows black until its first frame, as a `<video>` with nothing to
play does; the status bar and the home indicator are still the web view controller's to decide.

- **Tiles** (`CallVideo.apply`, from the plugin's `setTiles`): one `SampleBufferRenderer` per hole
  the page reports, keyed by the page's tile id, made as a tile first appears and taken off its
  track as it goes, attached to the surface's track from `CallTracks` (again on every report, for a
  track that arrives after the tile; the reference is weak, so a track gone with its connection
  frees the renderer to draw one of the same id again). Each is laid out where the page says, the
  page's CSS pixels being the web view's points, converted into the video view's coordinates; a
  full-screen tile takes the view's own bounds, laid out again in `layoutSubviews`, so a turn needs
  no round trip through the page. Self previews (mirrored tiles) are ordered above the other
  pictures. A tile's `cover` is the layer's `resizeAspectFill`, otherwise `resizeAspect`.
- **The renderer** (`SampleBufferRenderer`, an `RTCVideoRenderer`): each frame WebRTC delivers is
  wrapped as a `CMSampleBuffer` marked to display immediately (WebRTC already paced it to its sound,
  as the voice server keeps a person's voice and camera in one stream) and enqueued on an
  `AVSampleBufferDisplayLayer`. The camera's and the hardware decoder's frames are NV12 pixel
  buffers shown as they are; the software decoder's I420 planes are written into an NV12 buffer from
  a pool. A frame's rotation (the camera's own frames carry the phone's; received frames arrive
  upright, the sender having turned them, since no orientation extension is negotiated) and a self
  preview's mirror are the layer's transform, with the layer's bounds swapped for a quarter turn so
  it still fills the tile's box. The picture's shape, upright, is reported as the first frame
  arrives and whenever it changes (`shape` events to the page).
- **The camera** (`CallCamera`, `CallEngine.camera`): one capturer and one `RTCVideoSource` and
  track for the process. `startCamera` asks the system for the camera (`AVCaptureDevice`'s
  authorization, as `join` asks for the microphone) and opens the camera chosen by `uniqueID`, or
  the front camera, through `RTCCameraVideoCapturer` in the largest format at most 1280 by 720 at up
  to 30 frames a second (the engine's encoding bounds what is sent); the track goes to the engine as
  its `camera` source. Where the system allows it (iOS 16 and later, an app with the VoIP background
  mode), the capture session is given multitasking camera access, so the camera stays on in a
  picture in picture; whether a shipping iOS grants it is on the checklist. `switchCamera` starts
  the capturer on the other camera with the track kept, so the engine's sender needs nothing. The
  camera is on only while its engine sends it: the plugin follows the snapshots and stops the
  capture when the call ends or the voice server refuses the camera, so the camera's light goes off
  with the picture. A failure to open is `denied`, `none`, or `failed` (`CallCameraFailure`), the
  plugin's rejection code and the page's `cameraError`. The capturer is behind `CallCameraCapturer`,
  which the simulator's proof, having no camera, implements in software
  (`AppTests/SyntheticVideoCapturer.swift`).
- **Picture in picture** (`CallPictureInPicture`): the tile the page marks `pictureInPicture` has
  its renderer's layer made the content source of an `AVPictureInPictureController`, which starts
  the picture in picture itself when the app leaves the screen
  (`canStartPictureInPictureAutomaticallyFromInline`), the content live with no timeline. The page
  hears it start and stop (`pip` events) and is otherwise unaffected: the system draws the picture.
  `AVPictureInPictureController.isPictureInPictureSupported()` is false on the simulator, so it is
  the device's to show.

The page's own motion is the design's known weak point: a picture follows its hole a frame or two
behind while the hole moves, since the page measures and reports rectangles after it has laid them
out. The call screen on a phone pages rather than scrolls for that reason (`voice/call-screen.md`,
On a phone). `scripts/check_mobile_call.py --record-trailing` measures it: the bridge's test
`testPictureFollowsAMovingHole` hides the page, moves a red-bordered box across the screen and back
(three passes of 200 points in 800 ms each) reporting its inside as a hole every animation frame
with the camera in software drawn in it, `xcrun simctl io booted recordVideo` records the screen
around it, and `scripts/measure_video_trailing.py` reads every recorded frame for how far the
picture's left edge is from the border's inside against the direction of motion. On the simulator
(an iPhone 18 Pro on the iOS 27 runtime, whose web view moved the box about 12 points a recorded
frame): 47 moving frames, the picture never more than 4.7 points from its hole either way, a median
of 4 points ahead of it and at worst 4.7 behind, which at the box's speed is a fraction of a frame
(a median of 5 ms ahead, at worst 30 ms behind in one frame), so the layer reaches the screen in the
same frame as the page's paint, a little before it as often as after. Judging it by eye on a device,
at a phone's own frame rate, is on the checklist.

### What only a device can show

The simulator's CallKit cannot hold an outgoing call: its daemon opens `facetime://?showInCallUI=1`
for every outgoing call, the simulator has no such app, and it ends a call whose UI it could not
open about half a second after the start action is fulfilled, with an end action the provider
performs (`callservicesd`'s log: `Failed to open URL facetime://?showInCallUI=1`, then
`disconnect reason: 55`). So the simulator shows the start action performed and the end action
handled, and nothing after; it plays through the Mac, with no receiver, headset, or cellular radio;
and its audio server starts no output for a test process, so the call's sounds are proved into
players in software. These stand to be checked on an iPhone:

- [ ] An outgoing call stands: started from the app in the foreground, it shows no system in-call UI
      over the app, is reported connected, and ends when the user leaves.
- [ ] CallKit activates the session for the call and `didActivate` starts WebRTC's built-in audio
      device module (the simulator runs on `ToneAudioDevice`): both sides hear each other, with the
      app in the foreground, in the background, and locked.
- [ ] The system's mute and hold actions (`CXSetMutedCallAction`, `CXSetHeldCallAction`) reach the
      engine: the others see the user on hold and hear nothing of them, the user hears nothing, and
      the hold's end brings both back with the user's own mute standing; the app's own mute shows in
      the system's controls.
- [ ] The receiver is listed, chosen, and heard at the ear; the proximity sensor turns the screen
      off there and back on when the call moves to the loudspeaker. The app's Speaker button does
      the same through the plugin (the simulator has no receiver port, so its route stays the
      loudspeaker when the receiver is chosen).
- [ ] The call's sounds (`CallSounds`): the joining is heard as the call connects and as someone
      else joins, the leaving as someone leaves and as the user leaves (CallKit deactivates the
      session soon after the end action, which stops a sound under way: whether the leaving is heard
      whole), a share starting, and the call lost and back (cut the network for a few seconds), each
      on the receiver, on the loudspeaker, and on a headset, with the app in the foreground, in the
      background, and locked, at the call's volume beside the voices.
- [ ] A cue that arrives before CallKit activates the session (the engine connecting first) plays as
      the session comes up, and one during a phone call that interrupts ours does not.
- [ ] A join with the microphone refused in Settings fails with `errorKind` `microphone` in the call
      bar, and one in a channel without Speak joins to listen.
- [ ] A Bluetooth headset is listed by kind with its name, chosen, and the call moves to it;
      choosing the loudspeaker brings it back; connecting a headset during a call takes the call and
      ends the choice; disconnecting it leaves the call on the choice made before.
- [ ] A wired headset does the same, and a car over Bluetooth is listed as `car`.
- [ ] A phone call answered on top of ours holds it (the others see it on hold), and when it ends
      ours resumes through `didActivate`, with the same route and off hold; a phone call declined
      leaves ours running.
- [ ] Siri, an alarm, and a timer interrupt, holding the call for their length, and give the session
      back.
- [ ] The system's call UI on the lock screen ends, mutes, and holds the call, and shows the name
      given to `join`.
- [ ] A call held for an hour with the screen locked stays connected.
- [ ] The camera: the front camera's preview is mirrored and upright however the phone is held, the
      back camera's is not mirrored, the flip moves between them with the picture pausing only for
      the camera's own start, and the other side sees the picture upright through a turn.
- [ ] The camera in the background: with the app behind another or the phone locked in a picture in
      picture, the camera stays on (multitasking camera access) and the others keep seeing it.
- [ ] The picture in picture: leaving the app while someone's camera or screen shows starts it on
      the focused screen (else the first camera of someone else), it follows the call as the picture
      shown changes, and coming back ends it; `isPictureInPicturePossible` and the `pip` events say
      so on a device, the simulator supporting none.
- [ ] Rotation: a full-screen picture turns with the phone without a flash of the old layout, and
      the paged tiles lay out again beneath their holes after a turn.
- [ ] Trailing by eye: while paging between tiles and while a transition moves one, the native
      picture follows its hole; frame-step a screen recording (`xcrun simctl io booted recordVideo`
      on the simulator, the device's own recording on a phone) and judge how far it trails and
      whether the paged grid hides it.

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
and unmutes, puts the call on hold and releases it (the browser's report, `onHold` in
`calls/page.ts`, lists the simulator's user meanwhile, neither side's packets reach the other, and
the release waits for the server's word), cuts its socket and watches the engine take its place back
with `resume`, the call connected throughout, and leaves; its log line also names what the
signalling socket negotiated. Between the hold and the cut it proves the video both ways: the
browser shows video and turns Chromium's fake camera on, the engine is asked to watch it, and the
track that arrives is drawn by `CallEngine.video` where the page would put its hole, the renderer
counting frames (at least five) and reporting an upright shape and its layer lying in the rectangle
given; then a camera in software (`SyntheticVideoCapturer`, 320 by 240 frames carried at 90 degrees
as a phone's upright camera's are) is opened through `CallCamera` on the test's own factory and sent
as the engine's camera, its preview drawn 240 by 320, and the browser decodes its frames
(`framesDecoded` in `calls/page.ts`'s report) and sees a picture in them (`cameraSample`: a frame
drawn to a canvas, brighter than black and taller than wide); the camera is moved to the other
synthetic camera with the track kept; video flows on across the resume; and the camera is stopped
before the leave. The picture in picture is started on the browser's camera where the system
supports one, and the simulator's does not, which the log line says. Through it all it watches the
call's sounds (`CallSounds.listen`): the joining is played as the engine connects and again as the
browser joins, the leaving as the call is left, and those three alone, since a socket taken back
within the grace keeps the call connected and is no loss (the new socket's frames, read through the
test's `WatchedSocket`, say the `resume` was presented and answered `ready`, and no snapshot showed
`rejoining`), and each is a player playing as the cue is given, on the session the test activated as
CallKit would (play and record, voice chat), which the log line names too; the players are in
software (`SoftwareSoundPlayer`, Sounds above), each having read its bundled recording, since the
simulator's audio server starts no output for a test process. `AppTests/CallRoutesTests` reads the
simulator's routes (its loudspeaker in use, a receiver listed for the iPhone idiom), the kind of
each port type, and the proximity rule, and holds and resets a choice.
`AppTests/AspenCallPluginTests` drives the bridge itself: the app under test is the app, whose
`MainViewController` has loaded the web shell with the plugin registered, so the test finds its
`WKWebView` and runs `Capacitor.registerPlugin("AspenCall")` in the page, as the app's `NativeCall`
does. From the page it reads a fresh snapshot (the engine's JSON, idle, revision 0, of its origin),
the routes (the receiver listed, the loudspeaker in use), chooses the loudspeaker and reads
`AVAudioSession.currentRoute` as the built-in speaker, chooses the receiver (the choice stands; the
simulator has no receiver port) and clears it, mutes and unmutes outside a call and sees each in a
snapshot, has malformed commands refused by name, reports whether the page's `AudioContext` runs
with the call's session active, and joins the channel through the plugin: the start action's
`joining` snapshot reaches the page, then the simulator's daemon ends the call and its `idle` does,
in revision order, with no snapshot twice, and a fresh `state()` is the latest published. Then,
since the simulator's CallKit holds no call, it joins the engine the plugin made directly
(`AspenCallPlugin.engine(at:)` is internal for it), the test standing in for `didActivate`, and
drives the video from the page: `cameras()` lists the two synthetic cameras with their facings,
`startCamera` opens the front one and the snapshot shows the camera among `sources`, a hole reported
with `setTiles` for its track has its shape come back as a `shape` event (240 by 320) and its layer
laid out in the rectangle given, `switchCamera` moves to the back one, a second participant from the
browser turns its camera on and `setWatched` has the snapshot show its track, whose hole hears its
shape too, malformed `setWatched` and `setTiles` are refused by name, and `stopCamera` takes the
source out of the snapshot and the capture off. Before any engine is made the test replaces
`CallEngine.peerConnections` with a factory on `ToneAudioDevice` and the camera's capturer with the
synthetic one, as the simulator's own audio unit and lack of a camera require.
`AppTests/CallRingsTests` feeds `CallRings.handle` a `call` pointer encrypted to a key it writes
into the kept push state, as PushKit would hand the payload over, naming a stand-in deployment of
the test's own (a loopback listener that keeps each request and serves nothing), and watches
`CXCallObserver` show the incoming call before the handler returns and the stand-in be asked for the
event stream by the ring watch; then, the simulator's CallKit ending an incoming call on its own
within a second or two (a decline, as far as the app can tell), it checks that the decline reaches
the stand-in as the account (`DELETE .../voice/rings/@me`, the kept session token), or, on a CallKit
that lets the call ring, that the watch ends it at the ring's clock and kept asking for the stream
meanwhile (`Status.ringEnd` says which); a push that does not decrypt is reported and ended at once;
one the system says need not be reported shows nothing. Its answer test performs the answer action
on the provider's delegate as CallKit delivers one (`CallSystem.provider` is internal for it), since
the simulator's CallKit may have ended the call before an answer requested of it arrives, with the
engines made on `ToneAudioDevice` as the bridge's test makes them: the ring stops, the system's call
is the answered one, and the engine's `joining` snapshot carries the microphone the answer opened
(the engine's own, `CallAudio.openMicrophone` answering the same track) and the ring's channel; the
stand-in is asked to join as the account, answers `404`, and the join's failure ends the system's
call. `scripts/check_mobile_call.py --platform ios` at the repository's root runs them end to end
from Linux, with the same deployment and browser the Android check uses (`android-builds.md`, The
call engine): a deployment of its own serving the call page (`client/packages/protocol/calls/`), SSH
reverse tunnels so the simulator reaches the API server and the voice server at the Mac's own
loopback (plain HTTP there, as a development build allows; media goes to the voice server's
announced address over the network), Chromium as the second participant with a tone WAV as its
microphone and its fake camera (`calls/second.ts`, which the script's helper drives: showing video,
the camera on and off, and a frame of someone's camera sampled), the simulator granted the camera
beside the microphone, and `xcodebuild test-without-building` on the Mac of the engine's, the
bridge's, and the rings' tests, with `--build` first building the engine, the bindings, and the app
for the simulator there. The test reads its deployment, channel, user, refresh token, and the
script's helper from the environment (`TEST_RUNNER_ASPEN_CALL_*`; the test's comment lists them) and
skips without them. CI runs the Android half of this check (`android-builds.md`) and not this one:
the deployment runs in Docker, which GitHub's macOS runners lack, so the simulator's call is checked
from a development machine with a Mac beside it.
