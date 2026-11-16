# Android builds in CI

CI's `android` job in `.github/workflows/ci.yml` builds the Android app, runs its JVM tests and its
device tests on an emulator, and keeps the APK it built as the artifact `android-apk-debug` on every
pull request and every push to `main`, for 14 days. The app also packages the mobile call engine and
the Kotlin layer under it, which the second half of this page describes; CI's job builds without
them, since it has no WebRTC library.

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
engine holds will be a self-managed call through Telecom's `ConnectionService`, which Android has
from 8.0, and WebRTC's audio output takes its low-latency mode from the same version.

## The call engine

The mobile apps' calls run natively, in the call engine (`voice_client/` at the repository's root, a
Rust crate: `voice_client/src/lib.rs` describes it), which drives the platform's WebRTC, HTTP, and
WebSockets through interfaces each platform implements. The Android implementation is
`app/src/main/java/com/aspen_chat/client/call/`, Kotlin, compiled into the app with the engine's
generated Kotlin package (`com.aspen_chat.client.voice`). Nothing in the web view uses it yet: the
Capacitor plugin that will command it and show its state, the call's sounds, Telecom, the foreground
service a call needs to go on behind other apps, the camera, and the choice of route are later
pieces.

### What the app packages, and where it comes from

| Piece                                                                                                     | Where the Gradle build reads it                                                                                                                                                                                                   | Made by                                                                                                                                                            |
| --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Google's WebRTC, `libwebrtc.aar` (`org.webrtc`, its library for `arm64-v8a`, `armeabi-v7a`, and `x86_64`) | `target/webrtc/`, as a file dependency                                                                                                                                                                                            | `scripts/webrtc_build.py build` on Linux, once (`docs/development/webrtc-build.md`)                                                                                |
| The engine, `libvoice_client.so`                                                                          | `target/x86_64-linux-android/release/` for the emulator, `target/aarch64-linux-android/release/` and `target/armv7-linux-androideabi/release/` for devices, laid out by ABI into the APK's `jniLibs` by the task `copyCallEngine` | `android/scripts/call_engine.py` (`cargo rustc --crate-type cdylib` for each target, linked with the NDK's clang)                                                  |
| Its Kotlin package, `voice_client.kt`                                                                     | `app/src/main/java/com/aspen_chat/client/voice/`, gitignored                                                                                                                                                                      | `scripts/voice_client_bindings.py --language kotlin` (`voice_client/README.md`)                                                                                    |
| JNA (`net.java.dev.jna:jna`, the `aar`), kotlinx-coroutines, OkHttp                                       | Maven Central; the versions are in `variables.gradle`                                                                                                                                                                             | The bindings load the engine's library through JNA and suspend through coroutines; the layer's HTTP and WebSockets are OkHttp's, which the app did not have before |

`android/scripts/call_engine.py` builds the engine for the three targets (or `--emulator-only`) and
writes the bindings, and says when the WebRTC library is missing. The app is Java with this Kotlin
beside it, so `build.gradle` applies the Kotlin plugin, compiling to the same JVM target as the
Java. When the WebRTC library or the bindings are absent, as in CI's job, `app/build.gradle` leaves
the `call/` and `voice/` sources out of the build and says so, and the app builds as it did without
them.

### The layer

Each file under `call/` implements one of the engine's interfaces over the platform, as
`voice_client/src/bindings.rs` declares them, mirroring the iOS layer (`ios-app.md`, The call
engine) where Android allows:

