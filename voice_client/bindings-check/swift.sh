#!/usr/bin/env bash
# On a Mac: builds the engine for this machine, writes its Swift bindings, compiles them with
# `Check.swift` against the static library, and runs the check.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/target/bindings-check"

rm -rf "$out/swift"
python3 "$root/scripts/voice_client_bindings.py" --language swift --swift-dir "$out/swift"
swiftc -parse-as-library \
  -I "$out/swift" -Xcc -fmodule-map-file="$out/swift/VoiceClientFFI.modulemap" \
  "$out/swift/VoiceClient.swift" "$root/voice_client/bindings-check/Check.swift" \
  "$root/target/debug/libvoice_client.a" \
  -o "$out/check-swift"
"$out/check-swift"
