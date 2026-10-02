#!/bin/sh
# Compiles the extension's Web Push decryption with RFC 8291's example and runs it, on a Mac
# with Xcode's toolchain; no project or target is needed.
set -e
cd "$(dirname "$0")/.."
out="$(mktemp -d)"
# Top-level code compiles only in a file named main.swift.
cp scripts/check_webpush.swift "$out/main.swift"
xcrun swiftc -O -o "$out/check" App/AspenPush/WebPush.swift "$out/main.swift"
"$out/check"
rm -rf "$out"
