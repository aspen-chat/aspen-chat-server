#!/bin/sh
# Runs the notification service extension's work as a Mac program (`check_notification.swift`,
# with the extension's own sources) on a kept PushState and an APNs payload, on a Mac with
# Xcode's toolchain; no project or target is needed. The deployment the state names is reached.
#
#   scripts/check_notification.sh state.json payload.json
set -e
if [ "$#" -ne 2 ]; then
    echo "usage: $0 <state.json> <payload.json>" >&2
    exit 2
fi
state="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
payload="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
cd "$(dirname "$0")/.."
out="$(mktemp -d)"
# Top-level code compiles only in a file named main.swift.
cp scripts/check_notification.swift "$out/main.swift"
xcrun swiftc -O -o "$out/check" App/AspenPush/WebPush.swift App/AspenPush/PushState.swift \
    App/AspenNotificationService/NotificationService.swift "$out/main.swift"
"$out/check" "$state" "$payload"
rm -rf "$out"
