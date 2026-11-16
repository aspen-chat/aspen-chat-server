#!/usr/bin/env python3
"""Builds what the Android app packages of the call engine: the engine (`voice_client`) for the
emulator and for every ARM device, and its Kotlin bindings, where the Gradle build reads them
(`client/docs/architecture/android-builds.md`, The call engine).

    client/packages/mobile/android/scripts/call_engine.py [--emulator-only | --abi <abi>...]

The engine's shared library is built with cargo for `x86_64-linux-android`, `aarch64-linux-android`,
and `armv7-linux-androideabi` (which `rust-toolchain.toml` lists, so rustup installs them) with the
NDK's clang, into `target/<triple>/release/`, from where `app/build.gradle` lays it out by ABI
(`--abi` names the ABIs to build for, `arm64-v8a` for a phone attached by USB; `--emulator-only` is
`--abi x86_64`), and
`scripts/voice_client_bindings.py` writes the Kotlin package into the app's
`java/com/aspen_chat/client/voice/`. The WebRTC library the app links, `target/webrtc/libwebrtc.aar`,
is built once with `scripts/webrtc_build.py build` (`docs/development/webrtc-build.md`); this says
so when it is missing, and builds it never. The NDK is the newest under `$ANDROID_HOME/ndk`
(`~/Android/Sdk` when unset), or the one `ANDROID_NDK_HOME` names.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

ANDROID = Path(__file__).resolve().parent.parent
REPO = ANDROID.parents[3]
# The cargo target and the NDK's triple of each ABI the app ships, so every phone runs the same
# call path; `app/build.gradle` reads the same ABI from each target's directory.
TARGETS = {
    "x86_64-linux-android": "x86_64-linux-android",
    "aarch64-linux-android": "aarch64-linux-android",
    "armv7-linux-androideabi": "armv7a-linux-androideabi",
}
ABIS = {
    "x86_64": "x86_64-linux-android",
    "arm64-v8a": "aarch64-linux-android",
    "armeabi-v7a": "armv7-linux-androideabi",
}
# What the app's `minSdkVersion` is, which names the NDK's clang for each target.
MIN_SDK = 26
WEBRTC = REPO / "target" / "webrtc" / "libwebrtc.aar"


def ndk() -> Path:
    named = os.environ.get("ANDROID_NDK_HOME")
    if named:
        return Path(named)
    sdk = Path(os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or Path.home() / "Android" / "Sdk")
    versions = sorted((sdk / "ndk").glob("*"), key=lambda p: [int(n) for n in p.name.split(".") if n.isdigit()])
    if not versions:
        sys.exit(f"call_engine.py: no NDK under {sdk / 'ndk'}; install one with sdkmanager, or name it in ANDROID_NDK_HOME")
    return versions[-1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    which = parser.add_mutually_exclusive_group()
    which.add_argument("--emulator-only", action="store_true", help="build the engine for the x86_64 emulator alone")
    which.add_argument("--abi", action="append", choices=sorted(ABIS), metavar="ABI",
                       help=f"build for this ABI alone, repeatable ({', '.join(sorted(ABIS))}); every ABI when not given")
    args = parser.parse_args()
    if not WEBRTC.exists():
        sys.exit(f"call_engine.py: {WEBRTC} is missing; build it with scripts/webrtc_build.py build")
    tools = ndk() / "toolchains" / "llvm" / "prebuilt" / "linux-x86_64" / "bin"
    if not tools.exists():
        sys.exit(f"call_engine.py: {tools} is missing; the engine is built for Android on x86-64 Linux")
    if args.emulator_only:
        targets = [ABIS["x86_64"]]
    elif args.abi:
        targets = [ABIS[abi] for abi in dict.fromkeys(args.abi)]
    else:
        targets = list(TARGETS)
    for target in targets:
        print(f"call_engine.py: building voice_client for {target}", flush=True)
        # Cargo links with the NDK's clang for the target and the app's lowest Android, and any C
        # a crate compiles goes through it too; cargo's environment names are the triple with
        # underscores for `CC` and upper-cased for the linker.
        clang = tools / f"{TARGETS[target]}{MIN_SDK}-clang"
        underscored = target.replace("-", "_")
        env = {
            **os.environ,
            f"CARGO_TARGET_{underscored.upper()}_LINKER": str(clang),
            f"CC_{underscored}": str(clang),
            f"AR_{underscored}": str(tools / "llvm-ar"),
        }
        # Only the shared library is built: the crate's `staticlib` is iOS's.
        subprocess.run(
            ["cargo", "rustc", "--release", "-p", "voice_client", "--lib", "--target", target,
             "--crate-type", "cdylib"],
            cwd=REPO, env=env, check=True,
        )
    print("call_engine.py: writing the Kotlin bindings", flush=True)
    subprocess.run([sys.executable, str(REPO / "scripts" / "voice_client_bindings.py"), "--language", "kotlin"],
                   cwd=REPO, check=True)


if __name__ == "__main__":
    main()
