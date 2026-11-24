# Android builds in CI

CI's `android` job in `.github/workflows/ci.yml` builds the Android app, runs its JVM tests and its
device tests on an emulator, and keeps the APK it built as the artifact `android-apk-debug` on every
pull request and every push to `main`, for 14 days. The app also packages the mobile call engine and
the Kotlin layer under it, which the second half of this page describes: the job builds the engine
for every ABI on the WebRTC archive the `webrtc-android` job built or took from its cache
(`docs/development/webrtc-build.md`), checks its Kotlin bindings on the JVM
(`voice_client/bindings-check/kotlin.sh`), and checks that the APK carries the engine and WebRTC for
each ABI; a second job, `android-call`, runs the engine's call on an emulator (The call engine).

## The APK

It is the debug build, signed with the runner's own debug key, with no push: it names no relay and
carries no `google-services.json` (`push.md`). It is one APK for every ABI: with the call engine,
its library and WebRTC's for `arm64-v8a`, `armeabi-v7a`, and `x86_64`; without it, no native code at
all. The artifact is uploaded before the device tests run, so a failing emulator still leaves it to
install.

Install it on a device with USB debugging on:

```
unzip android-apk-debug.zip && adb install -r app-debug.apk
```

Each runner makes a debug key of its own, so an APK from one run installs over another's only after
the one installed is removed (`adb uninstall com.aspen_chat.client`).

## Versions

`app/build.gradle` reads two environment variables, which the job sets:

| Variable             | Becomes                                                      | Unset |
| -------------------- | ------------------------------------------------------------ | ----- |
| `ASPEN_BUILD_NUMBER` | `versionCode` (the workflow's run number)                    | `1`   |
| `ASPEN_BUILD_COMMIT` | build metadata on `versionName`, its first twelve characters | none  |

`versionName` is `packages/mobile/package.json`'s version, so a CI build reads `0.1.0+ce1a1be2abcd`
in About Aspen (`about.md`), and names the commit it was built from. A later run's `versionCode` is
higher, so a build signed with the same key as the one installed updates it in place.

The app runs on Android 8.0 and newer (`minSdkVersion` 26 in `variables.gradle`): a call the native
engine holds is the app's own call in Telecom (`androidx.core:core-telecom`, which needs 8.0, a
self-managed `ConnectionService` beneath it there and Telecom's transactional calls from Android
14), and WebRTC's audio output takes its low-latency mode from the same version.

## The call engine

The mobile apps' calls run natively, in the call engine (`voice_client/` at the repository's root, a
Rust crate: `voice_client/src/lib.rs` describes it), which drives the platform's WebRTC, HTTP, and
WebSockets through interfaces each platform implements. The Android implementation is
`app/src/main/java/com/aspen_chat/client/call/`, Kotlin, compiled into the app with the engine's
generated Kotlin package (`com.aspen_chat.client.voice`): the engine's platform, the system's call
(Telecom), the routes, the files its transfers read and write, the Capacitor plugin the web view
commands it through and reads its state from (`AspenCallPlugin`; `voice/native-calls.md` is the
page's side), the call's video: the camera, the pictures drawn behind the web view, and the
picture-in-picture mode (Video below), the share of the screen with the sound of other apps (Sharing
the screen below), the call's sounds, played as a voice communication (Sounds below), and a DM's
call ringing the phone (Rings below).

### What the app packages, and where it comes from

| Piece                                                                                                     | Where the Gradle build reads it                                                                                                                                                                                                   | Made by                                                                                                                                                                                             |
| --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Google's WebRTC, `libwebrtc.aar` (`org.webrtc`, its library for `arm64-v8a`, `armeabi-v7a`, and `x86_64`) | `target/webrtc/`, as a file dependency                                                                                                                                                                                            | `scripts/webrtc_build.py build` on Linux, once (`docs/development/webrtc-build.md`), with `scripts/webrtc_patches/` applied: the audio device module's `AudioRecord` factory, for the share's sound |
| The engine, `libvoice_client.so`                                                                          | `target/x86_64-linux-android/release/` for the emulator, `target/aarch64-linux-android/release/` and `target/armv7-linux-androideabi/release/` for devices, laid out by ABI into the APK's `jniLibs` by the task `copyCallEngine` | `android/scripts/call_engine.py` (`cargo rustc --crate-type cdylib` for each target, linked with the NDK's clang)                                                                                   |
| Its Kotlin package, `voice_client.kt`                                                                     | `app/src/main/java/com/aspen_chat/client/voice/`, gitignored                                                                                                                                                                      | `scripts/voice_client_bindings.py --language kotlin` (`voice_client/README.md`)                                                                                                                     |
| JNA (`net.java.dev.jna:jna`, the `aar`), kotlinx-coroutines, OkHttp                                       | Maven Central; the versions are in `variables.gradle`                                                                                                                                                                             | The bindings load the engine's library through JNA and suspend through coroutines; the layer's HTTP and WebSockets are OkHttp's, which the app did not have before                                  |
| `androidx.core:core-telecom` 1.1.0                                                                        | Google's Maven, only when the layer is built; the version is in `variables.gradle`                                                                                                                                                | The newest stable release, the first with `registerAppWithTelecom`; it holds the layer's calls in Telecom (`CallTelecom.kt`) and gives their routes (`CallRoutes.kt`)                               |

`android/scripts/call_engine.py` builds the engine for the three targets (or `--emulator-only`) and
writes the bindings, and says when the WebRTC library is missing. The app is Java with this Kotlin
beside it, so `build.gradle` applies the Kotlin plugin, compiling to the same JVM target as the
Java. When the WebRTC library or the bindings are absent, as on a machine that has built neither,
`app/build.gradle` leaves the `call/` and `voice/` sources out of the build and says so, and the app
builds as it did without them: `MainActivity` registers the plugin by its class name
(`Class.forName`), so a build without the layer has no `AspenCall` plugin and the web view's calls
run in the page as they do in a browser.

### The layer

Each file under `call/` implements one of the engine's interfaces over the platform, as
`voice_client/src/bindings.rs` declares them, mirroring the iOS layer (`ios-app.md`, The call
engine) where Android allows:

