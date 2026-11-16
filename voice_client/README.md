# voice_client

The mobile apps' call engine: a client's side of a call without a browser, which drives the
platform's WebRTC, HTTP, and WebSockets through interfaces the iOS and Android layers implement.
`src/lib.rs` describes the crate; this file is about reaching it from Swift and Kotlin.

## What the platform layers lean on

- **The snapshot as JSON.** Beside each `CallState` the observer is handed the same snapshot as the
  JSON `state::CallState` serializes to (`CallObserver::state_changed`'s `json`;
  `Engine::state_json` for the latest), camelCase, whose shape is `call_bridge_schema.json`:
  `cargo run -p voice_client --bin call-bridge-schema` writes it at the repository root
  (gitignored), the client's `pnpm codegen` reads it into
  `packages/protocol/src/generated/callBridge.ts`, and the apps' `NativeCall` reads every snapshot
  as that type. The platform layers' `AspenCall` plugins hand the string to the web view untouched,
  beside the deployment's origin, and never convert a field; a field added to the state reaches the
  page by regenerating. Each snapshot's `revision` is what the page orders them by.
- **No microphone, no join where the user may speak.** `set_microphone(None)` before `join` is how a
  plugin joins when the system refused the microphone: the engine fails the join with
  `ErrorKind::Microphone` where the channel grants Speak, and joins to listen where it does not, as
  the web client does when the browser refuses it.

- **Hold.** `set_held` is the system's hold (CallKit's `CXSetHeldCallAction` and an interruption on
  iOS, Telecom's `onSetInactive` on Android): the engine sends `setState {muted, deafened, held}`,
  shows `CallState.held` at once, takes the microphone's track off its sender as it does for a mute,
  and clears `held` only when the server's `participantState` says so, as the web client's
  `VoiceCall.setHeld` does. The voice server pauses everything the user sends (a camera or share
  stays on its sender here, paused there), sends them nothing, keeps their own mute and deafen for
  the release, and tells everyone; each `Participant.held` is another's hold, from `ready` and
  `participantState`. Holding is never rate limited, releasing is, and a refused release leaves the
  call on hold like a refused unmute. The layers keep no mute or deafen of their own around a hold:
  the server keeps them.
- **Nothing is kept per mid between offers.** Every offer is read whole (`Live::sections` and the
  transceiver of each mid are rebuilt from it, and each received track read again through
  `receiver_track`), since the server may reuse a section for other content: a section moved into
  another msid stream arrives on new SSRCs, one taken over within its stream keeps them. The
  platform's peer connection applies the remote description itself and derives its receive streams
  from each offer's `a=ssrc` lines; the layers key their transceivers by the transceiver (iOS by its
  sender's id, Android by its number), never by mid, and a platform layer that cached anything by
  mid across offers would show the wrong person.

## The bindings

[UniFFI](https://mozilla.github.io/uniffi-rs/) writes a Swift module, `VoiceClient`, and a Kotlin
package, `com.aspen_chat.client.voice`, from the interface the crate marks for export
(`src/bindings.rs` says what crosses as it is and what is mirrored at the boundary). Both are
written from the engine built for this machine, since the generator reads the interface from the
built library:

```
scripts/voice_client_bindings.py
```

That builds the crate (`cargo build -p voice_client`) and writes `VoiceClient.swift`,
`VoiceClientFFI.h`, and `VoiceClientFFI.modulemap` to `client/packages/mobile/ios/VoiceClient/` and
`voice_client.kt` to
`client/packages/mobile/android/app/src/main/java/com/aspen_chat/client/voice/`. Both places are
gitignored, like the client's other generated code; write them again whenever the exported interface
changes. `--language swift|kotlin` writes one, `--swift-dir` and `--kotlin-dir` write elsewhere, and
`--release` reads the release build. The generator itself is the crate's `uniffi-bindgen` binary
(`src/bin/uniffi-bindgen.rs`), behind the `bindgen` feature so the apps' builds never compile it;
`uniffi.toml` names the module and the package.

The apps link the crate built for their targets, which `rust-toolchain.toml` lists: the iOS app and
its simulator the `staticlib` (`aarch64-apple-ios`, `aarch64-apple-ios-sim`, each built against its
SDK by `client/packages/mobile/ios/scripts/call_engine.py`), the Android app and its emulator the
`cdylib` (`aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`, each built
with the NDK's clang as `CARGO_TARGET_<TARGET>_LINKER` and `CC_<target>` by
`client/packages/mobile/android/scripts/call_engine.py`, which writes the Kotlin package too). The
Kotlin package needs JNA and kotlinx-coroutines on the app's classpath; the Swift module needs only
Foundation. Each app's platform layer and how it is checked against a real deployment are
`client/docs/architecture/ios-app.md` and `android-builds.md`, The call engine.

## Checking the bindings

`bindings-check/` proves each side compiles and talks to the engine: stand-ins for the platform
start the engine, join a call the stand-in API server refuses, read the refusal from `state()` and
the observer, and stop it.

- `bindings-check/kotlin.sh` runs on any machine with a JDK and `kotlinc` (the
  [Kotlin compiler release](https://github.com/JetBrains/kotlin/releases), on the PATH or named by
  `KOTLINC`); it fetches JNA and kotlinx-coroutines from Maven Central once into
  `target/bindings-check/jars/` and runs the check with the host `cdylib`.
- `bindings-check/swift.sh` runs on a Mac with Xcode's `swiftc`, against the host `staticlib`.
