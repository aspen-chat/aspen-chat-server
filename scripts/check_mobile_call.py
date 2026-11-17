#!/usr/bin/env python3
"""Check the mobile call engine with a phone's platform under it, on the iOS simulator of a Mac, an
iPhone paired with that Mac, the Android emulator here, or an Android phone attached here by USB,
against built servers here.

Runs a deployment of its own (`stack.py`), whose web client is the call page of the client's own
call code (`client/packages/protocol/calls/`), and runs the platform's tests of the engine against
it: the engine's own (`CallEngineTests` in the iOS app's `AppTests`, `CallEngineTest` in the Android
app's `androidTest`), in which the engine joins the call, a second participant is brought in from
Chromium here with a tone as its microphone, each side's statistics show the other's sound, a file
is sent each way between them and its bytes compared by hash (the browser's offer accepted directly,
the phone's through the relay), the socket is cut and resumed, and the call is left; and the web
view's bridge to it
(`AspenCallPluginTests`, `AspenCallPluginTest`), in which the app's own page drives the `AspenCall`
plugin through the Capacitor bridge, joins the same channel, reads the engine's snapshots and
routes as they reach it, and shares the screen, which the browser decodes and hears the sound of;
and, on iOS, the rings' (`CallRingsTests`, against a stand-in deployment of their own), in which a
call push is reported to CallKit and its answer joins the engine with the microphone. A simulator
or an emulator reaches the API server and the voice server at its own loopback, `http://127.0.0.1`,
which the engine takes in plain text as a debuggable build does; a phone reaches them over the
network, and media goes straight to the voice server's announced address over the network either
way.

    scripts/check_mobile_call.py --platform ios --bin target/debug [--build] [--mac aspen-mac]
        [--worktree '~/aspen-native-calls'] [--derived '~/aspen-derived-native-calls']
        [--device <udid> [--address <this machine's address>]]
    scripts/check_mobile_call.py --platform android --bin target/debug [--build]
        [--avd aspen-test | --device <serial>] [--microphone silent|live] [--only <class>]...

iOS: the simulator shares the Mac's network, so the Mac's loopback is tunnelled back here over SSH.
`--build` first builds the engine for the simulator and its bindings on the Mac
(`client/packages/mobile/ios/scripts/call_engine.py`) and the app and tests for the simulator
(`build-for-testing`); the Mac's worktree must hold this commit and the WebRTC framework. It refuses
to run while another `xcodebuild` is on the Mac. `--runner simctl` runs the built tests without
`xcodebuild test`: the app built for testing is installed on the booted simulator and launched with
XCTest's bundle injection, for a Mac whose `xcodebuild` cannot see simulators (an Xcode whose
CoreSimulator was never installed, which takes an administrator); its `--build` builds for the
generic simulator destination for the same reason.

With `--device <udid>` (`xcrun devicectl list devices` on the Mac) the check runs on an iPhone
paired with the Mac instead, which reaches this machine over the network they share and takes
nothing in plain text there: the API server, the voice server, and the helper are each served over
TLS on this machine's address on that network (`--address`, else the address this machine reaches
the Mac from), with a certificate from the development authority (`stack.TlsFront`,
`scripts/dev_ca.py`), which the phone must trust (`scripts/dev_ca.py profile` writes it as a
configuration profile; `docs/development/local-https.md`, and `client/docs/architecture/ios-app.md`,
The proof, say the steps on the phone), and the voice server is registered at its TLS address, which
the browser here reaches by pinning the certificate's key. `--build` builds the engine for devices
(`call_engine.py --device-only`) and the app and tests for a device (`-sdk iphoneos`, the generic
device destination, so the build never touches the phone), signed for the team with automatic
provisioning (`-allowProvisioningUpdates`) by the identity in `--keychain`, a keychain of the Mac's
made for builds over SSH, whose password `--keychain-password-file` holds, since an SSH session
cannot use the login keychain; the identity and profile it signed with are reported. Before the
tests the phone must be paired and reachable from the Mac with Developer Mode on
(`Ios.ready_phone`), and unlocked, which `devicectl` cannot see, so it is said; then
`xcodebuild test-without-building` installs the app and its tests on the phone and runs the
engine's and the bridge's tests there (not the rings': the simulator's CallKit takes their
synthetic incoming calls quietly, where a phone would ring; a real ring is the checklist's). Each
test's first request says by name when the phone does not trust the authority (`trustFailure`) or
cannot reach this machine (`unreachable`), which the check repeats with what to do; the phone asks
to allow Aspen on the local network and to use the microphone on the first run, which someone at
the phone answers. On the phone the tests expect what a phone gives (`ASPEN_CALL_MICROPHONE=live`,
`ASPEN_CALL_SYSTEM_CALL=stands`: WebRTC's own audio device, CallKit keeping the call and activating
the session), on the simulator what it gives (`tone`, `ends`); the tests' `CallCheck` says what
each means.

Android: the emulator or phone reaches this machine's loopback through `adb reverse`, which a USB
device takes as an emulator does; media goes to the voice server's announced address, which a
phone reaches over the network it shares with this machine. With `--avd` the emulator is started
here, headless, recording this machine's microphone (`-allow-host-audio`, without which it
records silence), and refused while another emulator is running; with `--device` the phone `adb`
lists under that serial is used, and an emulator is refused (its serial is `emulator-<port>`, and
`ro.kernel.qemu` is set on it); without either, the one emulator `adb` sees is used, and a phone
attached alone is refused until it is given as `--device`. Whichever it is, every `adb` and Gradle
command is pinned to it (`ANDROID_SERIAL`), so whatever else is attached, a phone beside the
emulator or an emulator beside the phone, is left alone.

What the tests expect the browser to hear is `--microphone`: `silent` on an emulator, since one
whose host audio never reaches it carries silence, as here, where the emulator's PulseAudio backend
fails to start against PipeWire's server ("Could not init `pa' audio driver" in
`target/mobile-call/emulator.log`), and the test checks the browser hears silence of it; `live` on
a phone, whose microphone hears the room, where the test checks that the browser receives and
decodes the phone's packets and reports the level it hears, whatever it is, since what a room
sounds like is no test's to assert. `--build` first builds the engine for the emulator (`--avd`
or neither) or for the phone's ABI (`--device`) and its bindings
(`client/packages/mobile/android/scripts/call_engine.py`); the Gradle build then runs the tests
(`connectedDebugAndroidTest`), on a phone `CallDeviceTest` too, what only a phone can show, which
needs the web shell copied in (`npx cap sync android` in `client/packages/mobile`) and
`target/webrtc/libwebrtc.aar`. `--only` runs one test class (or several, repeated) against the same
deployment, for working on one test's failure without the others' minutes before it.

The servers' logs of the run are left as `target/mobile-call/api.log` and `voice.log`, and on
Android the device's log, its crash buffer included, as `target/mobile-call/logcat.txt`: a native
crash's tombstone is in it, for `ndk-stack -sym <dir of the unstripped library>`.

`--record-trailing` records instead of checking: it runs the one test that moves a hole across the
screen with the camera drawn in it (`testPictureFollowsAMovingHole`, `pictureFollowsAMovingHole`)
while the simulator's or emulator's screen is recorded, and leaves the recording as
`target/mobile-call/trailing-<platform>.mp4` for `scripts/measure_video_trailing.py`, which says
how far the native picture trails the page's box in each frame. It needs no deployment, so none is
run and nothing is tunnelled or reversed.

Both need the client's dependencies installed here and Playwright's Chromium.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import math
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import wave
import xml.etree.ElementTree as ElementTree
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import dev_ca
from check_calls import build_page, PAGE, PROTOCOL, person
from stack import Failed, Ports, Stack, TlsFront, clean_env, start_services, wait_for, wait_for_services

REPO = Path(__file__).resolve().parent.parent
IOS = REPO / "client" / "packages" / "mobile" / "ios"
ANDROID = REPO / "client" / "packages" / "mobile" / "android"
WORK = REPO / "target" / "mobile-call"
PORTS = Ports(nats=14923, api=18910, voice=18911, api_metrics=19966, voice_metrics=19967, rtc_min=48300,
              rtc_max=48399, transfer=13981, relay_min=48400, relay_max=48499)
HELPER_PORT = 18919
# The TLS listeners in front of the API server, the voice server, and the helper for a phone on
# the network (`TlsFront`), each to its plain port on loopback.
TLS_PORTS = {18912: PORTS.api, 18913: PORTS.voice, 18914: HELPER_PORT}


def tls_port(upstream: int) -> int:
    """The TLS listener in front of the plain port `upstream`."""
    return next(listening for listening, plain in TLS_PORTS.items() if plain == upstream)
SIMULATOR = "iPhone 18 Pro"
BUNDLE = "com.aspen-chat.client"
PACKAGE = "com.aspen_chat.client"
MAC_PATH = "export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH"
# The keychain a Mac's builds over SSH sign with (the login keychain is closed to an SSH session),
# holding the team's Apple Development identity, and the file holding its password.
KEYCHAIN = "$HOME/Library/Keychains/aspen-build.keychain-db"
KEYCHAIN_PASSWORD_FILE = "$HOME/.aspen-build-keychain"
ANDROID_TESTS = ["com.aspen_chat.client.call.CallEngineTest", "com.aspen_chat.client.call.AspenCallPluginTest"]
# What only a phone can show, run on `--device` after the two above; it captures the incoming call
# screen as a call rings the locked phone, and as the call goes on there once answered.
ANDROID_DEVICE_TEST = "com.aspen_chat.client.call.CallDeviceTest"
# The line each Android test prints once it has done its work, by which a run is judged to have
# done it; a test that passed without one did nothing.
ANDROID_TEST_LINES = {
    ANDROID_TESTS[0]: "call-engine-test",
    ANDROID_TESTS[1]: "call-plugin-test",
    ANDROID_DEVICE_TEST: "call-device-test",
}
DEVICE_CAPTURES = ["/data/local/tmp/aspen-ring-dark.png", "/data/local/tmp/aspen-ring-lock-screen.png",
                   "/data/local/tmp/aspen-ring-answered-locked.png"]
IOS_TESTS = ["AppTests/CallEngineTests", "AppTests/AspenCallPluginTests", "AppTests/CallRingsTests"]
# On a phone, the engine's and the bridge's: the rings' tests report synthetic incoming calls to
# CallKit, which the simulator's ends quietly and a phone's rings aloud, and a real ring is the
# checklist's (`ios-app.md`, Rings).
IOS_DEVICE_TESTS = IOS_TESTS[:2]
# The line each iOS test prints once it has done its work, as `ANDROID_TEST_LINES`.
IOS_TEST_LINES = {
    IOS_TESTS[0]: "call-engine-test",
    IOS_TESTS[1]: "call-plugin-test",
    IOS_TESTS[2]: "call-rings-test",
}
ANDROID_TRAILING_TEST = "com.aspen_chat.client.call.AspenCallPluginTest#pictureFollowsAMovingHole"
IOS_TRAILING_TEST = "AppTests/AspenCallPluginTests/testPictureFollowsAMovingHole"


def say(message: str) -> None:
    print(f"mobile-call: {message}", flush=True)


def tone(path: Path) -> None:
    """Two seconds of a 440 Hz sine at 48 kHz, mono, 16-bit, for Chromium to loop as a microphone."""
    rate, seconds, frequency = 48000, 2, 440
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes(b"".join(
            struct.pack("<h", int(12000 * math.sin(2 * math.pi * frequency * n / rate)))
            for n in range(rate * seconds)
        ))


class Second:
    """The second participant: `calls/second.ts` in Chromium, driven over its standard input, in
    the call of `channel` unless `join` names another."""

    def __init__(self, base: str, channel: str, session: dict, tone_path: Path, trusted_keys: list[str] = ()):
        """`trusted_keys` are the public keys (`TlsFront.spki_sha256`) Chromium is to accept
        without a trusted authority: the voice server's, when it is registered at a TLS address
        for a phone's sake."""
        self.base = base
        self.channel = channel
        self.session = session
        self.tone_path = tone_path
        self.trusted_keys = list(trusted_keys)
        self.process: subprocess.Popen | None = None
        self.joined: str | None = None
        self.lock = threading.Lock()

    def join(self, channel: str | None = None) -> None:
        channel = channel or self.channel
        with self.lock:
            if self.process is not None:
                if self.joined == channel:
                    return
                raise Failed(f"the second participant is in the call of {self.joined}, not {channel}")
            say(f"the second participant joins the call of {channel} from Chromium")
            arguments = json.dumps({"base": self.base, "channel": channel, "session": self.session, "tone": str(self.tone_path),
                                    "trustedKeys": self.trusted_keys})
            self.process = subprocess.Popen(
                ["pnpm", "--silent", "calls:second"], cwd=PROTOCOL,
                env={**clean_env(), "ASPEN_CALL_SECOND": arguments},
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=sys.stderr, text=True, bufsize=1,
            )
            self.joined = channel
            line = self.process.stdout.readline().strip()
            if line != "connected":
                raise Failed(f"the second participant did not connect: {line!r}")
            say("the second participant is connected")

    def ask(self, command: str) -> str:
        with self.lock:
            if self.process is None or self.process.poll() is not None:
                raise Failed("the second participant is not in the call")
            self.process.stdin.write(f"{command}\n")
            self.process.stdin.flush()
            return self.process.stdout.readline().strip()

    def leave(self) -> None:
        with self.lock:
            if self.process is None:
                return
            if self.process.poll() is None:
                try:
                    self.process.stdin.write("leave\n")
                    self.process.stdin.flush()
                    self.process.wait(timeout=20)
                except (OSError, subprocess.TimeoutExpired):
                    self.process.kill()
            self.process = None
            self.joined = None