| File                      | Implements                                                                                  | Over                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ------------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CallEngine.kt`           | The assembly                                                                                | Starts an `Engine` for one deployment with the parts below; one WebRTC factory and one track registry for the process                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `CallSnapshots.kt`        | `CallObserver`                                                                              | Keeps the latest `CallState` and calls each listener on the main thread; hands each sound the engine cues (`playSound`) to `CallSounds`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `CallSounds.kt`           | The call's sounds                                                                           | One `MediaPlayer` per cue (two while one plays) with the attributes of a voice call (`USAGE_VOICE_COMMUNICATION`), on the recordings `copyCallSounds` lays out as raw resources, asking for no audio focus; see Sounds below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `CallHttp.kt`             | `Http`                                                                                      | OkHttp: no cookies, no redirects followed, every status answered as a response; a bodiless POST, which the engine's are, is sent with an empty body, since OkHttp refuses one without                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `CallSockets.kt`          | `SocketFactory`, `Socket`                                                                   | OkHttp's WebSocket, which answers pings itself and offers `permessage-deflate` on every socket (`RealWebSocket.connect` sets the header; no switch), so the voice server's offers arrive compressed (`negotiatedExtensions` is the server's answer, `permessage-deflate; server_no_context_takeover; client_no_context_takeover` in the proof); `connect` waits for the upgrade, so a refusal is a `TransportException`; frames queue for `receive`, and a close from either side ends the queue                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `CallCredentials.kt`      | `Credentials`                                                                               | The account at the deployment among those `PushStore` keeps (the state the web view writes on sign-in): its session token for each request, renewed through `SessionRefresh` (shared with the push handler) and written back when the API server refuses it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `CallTracks.kt`           | The track ids the engine passes around                                                      | WebRTC tracks by their own `id()`: the microphone as an object, and each received track as the connection that receives it, which gives a live wrapper on demand                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `CallPeerConnection.kt`   | `PeerConnectionFactory`, `PeerConnection`, `PeerConnectionObserver`'s caller, `DataChannel` | `org.webrtc.PeerConnection`, unified plan, `max-bundle`, RTCP muxed; the factory with the device's hardware video codecs through one EGL context and `JavaAudioDeviceModule` with the device's echo canceller and noise suppressor where it has them, for the call connection and a transfer's, and the share's factory (`captureFactory`, which `CallScreen` makes for each share) for the capture connection (`PeerConnectionKind`). `getTransceivers()` makes new Java wrappers on every call and disposes the ones before, their senders, receivers, and the receivers' tracks with them, so no wrapper is kept: transceivers are told apart by their sender's id and numbered in the order first seen, and each use reads them afresh under a lock. A sender's parameters object is kept between `getParameters` and `setParameters`; a receiver's volume is its `AudioTrack.setVolume`; a statistics report is written as the W3C array with `id`, `type`, and `timestamp` in milliseconds beside each entry's values; `close` disposes the connection, after which `connectionState` is `Closed`. A transfer's connection trickles its candidates through the observer and carries one data channel (`CallDataChannel`, ordered and reliable), whose low-buffer mark is read from each change of the buffered amount, since WebRTC tells only of the amount before it, and whose message buffers are copied, being WebRTC's own; closing it disposes it |
| `CallFiles.kt`            | `Files`, `FileSource`, `FileSink`                                                           | `FileRegistry` (in `files/`, which every build has, beside `AspenFilesPlugin`), where the plugin's pickers put what the user chose: a picked document (held with a persistable read permission until released) is opened anew for each transfer of it and read a chunk at a time on the IO dispatcher, and a received document is written a chunk at a time into the one the create picker chose (`DocumentSink`), kept by `finish` or deleted by `discard`; the bytes never cross to the page                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `CallAudio.kt`            | The audio mode and the microphone                                                           | `AudioManager` in `MODE_IN_COMMUNICATION` for the call (`configureForCall` and `release`, which `CallTelecom` drives; Telecom sets the same mode and holds the audio focus for the call itself), and the microphone as a track from WebRTC's audio device module                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `CallTelecom.kt`          | The system's side of the engine's call                                                      | Follows the engine's snapshots: the call in Telecom, `CallService`, the audio mode, and `CallRoutes` attached for the call's length; Telecom's callbacks go to the engine. See The system's call below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `CallService.kt`          | The foreground service and the call notifications                                           | A service of the kinds `phoneCall` and `microphone` for the call's length, `camera` while the call's camera is on (Video below), and `mediaProjection` while the screen is shared (Sharing the screen below), posting a `CallStyle` notification (the call's name, In a call, the system's Hang up, a tap back into the app) as it starts; the incoming call's notification (`showIncoming`), whose actions and whose full-screen intent's screen come back to it, so `answer`, `decline`, and `hangUp` run on one path                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `CallRinger.kt`           | The ringtone and vibration of an incoming call                                              | The system's default ringtone looped and the ring vibration pattern, both as the ringtone usage, so the ringer mode, Do Not Disturb, and Vibrate for calls apply as they do to the phone's own calls; Telecom rings for no call an app manages itself. See Rings below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `IncomingCallActivity.kt` | The incoming call screen                                                                    | Opened by the incoming call notification's full-screen intent over the lock screen, turning the screen on; the caller and the deployment, Answer and Decline through `CallService`; follows `CallTelecom.listen`, and after an answer on a locked phone stays as the call's screen until the phone is unlocked. See Rings below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `CallRoutes.kt`           | Where the call plays                                                                        | Telecom's endpoints for the call, each with a kind from its type; selection by kind through `requestEndpointChange`, kept as the call's `chosen` kind and applied once Telecom lists the endpoints; the proximity sensor while the route is the earpiece                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallVideo.kt`            | Where video is drawn                                                                        | A container behind the web view with one `SurfaceViewRenderer` per hole the page reports, each drawing its surface's track; see Video below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `CallCamera.kt`           | The camera                                                                                  | `Camera2Enumerator`'s capturer into one `VideoSource` and track for the process, listed with each camera's facing, moved between cameras with the track kept; see Video below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CallScreen.kt`           | The share                                                                                   | The screen through `ScreenCapturerAndroid` on the media projection the user consented to, as a video track and, with other apps' sound captured from their playback, an audio track of a factory of the share's own, sent on the capture connection; see Sharing the screen below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `AspenCallPlugin.kt`      | The web view's way in                                                                       | `AspenCall`: each account's engine from `CallEngines`, followed for the page from the first time it names the account (`origin` and `userId`); `join` asks for `RECORD_AUDIO` (Capacitor's permission callback), opens the microphone, names the call for Telecom, and joins; each snapshot's JSON (`CallSnapshots.listenJson`) is a `state` event and `state()`'s answer, untouched; the routes are `routes` events, `setRoute` and `clearRoute`; registered by name in `MainActivity`; the cameras, `startCamera`, `switchCamera`, `stopCamera`, `clearCameraError`, `setWatched`, and `setTiles`, with `shape` and `pip` events back; `startScreenShare` (through the media projection dialog's answer), `stopScreenShare`, and `clearShareError`, with `share` events back; the files' commands (`offerFile`, `withdrawOffer`, `acceptOffer`, `cancelTransfer`, `dismissTransfer`, `clearFileFailure`), each by the reference or destination `FileRegistry` holds; attaches `CallVideo` to the activity's container as it loads                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `CallEngines.kt`          | The engines of the process, one per account                                                 | `CallEngines.forAccount(context, account)` makes a `CallEngine` for a `CallAccount` (a deployment and the user signed in there) on first use and keeps it, so an incoming call answered (`CallRings`, through `CallTelecom`) and the plugin the web view commands drive the same engine, and its routes are asked of every engine made (`all`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `CallRings.kt`            | A DM's call ringing the phone                                                               | The push handler's entry for a `call` pointer, by name, since a build without the layer has no such class; declining at the deployment. The call itself is `CallTelecom.ring`; see Rings below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

Where it differs from iOS: the iOS layer keeps WebRTC's transceiver objects, whose lifetime is the
connection's; Android's are read anew each time, as above. iOS's audio session is configured and
activated in two steps for CallKit's sake; Android sets one mode, which Telecom sets for the call
too. WebRTC's Java encodings take a whole number of frames a second, so a fractional `maxFramerate`
is rounded.

### The system's call

A call the engine is in is the app's own call in Telecom, so the system knows a call is on: a phone
call taken holds it, a watch or a car can end it, the microphone stays the app's behind other apps,
the audio focus is held for it, and the sound goes where Telecom's endpoints say. `CallTelecom` is
not commanded; it follows the engine's snapshots (`CallSnapshots`), so whoever joins through the
engine, the interface or later a ring answered, has the system's call made for them, and whatever
ends the engine's call (the user, a moderator, a failure) ends it. The process holds one
`CallsManager`, registered once with Telecom as an app that can make video calls
(`registerAppWithTelecom`, `CallTelecom.callsManager`).

On the way in, as the first snapshot of a call (`joining`) arrives, in this order: the audio mode is
set to `MODE_IN_COMMUNICATION`; `CallService` is started, which posts the ongoing call's `CallStyle`
notification as it enters the foreground, so the notification is up within the five seconds Telecom
allows; and the call is added to Telecom (`CallsManager.addCall`, held in a coroutine for the call's
length) as an outgoing call, named `displayName` (what the interface that commands the engine sets
before joining; the app's name until then), addressed `aspen://call/<channel>`, of the audio or the
video kind as the engine's grants say, and able to be held (`SUPPORTS_SET_INACTIVE`). Once Telecom
hands the control scope over, `CallRoutes` is attached to it, and when the engine connects the call
is set active (`setActive`); a grant of the camera or the screen that arrives with the connection
moves an audio call to the video kind (`requestCallType`). Telecom refusing the call (another call
being set up, too many calls) leaves the engine's call: a call Telecom does not hold would have its
microphone taken by any other call.

The audio focus is Telecom's: it requests the focus for every call it holds, as a voice
communication (`AudioFocus_For_Phone_Ring_And_Calls` in the system's log), and sets the audio mode
with it. A request of the app's own beside it is displaced by Telecom's the moment the call is
added, which the app would read as a transient loss and an interruption, so the app makes none; an
interruption (a phone call's ring taken) reaches the call as Telecom's hold.

On the way out, the engine's `idle` or `failed` snapshot disconnects Telecom's call with the cause
its end says (`LOCAL` for the user's own leaving and for a failure, `REMOTE` for a call ended from
elsewhere, removed by a moderator, access lost, or the server's idle end; a call added through
`CallsManager` may be disconnected only with `LOCAL`, `REMOTE`, `MISSED`, or `REJECTED`, and
`CallControl` refuses every other code), and once Telecom has done it (or when Telecom itself ends
the call), in this order: the routes are detached (the proximity sensor released), the service
stopped (its notification with it), and the audio mode given back. A call the engine joined after
that end was asked, while Telecom was still letting the last one go, is then added. Nothing is
launched in Telecom's control scope itself, and the collections of its endpoints and mute state run
in scopes of the layer's own, cancelled at the end: a collection left running in the control scope
keeps Telecom's session, and `addCall`, open after the call is disconnected.

What Telecom asks goes to the engine: its disconnect (the notification's Hang up goes through the
service to the engine's `leave` the same way) leaves; its hold (`onSetInactive`, a phone call taken
or Telecom unable to make the call active beside another) is the engine's `setHeld`, which
`onSetActive` lifts: the voice server pauses everything the user sends, a camera or screen included,
sends them nothing, keeps their own mute and deafen for the hold's end, and tells the others, who
see the user on hold rather than muted (`CallState.held` is the user's own, each participant's
`held` the others', both in every snapshot `CallSnapshots` hands on; the engine shows the hold at
once and clears it only on the server's word); a mute from the system (`isMuted`, a car's or a
watch's button) is the engine's mute, the first value being the state the call started in and left
alone. The notification shows no hold; `Phase.HELD` is where the interface reads it.

`CallService` runs for the call's length with the types `phoneCall` and `microphone`
(`FOREGROUND_SERVICE_PHONE_CALL` and `FOREGROUND_SERVICE_MICROPHONE` in the manifest, with
`MANAGE_OWN_CALLS` for Telecom); where Android 14 refuses the microphone kind to a service started
from the background it runs as a phone call alone, and where Android 12 and later refuse to start it
at all the call goes on under Telecom, which from Android 14 keeps a process with a call and a
`CallStyle` notification alive itself. The notification is posted only through `startForeground`,
since Android 12 refuses a `CallStyle` notification that is neither a foreground service's nor
full-screen. Its channel, Ongoing calls, makes no sound. `POST_NOTIFICATIONS`, which the app asks
for when it signs up for pushes, lets it show.

`CallRoutes` is the one list of places a call can play, each a `Route` with a `Kind` from the
endpoint's type (earpiece, speaker, Bluetooth, wired headset, or other, for streaming and kinds this
build does not know), the system's name for display alone, and whether it is current; `hasEarpiece`
reads `AudioManager`'s devices, so it answers before a call too. Selection is by kind (`select`),
through Telecom's `requestEndpointChange`, never `AudioManager.setCommunicationDevice` beside
Telecom, which would fight it for the device. While the current route is the earpiece a
`PROXIMITY_SCREEN_OFF_WAKE_LOCK` is held, so the screen turns off at the ear. Listeners hear of
every change on the main thread; outside a call the list is empty. A kind chosen before Telecom
lists the call's endpoints, as the app chooses as a call starts, is kept (`chosen`) and applied once
they arrive; Telecom's own moves afterwards stand, and `clear` or the call's end forgets it. The
app's Speaker button reaches it through the plugin's `setRoute` and `clearRoute`, and reads the list
through `routes` (`voice/native-calls.md`, Where the call plays); the web view's own audio has no
route of its own. The endpoints Telecom offers before a call (`getAvailableStartingCallEndpoints`),
for a Speaker button shown before joining, are not read yet.

### Sounds

The call's sounds are the engine's cues (`CallObserver.playSound`, derived in `voice_client::sounds`
from the call's own transitions; `voice/native-calls.md`, Sounds), played by `CallSounds`: each cue
is a recording of the Aspen Sound Effects Pack, the web client's own files, which the Gradle task
`copyCallSounds` (`app/build.gradle`, with the call layer) copies from
`client/packages/app/src/features/notifications/sounds/aspen-sound-effects-pack` into
`build/callSounds/res/raw/` under the names Android allows (`call-joined.wav` as `call_joined`), a
resource directory of the app's, so the two never differ. Each is played by a `MediaPlayer` with
`USAGE_VOICE_COMMUNICATION` and `CONTENT_TYPE_SONIFICATION`, which the system routes where Telecom
routes the call (the earpiece, the loudspeaker, a headset) at the call's volume, mixed with the
voices and playing with the app in the background. No audio focus is asked for: Telecom holds it for
the call, and a request of the app's own beside it would displace Telecom's (The system's call
above). A cue plays as it arrives: the audio mode is set for the call on the engine's first snapshot
of it, before any cue, and given back only once Telecom has let the call go, after the leaving's. At
most two of one cue play at once (`MAX_PLAYING_PER_CUE`, the web client's `MAX_PLAYING_PER_SOUND`),
a third not being heard; the engine spaces repeats of a cue itself. `play` takes the cue on the
engine's thread and posts to the main thread; `CallEngine.close` releases the players.

### Rings

A DM's call ringing the phone (`spec/push.md`, Calls and Rings) comes as a high-priority FCM message
marked `k: call`, which `AspenMessagingService` hands to `PushHandler` like any push; the decrypted
pointer's kind `call` is what decides, and the handler calls `CallRings.incoming` by name
(`Class.forName`), since a build without the call layer (`callLayer` in `app/build.gradle`) has no
such class and shows nothing for a call. Everything after runs on the main thread, within the five
seconds a service started by a push has:

1. `CallEngines.forAccount` gives the engine of the account the push's subscription names, the one
   the web view's plugin commands too, so the page shows the answered call as any call, and
   `CallTelecom.ring` adds the call to Telecom as incoming (`DIRECTION_INCOMING`, the caller's name,
   `aspen://call/<channel>`), posts the incoming call's notification (`CallService.showIncoming`: a
   `CallStyle.forIncomingCall` with the system's Answer and Decline, a tap that opens the app, and a
   full-screen intent that opens the incoming call screen, which `USE_FULL_SCREEN_INTENT` in the
   manifest allows, on the channel Incoming calls, which makes no sound; posted directly, the
   full-screen intent being what Android 12 asks of a call notification that is no foreground
   service's), starts the ringtone (`CallRinger`, below), and starts the engine's ring watch
   (`RingWatch`, `voice_client/src/ring.rs`) over `CallHttp`, `CallSockets`, and `CallCredentials`.
   The call's `Phase` is `RINGING`, and the engine's snapshots are left alone until it is answered.
   One call at a time: a ring while Telecom holds a call is shown nowhere here. Whoever shows the
   ring follows the phase through `CallTelecom.listen`, told on the main thread of the phase as it
   is and of every change.
2. The watch's end reaches `CallTelecom.ringEnded` on the main thread, and Telecom's call is
   disconnected with one of the four causes `CallControl` takes: `REMOTE` for a ring answered on
   another device or a call its caller ended, `REJECTED` for a decline on another device, `MISSED`
   at the ring's `until` on the deployment's clock, `LOCAL` when the account has no session. The
   ringtone, the notification, and the screen go with it. Nothing is pushed to end a ring.
3. **Answer**, from the notification or the screen (`CallService`'s `ACTION_ANSWER`, which
   `CallService.answer` starts the service with, so both run the one path `onStartCommand` holds) or
   from Telecom itself (`onAnswer`, a headset): the ringtone and the watch stop, the audio mode is
   set, `CallService` is started with the ongoing call's notification in place of the incoming one
   (the same notification id), Telecom's call is answered (`CallControlScope.answer`; Telecom's own
   answer has done it already), and the engine joins the ring's channel, which ends the ring at the
   deployment. From there the call is any call: `connected` marks it active without asking Telecom
   again, and its end disconnects it.
4. **Decline**, from the notification or the screen (`ACTION_DECLINE`, `CallService.decline`) or
   from Telecom (`onDisconnect` while ringing): the ringtone and the watch stop, `CallRings.decline`
   sends `DELETE /channels/{channel}/voice/rings/@me` as the account on a thread of its own
   (renewing its session once on `401`), so the person's other devices stop ringing too, and
   Telecom's call is disconnected `REJECTED`.

**The ringtone.** Telecom plays no ringtone for a call an app manages itself, as `CallsManager`'s
are (the phone's own ring is its dialer's; through every ring of the device proof the only ringtone
player the audio system opened, `AudioFlinger`'s `OpPlayAudio` line for each
`USAGE_NOTIFICATION_RINGTONE` track, was the app's own), and a notification channel's sound plays
once, so `CallRinger` rings: the system's default ringtone (`RingtoneManager`'s actual default;
Aspen has no ringtone of its own) looped as `USAGE_NOTIFICATION_RINGTONE`, and the ring vibration
pattern (a second on, a second off) as `VibrationAttributes.USAGE_RINGTONE`, from `CallTelecom.ring`
until whatever ends the ring. Whether either is heard is the system's decision, made where it makes
it for the phone's own calls: the ringer mode mutes the ring stream in silent and vibrate and stops
ring vibrations in silent, Do Not Disturb applies its rules for calls to that usage in the audio and
vibrator services (the player stands in `dumpsys audio` with a `mutedState`), and Vibrate for calls
turns the vibration off. The layer reads none of those settings itself. Before Android 9 a ringtone
cannot loop and says nothing when it ends, so it is watched and played again.

**The screen.** The full-screen intent opens `IncomingCallActivity`, declared with `showWhenLocked`
and `turnScreenOn` (set as window flags on Android 8.0, which lacks the attributes), in a task of
its own that the recents never list (`singleInstance`, `excludeFromRecents`, an empty
`taskAffinity`), with the theme `AppTheme.IncomingCall` (charcoal edge to edge). The system opens it
when the phone is locked or its screen is off, and shows the notification heads-up instead when the
phone is unlocked and in use. `MainActivity` has neither attribute and must not: the app shows
nothing over the lock screen. The screen takes the deployment's `public_url` from its intent, finds
its engine (`CallEngines.existing`; a stale intent with no call to show closes it), shows
`displayName` and the deployment's authority (Incoming call on `chat.example.org`), keeps the
display on while ringing, and follows the phase: `RINGING` shows Answer and Decline (Phosphor's
Phone and PhoneDisconnect on the web client's accent and danger); `IDLE` closes it; a call
(`DIALING`, `ACTIVE`, `HELD`, however the ring was answered) hands over to the app, whose web view
shows the call and cannot show over the lock screen. On an unlocked phone it opens `MainActivity` at
once and closes. On a locked one it stays as the call's screen, with the call's state from the
engine's snapshots (Connecting, In a call, On hold) and Hang up (`CallService.hangUp`, the
notification's path), until the phone is unlocked: by the user on their own (`ACTION_USER_PRESENT`),
or through Open Aspen, which asks the keyguard to be dismissed
(`KeyguardManager.requestDismissKeyguard`, the system's own unlock prompt; cancelled, the screen
stays). The keyguard is never asked to go for the ring itself. A ring for another deployment while
the screen shows reaches `onNewIntent` and rebinds it.

What only a phone can show, since the emulator receives no FCM message and shows its lock screen to
no one. `CallDeviceTest` (The proof below) checks what a phone attached by USB can, and did on a
Pixel 8a (Android 17, build CP4A.261105.040.A1) on 2026-11-16, on this branch at the commit that
added it; what it could not is marked with why:

- [ ] A real FCM message from the relay, marked as a call, wakes the app from not running, in Doze,
      and the incoming call's notification is up within five seconds, ringing with the system's
      ringtone, full screen over the lock screen. Verified in part on the Pixel on 2026-11-16 (build
      CP4A.261105.040.A1), with the deployment's own ring (the browser's participant calling the
      user's DM) fed to the push handler in the instrumented process (the debug APK carries no
      `google-services.json` and the relay sends to no test phone): the notification was posted in
      117 ms with the screen dark and locked and in 106 ms under Doze forced on, and the incoming
      call screen, sent its full-screen intent, turned the screen on from `Dozing`
      (`PowerManagerService: Waking up from Dozing ... handleTurnScreenOn`) and showed the caller
      over the keyguard (`isKeyguardShowing=true`; `target/mobile-call/aspen-ring-dark.png`,
      `aspen-ring-lock-screen.png`). The ringtone: `dumpsys audio` listed this process's
      `MediaPlayer` with `USAGE_NOTIFICATION_RINGTONE` started on every ring and gone on every end,
      and, the phone being in Do Not Disturb (priority only, calls from no one Aspen knows),
      `mutedState:opPlayAudio`, the system's own silencing of a call it does not let through
      (`AudioFlinger: OpPlayAudio: ... usage:6 muted`), so not verified by ear. Not verified by the
      system's own hand: on this phone Do Not Disturb intercepts the notification and suppresses its
      full-screen intent (its record's `mIntercept=true`, `mSuppressedVisualEffects=159`, SystemUI's
      `FSI suppressed only by DND`), the phone's Don't turn on screen, so the test sent the intent
      itself, as the system does on a phone whose settings let it; a phone not in Do Not Disturb,
      and the wake of a process that is not running, which only a real push to a phone with the
      relay's credentials shows, are still to be seen.
- [ ] Answer from the lock screen joins the DM's call with audio both ways; Decline ends the ring on
      every other device; a ring answered or declined on another device takes the notification down
      with Telecom's matching cause in the call log. Verified on the Pixel on 2026-11-16: the
      screen's Answer, tapped through the system's input on the locked phone, joined the DM's call
      through the engine, Telecom holding the call active and the ongoing call's notification in the
      incoming one's place, the screen staying up as the call's (In a call, Hang up, Open Aspen;
      `aspen-ring-answered-locked.png`), and its Hang up left the call (what flows once a call is
      joined is `CallEngineTest`'s, which passed on the same phone); the screen's Decline, the
      phone's end key, and the notification's Decline each ended the ring, and the deployment held
      no ring for the user afterwards (`DELETE .../rings/@me` answered); a ring declined elsewhere
      is `CallRingsTest`, which passed on the Pixel too. Not verified: the cause in the phone's call
      log, which a self-managed call does not reach, and the hand-over to the app on unlocking,
      which takes the phone's credential.
- [ ] The ring ends by itself at thirty seconds when no one answers, and when the caller hangs up.
      Verified in part on the Pixel on 2026-11-16: the caller hanging up (the browser's participant
      leaving, which ends the DM's call) ended the ring, took the notification down, and closed the
      screen within ten seconds. Not verified: the thirty seconds, which the test never waits.
- [ ] A Bluetooth headset's button answers and declines through Telecom. No headset was at hand.
- [ ] Android 8 to 13 (the `ConnectionService` path): the incoming call appears in the phone's own
      call screen. The Pixel runs Android 17.
- [ ] Android 14 and later: `NotificationManager.canUseFullScreenIntent()` is true after installing
      from the store, whose review decides it for an app that is not a phone or alarm app; if it is
      not, the notification still rings and heads up, and the app must ask for the permission in its
      settings. True on the Pixel for the debug APK installed by `adb`; a store install is not
      verifiable here.
- [ ] A phone whose clock runs a minute ahead of the deployment's still rings for the whole ring
      (the watch gives the deployment's clock the say once `ready` arrives). Not verified: the
      Pixel's clock is the network's, and the test does not move it.

### The proof

`androidTest`'s `CallEngineTest` runs the engine with this layer against a real deployment, in the
app's process on the emulator, WebRTC on the real `JavaAudioDeviceModule`: it joins a voice
channel's call, has a second participant brought in from a browser, reads both sides' statistics for
the other's sound (the browser's as the loudest sample off its received tracks, `loudness` in
`calls/page.ts`, since Chromium's own level statistics read zero behind the muted element it plays
each track in), puts the call on hold and releases it (the browser's report, `onHold` in
`calls/page.ts`, lists the device's user meanwhile, neither side's packets reach the other, and the
release waits for the server's word), cuts its socket and watches the engine take its place back
with `resume`, the call connected throughout, and leaves; its log line also names what the
signalling socket negotiated. Between the hold and the cut it proves the video both ways: the
browser shows video and turns Chromium's fake camera on, the engine is asked to watch it, and the
track that arrives is retained as `CallVideo` retains one and given a sink that counts its frames
(at least five) and reads their upright shape (drawing needs a window, which the bridge's proof
has); then the emulator's front camera is opened through `CallCamera` and sent as the engine's
camera, its own frames counted with their rotation, and the browser decodes its frames
(`framesDecoded` in `calls/page.ts`'s report) and sees a picture in them (`cameraSample`: a frame
drawn to a canvas, brighter than black); the camera is moved to the back camera with the track kept;
video flows on across the resume; and the camera is stopped before the leave. Then, still before the
cut, it moves a file each way: it receives a file the browser offers (`offerFile` in
`calls/page.ts`) directly into a document held by `FileRegistry` as the create picker's would be and
compares its SHA-256 with the browser's, and offers one of its own, held as the open picker's would
be, which the browser accepts `relayOnly` and hashes alike, both routes read from each side. Around
that it checks the system's side: Telecom holds the call as outgoing (`dialing`) and then active,
and `dumpsys telecom` (run through the instrumentation's shell) names the call by the id Telecom
gave it; the call's service runs in the foreground with its `CallStyle` notification posted, and the
audio mode is the communication mode; the routes are Telecom's endpoints, each with a kind, one
current, and selecting the loudspeaker (and the earpiece again, where the device has one) moves the
call there; and leaving ends Telecom's call, stops the service, takes the notification down, empties
the routes, and gives the audio mode back. What the browser must hear is what the device's
microphone carried, which the device's own `media-source` statistics tell: sound, when a tone is
played to it, or silence, as on an emulator whose host audio never reaches it. Through it all it
watches the call's sounds (`CallSounds.listen`): the joining is played as the engine connects and
again as the browser joins, each transfer's acceptance as its link begins and its completion as the
file arrives whole, the leaving as the call is left, and those alone, since a socket taken back
within the grace keeps the call connected and is no loss (the new socket's frames, read through the
test's `WatchedSocket`, say the `resume` was presented and answered `ready`, and no snapshot showed
`rejoining`), each a `MediaPlayer` playing as the cue is given, and for the first the system's own
word on it, `dumpsys audio`'s players read on the spot: one of this process as a `MediaPlayer` with
`USAGE_VOICE_COMMUNICATION` and `CONTENT_TYPE_SONIFICATION`, started, which the log line quotes.
`AspenCallPluginTest` drives the bridge itself: it launches `MainActivity`, which loads the web
shell with the plugin registered, and runs `Capacitor.registerPlugin("AspenCall")` in the page
through the bridge's web view, as the app's `NativeCall` does. From the page it reads a fresh
snapshot (the engine's JSON, idle, revision 0, of its origin) and the empty routes of no call, has
malformed commands refused by name, joins the channel through the plugin and watches the `joining`
and `connected` snapshots reach the page, moves the call to the loudspeaker (and the earpiece again,
where the device has one) and watches the `routes` events say so as Telecom's current endpoint
changes, clears the choice, mutes and unmutes, and then drives the video: `cameras()` lists the
emulator's cameras with their facings, `startCamera` opens the front one and the snapshot shows the
camera among `sources`, a hole reported with `setTiles` for its track has its shape come back as a
`shape` event and its surface view laid out at the hole's rectangle in device pixels and drawing
(`onFirstFrameRendered`), `switchCamera` moves to the back one, a second participant from the
browser turns its camera on and `setWatched` has the snapshot show its track, whose hole hears its
shape and draws too, the activity enters the picture-in-picture mode on it (`enterPictureInPicture`,
standing for the user's leaving) and the page hears `pip` while `isInPictureInPictureMode` holds,
starting the activity again leaves the mode and the page hears that too, the browser decodes the
camera, malformed `setWatched` and `setTiles` are refused by name, and `stopCamera` takes the source
out of the snapshot and the capture off; then it accepts a file the browser offers through the
plugin (`acceptOffer`, into a document held by `FileRegistry` as the create picker's would be, an
acceptance naming none refused by name) and watches the snapshot's `files` show the transfer
complete, direct, with the bytes offered; then it shares the screen: `startScreenShare` opens the
system's media projection dialog, which the test accepts through UI Automator (the whole screen,
then Start, whichever words this Android gives them), the snapshot shows `screen` and `screenAudio`
live as external sources while the service runs with the `mediaProjection` kind, the preview's hole
hears its shape and draws, the browser lists the share, decodes a picture of it (`screenSample`, the
emulator's screen) and receives packets of its sound on the audio section of the `{user}-share`
stream (silence, since nothing plays on the emulator for the capture to hear), `stopScreenShare`
takes the sources out, the share's factory goes once its capture connection has closed
(`CallScreen.factoryDisposed`), the browser sees the share go, the service drops the kind, and three
seconds on the call is still connected with the browser still receiving the microphone, which is
then done a second time, the share started afresh and stopped again (the stop's order is what a
phone crashed on, Sharing the screen below; the emulator runs the same order and shows that the
factory goes and the call outlives the share), and the dialog declined rejects the call with
`denied`; then it leaves: the snapshots arrive in revision order with none twice, a fresh `state()`
is the latest published, the renewed session token is kept, and the routes empty again.
`androidTest`'s `CallRingsTest` feeds `PushHandler` a `call` pointer encrypted to a key it writes
into the push state, as the messaging service would, and checks that within five seconds the
incoming call's notification is posted with its full-screen intent and Telecom holds the call as
ringing; then, standing in for the deployment's event stream (a WebSocket server of the standard
library's sockets), it takes the ring watch's `identify`, answers `ready`, and sends the ring's
`voiceRing` `delete`, after which the call is gone and the notification with it; between, it sends
the notification's full-screen intent itself, as the system would for a locked phone, and checks the
incoming call screen is resumed naming the caller and the deployment, that the ringtone plays
meanwhile, and that the ring's end closes the screen and stops it; a push that does not decrypt
rings nothing. It needs no deployment and runs with the rest of `connectedDebugAndroidTest`.
`scripts/check_mobile_call.py --platform android` at the repository's root runs `CallEngineTest` and
`AspenCallPluginTest` end to end, with the same deployment and browser the iOS check uses: a
deployment of its own serving the call page (`client/packages/protocol/calls/`), `adb reverse` so
the emulator reaches the API server, the voice server, and the script's helper at its own loopback
(plain HTTP there, as a development build allows; media goes to the voice server's announced address
over the network), Chromium as the second participant with a tone WAV as its microphone and its fake
camera (`calls/second.ts`, which the script's helper drives: showing video, the camera on and off,
and a frame of someone's camera sampled), and `gradlew connectedDebugAndroidTest` on these tests
alone, judging their JUnit reports (every class ran, none failed or erred, and none skipped but the
trailing test, which skips itself outside `--record-trailing`) and that each printed its own line,
with every `adb` and Gradle command pinned to the device's serial (`ANDROID_SERIAL`), so whatever
else is attached, a phone beside the emulator or an emulator beside the phone, is never touched;
`--only <class>` runs one of them (or several, repeated) against the same deployment, for one test's
failure without the others' minutes before it. The device's log of the run, its crash buffer
included, is left as `target/mobile-call/logcat.txt`, so a native crash's tombstone is there to
symbolize (`ndk-stack -sym` with the unstripped `libjingle_peerconnection_so.so` of the WebRTC
build, `lib.unstripped/` under its architecture's output directory). With `--avd` it starts the
emulator headless, recording the machine's microphone (`-allow-host-audio`, without which the
emulator records silence); the emulator's PulseAudio backend does not start against PipeWire's
server (``Could not init `pa' audio driver``), so there its microphone is silent. What the browser
is to hear of the device's microphone is the script's to say (`ASPEN_CALL_MICROPHONE`, from its
`--microphone`): `silent` on an emulator, where the test checks the device's own source measured
silence and the browser heard silence; `live` on a phone, whose microphone hears the room, where the
test checks that the browser received and decoded the phone's packets and reports the level it
heard, whatever a room sounds like. With `--device <serial>` the check runs on a phone attached by
USB instead (`adb reverse` works on one as on the emulator, and media reaches the voice server's
announced address over the network the phone shares with the machine): an emulator's serial is
refused (`emulator-<port>`, or `ro.kernel.qemu` set), `--build` builds the engine for the phone's
ABI (`call_engine.py --abi arm64-v8a`), and `CallDeviceTest` runs after the engine's and the
bridge's tests: what only a phone can show, read from the system itself through the
instrumentation's shell. Before any test the check reads the phone's state and refuses, naming what
to change, what would fail a test from inside (`Android.ready_phone`): the bridge's test enters the
picture in picture, which needs its activity resumed, so the screen must be on with no lock screen
over it and must stay so through the engine's test before it, which Stay awake (Developer options)
keeps on USB; a screen that is off, or a swipe lock screen, the check wakes and dismisses itself
(`wm dismiss-keyguard`) and says so, and a PIN, pattern, or password lock screen showing is the
owner's to unlock, or to set to Swipe, and is refused by its kind (`dumpsys lock_settings`). In a
call with the browser, it selects the earpiece and then the loudspeaker and reads `dumpsys audio`'s
active communication device and `dumpsys power`'s proximity wake lock after each, reads the
service's foreground types and the `CallStyle` notification from `dumpsys activity services` and
`dumpsys notification` (its record found by the key the system gave the notification the app sees
posted, and a failure quoting what the dump named of the package), turns the screen off
(`input keyevent KEYCODE_SLEEP`) for three minutes and forces Doze (`dumpsys deviceidle force-idle`)
for thirty seconds, checking every half minute that the call is connected and packets reach both
sides, and leaves. Then it feeds the push handler a `call` pointer, as `CallRingsTest` does, with
the screen dark and locked, and records whether the full-screen intent woke the screen and captures
it (`aspen-ring-dark.png` in `target/mobile-call/`, which the script pulls), rings again with the
screen lit and captures that too, answers through the notification's Answer (found by its
`PendingIntent`, never its wording, `CallService.answerIntent`), which must have the engine
connected in the ring's channel and Telecom holding the call active, declines through the phone's
end key (`KEYCODE_ENDCALL`, Telecom's own disconnect of a ringing call) and through the
notification's Decline, and rings once more under Doze. It gives the phone back as it found it: Doze
unforced, the screen awake, the push state as it was. The tests read their deployment, channel,
user, refresh token, and the script's helper from the instrumentation's arguments
(`-Pandroid.testInstrumentationRunnerArguments.ASPEN_CALL_*`; `CallEngineTest`'s comment lists them)
and are skipped without them, so the `android` job's `connectedDebugAndroidTest` runs them as skips;
`CallTestSupport` holds what the tests share (the arguments, waiting on the engine and Telecom, the
system's word through the instrumentation's shell, the incoming call screen and taps on it, the
ringtone players, the browser's statistics, the helper's rings, and the push keys and encryption
`CallRingsTest` and `CallDeviceTest` feed the push handler with). CI's `android-call` job runs the
check itself: it builds the servers, the engine for the emulator, and the web shell, and runs
`check_mobile_call.py --start-services` inside the emulator action's step, against the emulator it
started (no `--avd`), whose audio is off, so there too the test checks the browser hears silence.

### Video

The page draws no picture itself: every picture is a surface the phone draws behind a transparent
web view, where the page leaves a hole (`voice/native-calls.md`, Video). The app's copy of
Capacitor's layout (`res/layout/capacitor_bridge_layout_main.xml`) puts a `FrameLayout`,
`video_container`, black, beneath the web view; `AspenCallPlugin` attaches `CallEngine.video` to it
as it loads and makes the web view's background transparent, so a hole shows black until its first
frame, as a `<video>` with nothing to play does.

- **Tiles** (`CallVideo.apply`, from the plugin's `setTiles`): one `SurfaceViewRenderer` per hole
  the page reports, keyed by the page's tile id, made as a tile first appears and released as it
  goes, on the factory's EGL context (`CallPeerConnections.eglContext`, which the codecs and the
  camera's texture share). Surface views lie behind the window by design, so the transparent web
  view shows them through its holes; a self preview (a mirrored tile) is ordered above the other
  pictures with `setZOrderMediaOverlay`, still beneath the window. Each is laid out where the page
  says, the page's CSS pixels times the display's density, from where the web view lies in the
  container; a full-screen tile takes the container's own size, laid out again on its layout change.
  A tile's `cover` is `SCALE_ASPECT_FILL`, otherwise `SCALE_ASPECT_FIT`; `mirrored` is the view's
  mirror. The picture's shape, upright, is reported as the first frame arrives and whenever it
  changes (`RendererEvents.onFrameResolutionChanged`, as `shape` events to the page), and the frames
  handed to each view are counted for the proof.
- **Received tracks are retained** (`CallTracks.retainedVideoTrack`, `org.webrtc.TrackRefs`):
  `PeerConnection.getTransceivers()` makes new Java wrappers of the transceivers, receivers, and
  tracks on every call and disposes those it made before, and a disposed `VideoTrack` wrapper takes
  the sinks it added with it, so a renderer added through one of those wrappers would stop drawing
  at the engine's next read of the transceivers. `TrackRefs`, in WebRTC's own package since the
  native pointer and the reference count are package-private there, holds a reference of its own on
  the native track and wraps it anew; the wrapper lives until the renderer gives it back
  (`releaseVideoTrack`). The build without the call layer excludes `org/webrtc/**` with the rest.
- **The camera** (`CallCamera`, `CallEngine.camera`): one Camera2 capturer and one `VideoSource` and
  track for the process, the frames through a `SurfaceTextureHelper` on the shared EGL context.
  `startCamera` asks for `CAMERA` through Capacitor's permission callback, as `join` asks for the
  microphone, then opens the camera chosen by Camera2's id, or the front camera, at most 1280 by 720
  at 30 frames a second (the engine's encoding bounds what is sent), resolving once the first frame
  arrives; the track goes to the engine as its `camera` source. `switchCamera` is the capturer's own
  `switchCamera` to the named camera, the track kept, so the engine's sender needs nothing. Camera2
  names no camera, so the page names each by its facing. The camera is on only while its engine
  sends it: the plugin follows the snapshots and stops the capture when the call ends or the voice
  server refuses the camera. A failure to open is `denied`, `none`, or `failed`
  (`CallCameraFailure`), the plugin's rejection code and the page's `cameraError`. While the camera
  is on, `CallService` is started again with the `camera` foreground service type beside `phoneCall`
  and `microphone` (`setCamera`; the manifest declares the type and `FOREGROUND_SERVICE_CAMERA`), so
  the camera may go on while the app is not in front, and the type comes off as the camera does.
- **Picture in picture**: the activity declares `supportsPictureInPicture`, and its configuration
  changes keep it, and so the call's page, through the shrink and the turn. The tile the page marks
  `pictureInPicture` sets the activity's `PictureInPictureParams`: the picture's aspect ratio
  (within the 1:2.39 to 2.39:1 the system allows) and its bounds as the source rectangle, and from
  Android 12 `setAutoEnterEnabled`, so the activity enters the mode itself as the user leaves;
  before Android 12 the plugin enters it in `onUserLeaveHint`. `MainActivity` tells the plugin of
  both through `CallPictureInPictureHost` (in the app's main package, so a build without the call
  layer has it), and the plugin tells the page as `pip` events; the page then shows that one picture
  alone, filling the shrunken page, whose hole the surface view follows.

The page's own motion is the design's known weak point: a picture follows its hole a frame or two
behind while the hole moves, since the page measures and reports rectangles after it has laid them
out. The call screen on a phone pages rather than scrolls for that reason (`voice/call-screen.md`,
On a phone). `scripts/check_mobile_call.py --record-trailing` measures it: the bridge's test
`pictureFollowsAMovingHole` hides the page, moves a red-bordered box across the screen and back
(three passes of 200 CSS pixels in 800 ms each) reporting its inside as a hole every animation frame
with the emulator's camera drawn in it, `adb shell screenrecord` records the screen around it, and
`scripts/measure_video_trailing.py` reads every recorded frame for how far the picture's left edge
is from the border's inside against the direction of motion. On the emulator (`aspen-test`, API 36,
x86-64, software rendering), which keeps a recorded frame only when the screen changes and showed
the box in steps of about 20 CSS pixels: 29 moving frames, the picture behind its hole by a median
of 2.4 CSS pixels and at worst 4.5, never ahead by more than 2.4, which at the box's speed is under
a frame (a median of 0.5 ms, at worst 12 ms); so the surface view and the page reach the screen in
the same frame most of the time and never more than one apart. Judging it by eye on a device, at a
phone's own frame rate, is on the checklist.

### Sharing the screen

The share is `CallScreen` (`CallEngine.screen`): one capture at a time for the process, whose
picture and sound go to the engine as external sources (`startExternalSource`, the engine's `screen`
and `screenAudio`) on the capture connection, a peer connection of the share's own. The consent is
the system's media projection dialog (`MediaProjectionManager .createScreenCaptureIntent`, which
`startScreenShare` opens through Capacitor's `startActivityForResult` and `projectionAnswered` takes
the answer of): the user chooses the whole screen or one app there, and declining rejects the
plugin's call with `denied`. Consented, the call's service is started again with the
`mediaProjection` kind beside its others (`CallService.setProjection`, which tells `CallScreen` once
the service runs with it; the manifest declares the type and `FOREGROUND_SERVICE_MEDIA_PROJECTION`),
since from Android 14 the system refuses to make a projection for a process without one and under it
the share goes on while the app is not in front; then WebRTC's `ScreenCapturerAndroid` takes the
projection, through a `SurfaceTextureHelper` on the shared EGL context, at the display's own size
(no larger than 1920 on a side, `MAX_SIDE`) at 30 frames a second, followed through a turn by
`changeCaptureFormat` on the configuration change. The engine is given the tracks as the capture
starts, since a WebRTC video source hands frames on only once a sender wants them, so a first frame
cannot be the signal; frames follow as the screen changes. The self preview is drawn from the same
video track (`CallVideo`). The voice server offers H.264 first on the capture connection and VP8
after it, so a device without a hardware H.264 encoder (the emulator) sends VP8. The projection's
own stop (the system's notification, another app's projection taking over) ends the share, and so
does the call ending or the voice server refusing the share, which the plugin follows in the
snapshots; `stopScreenShare` ends it from the page, the service dropping the kind.

A WebRTC factory has one audio capture, its audio device module's, for every connection it makes, so
the share's sound cannot be a second track beside the microphone. `CallScreen` makes a
`PeerConnectionFactory` for each share (`CallPeerConnections.makeCaptureFactory`, on the same codecs
and EGL context), whose `JavaAudioDeviceModule` records what other apps play: an `AudioRecord` on an
`AudioPlaybackCaptureConfiguration` of the projection, matching media, game, and unstated usages and
never a voice communication, so the call's own sound is left out (Android 10 and later; before it
the share has no sound). WebRTC's module makes its own `AudioRecord`, of a microphone source, and
nothing in its builder takes a playback capture, so
`scripts/webrtc_patches/android-audio-record-factory.patch` gives the builder
`setAudioRecordFactory`, which makes the record with the format and buffer size the module asks for
(`docs/development/webrtc-build.md`). The audio source is made without WebRTC's voice processing and
the module without the hardware effects (`shareAudioConstraints`), the sound being an application's,
in stereo. The engine makes the capture connection from that factory (`create(CAPTURE, …)`), so one
transport carries the share's picture and sound and the voice server puts them in one stream for
everyone else (`{user}-share`).

The factory outlives that connection (`CaptureFactory`, in `CallPeerConnection.kt`). Disposing a
WebRTC factory deletes its network, worker, and signalling threads, and a peer connection made from
it runs on them until it is closed: its audio encoder posts each encoded frame to the worker thread
(`ChannelSend::SendData`), and the engine's own calls on it (`replaceTrack`, `close`) read it. The
engine closes the capture connection on its own thread, after a stop: the plugin's `stopScreenShare`
tells it to stop both sources (`stop_source`, which replaces each sender's track and then closes the
connection) and stops the share, so the connection is still open as `CallScreen.stop` runs. The stop
takes the capture and the projection down at once and retires the factory (`retire`, with the
share's tracks and sources); the factory, its module, and those are disposed, in that order, by
whichever of the connection's close (`CallPeerConnection.closePeerConnection`, which tells the
factory through `onClosed`) and the retirement comes second, on its thread, and no connection is
made from a retired factory (`open` refuses). On a Pixel 8a the earlier order, the factory disposed
at the stop, crashed the process within half a second of every stop: once in the share's
`AudioEncoderQueue` thread, jumping through the deleted worker thread's freed vtable (`SEGV_ACCERR`,
"trying to execute non-executable memory"), and once in the engine's `replaceTrack`, whose read of
the freed connection's signalling state (`0xAAAAAAAA`, scudo's fill) threw inside WebRTC's JNI,
which aborts on an exception; the emulator, whose freed pages happened to stay mapped, never showed
it.

### What only a phone can show

The emulator offers one route, its loudspeaker (`hasEarpiece` is false there), and has no other
call, so these are checked on a device: `CallDeviceTest` reads what a phone attached by USB can show
from the system itself, and did on a Pixel 8a (Android 17, build CP4A.261105.040.A1) on 2026-11-16,
on this branch at the commit that added it; the rest take a hand, a headset, a SIM, or another
Android, and are marked with why:

- [ ] The earpiece: the call plays at the ear, the screen turns off as the phone is raised to it and
      comes back as it is lowered, and the Speaker route turns the screen back on; the app's Speaker
      button does the same through the plugin. Verified on the Pixel as far as the system tells:
      Telecom lists `EARPIECE(Earpiece)` and `SPEAKER(Speaker)`, selecting each makes it the audio
      system's active communication device (`dumpsys audio`), the proximity wake lock `aspen:call`
      is held at the earpiece and released at the loudspeaker (`dumpsys power`), and the mode is the
      communication mode. Not verified: the screen going dark at the ear itself, which takes a hand
      over the sensor, and the app's own button.
- [ ] A join with `RECORD_AUDIO` refused fails with `errorKind` `microphone` in the call bar (the
      permission's prompt is Capacitor's; the emulator's test grants it beforehand), and one in a
      channel without Speak joins to listen. Not verified: both take the app's page, with a hand on
      the prompt.
- [ ] A Bluetooth headset: it is listed as `BLUETOOTH` with its name, a call joined with it
      connected starts on it, connecting it during a call moves the call to it, disconnecting it
      moves the call back, and with `BLUETOOTH_CONNECT` granted Telecom names every headset. No
      headset was at hand.
- [ ] A wired or USB headset: listed as `WIRED_HEADSET`, taken on connection, given up on removal.
      None at hand, and the USB port carries `adb`.
- [ ] A phone call interrupting ours: the ring holds the call (`onSetInactive`, the user shown on
      hold to the others, who hear nothing of them, and hearing nothing themself), hanging the phone
      call up or declining it resumes ours (`onSetActive`) with the user's own mute and deafen
      standing, and ending ours from the phone's call screen or a watch leaves the engine's call.
      The Pixel has no SIM, and no second phone was at hand; the hold itself, asked through the
      engine as Telecom asks it, is `CallEngineTest`'s.
- [ ] A navigation prompt or an alarm over the call: what Telecom's audio focus does with it, and
      whether the call is held or plays on beside it. Not verified.
- [ ] The call's sounds (`CallSounds`): the joining is heard as the call connects and as someone
      else joins, the leaving as someone leaves and as the user leaves, a share starting, and the
      call lost and back (cut the network for a few seconds), each on the earpiece, on the
      loudspeaker, and on a headset, with the app in the foreground, in the background, and the
      screen off, at the call's volume beside the voices; and none while a phone call holds ours.
      Not verified: takes an ear, a hand, and a headset.
- [ ] The notification: Hang up leaves the call, a tap opens the app, and on Android 14 and later
      the call outlives the app in the background and the screen off for an hour, with the battery
      use the system attributes to it recorded. Verified on the Pixel: the service runs in the
      foreground as a phone call and the microphone (`types=0x84` in `dumpsys activity services`)
      with its `CallStyle` notification (`dumpsys notification`), and the call stayed connected with
      media both ways through three minutes of the screen off and thirty seconds of Doze forced on
      (`dumpsys deviceidle force-idle`, which this phone allows while charging), and leaving took
      the service, the notification, and the wake lock down. Not verified: Hang up and the tap, an
      hour, and the battery figure; and the test's process is instrumented, which keeps it alive
      itself, so what the system does to the app's own process over an hour is still the app's to
      show.
- [ ] Android 8 to 13 (the `ConnectionService` path): the call appears in the phone's call screen,
      the same holds and resumes, and the foreground service alone keeps the process alive. The
      Pixel runs Android 17.
- [ ] The camera: the front camera's preview is mirrored and upright however the phone is held, the
      back camera's is not mirrored, the flip moves between them with the picture pausing only for
      the camera's own start, and the other side sees the picture upright through a turn. Not
      verified: takes a hand to hold and turn the phone.
- [ ] The camera in the background: with the app behind another, or in the picture-in-picture
      window, the camera stays on under the service's `camera` type and the others keep seeing it.
      Not verified.
- [ ] The picture in picture: leaving the app (home, recents) while someone's camera or screen shows
      enters the mode on the focused screen (else the first camera of someone else), the page shows
      that picture alone in the window, the window's shape is the picture's, and opening the app
      again leaves the mode; on a phone before Android 12 the plugin enters it from
      `onUserLeaveHint`. Not verified: takes a hand on the home button.
- [ ] Rotation: a full-screen picture turns with the phone without a flash of the old layout, and
      the paged tiles lay out again behind their holes after a turn. Not verified: takes a hand to
      turn the phone.
- [ ] Trailing by eye: while paging between tiles and while a transition moves one, the surface view
      follows its hole; frame-step a screen recording (`adb shell screenrecord`) and judge how far
      it trails and whether the paged grid hides it. Not verified by eye on a device; the emulator's
      measurement is under Video.
- [ ] Sharing: the share button opens the system's media projection dialog (whole screen or one
      app); consented, the others see the screen with the sound of another app playing (music, a
      game, YouTube) and not the call's own sound, apps that forbid capture of their sound
      (`allowedCapturePolicy`) stay silent, the self preview shows it, the dialog declined shows no
      error, and the projection's own stop (the system's notification) ends the share in the call.
      Verified in part on the Pixel on 2026-11-16 (build CP4A.261105.040.A1), by
      `AspenCallPluginTest` through the plugin: the dialog opened and was accepted (the whole
      screen), the share went out at 864×1920 through the phone's hardware H.264 encoder
      (`c2.exynos.h264.encoder`), the browser decoded it and received packets of its sound (silence:
      nothing was playing), the self preview drew, the stop ended it with the call going on and the
      share's factory going only once its capture connection had closed (the order that crashed the
      process here before `CaptureFactory`), a second share started and stopped the same way, and
      the dialog declined rejected with `denied` and asked for no source. Not verified: another
      app's sound in the share, an app that forbids capture, and the projection's own stop from the
      system's notification, which take a hand.
- [ ] Sharing in the background and through a turn: with the app behind another, the share goes on
      under the service's `mediaProjection` kind, and turning the phone sends the display's new size
      (the others see it upright and the picture whole).
- [ ] Sharing a game at 30 frames a second: the frame rate and resolution the others receive, and
      the battery use the system attributes to the share.
- [ ] A phone without an earpiece (a tablet): `hasEarpiece` is false and the routes are the
      loudspeaker and whatever is connected. No tablet at hand.
- [ ] A large file (a gigabyte) picked from a documents provider is sent whole, its progress moving,
      with the read permission held until the offer runs out; one received into a provider's
      document is whole there.
- [ ] A transfer across networks (the phone on cellular, the other side elsewhere) made `relayOnly`
      goes through the relay and its row says so; one `directPreferred` finds a direct route where
      the networks allow.
- [ ] A transfer under way goes on with the app in the background and the screen off, under the
      call's foreground service, and the file is whole when the app returns.
- [ ] The app's cold start from the launcher after installing the APK: the web shell loads and the
      plugin that commands the engine is available. Not verifiable on the Pixel by USB: it is locked
      with a credential the test does not have, so an activity started with `am start` stays behind
      the lock screen.

What the Pixel found, fixed on the same day: a ring answered joined with no microphone, which the
engine refuses when the call lets the user speak (`CallTelecom.answer` opens one, and the call's end
closes it); the `idle` snapshot the microphone's setting publishes before the join read as the
answered call's end, disconnecting it from Telecom as it was joining (`SystemCall.joined`); and a
ring its caller ended was disconnected with `CANCELED`, which `CallControl` refuses with an
exception (it takes `LOCAL`, `REMOTE`, `MISSED`, and `REJECTED` alone), so every ring's end and
every failure now maps to one of those four.
