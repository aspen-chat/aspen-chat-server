# voice_client

The mobile apps' call engine: a client's side of a call without a browser, which drives the
platform's WebRTC, HTTP, and WebSockets through interfaces the iOS and Android layers implement.
`src/lib.rs` describes the crate; this file is about reaching it from Swift and Kotlin.

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
`cdylib` (`aarch64-linux-android`, `x86_64-linux-android`, each built with the NDK's clang as
`CARGO_TARGET_<TARGET>_LINKER` and `CC_<target>`). The Kotlin package needs JNA and
kotlinx-coroutines on the app's classpath; the Swift module needs only Foundation.

## Checking the bindings

`bindings-check/` proves each side compiles and talks to the engine: stand-ins for the platform
start the engine, join a call the stand-in API server refuses, read the refusal from `state()` and
the observer, and stop it.

- `bindings-check/kotlin.sh` runs on any machine with a JDK and `kotlinc` (the
  [Kotlin compiler release](https://github.com/JetBrains/kotlin/releases), on the PATH or named by
  `KOTLINC`); it fetches JNA and kotlinx-coroutines from Maven Central once into
  `target/bindings-check/jars/` and runs the check with the host `cdylib`.
- `bindings-check/swift.sh` runs on a Mac with Xcode's `swiftc`, against the host `staticlib`.
