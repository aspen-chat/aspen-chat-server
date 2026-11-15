#!/usr/bin/env python3
"""Check the mobile call engine with its iOS platform under it, on the iOS simulator of a Mac,
against built servers here.

Runs a deployment of its own (`stack.py`), whose web client is the call page of the client's own
call code (`client/packages/protocol/calls/`), reaches it from the Mac's loopback through SSH
reverse tunnels (so the simulator, which shares the Mac's network, finds the API server and the
voice server at `http://127.0.0.1`, which the engine takes in plain text as a development build
does; media goes straight to the voice server's announced address over the LAN), and runs the
app's `CallEngineTests` there (`client/packages/mobile/ios/App/AppTests/`): the engine joins the
call, a second participant is brought in from Chromium here with a tone as its microphone, each
side's statistics show the other's sound, the socket is cut and resumed, and the call is left.

    scripts/check_ios_call.py --bin target/debug [--build] [--mac aspen-mac]
        [--worktree '~/aspen-native-calls'] [--derived '~/aspen-derived-native-calls']

`--build` first builds the engine for the simulator and its bindings on the Mac
(`client/packages/mobile/ios/scripts/call_engine.py`) and the app and tests for the simulator
(`build-for-testing`); the Mac's worktree must hold this commit and the WebRTC framework. It
needs the client's dependencies installed here and Playwright's Chromium, and refuses to run
while another `xcodebuild` is on the Mac.

`--runner simctl` runs the built tests without `xcodebuild test`: the app built for testing is
installed on the booted simulator and launched with XCTest's bundle injection, for a Mac whose
`xcodebuild` cannot see simulators (an Xcode whose CoreSimulator was never installed, which takes an
administrator); its `--build` builds for the generic simulator destination for the same reason.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import threading
import time
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from check_calls import build_page, PAGE, PROTOCOL, person
from stack import Failed, Ports, Stack, clean_env, start_services, wait_for, wait_for_services

REPO = Path(__file__).resolve().parent.parent
IOS = REPO / "client" / "packages" / "mobile" / "ios"
WORK = REPO / "target" / "ios-call"
PORTS = Ports(nats=14923, api=18910, voice=18911, api_metrics=19966, voice_metrics=19967, rtc_min=48300,
              rtc_max=48399, transfer=13981, relay_min=48400, relay_max=48499)
HELPER_PORT = 18919
SIMULATOR = "iPhone 18 Pro"
BUNDLE = "com.aspen-chat.client"
MAC_PATH = "export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH"


def say(message: str) -> None:
    print(f"ios-call: {message}", flush=True)


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
    """The second participant: `calls/second.ts` in Chromium, driven over its standard input."""

    def __init__(self, base: str, channel: str, session: dict, tone_path: Path):
        self.arguments = json.dumps({"base": base, "channel": channel, "session": session, "tone": str(tone_path)})
        self.process: subprocess.Popen | None = None
        self.lock = threading.Lock()

    def join(self) -> None:
        with self.lock:
            if self.process is not None:
                return
            say("the second participant joins from Chromium")
            self.process = subprocess.Popen(
                ["pnpm", "--silent", "calls:second"], cwd=PROTOCOL,
                env={**clean_env(), "ASPEN_CALL_SECOND": self.arguments},
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=sys.stderr, text=True, bufsize=1,
            )
            line = self.process.stdout.readline().strip()
            if line != "connected":
                raise Failed(f"the second participant did not connect: {line!r}")
            say("the second participant is connected")

    def report(self) -> str:
        with self.lock:
            if self.process is None or self.process.poll() is not None:
                raise Failed("the second participant is not in the call")
            self.process.stdin.write("report\n")
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


def helper(second: Second, bob: str) -> ThreadingHTTPServer:
    """Answers the test on loopback: `POST /second-participant` brings Bob in and names him,
    `GET /second-participant/report` is his page's report, `POST /second-participant/leave`."""

    class Handler(BaseHTTPRequestHandler):
        def answer(self, status: int, body: str) -> None:
            data = body.encode()
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self) -> None:  # noqa: N802 - the server's naming
            try:
                if self.path == "/second-participant":
                    second.join()
                    self.answer(200, json.dumps({"user": bob}))
                elif self.path == "/second-participant/leave":
                    second.leave()
                    self.answer(200, "{}")
                else:
                    self.answer(404, "{}")
            except Failed as failure:
                self.answer(500, json.dumps({"error": str(failure)}))

        def do_GET(self) -> None:  # noqa: N802
            try:
                if self.path == "/second-participant/report":
                    self.answer(200, second.report())
                else:
                    self.answer(404, "{}")
            except Failed as failure:
                self.answer(500, json.dumps({"error": str(failure)}))

        def log_message(self, format: str, *args) -> None:
            say(f"helper: {format % args}")

    server = ThreadingHTTPServer(("127.0.0.1", HELPER_PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


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
            f"{exports} xcrun simctl launch --console-pty booted {BUNDLE} 2>&1 "
            "| grep -E --line-buffered 'Test Case|Test Suite|call-engine-test|error|failed|skipped'")


def main() -> None:
    parser = argparse.ArgumentParser(description="Check the call engine on the iOS simulator against built servers.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--mac", default="aspen-mac", help="the Mac, as ssh names it")
    parser.add_argument("--worktree", default="~/aspen-native-calls", help="this repository's worktree on the Mac")
    parser.add_argument("--derived", default="~/aspen-derived-native-calls", help="Xcode's derived data on the Mac")
    parser.add_argument("--simulator", default=SIMULATOR)
    parser.add_argument("--build", action="store_true", help="build the engine, the app, and the tests on the Mac first")
    parser.add_argument("--runner", choices=["xcodebuild", "simctl"], default="xcodebuild",
                        help="how the test is run on the simulator (see the module's doc)")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    args = parser.parse_args()
    if shutil.which("docker") is None:
        sys.exit("ios-call: docker is required")
    WORK.mkdir(parents=True, exist_ok=True)
    project = f"{args.worktree}/client/packages/mobile/ios/App"
    destination = f"platform=iOS Simulator,name={args.simulator}" if args.runner == "xcodebuild" else "generic/platform=iOS Simulator"
    xcodebuild = (f"cd {project} && xcodebuild -project App.xcodeproj -scheme App -sdk iphonesimulator "
                  f"-destination '{destination}' -derivedDataPath {args.derived}")
    try:
        if ssh_output(args.mac, "pgrep -x xcodebuild || true"):
            raise Failed("an xcodebuild is running on the Mac; wait for it")
        if args.build:
            say("building the engine for the simulator and its bindings on the Mac")
            if ssh(args.mac, f"cd {args.worktree} && python3 client/packages/mobile/ios/scripts/call_engine.py --simulator-only",
                   log=WORK / "engine.log") != 0:
                raise Failed("the engine did not build on the Mac")
            say("building the app and its tests for the simulator")
            if ssh(args.mac, f"{xcodebuild} build-for-testing 2>&1 | grep -E 'error|warning: ld|BUILD' ; exit ${{pipestatus[1]}}",
                   log=WORK / "build.log") != 0:
                raise Failed(f"the app did not build; see {WORK / 'build.log'}")
        if args.start_services:
            start_services()
        build_page()
        wait_for_services()
        tone_path = WORK / "tone.wav"
        tone(tone_path)
        say("starting a database and a NATS of its own, the chat server, and the voice server")
        with Stack(args.bin.resolve(), "ioscall", PORTS, web_client=PAGE) as stack:
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
            second = Second(stack.base, channel, bob, tone_path)
            server = helper(second, bob["userId"])
            tunnel = tunnels(args.mac)
            try:
                variables = {
                    "ASPEN_CALL_API": stack.base,
                    "ASPEN_CALL_CHANNEL": channel,
                    "ASPEN_CALL_USER_ID": ann["userId"],
                    "ASPEN_CALL_REFRESH_TOKEN": ann["refreshToken"],
                    "ASPEN_CALL_HELPER": f"http://127.0.0.1:{HELPER_PORT}",
                }
                ssh_output(args.mac, f"xcrun simctl privacy booted grant microphone {BUNDLE}")
                say(f"running CallEngineTests on the simulator through {args.runner}")
                log = WORK / "test.log"
                if args.runner == "xcodebuild":
                    environment = " ".join(f"TEST_RUNNER_{name}='{value}'" for name, value in variables.items())
                    code = ssh(args.mac,
                               f"{environment} {xcodebuild} test-without-building -only-testing:AppTests/CallEngineTests 2>&1 "
                               "| grep -E 'Test Case|Test Suite|call-engine-test|error:|failed|skipped|TEST' ; exit ${pipestatus[1]}",
                               timeout=900, log=log)
                    passed = code == 0 and "** TEST EXECUTE SUCCEEDED **" in log.read_text()
                else:
                    code = ssh(args.mac, launched(args.derived, variables), timeout=900, log=log)
                    text = log.read_text()
                    passed = ("Test Suite 'CallEngineTests' passed" in text and "Test Suite 'CallEngineTests' failed" not in text
                              and " error: " not in text)
                if not passed or "skipped" in log.read_text():
                    raise Failed(f"the test did not pass (exit {code}); see {log}")
            finally:
                second.leave()
                server.shutdown()
                tunnel.terminate()
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


if __name__ == "__main__":
    main()