class Dm:
    """Bob's DM with the device's user, whose call rings the device: `ring` has Bob start it from
    Chromium and answers with the ring the deployment holds for the user, as the `call` pointer
    carries it, `standing` with that ring while it stands, and `end` has Bob leave, which ends the
    call and with it the ring."""

    def __init__(self, stack: Stack, second: Second, channel: str, user: dict, caller: dict, caller_name: str):
        self.stack = stack
        self.second = second
        self.channel = channel
        self.user = user
        self.caller = caller
        self.caller_name = caller_name

    def standing(self) -> dict | None:
        included = self.stack.api("GET", "/users/@me/dms?include=voice", token=self.user["sessionToken"]).get("included", {})
        for ring in included.get("voiceRings", []):
            if ring["channel"] == self.channel and ring["user"] == self.user["userId"]:
                return {"channel": ring["channel"], "session": ring["session"], "caller": ring["caller"],
                        "callerName": self.caller_name, "until": ring["until"]}
        return None

    def ring(self) -> dict:
        self.end()
        self.second.join(self.channel)
        ring: dict | None = None

        def rung() -> bool:
            nonlocal ring
            ring = self.standing()
            return ring is not None
        wait_for("the DM's call to ring the device's user", rung, 20)
        return ring

    def end(self) -> None:
        self.second.leave()
        wait_for("the DM's call to end", lambda: self.stack.api("GET", f"/channels/{self.channel}/voice", token=self.caller["sessionToken"]).get("session") is None, 30)


