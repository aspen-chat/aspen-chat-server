#!/usr/bin/env python3
"""Writes the call engine's Swift module and Kotlin package from the engine built for this machine.

UniFFI reads the engine's interface from the metadata the built library carries, so the library is
built first (`cargo build -p voice_client`, for the host) and the generator is run on it. The Swift
module (`VoiceClient.swift`, `VoiceClientFFI.h`, `VoiceClientFFI.modulemap`) goes to the iOS app's
`VoiceClient/` directory and the Kotlin package (`voice_client.kt`) under the Android app's
`java/com/aspen_chat/client/voice/`; both are gitignored. `voice_client/README.md` says more.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SWIFT_DIR = ROOT / "client/packages/mobile/ios/VoiceClient"
# The generator appends the package's path (`com/aspen_chat/client/voice/`) to this itself.
KOTLIN_DIR = ROOT / "client/packages/mobile/android/app/src/main/java"


def built_library(release: bool) -> Path:
    """Builds the engine for this machine and returns its shared library."""
    command = ["cargo", "build", "-p", "voice_client", "--message-format=json-render-diagnostics"]
    if release:
        command.append("--release")
    result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True)
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        if not message["package_id"].split("#")[0].endswith("voice_client") and "voice_client" not in message["target"]["name"]:
            continue
        if "cdylib" not in message["target"]["crate_types"]:
            continue
        for filename in message["filenames"]:
            if filename.endswith((".so", ".dylib", ".dll")):
                return Path(filename)
    sys.exit("cargo built no shared library for voice_client")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--language", choices=["swift", "kotlin"], action="append",
                        help="write only this language (default: both)")
    parser.add_argument("--swift-dir", type=Path, default=SWIFT_DIR)
    parser.add_argument("--kotlin-dir", type=Path, default=KOTLIN_DIR)
    parser.add_argument("--release", action="store_true", help="build and read the release library")
    args = parser.parse_args()
    languages = args.language or ["swift", "kotlin"]
    library = built_library(args.release)
    for language in languages:
        out_dir = args.swift_dir if language == "swift" else args.kotlin_dir
        out_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["cargo", "run", "--quiet", "-p", "voice_client", "--features", "bindgen",
             "--bin", "uniffi-bindgen", "--", "generate", "--library", str(library),
             "--language", language, "--out-dir", str(out_dir), "--no-format"],
            cwd=ROOT, check=True,
        )
        print(f"{language}: {out_dir}")


if __name__ == "__main__":
    main()