| File                    | Implements                                                                   | Over                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CallEngine.kt`         | The assembly                                                                 | Starts an `Engine` for one deployment with the parts below; one WebRTC factory and one track registry for the process                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CallSnapshots.kt`      | `CallObserver`                                                               | Keeps the latest `CallState` and calls each listener on the main thread                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `CallHttp.kt`           | `Http`                                                                       | OkHttp: no cookies, no redirects followed, every status answered as a response; a bodiless POST, which the engine's are, is sent with an empty body, since OkHttp refuses one without                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `CallSockets.kt`        | `SocketFactory`, `Socket`                                                    | OkHttp's WebSocket, which answers pings itself; `connect` waits for the upgrade, so a refusal is a `TransportException`; frames queue for `receive`, and a close from either side ends the queue                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallCredentials.kt`    | `Credentials`                                                                | The account at the deployment among those `PushStore` keeps (the state the web view writes on sign-in): its session token for each request, renewed through `SessionRefresh` (shared with the push handler) and written back when the API server refuses it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `CallTracks.kt`         | The track ids the engine passes around                                       | WebRTC tracks by their own `id()`: the microphone as an object, and each received track as the connection that receives it, which gives a live wrapper on demand                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `CallPeerConnection.kt` | `PeerConnectionFactory`, `PeerConnection`, `PeerConnectionObserver`'s caller | `org.webrtc.PeerConnection`, unified plan, `max-bundle`, RTCP muxed; the factory with the device's hardware video codecs through one EGL context and `JavaAudioDeviceModule` with the device's echo canceller and noise suppressor where it has them. `getTransceivers()` makes new Java wrappers on every call and disposes the ones before, their senders, receivers, and the receivers' tracks with them, so no wrapper is kept: transceivers are told apart by their sender's id and numbered in the order first seen, and each use reads them afresh under a lock. A sender's parameters object is kept between `getParameters` and `setParameters`; a receiver's volume is its `AudioTrack.setVolume`; a statistics report is written as the W3C array with `id`, `type`, and `timestamp` in milliseconds beside each entry's values; `close` disposes the connection, after which `connectionState` is `Closed` |
| `CallAudio.kt`          | The audio mode and the microphone                                            | `AudioManager` in `MODE_IN_COMMUNICATION` for the call, set by the app until a self-managed Telecom connection owns it (`configureForCall` and `release` are the seams its state changes will drive), and the microphone as a track from WebRTC's audio device module                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |

Where it differs from iOS: the iOS layer keeps WebRTC's transceiver objects, whose lifetime is the
connection's; Android's are read anew each time, as above. iOS's audio session is configured and
activated in two steps for CallKit's sake; Android sets one mode, and Telecom will set it instead.
WebRTC's Java encodings take a whole number of frames a second, so a fractional `maxFramerate` is
rounded.

### The proof

`androidTest`'s `CallEngineTest` runs the engine with this layer against a real deployment, in the
app's process on the emulator, WebRTC on the real `JavaAudioDeviceModule`: it joins a voice
channel's call, has a second participant brought in from a browser, reads both sides' statistics for
the other's sound (the browser's as the loudest sample off its received tracks, `loudness` in
`calls/page.ts`, since Chromium's own level statistics read zero behind the muted element it plays
each track in), cuts its socket and watches the engine take its place back with `resume`, and
leaves. What the browser must hear is what the device's microphone carried, which the device's own
`media-source` statistics tell: sound, when a tone is played to it, or silence, as on an emulator
whose host audio never reaches it. `scripts/check_mobile_call.py --platform android` at the
repository's root runs it end to end, with the same deployment and browser the iOS check uses: a
deployment of its own serving the call page (`client/packages/protocol/calls/`), `adb reverse` so
the emulator reaches the API server, the voice server, and the script's helper at its own loopback
(plain HTTP there, as a development build allows; media goes to the voice server's announced address
over the network), Chromium as the second participant with a tone WAV as its microphone
(`calls/second.ts`), and `gradlew connectedDebugAndroidTest` on this test alone. With `--avd` it
starts the emulator headless, recording the machine's microphone (`-allow-host-audio`, without which
the emulator records silence); the emulator's PulseAudio backend does not start against PipeWire's
server (`Could not init \`pa' audio
driver`), so there its microphone is silent and the test checks the browser hears silence. The test reads its deployment, channel, user, refresh token, and the script's helper from the instrumentation's arguments (`-Pandroid.testInstrumentationRunnerArguments.ASPEN_CALL_*`; the test's comment lists them) and is skipped without them, so CI's `connectedDebugAndroidTest`
runs it as a skip.