def helper(second: Second, bob: str, dm: Dm) -> ThreadingHTTPServer:
    """Answers the test on loopback: `POST /second-participant` brings Bob in and names him (into
    the call of the `channel` its JSON body names, else the voice channel's), `GET
    /second-participant/report` is his page's report, `GET /second-participant/loudness` the
    loudest sample his audio slots carried over a second (`{"peak": …}`),
    `POST /second-participant/show-video` has his page show others' video,
    `POST /second-participant/camera/start` and `/camera/stop` turn his fake camera on and off,
    `GET /second-participant/camera-sample?user=<id>` is a frame of that user's camera as his page
    plays it (`CameraSample`, or `null`), `GET /second-participant/screen-sample?user=<id>` the same
    of their shared screen, `POST /second-participant/offer`
    (`{"name", "size", "allowDirect"}`) has him offer a file and answers its id and SHA-256,
    `GET /second-participant/files` his page's files, `POST /second-participant/accept`
    (`{"offer", "mode"}`) has him accept one into memory,
    `GET /second-participant/received?offer=…&peer=…` the SHA-256 of what he received (`null`
    until it has all arrived), and `POST /second-participant/leave`; `POST /ring` has Bob call the
    device's user in their DM and answers the ring the deployment holds for them (`Dm.ring`),
    `GET /ring` that ring while it stands (`{"ring": null}` once it does not), and `POST /ring/end`
    has Bob leave the DM's call."""

    class Handler(BaseHTTPRequestHandler):
        def answer(self, status: int, body: str) -> None:
            data = body.encode()
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def body(self) -> dict:
            length = int(self.headers.get("content-length") or 0)
            return json.loads(self.rfile.read(length)) if length else {}

        def do_POST(self) -> None:  # noqa: N802 - the server's naming
            try:
                if self.path == "/second-participant":
                    second.join(self.body().get("channel"))
                    self.answer(200, json.dumps({"user": bob}))
                elif self.path == "/second-participant/leave":
                    second.leave()
                    self.answer(200, "{}")
                elif self.path == "/second-participant/show-video":
                    self.answer(200 if second.ask("show") == "ok" else 500, "{}")
                elif self.path == "/second-participant/camera/start":
                    self.answer(200 if second.ask("camera on") == "ok" else 500, "{}")
                elif self.path == "/second-participant/camera/stop":
                    self.answer(200 if second.ask("camera off") == "ok" else 500, "{}")
                elif self.path == "/second-participant/offer":
                    self.answer(200, second.ask("offer " + json.dumps(self.body())))
                elif self.path == "/second-participant/accept":
                    asked = self.body()
                    self.answer(200, second.ask(f"accept {asked['offer']} {asked['mode']}"))
                elif self.path == "/ring":
                    self.answer(200, json.dumps(dm.ring()))
                elif self.path == "/ring/end":
                    dm.end()
                    self.answer(200, "{}")
                else:
                    self.answer(404, "{}")
            except Failed as failure:
                self.answer(500, json.dumps({"error": str(failure)}))

        def do_GET(self) -> None:  # noqa: N802
            try:
                parsed = urlparse(self.path)
                query = parse_qs(parsed.query)
                if parsed.path == "/second-participant/report":
                    self.answer(200, second.ask("report"))
                elif parsed.path == "/second-participant/loudness":
                    self.answer(200, json.dumps({"peak": float(second.ask("loudness"))}))
                elif parsed.path == "/second-participant/camera-sample":
                    user = query.get("user", [""])[0]
                    self.answer(200, second.ask(f"sample {user}"))
                elif parsed.path == "/second-participant/screen-sample":
                    user = query.get("user", [""])[0]
                    self.answer(200, second.ask(f"screen {user}"))
                elif parsed.path == "/second-participant/files":
                    self.answer(200, second.ask("files"))
                elif parsed.path == "/second-participant/received":
                    offer, peer = query.get("offer", [""])[0], query.get("peer", [""])[0]
                    self.answer(200, second.ask(f"received {offer} {peer}"))
                elif parsed.path == "/ring":
                    self.answer(200, json.dumps({"ring": dm.standing()}))
                else:
                    self.answer(404, "{}")
            except Failed as failure:
                self.answer(500, json.dumps({"error": str(failure)}))

        def log_message(self, format: str, *args) -> None:
            say(f"helper: {format % args}")

    server = ThreadingHTTPServer(("127.0.0.1", HELPER_PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


# --- iOS ------------------------------------------------------------------------------------------


def ssh(mac: str, command: str, timeout: float = 600, log: Path | None = None) -> int:
    """Runs `command` on the Mac in a shell with the Homebrew and cargo paths, streaming its
    output here and into `log` when given; the exit code."""
    with subprocess.Popen(
        ["ssh", mac, f"{MAC_PATH}; {command}"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
    ) as process:
        with open(log, "w") if log else open(os.devnull, "w") as out:
            for line in process.stdout:
                out.write(line)
                sys.stdout.write(line)
        try:
            return process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            return -1


def ssh_output(mac: str, command: str) -> str:
    done = subprocess.run(["ssh", mac, f"{MAC_PATH}; {command}"], capture_output=True, text=True, timeout=120)
    return done.stdout.strip()


def tunnels(mac: str) -> subprocess.Popen:
    """Reverse tunnels from the Mac's loopback to this machine's deployment and helper."""
    forwards = []
    for port in (PORTS.api, PORTS.voice, HELPER_PORT):
        forwards += ["-R", f"{port}:127.0.0.1:{port}"]
    process = subprocess.Popen(["ssh", "-N", "-o", "ExitOnForwardFailure=yes", *forwards, mac])
    wait_for("the tunnels to the Mac",
             lambda: ssh_output(mac, f"curl -s -o /dev/null -w '%{{http_code}}' http://127.0.0.1:{PORTS.api}/api/v1/health") == "204",
             30)
    return process


def launched(derived: str, variables: dict[str, str]) -> str:
    """The command that runs the built tests on the booted simulator without xcodebuild: the app
    built for testing, which carries the test bundle in its `PlugIns`, is installed and launched
    with XCTest's bundle injection (`libXCTestBundleInject`), `variables` in its environment, and
    its console followed until the tests end."""
    app = f"{derived}/Build/Products/Debug-iphonesimulator/App.app"
    developer = "$(xcode-select -p)"
    platform = f"{developer}/Platforms/iPhoneSimulator.platform/Developer"
    exports = " ".join([
        f"SIMCTL_CHILD_XCTestBundlePath={app}/PlugIns/AppTests.xctest",
        f"SIMCTL_CHILD_DYLD_INSERT_LIBRARIES={platform}/usr/lib/libXCTestBundleInject.dylib",
        f"SIMCTL_CHILD_DYLD_FRAMEWORK_PATH={platform}/Library/Frameworks",
        f"SIMCTL_CHILD_DYLD_LIBRARY_PATH={platform}/usr/lib",
        *(f"SIMCTL_CHILD_{name}='{value}'" for name, value in variables.items()),
    ])
    return (f"xcrun simctl terminate booted {BUNDLE} 2>/dev/null; xcrun simctl install booted {app} && "
            f"xcrun simctl privacy booted grant microphone {BUNDLE} && "
            f"xcrun simctl privacy booted grant camera {BUNDLE} && "
            f"{exports} xcrun simctl launch --console-pty booted {BUNDLE} 2>&1 "
            "| grep -E --line-buffered 'Test Case|Test Suite|call-engine-test|call-routes-test|call-plugin-test|call-rings-test|error|failed|skipped'")


def address_toward(mac: str) -> str:
    """This machine's address on the network it reaches the Mac over (`ssh -G`'s `hostname`),
    which a phone paired with the Mac shares."""
    config = subprocess.run(["ssh", "-G", mac], capture_output=True, text=True, check=True).stdout
    host = next((line.split(None, 1)[1] for line in config.splitlines() if line.startswith("hostname ")), mac)
    with socket.create_connection((host, 22), timeout=10) as connection:
        return connection.getsockname()[0]


class Ios:
    """The tests on a Mac's simulator, or on an iPhone paired with the Mac, reached over SSH."""

    def __init__(self, args: argparse.Namespace):
        self.mac = args.mac
        self.worktree = args.worktree
        self.derived = args.derived
        self.runner = args.runner
        self.device: str | None = args.device
        self.keychain = args.keychain
        self.keychain_password_file = args.keychain_password_file
        self.project = f"{args.worktree}/client/packages/mobile/ios/App"
        if self.device:
            if args.runner != "xcodebuild":
                raise Failed("--device runs through xcodebuild")
            self.tests = IOS_DEVICE_TESTS
            self.address = args.address or address_toward(self.mac)
            self.sdk = "iphoneos"
            # The build names no device, so it never touches the phone; the test names it.
            self.build_destination = "generic/platform=iOS"
            self.test_destination = f"platform=iOS,id={self.device}"
        else:
            self.tests = IOS_TESTS
            self.address = None
            self.sdk = "iphonesimulator"
            self.build_destination = (f"platform=iOS Simulator,name={args.simulator}" if args.runner == "xcodebuild"
                                      else "generic/platform=iOS Simulator")
            self.test_destination = f"platform=iOS Simulator,name={args.simulator}"
        if ssh_output(self.mac, "pgrep -x xcodebuild || true"):
            raise Failed("an xcodebuild is running on the Mac; wait for it")

    def xcodebuild(self, destination: str) -> str:
        """The xcodebuild invocation for `destination`, to run from `self.project`; the
        environment the test reads is set on this command alone, so a `cd` cannot be part of it.
        A device build signs with the identity in the build keychain and the team's profiles
        (`-allowProvisioningUpdates`), which over SSH are those already on the Mac: Xcode's
        accounts are the login keychain's, so a capability or profile this tree adds is
        registered once in Xcode on the Mac, signed in, and the profiles then serve here."""
        command = (f"xcodebuild -project App.xcodeproj -scheme App -sdk {self.sdk} "
                   f"-destination '{destination}' -derivedDataPath {self.derived}")
        if self.device:
            command += f" -allowProvisioningUpdates OTHER_CODE_SIGN_FLAGS='--keychain {self.keychain}'"
        return command

    def unlocked_keychain(self) -> str:
        """The command that unlocks the build keychain for the signing that follows it."""
        return f"security unlock-keychain -p \"$(cat {self.keychain_password_file})\" {self.keychain}"

    def where(self) -> str:
        return f"the phone {self.device}" if self.device else "the simulator"

    def build(self) -> None:
        target = "devices" if self.device else "the simulator"
        say(f"building the engine for {target} and its bindings on the Mac")
        only = "--device-only" if self.device else "--simulator-only"
        if ssh(self.mac, f"cd {self.worktree} && python3 client/packages/mobile/ios/scripts/call_engine.py {only}",
               log=WORK / "engine.log") != 0:
            raise Failed("the engine did not build on the Mac")
        say(f"building the app and its tests for {'a device' if self.device else 'the simulator'}")
        before = f"{self.unlocked_keychain()} && " if self.device else ""
        if ssh(self.mac, f"cd {self.project} && {before}{self.xcodebuild(self.build_destination)} build-for-testing 2>&1 "
                         "| grep -E 'error|warning: ld|BUILD|Signing Identity|Provisioning Profile' ; exit ${pipestatus[1]}",
               log=WORK / "build.log") != 0:
            raise Failed(f"the app did not build; see {WORK / 'build.log'}")
        if self.device:
            signed = sorted({line.strip() for line in (WORK / "build.log").read_text().splitlines()
                             if line.strip().startswith(("Signing Identity:", "Provisioning Profile:"))})
            if not signed:
                raise Failed(f"the device build named no signing identity; see {WORK / 'build.log'}")
            say("signed with " + "; ".join(signed))

    def ready_phone(self) -> None:
        """Refuses a phone the run cannot use, naming what to change: one the Mac does not list
        as paired with its tunnel up, or without Developer Mode. Whether it is unlocked now
        `devicectl` cannot say (only that it has a passcode and was unlocked since boot), and
        `xcodebuild` refuses a locked phone, so that is said. The phone's address is not known
        here either: the tests' first request says whether it reaches this machine."""
        listed = json.loads(ssh_output(self.mac, "xcrun devicectl list devices --json-output - 2>/dev/null"))
        phone = next((each for each in listed["result"]["devices"]
                      if each.get("hardwareProperties", {}).get("udid") == self.device), None)
        if phone is None:
            udids = [each.get("hardwareProperties", {}).get("udid", "?") for each in listed["result"]["devices"]]
            raise Failed(f"the Mac lists no device {self.device} (xcrun devicectl list devices: {', '.join(udids) or 'none'})")
        connection = phone.get("connectionProperties", {})
        properties = phone.get("deviceProperties", {})
        name = f"{properties.get('name', '?')} ({phone.get('hardwareProperties', {}).get('marketingName', '?')}, iOS {properties.get('osVersionNumber', '?')})"
        if connection.get("pairingState") != "paired" or connection.get("tunnelState") != "connected":
            raise Failed(f"{name} is not reachable from the Mac (pairing {connection.get('pairingState')}, tunnel "
                         f"{connection.get('tunnelState')}): unlock it on the Mac's network, or plug it in, and run again")
        if properties.get("developerModeStatus") != "enabled":
            raise Failed(f"{name} has Developer Mode {properties.get('developerModeStatus')}: turn it on under Settings, "
                         "Privacy & Security, and run again")
        lock = json.loads(ssh_output(self.mac, f"xcrun devicectl device info lockState --device {self.device} --json-output - 2>/dev/null"))["result"]
        if not lock.get("unlockedSinceBoot", False):
            raise Failed(f"{name} has not been unlocked since it booted: unlock it and run again")
        say(f"{name} is paired over {connection.get('transportType')} with Developer Mode on"
            + (", has a passcode" if lock.get("passcodeRequired") else "")
            + "; keep it unlocked and awake through the run, since xcodebuild refuses a locked phone")

    def reachable_from_mac(self, api: str) -> None:
        """Checks the TLS front from the Mac, on the network the phone shares: the address is
        reachable from another machine there and the certificate is the authority's for it."""
        remote = "/tmp/aspen-dev-ca.pem"
        subprocess.run(["scp", "-q", str(dev_ca.CA), f"{self.mac}:{remote}"], check=True)
        answered = ssh_output(self.mac, f"curl -s --cacert {remote} -o /dev/null -w '%{{http_code}}' {api}/api/v1/health")
        if answered != "204":
            raise Failed(f"the Mac does not reach {api} with the development authority's certificate (curl answered "
                         f"{answered or 'nothing'}): is {self.address} this machine's address on the Mac's network? "
                         "give it with --address")
        say(f"the Mac reaches {api} over the network with the authority's certificate")

    def record_trailing(self, variables: dict[str, str]) -> Path:
        """Runs the trailing test alone, with the simulator's screen recorded around it, and brings
        the recording here."""
        if self.runner != "xcodebuild":
            raise Failed("--record-trailing runs through xcodebuild")
        ssh_output(self.mac, f"xcrun simctl privacy booted grant camera {BUNDLE}")
        remote = "/tmp/aspen-trailing-ios.mp4"
        # The recorder ends, and writes the file out, on an interrupt; it runs on the Mac's own
        # shell so the signal reaches it rather than the ssh session.
        ssh_output(self.mac, f"rm -f {remote}; nohup xcrun simctl io booted recordVideo --codec h264 --force {remote} "
                             f">/tmp/aspen-trailing-ios.log 2>&1 & echo $! > /tmp/aspen-trailing-ios.pid")
        time.sleep(2)
        say("recording the simulator while the hole moves")
        environment = " ".join(f"TEST_RUNNER_{name}='{value}'" for name, value in {**variables, "ASPEN_CALL_TRAILING": "1"}.items())
        code = ssh(self.mac,
                   f"cd {self.project} && {environment} {self.xcodebuild(self.test_destination)} test-without-building "
                   f"-only-testing:{IOS_TRAILING_TEST} 2>&1 | grep -E 'Test Case|call-plugin-test|error:|TEST' ; exit ${{pipestatus[1]}}",
                   timeout=600, log=WORK / "trailing-test-ios.log")
        ssh_output(self.mac, "kill -INT $(cat /tmp/aspen-trailing-ios.pid); sleep 3")
        if code != 0 or "recorded the picture" not in (WORK / "trailing-test-ios.log").read_text():
            raise Failed(f"the trailing test did not run (exit {code}); see {WORK / 'trailing-test-ios.log'}")
        recording = WORK / "trailing-ios.mp4"
        subprocess.run(["scp", "-q", f"{self.mac}:{remote}", str(recording)], check=True)
        return recording

    def run(self, variables: dict[str, str]) -> None:
        if self.device:
            self.ready_phone()
            self.reachable_from_mac(variables["ASPEN_CALL_API"])
            variables = {**variables, "ASPEN_CALL_MICROPHONE": "live", "ASPEN_CALL_SYSTEM_CALL": "stands"}
            say("xcodebuild now installs the app and its tests on the phone and runs them there; the phone may ask to "
                "allow Aspen on the local network and to use the microphone, which someone at it answers")
            tunnel = None
        else:
            variables = {**variables, "ASPEN_CALL_MICROPHONE": "tone", "ASPEN_CALL_SYSTEM_CALL": "ends"}
            tunnel = tunnels(self.mac)
        try:
            if not self.device:
                ssh_output(self.mac, f"xcrun simctl privacy booted grant microphone {BUNDLE}")
                ssh_output(self.mac, f"xcrun simctl privacy booted grant camera {BUNDLE}")
            say(f"running {', '.join(test.split('/')[1] for test in self.tests)} on {self.where()} through {self.runner}")
            log = WORK / "test.log"
            if self.runner == "xcodebuild":
                environment = " ".join(f"TEST_RUNNER_{name}='{value}'" for name, value in variables.items())
                only = " ".join(f"-only-testing:{test}" for test in self.tests)
                code = ssh(self.mac,
                           f"cd {self.project} && {environment} {self.xcodebuild(self.test_destination)} test-without-building {only} 2>&1 "
                           "| grep -E 'Test Case|Test Suite|call-engine-test|call-routes-test|call-plugin-test|call-rings-test|error:|failed|skipped|TEST' ; exit ${pipestatus[1]}",
                           timeout=1200, log=log)
                text = log.read_text()
                passed = code == 0 and "** TEST EXECUTE SUCCEEDED **" in text
            else:
                code = ssh(self.mac, launched(self.derived, variables), timeout=900, log=log)
                text = log.read_text()
                passed = (all(f"Test Suite '{test.split('/')[1]}' passed" in text for test in self.tests)
                          and "' failed" not in text and " error: " not in text)
            # Each test's own line says what it saw; a test that ran without one did not do its work.
            ran = all(f"{IOS_TEST_LINES[test]}:" in text for test in self.tests)
            # Only the trailing test skips itself here (outside `--record-trailing`).
            skipped = re.findall(r"Test Case '-\[AppTests\.\w+ (\w+)\]' skipped", text)
            if not passed or not ran or any(name != IOS_TRAILING_TEST.rsplit("/", 1)[1] for name in skipped):
                # The tests' first request names what the phone lacks (`CallCheck.reach`).
                if "trustFailure" in text:
                    say(f"the phone does not trust the development authority: write it as a profile with "
                        f"`scripts/dev_ca.py profile --serve {self.address}:8080`, open that address in Safari on the "
                        "phone, install the profile under Settings, General, VPN & Device Management, turn it on "
                        "under Settings, General, About, Certificate Trust Settings, and run again")
                elif "unreachable" in text:
                    say(f"the phone does not reach {self.address}: it must be on this machine's network (the Mac's), "
                        "and allow Aspen on the local network when iOS asks")
                raise Failed(f"the tests did not pass (exit {code}); see {log}")
        finally:
            if tunnel is not None:
                tunnel.terminate()


# --- Android ----------------------------------------------------------------------------------------


def sdk() -> Path:
    return Path(os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or Path.home() / "Android" / "Sdk")


def adb(*args: str, serial: str | None = None, timeout: float = 60) -> str:
    """`adb`'s output, pinned to the device `serial` when given, else to the one device attached."""
    pinned = ["-s", serial] if serial else []
    done = subprocess.run([str(sdk() / "platform-tools" / "adb"), *pinned, *args], capture_output=True, text=True, timeout=timeout)
    return done.stdout.strip()


def attached() -> dict[str, str]:
    """The devices `adb` lists, serial to state (`device`, `offline`, `unauthorized`)."""
    devices = {}
    for line in adb("devices").splitlines()[1:]:
        parts = line.split("\t")
        if len(parts) == 2:
            devices[parts[0]] = parts[1]
    return devices


def is_emulator(serial: str) -> bool:
    """Whether the attached device is an emulator: `adb` names one `emulator-<console port>`, and
    the emulated kernel sets `ro.kernel.qemu`, which no phone's does."""
    return serial.startswith("emulator-") or adb("shell", "getprop", "ro.kernel.qemu", serial=serial) == "1"


def emulators() -> list[str]:
    """The serials of the emulators `adb` sees, booted or not; a phone's serial is never one."""
    return [serial for serial, state in attached().items() if serial.startswith("emulator-") and state in ("device", "offline")]


class Emulator:
    """An emulator started here, headless, recording this machine's microphone; `serial` names it
    once it is up."""

    def __init__(self, avd: str):
        self.avd = avd
        self.process: subprocess.Popen | None = None
        self.serial: str | None = None

    def start(self) -> None:
        # Another emulator may be running (another check's, or one left up): the one started here
        # is told apart by its serial and is the only one driven.
        before = set(emulators())
        say(f"starting the emulator {self.avd}" + (f" beside {', '.join(sorted(before))}" if before else ""))
        log = open(WORK / "emulator.log", "w")
        self.process = subprocess.Popen(
            [str(sdk() / "emulator" / "emulator"), "-avd", self.avd, "-no-window", "-no-boot-anim", "-no-snapshot",
             "-gpu", "swiftshader_indirect", "-allow-host-audio"],
            stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        )
        wait_for("the emulator to appear", lambda: bool(set(emulators()) - before), 180)
        self.serial = sorted(set(emulators()) - before)[0]
        adb("wait-for-device", serial=self.serial, timeout=180)
        wait_for("the emulator to boot",
                 lambda: adb("shell", "getprop", "sys.boot_completed", serial=self.serial) == "1", 300)

    def stop(self) -> None:
        if self.process is not None:
            adb("emu", "kill", serial=self.serial)
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.process.kill()
            self.process = None


class Android:
    """The tests on the emulator, or on the phone `--device` names, through Gradle."""

    def __init__(self, args: argparse.Namespace):
        if args.avd and args.device:
            raise Failed("--avd starts an emulator and --device names a phone; give one")
        self.emulator = Emulator(args.avd) if args.avd else None
        self.device: str | None = args.device
        known = {name.rsplit(".", 1)[1]: name for name in (*ANDROID_TESTS, ANDROID_DEVICE_TEST)}
        self.only: list[str] | None = None
        if args.only:
            unknown = [name for name in args.only if name not in known]
            if unknown:
                raise Failed(f"--only names no test class: {', '.join(unknown)}; the classes are {', '.join(known)}")
            self.only = [known[name] for name in args.only]
        # What the tests expect the browser to hear of this side's microphone: a phone's hears
        # the room, an emulator's here carries silence.
        self.microphone = args.microphone or ("live" if args.device else "silent")
        self.env = {**os.environ, "ANDROID_HOME": str(sdk())}
        java_home = os.environ.get("JAVA_HOME")
        if java_home is None:
            jdks = sorted((Path.home() / ".local" / "share" / "jdks").glob("jdk-21*"))
            if jdks:
                self.env["JAVA_HOME"] = str(jdks[-1])
        if self.device:
            state = attached().get(self.device)
            if state != "device":
                raise Failed(f"no phone {self.device} is attached and ready (adb says {state or 'nothing'}); "
                             f"attached: {', '.join(attached()) or 'none'}")
            if is_emulator(self.device):
                raise Failed(f"{self.device} is an emulator; --device is for a phone (give --avd, or neither, for an emulator)")

    def gradle(self, *args: str, log: Path, timeout: float = 1800) -> int:
        with subprocess.Popen(["./gradlew", "--console=plain", *args], cwd=ANDROID, env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) as process:
            with open(log, "w") as out:
                for line in process.stdout:
                    out.write(line)
                    if "Test" in line or "error" in line.lower() or "BUILD" in line or "FAILED" in line:
                        sys.stdout.write(line)
            try:
                return process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                process.kill()
                return -1

    def build(self) -> None:
        if self.device:
            abi = adb("shell", "getprop", "ro.product.cpu.abi", serial=self.device)
            say(f"building the engine for the phone's {abi} and its bindings")
            subprocess.run([sys.executable, str(ANDROID / "scripts" / "call_engine.py"), "--abi", abi], cwd=REPO, check=True)
        else:
            say("building the engine for the emulator and its bindings")
            subprocess.run([sys.executable, str(ANDROID / "scripts" / "call_engine.py"), "--emulator-only"], cwd=REPO, check=True)

    def serial(self) -> str:
        """The device to drive: the emulator started here with `--avd`, the phone `--device` names,
        or the one emulator running. Everything after is pinned to it (`ANDROID_SERIAL`, which
        Gradle's `connected*` tasks and every `adb` under them take the device from), so whatever
        else is attached is never touched."""
        if self.emulator is not None:
            self.emulator.start()
            serial = self.emulator.serial
        elif self.device:
            serial = self.device
        else:
            running = emulators()
            if len(running) != 1:
                phones = [serial for serial, state in attached().items() if state == "device" and serial not in running]
                hint = f"; {', '.join(phones)} is a phone: give it as --device" if phones else ""
                raise Failed(f"one running emulator is needed, found {running or 'none'}{hint}; start one, or give --avd")
            serial = running[0]
        self.env["ANDROID_SERIAL"] = serial
        say(f"driving {self.where(serial)}")
        return serial

    def where(self, serial: str) -> str:
        return f"the phone {serial}" if self.device else f"the emulator {serial}"

    def ready_phone(self, serial: str) -> None:
        """Refuses a phone whose state would fail a test from inside rather than let it: the
        bridge's test launches `MainActivity` and enters the picture in picture, which needs the
        activity resumed, so the screen must be on with no lock screen over it, and must stay so
        through the engine's test before it, so Stay awake (Developer options) must be on, since
        the phone is on USB. A screen that is off, or a swipe lock screen (`CredentialType: NONE`
        in `dumpsys lock_settings` with the lock screen not disabled), is woken and dismissed
        here and said so; a PIN, pattern, or password lock screen that is showing is the owner's
        to unlock, or to set to Swipe, and is refused by name. The lock-screen ring tests sleep
        the screen themselves and work over either kind."""
        stay_on = adb("shell", "settings", "get", "global", "stay_on_while_plugged_in", serial=serial)
        usb = 2  # BatteryManager.BATTERY_PLUGGED_USB
        if not stay_on.isdigit() or int(stay_on) & usb == 0:
            raise Failed(f"{self.where(serial)} does not stay awake on USB (stay_on_while_plugged_in is {stay_on or 'unset'}): "
                         "turn Stay awake on in Developer options, so the screen is still on when the bridge's test needs "
                         "the activity resumed, and run again")
        credential = "unknown"
        for line in adb("shell", "dumpsys", "lock_settings", serial=serial).splitlines():
            if line.strip().startswith("CredentialType:"):
                credential = line.split(":", 1)[1].strip()
                break
        lock_disabled = adb("shell", "locksettings", "get-disabled", serial=serial).strip() == "true"
        secure = credential not in ("NONE", "unknown")

        def keyguard_showing() -> bool:
            policy = adb("shell", "dumpsys", "window", "policy", serial=serial)
            delegate = policy.find("KeyguardServiceDelegate")
            lines = policy[delegate:].splitlines()[1:12] if delegate >= 0 else []
            return any(line.strip() == "showing=true" for line in lines)

        def awake() -> bool:
            return any(line.strip() == "mWakefulness=Awake" for line in adb("shell", "dumpsys", "power", serial=serial).splitlines())

        if secure and (keyguard_showing() or not awake()):
            raise Failed(f"{self.where(serial)} is behind its {credential} lock screen (dumpsys lock_settings): "
                         "unlock it and leave the screen on, or set the lock to Swipe in Settings, Security, and run again")
        done = []
        if not awake():
            adb("shell", "input", "keyevent", "KEYCODE_WAKEUP", serial=serial)
            wait_for("the phone's screen to wake", awake, 10)
            done.append("woke the screen")
        if keyguard_showing():
            adb("shell", "wm", "dismiss-keyguard", serial=serial)
            wait_for("the swipe lock screen to be dismissed", lambda: not keyguard_showing(), 10)
            done.append("dismissed the swipe lock screen")
        lock = "no lock screen" if lock_disabled else f"a {'swipe' if not secure else credential} lock screen"
        say(f"{self.where(serial)} is awake with {lock}, not showing, and stays awake on USB"
            + (f" ({', '.join(done)})" if done else ""))

    def record_trailing(self, variables: dict[str, str]) -> Path:
        """Runs the trailing test alone, with the device's screen recorded around it, and pulls the
        recording here."""
        serial = self.serial()
        remote = "/sdcard/aspen-trailing.mp4"
        recorder = None
        try:
            adb("shell", "rm", "-f", remote, serial=serial)
            recorder = subprocess.Popen([str(sdk() / "platform-tools" / "adb"), "-s", serial, "shell", "screenrecord",
                                         "--time-limit", "120", remote], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(2)
            say(f"recording {self.where(serial)} while the hole moves")
            arguments = [f"-Pandroid.testInstrumentationRunnerArguments.{name}={value}"
                         for name, value in {**variables, "ASPEN_CALL_TRAILING": "1"}.items()]
            code = self.gradle(":app:connectedDebugAndroidTest",
                               f"-Pandroid.testInstrumentationRunnerArguments.class={ANDROID_TRAILING_TEST}",
                               *arguments, log=WORK / "trailing-test-android.log")
            adb("shell", "pkill", "-INT", "screenrecord", serial=serial)
            recorder.wait(timeout=30)
            recorder = None
            logcat = adb("logcat", "-d", serial=serial, timeout=120)
            if code != 0 or "recorded the picture" not in logcat:
                raise Failed(f"the trailing test did not run (exit {code}); see {WORK / 'trailing-test-android.log'}")
            recording = WORK / "trailing-android.mp4"
            adb("pull", remote, str(recording), serial=serial, timeout=120)
            return recording
        finally:
            if recorder is not None:
                recorder.kill()
            if self.emulator is not None:
                self.emulator.stop()

    def run(self, variables: dict[str, str]) -> None:
        serial = self.serial()
        if self.device:
            self.ready_phone(serial)
        classes = [*ANDROID_TESTS, ANDROID_DEVICE_TEST] if self.device else ANDROID_TESTS
        if self.only is not None:
            if not self.device and ANDROID_DEVICE_TEST in self.only:
                raise Failed(f"{ANDROID_DEVICE_TEST.rsplit('.', 1)[1]} runs on a phone alone; give --device")
            classes = [name for name in classes if name in self.only]
        # The device's log is followed for the run's length into `logcat.txt`, the crash buffer
        # included, so a crash's tombstone is there to symbolize (`ndk-stack`): a phone's log is
        # busy enough that a dump at the end would have lost the tests' first lines.
        adb("logcat", "-c", "-b", "all", serial=serial)
        logcat_file = open(WORK / "logcat.txt", "w")
        logcat = subprocess.Popen([str(sdk() / "platform-tools" / "adb"), "-s", serial, "logcat", "-b", "main", "-b", "system", "-b", "crash"],
                                  stdout=logcat_file, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        try:
            for port in (PORTS.api, PORTS.voice, HELPER_PORT):
                adb("reverse", f"tcp:{port}", f"tcp:{port}", serial=serial)
            say(f"running {', '.join(name.rsplit('.', 1)[1] for name in classes)} on {self.where(serial)} through Gradle, "
                f"expecting its microphone {self.microphone}")
            log = WORK / "test.log"
            variables = {**variables, "ASPEN_CALL_MICROPHONE": self.microphone}
            arguments = [f"-Pandroid.testInstrumentationRunnerArguments.{name}={value}" for name, value in variables.items()]
            code = self.gradle(":app:connectedDebugAndroidTest", f"-Pandroid.testInstrumentationRunnerArguments.class={','.join(classes)}",
                               *arguments, log=log)
            if code != 0:
                # A crashed process's tombstone reaches the log a moment after Gradle gives up on it.
                time.sleep(5)
            logcat.terminate()
            logcat.wait(timeout=30)
            logcat_file.close()
            lines = [ANDROID_TEST_LINES[name] for name in classes]
            printed = [line for line in (WORK / "logcat.txt").read_text(errors="replace").splitlines()
                       if any(f"{prefix}:" in line for prefix in lines)]
            for line in printed:
                print(line, flush=True)
            for capture in DEVICE_CAPTURES if self.device else []:
                adb("pull", capture, str(WORK / Path(capture).name), serial=serial)
                adb("shell", "rm", "-f", capture, serial=serial)
            results = ANDROID / "app" / "build" / "outputs" / "androidTest-results" / "connected"
            # Each test's own line says what it saw; a test that passed without one did not do its work.
            told = all(any(f"{prefix}:" in line for line in printed) for prefix in lines)
            if code != 0 or not passed(results, classes) or not told:
                raise Failed(f"the tests did not pass (exit {code}); see {log} and {results}")
        finally:
            if logcat.poll() is None:
                logcat.kill()
            logcat_file.close()
            for port in (PORTS.api, PORTS.voice, HELPER_PORT):
                adb("reverse", "--remove", f"tcp:{port}", serial=serial)
            if self.emulator is not None:
                self.emulator.stop()


def passed(results: Path, classes: list[str]) -> bool:
    """Whether Gradle's reports under `results` say every class of `classes` ran at least one test
    and none failed, erred, or was skipped, but for the trailing test, which skips itself outside
    `--record-trailing`. Gradle writes one suite per device, named for the first class, so each
    test is judged by its own `classname`."""
    trailing_class, trailing_test = ANDROID_TRAILING_TEST.split("#")
    outcomes: dict[str, list[tuple[str, str]]] = {}
    for path in results.rglob("*.xml") if results.exists() else []:
        for case in ElementTree.parse(path).iter("testcase"):
            if case.find("failure") is not None or case.find("error") is not None:
                outcome = "failed"
            elif case.find("skipped") is not None:
                outcome = "skipped"
            else:
                outcome = "passed"
            outcomes.setdefault(case.get("classname", ""), []).append((case.get("name", ""), outcome))
    ok = True
    for name in classes:
        cases = outcomes.get(name, [])
        if not cases:
            say(f"{name} did not run")
            ok = False
        for test, outcome in cases:
            if outcome == "skipped" and name == trailing_class and test == trailing_test:
                continue
            if outcome != "passed":
                say(f"{name}.{test} {outcome}")
                ok = False
    return ok


# --- The check --------------------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(description="Check the call engine on a phone's simulator or emulator against built servers.")
    parser.add_argument("--platform", choices=["ios", "android"], required=True)
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--build", action="store_true", help="build the engine, its bindings, and (on iOS) the app and tests first")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    parser.add_argument("--record-trailing", action="store_true",
                        help="record the screen while a hole moves instead of running the checks (see the module's doc)")
    parser.add_argument("--device", metavar="SERIAL|UDID",
                        help="run on a phone: the serial `adb devices` lists (Android), or the UDID "
                             "`xcrun devicectl list devices` lists on the Mac (iOS)")
    ios = parser.add_argument_group("ios")
    ios.add_argument("--mac", default="aspen-mac", help="the Mac, as ssh names it")
    ios.add_argument("--worktree", default="~/aspen-native-calls", help="this repository's worktree on the Mac")
    ios.add_argument("--derived", default="~/aspen-derived-native-calls", help="Xcode's derived data on the Mac")
    ios.add_argument("--simulator", default=SIMULATOR)
    ios.add_argument("--runner", choices=["xcodebuild", "simctl"], default="xcodebuild",
                     help="how the test is run on the simulator (see the module's doc)")
    ios.add_argument("--address", help="with --device, this machine's address on the network the phone and the Mac "
                                       "share (the address this machine reaches the Mac from, when not given)")
    ios.add_argument("--keychain", default=KEYCHAIN,
                     help="with --device, the Mac's keychain holding the signing identity (see the module's doc)")
    ios.add_argument("--keychain-password-file", default=KEYCHAIN_PASSWORD_FILE,
                     help="with --device, the file on the Mac holding that keychain's password")
    android = parser.add_argument_group("android")
    android.add_argument("--avd", help="start this emulator here, headless")
    android.add_argument("--microphone", choices=["silent", "live"],
                         help="what the browser is to hear of the device's microphone: silence, or the room, "
                              "whatever it sounds like (live with --device, silent otherwise)")
    android.add_argument("--only", metavar="CLASS", action="append",
                         help="run only this test class (CallEngineTest, AspenCallPluginTest, or on a phone CallDeviceTest), "
                              "repeatable, against the same deployment; for one test's failure without the others' minutes")
    args = parser.parse_args()
    if shutil.which("docker") is None:
        sys.exit("mobile-call: docker is required")
    WORK.mkdir(parents=True, exist_ok=True)
    try:
        platform = Ios(args) if args.platform == "ios" else Android(args)
        if args.build:
            platform.build()
        if args.record_trailing:
            recording = platform.record_trailing({})
            say(f"recorded {recording}; measure it with scripts/measure_video_trailing.py")
            return
        if args.start_services:
            start_services()
        build_page()
        wait_for_services()
        tone_path = WORK / "tone.wav"
        tone(tone_path)
        # A phone on the network reaches the deployment and the helper over TLS at this machine's
        # address there; a simulator or emulator at its own loopback.
        front = TlsFront(platform.address, TLS_PORTS) if isinstance(platform, Ios) and platform.device else None
        voice_url = front.url(tls_port(PORTS.voice)) if front else None
        with front or contextlib.nullcontext():
            say("starting a database and a NATS of its own, the chat server, and the voice server"
                + (f", served to the phone over TLS at {platform.address}" if front else ""))
            with Stack(args.bin.resolve(), "mobilecall", PORTS, web_client=PAGE, voice_url=voice_url) as stack:
                main_in_stack(args, platform, stack, tone_path, front)
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


def main_in_stack(args: argparse.Namespace, platform: "Ios | Android", stack: Stack, tone_path: Path,
                  front: TlsFront | None) -> None:
    """The check against `stack`, its people, channel, helper, and the platform's tests."""
    run = os.getpid()
    ann = person(stack, f"ann{run}")
    bob = person(stack, f"bob{run}")
    made = stack.api("POST", "/communities", {"name": "Calls"}, ann["sessionToken"])
    community = made.get("id") or made["data"]["id"]
    invite = stack.api("POST", f"/communities/{community}/invites", {}, ann["sessionToken"])
    stack.api("PUT", f"/communities/{community}/members/@me",
              {"inviteCode": invite.get("code") or invite.get("id")}, bob["sessionToken"])
    channel = stack.api("POST", "/channels",
                        {"community": community, "name": "Lounge", "sortIndex": 0, "ty": "voice"},
                        ann["sessionToken"])["id"]
    wait_for("a voice server to be offered",
             lambda: stack.status("POST", f"/channels/{channel}/voice/join", {}, ann["sessionToken"]) == 200, 90)
    # Bob's DM with Ann, whose call rings the device as a DM's call rings a phone.
    made = stack.api("POST", "/users/@me/dms", {"recipients": [ann["userId"]]}, bob["sessionToken"])
    dm_channel = made.get("id") or made["data"]["id"]
    # The browser here reaches the voice server at the address clients are told, the TLS one for a
    # phone's sake, whose key it pins.
    second = Second(stack.base, channel, bob, tone_path, trusted_keys=[front.spki_sha256] if front else [])
    server = helper(second, bob["userId"], Dm(stack, second, dm_channel, ann, bob, f"bob{run}"))
    variables = {
        "ASPEN_CALL_API": front.url(tls_port(PORTS.api)) if front else stack.base,
        "ASPEN_CALL_CHANNEL": channel,
        "ASPEN_CALL_USER_ID": ann["userId"],
        "ASPEN_CALL_REFRESH_TOKEN": ann["refreshToken"],
        "ASPEN_CALL_HELPER": front.url(tls_port(HELPER_PORT)) if front else f"http://127.0.0.1:{HELPER_PORT}",
    }
    try:
        platform.run(variables)
    finally:
        second.leave()
        server.shutdown()
        # Both servers' whole logs, for a failure the stack's own excerpt is too short for.
        for log in stack.work.glob("*.log"):
            shutil.copy(log, WORK / log.name)


if __name__ == "__main__":
    main()
