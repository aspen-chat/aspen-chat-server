#!/usr/bin/env python3
"""Builds what the iOS app links of the call engine, on a Mac: the engine (`voice_client`) for
the simulator and for devices, and its Swift bindings, where the Xcode project reads them
(`client/docs/architecture/ios-app.md`, The call engine).

    client/packages/mobile/ios/scripts/call_engine.py [--release] [--simulator-only]

The engine's static library is built with cargo for `aarch64-apple-ios-sim` and `aarch64-apple-ios`
(which `rust-toolchain.toml` lists, so rustup installs them), against each target's SDK, into
`target/<triple>/release/`, where the project's library search paths point, and
`scripts/voice_client_bindings.py` writes the Swift module to `client/packages/mobile/ios/VoiceClient/`. The WebRTC framework the project links,
`target/webrtc/WebRTC.xcframework`, is built once with `scripts/webrtc_build.py build`
(`docs/development/webrtc-build.md`); this says so when it is missing, and builds it never.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

IOS = Path(__file__).resolve().parent.parent
REPO = IOS.parents[3]
TARGETS = ["aarch64-apple-ios-sim", "aarch64-apple-ios"]
WEBRTC = REPO / "target" / "webrtc" / "WebRTC.xcframework"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--simulator-only", action="store_true", help="build the engine for the simulator alone")
    args = parser.parse_args()
    if sys.platform != "darwin":
        sys.exit("call_engine.py: the engine is built for iOS on a Mac")
    if not WEBRTC.exists():
        sys.exit(f"call_engine.py: {WEBRTC} is missing; build it with scripts/webrtc_build.py build")
    targets = TARGETS[:1] if args.simulator_only else TARGETS
    for target in targets:
        print(f"call_engine.py: building voice_client for {target}", flush=True)
        # The C the crates compile (aws-lc among them) and the objects are built against the
        # target's SDK rather than the Mac's own, and marked for the app's deployment target,
        # so the app's link has nothing to warn about. Only the static library is built: the
        # crate's `cdylib` is Android's, and a dynamic library cannot be linked for iOS here.
        sdk = "iphonesimulator" if target.endswith("-sim") else "iphoneos"
        sdk_path = subprocess.run(["xcrun", "--sdk", sdk, "--show-sdk-path"], capture_output=True, text=True,
                                  check=True).stdout.strip()
        env = {**os.environ, "SDKROOT": sdk_path, "IPHONEOS_DEPLOYMENT_TARGET": "15.0"}
        subprocess.run(
            ["cargo", "rustc", "--release", "-p", "voice_client", "--lib", "--target", target,
             "--crate-type", "staticlib"],
            cwd=REPO, env=env, check=True,
        )
    print("call_engine.py: writing the Swift bindings", flush=True)
    subprocess.run([sys.executable, str(REPO / "scripts" / "voice_client_bindings.py"), "--language", "swift"],
                   cwd=REPO, check=True)


if __name__ == "__main__":
    main()
