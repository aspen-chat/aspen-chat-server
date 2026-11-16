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
(Telecom), the routes, the Capacitor plugin the web view commands it through and reads its state
from (`AspenCallPlugin`; `voice/native-calls.md` is the page's side), the call's video: the camera,
the pictures drawn behind the web view, and the picture-in-picture mode (Video below), and the
call's sounds, played as a voice communication (Sounds below). A DM call ringing the phone is a
later piece.

### What the app packages, and where it comes from

| Piece                                                                                                     | Where the Gradle build reads it                                                                                                                                                                                                   | Made by                                                                                                                                                               |
| --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Google's WebRTC, `libwebrtc.aar` (`org.webrtc`, its library for `arm64-v8a`, `armeabi-v7a`, and `x86_64`) | `target/webrtc/`, as a file dependency                                                                                                                                                                                            | `scripts/webrtc_build.py build` on Linux, once (`docs/development/webrtc-build.md`)                                                                                   |
| The engine, `libvoice_client.so`                                                                          | `target/x86_64-linux-android/release/` for the emulator, `target/aarch64-linux-android/release/` and `target/armv7-linux-androideabi/release/` for devices, laid out by ABI into the APK's `jniLibs` by the task `copyCallEngine` | `android/scripts/call_engine.py` (`cargo rustc --crate-type cdylib` for each target, linked with the NDK's clang)                                                     |
| Its Kotlin package, `voice_client.kt`                                                                     | `app/src/main/java/com/aspen_chat/client/voice/`, gitignored                                                                                                                                                                      | `scripts/voice_client_bindings.py --language kotlin` (`voice_client/README.md`)                                                                                       |
| JNA (`net.java.dev.jna:jna`, the `aar`), kotlinx-coroutines, OkHttp                                       | Maven Central; the versions are in `variables.gradle`                                                                                                                                                                             | The bindings load the engine's library through JNA and suspend through coroutines; the layer's HTTP and WebSockets are OkHttp's, which the app did not have before    |
| `androidx.core:core-telecom` 1.1.0                                                                        | Google's Maven, only when the layer is built; the version is in `variables.gradle`                                                                                                                                                | The newest stable release, the first with `registerAppWithTelecom`; it holds the layer's calls in Telecom (`CallTelecom.kt`) and gives their routes (`CallRoutes.kt`) |

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

