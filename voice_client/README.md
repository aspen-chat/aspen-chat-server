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
- **The call's sounds as cues.** Beside each snapshot the observer is told which of the call's
  sounds the change cues (`CallObserver::play_sound`, a `SoundCue`: the call joined, left, lost,
  back, a share started, a transfer accepted or complete), derived in `sounds` from the engine's own
  transitions by the web client's rules, repeats already spaced. The platform plays the cue's
  recording on the call's own audio path: iOS on the session CallKit activated, Android as a voice
  communication, each at most two of one cue at once; the apps' layers say how
  (`client/docs/architecture/ios-app.md` and `android-builds.md`, Sounds).
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
- **Files are streams, never whole.** A transfer's file is read and written through `Files`,
  `FileSource`, and `FileSink` (`src/files.rs`), by references the platform gave the page: a picked
  file is opened anew for each transfer of it and read `CHUNK_BYTES` at a time as the channel takes
  them, a received file is written a chunk at a time into the place the user chose and kept by
  `finish` or removed by `discard`, and `release` lets a picked file go once nothing offers or sends
  it. Each transfer's peer connection is the platform's too, with a `DataChannel` the engine sends
  on and a `DataChannelObserver` it reads from; the platform tells of its candidates and of a
  channel the other side opened through `PeerConnectionObserver`. `state::FilesState` is what the
  page shows (`docs/architecture/file-transfers.md`, Client).
- **A share has a connection of its own.** A WebRTC factory has one audio capture, the audio device
  module's, for every connection it makes, so a share's sound (`screenAudio`) cannot be a second
  track beside the microphone on the call's connection. The phones send a share with
  `start_external_source`: the engine asks the server with `send {external: true}`, the server
  offers the capture connection (`ConnectionKind::Capture`, the one the desktop shell's game capture
  helper holds), and the engine answers it on a peer connection the platform's
  `PeerConnectionFactory::create` makes for `Capture`, from a factory whose audio capture is the
  share's sound; the call's connection is left alone. The share's picture and sound go on that one
  connection, so one transport carries them and the server puts them in one stream for everyone else
  (`{user}-share`). Each `LocalSource` says whether it is `external`. The connection is made for its
  first offer and closed with the last external source stopped, as the server lets its own go; a
  loss of it asks for ICE restarts of its own (`restartIce {connection: capture}`) and ends the
  share, never the call, after the recovery limit; a connection that cannot be made, an offer that
  cannot be applied, or the server stopping the share (`error` answering `send`, then `sources`
  naming what this client still sends) ends the share with the reason in `source_refused`, as the
  screen's.
- **Nothing is kept per mid between offers.** Every offer is read whole (`Live::sections` and the
  transceiver of each mid are rebuilt from it, and each received track read again through
  `receiver_track`), since the server may reuse a section for other content: a section moved into
  another msid stream arrives on new SSRCs, one taken over within its stream keeps them. The
  platform's peer connection applies the remote description itself and derives its receive streams
  from each offer's `a=ssrc` lines; the layers key their transceivers by the transceiver (iOS by its
  sender's id, Android by its number), never by mid, and a platform layer that cached anything by
  mid across offers would show the wrong person.

- **Rings.** A phone woken by a `call` push (`spec/push.md`, Calls) shows the incoming call from the
  push alone, and holds a `RingWatch` (`src/ring.rs`, `src/ring_bindings.rs`) while it rings: the
  watch opens the deployment's event stream as the account, over the same `Http` and `SocketFactory`
  a call runs on, and tells its `RingObserver` once when the ring ends anywhere else
  (`answeredElsewhere`, `declined`, `callEnded`, `timedOut` by the server's clock from `ready`'s
  `serverTime`, or `signedOut` when no session can be had), so the platform ends its incoming call
  with that reason; `stop` ends the watch with no report when this phone answered or declined.
  Answering is the engine's ordinary `join` of the ring's channel; declining is the platform's own
  `DELETE /channels/{channel}/voice/rings/@me`. Nothing is pushed to end a ring.

## The bindings

[UniFFI](https://mozilla.github.io/uniffi-rs/) writes a Swift module, `VoiceClient`, and a Kotlin
package, `com.aspen_chat.client.voice`, from the interface the crate marks for export
(`src/bindings.rs` says what crosses as it is and what is mirrored at the boundary;
`src/ring_bindings.rs` adds the ring watch over the same platform traits). Both are written from the
engine built for this machine, since the generator reads the interface from the built library:

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

`bindings-check/` proves each side compiles and talks to the engine: stand-ins for the platform (its
network, WebRTC, and files) start the engine, join a call the stand-in API server refuses, read the
refusal from `state()` and the observer, and stop it.

- `bindings-check/kotlin.sh` runs on any machine with a JDK and `kotlinc` (the
  [Kotlin compiler release](https://github.com/JetBrains/kotlin/releases), on the PATH or named by
  `KOTLINC`); it fetches JNA and kotlinx-coroutines from Maven Central once into
  `target/bindings-check/jars/` and runs the check with the host `cdylib`.
- `bindings-check/swift.sh` runs on a Mac with Xcode's `swiftc`, against the host `staticlib`.