| File                    | Implements                                                                   | Over                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CallEngine.kt`         | The assembly                                                                 | Starts an `Engine` for one deployment with the parts below; one WebRTC factory and one track registry for the process                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CallSnapshots.kt`      | `CallObserver`                                                               | Keeps the latest `CallState` and calls each listener on the main thread; hands each sound the engine cues (`playSound`) to `CallSounds`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `CallSounds.kt`         | The call's sounds                                                            | One `MediaPlayer` per cue (two while one plays) with the attributes of a voice call (`USAGE_VOICE_COMMUNICATION`), on the recordings `copyCallSounds` lays out as raw resources, asking for no audio focus; see Sounds below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `CallHttp.kt`           | `Http`                                                                       | OkHttp: no cookies, no redirects followed, every status answered as a response; a bodiless POST, which the engine's are, is sent with an empty body, since OkHttp refuses one without                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CallSockets.kt`        | `SocketFactory`, `Socket`                                                    | OkHttp's WebSocket, which answers pings itself and offers `permessage-deflate` on every socket (`RealWebSocket.connect` sets the header; no switch), so the voice server's offers arrive compressed (`negotiatedExtensions` is the server's answer, `permessage-deflate; server_no_context_takeover; client_no_context_takeover` in the proof); `connect` waits for the upgrade, so a refusal is a `TransportException`; frames queue for `receive`, and a close from either side ends the queue                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallCredentials.kt`    | `Credentials`                                                                | The account at the deployment among those `PushStore` keeps (the state the web view writes on sign-in): its session token for each request, renewed through `SessionRefresh` (shared with the push handler) and written back when the API server refuses it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `CallTracks.kt`         | The track ids the engine passes around                                       | WebRTC tracks by their own `id()`: the microphone as an object, and each received track as the connection that receives it, which gives a live wrapper on demand                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallPeerConnection.kt` | `PeerConnectionFactory`, `PeerConnection`, `PeerConnectionObserver`'s caller | `org.webrtc.PeerConnection`, unified plan, `max-bundle`, RTCP muxed; the factory with the device's hardware video codecs through one EGL context and `JavaAudioDeviceModule` with the device's echo canceller and noise suppressor where it has them. `getTransceivers()` makes new Java wrappers on every call and disposes the ones before, their senders, receivers, and the receivers' tracks with them, so no wrapper is kept: transceivers are told apart by their sender's id and numbered in the order first seen, and each use reads them afresh under a lock. A sender's parameters object is kept between `getParameters` and `setParameters`; a receiver's volume is its `AudioTrack.setVolume`; a statistics report is written as the W3C array with `id`, `type`, and `timestamp` in milliseconds beside each entry's values; `close` disposes the connection, after which `connectionState` is `Closed` |
| `CallAudio.kt`          | The audio mode and the microphone                                            | `AudioManager` in `MODE_IN_COMMUNICATION` for the call (`configureForCall` and `release`, which `CallTelecom` drives; Telecom sets the same mode and holds the audio focus for the call itself), and the microphone as a track from WebRTC's audio device module                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallTelecom.kt`        | The system's side of the engine's call                                       | Follows the engine's snapshots: the call in Telecom, `CallService`, the audio mode, and `CallRoutes` attached for the call's length; Telecom's callbacks go to the engine. See The system's call below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `CallService.kt`        | The foreground service and the ongoing call's notification                   | A service of the kinds `phoneCall` and `microphone` for the call's length, posting a `CallStyle` notification (the call's name, In a call, the system's Hang up, a tap back into the app) as it starts                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `CallRoutes.kt`         | Where the call plays                                                         | Telecom's endpoints for the call, each with a kind from its type; selection by kind through `requestEndpointChange`, kept as the call's `chosen` kind and applied once Telecom lists the endpoints; the proximity sensor while the route is the earpiece                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `CallVideo.kt`          | Where video is drawn                                                         | A container behind the web view with one `SurfaceViewRenderer` per hole the page reports, each drawing its surface's track; see Video below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `CallCamera.kt`         | The camera                                                                   | `Camera2Enumerator`'s capturer into one `VideoSource` and track for the process, listed with each camera's facing, moved between cameras with the track kept; see Video below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `AspenCallPlugin.kt`    | The web view's way in                                                        | `AspenCall`: a `CallEngine` per deployment origin, made on first use; `join` asks for `RECORD_AUDIO` (Capacitor's permission callback), opens the microphone, names the call for Telecom, and joins; each snapshot's JSON (`CallSnapshots.listenJson`) is a `state` event and `state()`'s answer, untouched; the routes are `routes` events, `setRoute` and `clearRoute`; registered by name in `MainActivity`; the cameras, `startCamera`, `switchCamera`, `stopCamera`, `clearCameraError`, `setWatched`, and `setTiles`, with `shape` and `pip` events back; attaches `CallVideo` to the activity's container as it loads                                                                                                                                                                                                                                                                                           |

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
its end says (`LOCAL` for the user's own leaving, `REMOTE` for a call ended from elsewhere, removed
by a moderator, access lost, or the server's idle end, `ERROR` for a failure, which Telecom before
SDK 38 reports as `LOCAL`), and once Telecom has done it (or when Telecom itself ends the call), in
this order: the routes are detached (the proximity sensor released), the service stopped (its
notification with it), and the audio mode given back. A call the engine joined after that end was
asked, while Telecom was still letting the last one go, is then added. Nothing is launched in
Telecom's control scope itself, and the collections of its endpoints and mute state run in scopes of
the layer's own, cancelled at the end: a collection left running in the control scope keeps
Telecom's session, and `addCall`, open after the call is disconnected.

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

### The proof

`androidTest`'s `CallEngineTest` runs the engine with this layer against a real deployment, in the
app's process on the emulator, WebRTC on the real `JavaAudioDeviceModule`: it joins a voice
channel's call, has a second participant brought in from a browser, reads both sides' statistics for
the other's sound (the browser's as the loudest sample off its received tracks, `loudness` in
`calls/page.ts`, since Chromium's own level statistics read zero behind the muted element it plays
each track in), puts the call on hold and releases it (the browser's report, `onHold` in
`calls/page.ts`, lists the device's user meanwhile, neither side's packets reach the other, and the
release waits for the server's word), cuts its socket and watches the engine take its place back
with `resume`, and leaves; its log line also names what the signalling socket negotiated. Between
the hold and the cut it proves the video both ways: the browser shows video and turns Chromium's
fake camera on, the engine is asked to watch it, and the track that arrives is retained as
`CallVideo` retains one and given a sink that counts its frames (at least five) and reads their
upright shape (drawing needs a window, which the bridge's proof has); then the emulator's front
camera is opened through `CallCamera` and sent as the engine's camera, its own frames counted with
their rotation, and the browser decodes its frames (`framesDecoded` in `calls/page.ts`'s report) and
sees a picture in them (`cameraSample`: a frame drawn to a canvas, brighter than black); the camera
is moved to the back camera with the track kept; video flows on across the resume; and the camera is
stopped before the leave. Around that it checks the system's side: Telecom holds the call as
outgoing (`dialing`) and then active, and `dumpsys telecom` (run through the instrumentation's
shell) names the call by the id Telecom gave it; the call's service runs in the foreground with its
`CallStyle` notification posted, and the audio mode is the communication mode; the routes are
Telecom's endpoints, each with a kind, one current, and selecting the loudspeaker (and the earpiece
again, where the device has one) moves the call there; and leaving ends Telecom's call, stops the
service, takes the notification down, empties the routes, and gives the audio mode back. What the
browser must hear is what the device's microphone carried, which the device's own `media-source`
statistics tell: sound, when a tone is played to it, or silence, as on an emulator whose host audio
never reaches it. Through it all it watches the call's sounds (`CallSounds.listen`): the joining is
played as the engine connects and again as the browser joins, the leaving as the call is left, the
cut socket is said lost and, with the resume, back, each a `MediaPlayer` playing as the cue is
given, and for the first the system's own word on it, `dumpsys audio`'s players read on the spot:
one of this process with `USAGE_VOICE_COMMUNICATION`, started, which the log line quotes.
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
out of the snapshot and the capture off; then it leaves: the snapshots arrive in revision order with
none twice, a fresh `state()` is the latest published, the renewed session token is kept, and the
routes empty again. `scripts/check_mobile_call.py --platform android` at the repository's root runs
both end to end, with the same deployment and browser the iOS check uses: a deployment of its own
serving the call page (`client/packages/protocol/calls/`), `adb reverse` so the emulator reaches the
API server, the voice server, and the script's helper at its own loopback (plain HTTP there, as a
development build allows; media goes to the voice server's announced address over the network),
Chromium as the second participant with a tone WAV as its microphone and its fake camera
(`calls/second.ts`, which the script's helper drives: showing video, the camera on and off, and a
frame of someone's camera sampled), and `gradlew connectedDebugAndroidTest` on these two tests
alone, judging their JUnit reports (two tests run, none failed or erred, and none skipped but the
trailing test, which skips itself outside `--record-trailing`) and that each printed its own line,
with `ANDROID_SERIAL` set to the emulator's serial so a phone plugged in beside it is never touched.
With `--avd` it starts the emulator headless, recording the machine's microphone
(`-allow-host-audio`, without which the emulator records silence); the emulator's PulseAudio backend
does not start against PipeWire's server (``Could not init `pa' audio driver``), so there its
microphone is silent and the test checks the browser hears silence. The test reads its deployment,
channel, user, refresh token, and the script's helper from the instrumentation's arguments
(`-Pandroid.testInstrumentationRunnerArguments.ASPEN_CALL_*`; the test's comment lists them) and is
skipped without them, so the `android` job's `connectedDebugAndroidTest` runs it as a skip. CI's
`android-call` job runs the check itself: it builds the servers, the engine for the emulator, and
the web shell, and runs `check_mobile_call.py --start-services` inside the emulator action's step,
against the emulator it started (no `--avd`), whose audio is off, so there too the test checks the
browser hears silence.

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

### What only a phone can show

The emulator offers one route, its loudspeaker (`hasEarpiece` is false there), and has no other
call, so these are checked by hand on a device, with the call engine's test or with the app once the
interface commands the engine:

- [ ] The earpiece: the call plays at the ear, the screen turns off as the phone is raised to it and
      comes back as it is lowered, and the Speaker route turns the screen back on; the app's Speaker
      button does the same through the plugin.
- [ ] A join with `RECORD_AUDIO` refused fails with `errorKind` `microphone` in the call bar (the
      permission's prompt is Capacitor's; the emulator's test grants it beforehand), and one in a
      channel without Speak joins to listen.
- [ ] A Bluetooth headset: it is listed as `BLUETOOTH` with its name, a call joined with it
      connected starts on it, connecting it during a call moves the call to it, disconnecting it
      moves the call back, and with `BLUETOOTH_CONNECT` granted Telecom names every headset.
- [ ] A wired or USB headset: listed as `WIRED_HEADSET`, taken on connection, given up on removal.
- [ ] A phone call interrupting ours: the ring holds the call (`onSetInactive`, the user shown on
      hold to the others, who hear nothing of them, and hearing nothing themself), hanging the phone
      call up or declining it resumes ours (`onSetActive`) with the user's own mute and deafen
      standing, and ending ours from the phone's call screen or a watch leaves the engine's call.
- [ ] A navigation prompt or an alarm over the call: what Telecom's audio focus does with it, and
      whether the call is held or plays on beside it.
- [ ] The call's sounds (`CallSounds`): the joining is heard as the call connects and as someone
      else joins, the leaving as someone leaves and as the user leaves, a share starting, and the
      call lost and back (cut the network for a few seconds), each on the earpiece, on the
      loudspeaker, and on a headset, with the app in the foreground, in the background, and the
      screen off, at the call's volume beside the voices; and none while a phone call holds ours.
- [ ] The notification: Hang up leaves the call, a tap opens the app, and on Android 14 and later
      the call outlives the app in the background and the screen off for an hour, with the battery
      use the system attributes to it recorded.
- [ ] Android 8 to 13 (the `ConnectionService` path): the call appears in the phone's call screen,
      the same holds and resumes, and the foreground service alone keeps the process alive.
- [ ] The camera: the front camera's preview is mirrored and upright however the phone is held, the
      back camera's is not mirrored, the flip moves between them with the picture pausing only for
      the camera's own start, and the other side sees the picture upright through a turn.
- [ ] The camera in the background: with the app behind another, or in the picture-in-picture
      window, the camera stays on under the service's `camera` type and the others keep seeing it.
- [ ] The picture in picture: leaving the app (home, recents) while someone's camera or screen shows
      enters the mode on the focused screen (else the first camera of someone else), the page shows
      that picture alone in the window, the window's shape is the picture's, and opening the app
      again leaves the mode; on a phone before Android 12 the plugin enters it from
      `onUserLeaveHint`.
- [ ] Rotation: a full-screen picture turns with the phone without a flash of the old layout, and
      the paged tiles lay out again behind their holes after a turn.
- [ ] Trailing by eye: while paging between tiles and while a transition moves one, the surface view
      follows its hole; frame-step a screen recording (`adb shell screenrecord`) and judge how far
      it trails and whether the paged grid hides it.
- [ ] A phone without an earpiece (a tablet): `hasEarpiece` is false and the routes are the
      loudspeaker and whatever is connected.
