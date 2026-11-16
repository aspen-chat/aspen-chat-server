#!/usr/bin/env python3
"""Check the mobile call engine with a phone's platform under it, on the iOS simulator of a Mac or
the Android emulator here, against built servers here.

Runs a deployment of its own (`stack.py`), whose web client is the call page of the client's own
call code (`client/packages/protocol/calls/`), and runs the platform's tests of the engine against
it: the engine's own (`CallEngineTests` in the iOS app's `AppTests`, `CallEngineTest` in the Android
app's `androidTest`), in which the engine joins the call, a second participant is brought in from
Chromium here with a tone as its microphone, each side's statistics show the other's sound, the
socket is cut and resumed, and the call is left; and the web view's bridge to it
(`AspenCallPluginTests`, `AspenCallPluginTest`), in which the app's own page drives the `AspenCall`
plugin through the Capacitor bridge, joins the same channel, and reads the engine's snapshots and
routes as they reach it. Each platform reaches the API server and the voice server at its own
loopback, `http://127.0.0.1`, which the engine takes in plain text as a debuggable build does, and
media goes straight to the voice server's announced address over the network.

    scripts/check_mobile_call.py --platform ios --bin target/debug [--build] [--mac aspen-mac]
        [--worktree '~/aspen-native-calls'] [--derived '~/aspen-derived-native-calls']
    scripts/check_mobile_call.py --platform android --bin target/debug [--build] [--avd aspen-test]

iOS: the simulator shares the Mac's network, so the Mac's loopback is tunnelled back here over SSH.
`--build` first builds the engine for the simulator and its bindings on the Mac
(`client/packages/mobile/ios/scripts/call_engine.py`) and the app and tests for the simulator
(`build-for-testing`); the Mac's worktree must hold this commit and the WebRTC framework. It refuses
to run while another `xcodebuild` is on the Mac. `--runner simctl` runs the built tests without
`xcodebuild test`: the app built for testing is installed on the booted simulator and launched with
XCTest's bundle injection, for a Mac whose `xcodebuild` cannot see simulators (an Xcode whose
CoreSimulator was never installed, which takes an administrator); its `--build` builds for the
generic simulator destination for the same reason.

Android: the emulator reaches this machine's loopback through `adb reverse`. With `--avd` the
emulator is started here, headless, recording this machine's microphone (`-allow-host-audio`,
without which it records silence); without it, the one emulator `adb` sees is used. Only an
emulator is ever driven: `adb` and Gradle are given its serial (`ANDROID_SERIAL`), so a phone
plugged in beside it is left alone, and the check refuses to run with no emulator or several. What the
emulator's microphone carries decides what the test expects the browser to hear: its sound, or its
silence, which is what an emulator whose host audio never reaches it carries, as here, where the
emulator's PulseAudio backend fails to start against PipeWire's server ("Could not init `pa'
audio driver" in `target/mobile-call/emulator.log`). `--build` first builds the engine for the
emulator and its bindings (`client/packages/mobile/android/scripts/call_engine.py`); the Gradle
build then runs the test (`connectedDebugAndroidTest`), which needs the web shell copied in
(`npx cap sync android` in `client/packages/mobile`) and `target/webrtc/libwebrtc.aar`.

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
import json
import math
import os
import re
import shutil
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

from check_calls import build_page, PAGE, PROTOCOL, person
from stack import Failed, Ports, Stack, clean_env, start_services, wait_for, wait_for_services

REPO = Path(__file__).resolve().parent.parent
IOS = REPO / "client" / "packages" / "mobile" / "ios"
ANDROID = REPO / "client" / "packages" / "mobile" / "android"
WORK = REPO / "target" / "mobile-call"
PORTS = Ports(nats=14923, api=18910, voice=18911, api_metrics=19966, voice_metrics=19967, rtc_min=48300,
              rtc_max=48399, transfer=13981, relay_min=48400, relay_max=48499)
HELPER_PORT = 18919
SIMULATOR = "iPhone 18 Pro"
BUNDLE = "com.aspen-chat.client"
PACKAGE = "com.aspen_chat.client"
MAC_PATH = "export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH"
ANDROID_TESTS = ["com.aspen_chat.client.call.CallEngineTest", "com.aspen_chat.client.call.AspenCallPluginTest"]
IOS_TESTS = ["AppTests/CallEngineTests", "AppTests/AspenCallPluginTests"]
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


def helper(second: Second, bob: str) -> ThreadingHTTPServer:
    """Answers the test on loopback: `POST /second-participant` brings Bob in and names him,
    `GET /second-participant/report` is his page's report, `GET /second-participant/loudness` the
    loudest sample his audio slots carried over a second (`{"peak": …}`),
    `POST /second-participant/show-video` has his page show others' video,
    `POST /second-participant/camera/start` and `/camera/stop` turn his fake camera on and off,
    `GET /second-participant/camera-sample?user=<id>` is a frame of that user's camera as his page
    plays it (`CameraSample`, or `null`), and `POST /second-participant/leave`."""

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
                elif self.path == "/second-participant/show-video":
                    self.answer(200 if second.ask("show") == "ok" else 500, "{}")
                elif self.path == "/second-participant/camera/start":
                    self.answer(200 if second.ask("camera on") == "ok" else 500, "{}")
                elif self.path == "/second-participant/camera/stop":
                    self.answer(200 if second.ask("camera off") == "ok" else 500, "{}")
                else:
                    self.answer(404, "{}")
            except Failed as failure:
                self.answer(500, json.dumps({"error": str(failure)}))

        def do_GET(self) -> None:  # noqa: N802
            try:
                parsed = urlparse(self.path)
                if parsed.path == "/second-participant/report":
                    self.answer(200, second.ask("report"))
                elif parsed.path == "/second-participant/loudness":
                    self.answer(200, json.dumps({"peak": float(second.ask("loudness"))}))
                elif parsed.path == "/second-participant/camera-sample":
                    user = parse_qs(parsed.query).get("user", [""])[0]
                    self.answer(200, second.ask(f"sample {user}"))
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
            "| grep -E --line-buffered 'Test Case|Test Suite|call-engine-test|call-routes-test|call-plugin-test|error|failed|skipped'")


class Ios:
    """The tests on a Mac's simulator, reached over SSH."""

    def __init__(self, args: argparse.Namespace):
        self.mac = args.mac
        self.worktree = args.worktree
        self.derived = args.derived
        self.runner = args.runner
        self.project = f"{args.worktree}/client/packages/mobile/ios/App"
        destination = (f"platform=iOS Simulator,name={args.simulator}" if args.runner == "xcodebuild"
                       else "generic/platform=iOS Simulator")
        # Run from `self.project`; the environment the test reads is set on this command alone,
        # so a `cd` cannot be part of it.
        self.xcodebuild = (f"xcodebuild -project App.xcodeproj -scheme App -sdk iphonesimulator "
                           f"-destination '{destination}' -derivedDataPath {args.derived}")
        if ssh_output(self.mac, "pgrep -x xcodebuild || true"):
            raise Failed("an xcodebuild is running on the Mac; wait for it")

    def build(self) -> None:
        say("building the engine for the simulator and its bindings on the Mac")
        if ssh(self.mac, f"cd {self.worktree} && python3 client/packages/mobile/ios/scripts/call_engine.py --simulator-only",
               log=WORK / "engine.log") != 0:
            raise Failed("the engine did not build on the Mac")
        say("building the app and its tests for the simulator")
        if ssh(self.mac, f"cd {self.project} && {self.xcodebuild} build-for-testing 2>&1 | grep -E 'error|warning: ld|BUILD' ; exit ${{pipestatus[1]}}",
               log=WORK / "build.log") != 0:
            raise Failed(f"the app did not build; see {WORK / 'build.log'}")

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
                   f"cd {self.project} && {environment} {self.xcodebuild} test-without-building -only-testing:{IOS_TRAILING_TEST} 2>&1 "
                   "| grep -E 'Test Case|call-plugin-test|error:|TEST' ; exit ${pipestatus[1]}",
                   timeout=600, log=WORK / "trailing-test-ios.log")
        ssh_output(self.mac, "kill -INT $(cat /tmp/aspen-trailing-ios.pid); sleep 3")
        if code != 0 or "recorded the picture" not in (WORK / "trailing-test-ios.log").read_text():
            raise Failed(f"the trailing test did not run (exit {code}); see {WORK / 'trailing-test-ios.log'}")
        recording = WORK / "trailing-ios.mp4"
        subprocess.run(["scp", "-q", f"{self.mac}:{remote}", str(recording)], check=True)
        return recording

    def run(self, variables: dict[str, str]) -> None:
        tunnel = tunnels(self.mac)
        try:
            ssh_output(self.mac, f"xcrun simctl privacy booted grant microphone {BUNDLE}")
            ssh_output(self.mac, f"xcrun simctl privacy booted grant camera {BUNDLE}")
            say(f"running {', '.join(test.split('/')[1] for test in IOS_TESTS)} on the simulator through {self.runner}")
            log = WORK / "test.log"
            if self.runner == "xcodebuild":
                environment = " ".join(f"TEST_RUNNER_{name}='{value}'" for name, value in variables.items())
                only = " ".join(f"-only-testing:{test}" for test in IOS_TESTS)
                code = ssh(self.mac,
                           f"cd {self.project} && {environment} {self.xcodebuild} test-without-building {only} 2>&1 "
                           "| grep -E 'Test Case|Test Suite|call-engine-test|call-routes-test|call-plugin-test|error:|failed|skipped|TEST' ; exit ${pipestatus[1]}",
                           timeout=900, log=log)
                text = log.read_text()
                passed = code == 0 and "** TEST EXECUTE SUCCEEDED **" in text
            else:
                code = ssh(self.mac, launched(self.derived, variables), timeout=900, log=log)
                text = log.read_text()
                passed = (all(f"Test Suite '{test.split('/')[1]}' passed" in text for test in IOS_TESTS)
                          and "' failed" not in text and " error: " not in text)
            # Each test's own line says what it saw; a test that ran without one did not do its work.
            ran = all(f"{line}:" in text for line in ("call-engine-test", "call-plugin-test"))
            # Only the trailing test skips itself here (outside `--record-trailing`).
            skipped = re.findall(r"Test Case '-\[AppTests\.\w+ (\w+)\]' skipped", text)
            if not passed or not ran or any(name != IOS_TRAILING_TEST.rsplit("/", 1)[1] for name in skipped):
                raise Failed(f"the tests did not pass (exit {code}); see {log}")
        finally:
            tunnel.terminate()


# --- Android ----------------------------------------------------------------------------------------


def sdk() -> Path:
    return Path(os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or Path.home() / "Android" / "Sdk")


def adb(*args: str, serial: str | None = None, timeout: float = 60) -> str:
    """Runs `adb`, against the device `serial` names when given."""
    target = [] if serial is None else ["-s", serial]
    done = subprocess.run([str(sdk() / "platform-tools" / "adb"), *target, *args], capture_output=True, text=True,
                          timeout=timeout)
    return done.stdout.strip()


def emulators() -> list[str]:
    """The serials of the emulators `adb` sees, booted or not; a phone's serial is never one."""
    serials = []
    for line in adb("devices").splitlines()[1:]:
        parts = line.split("\t")
        if len(parts) == 2 and parts[0].startswith("emulator-") and parts[1] in ("device", "offline"):
            serials.append(parts[0])
    return serials


class Emulator:
    """An emulator started here, headless, recording this machine's microphone; `serial` names it
    once it is up."""

    def __init__(self, avd: str):
        self.avd = avd
        self.process: subprocess.Popen | None = None
        self.serial: str | None = None

    def start(self) -> None:
        before = set(emulators())
        if before:
            raise Failed(f"an emulator is running already ({', '.join(sorted(before))}); use it (no --avd) or stop it")
        say(f"starting the emulator {self.avd}")
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


def tests_passed(results: Path, expected: int) -> bool:
    """Whether the JUnit reports under `results` say exactly `expected` tests ran, none failing
    or erroring, and none skipped but the trailing test, which skips itself outside
    `--record-trailing`."""
    counts = {"tests": 0, "failures": 0, "errors": 0, "skipped": 0}
    skipped: list[str] = []
    for path in results.rglob("*.xml"):
        suite = ElementTree.parse(path).getroot()
        for name in counts:
            counts[name] += int(suite.get(name, "0"))
        skipped.extend(case.get("name", "") for case in suite.iter("testcase") if case.find("skipped") is not None)
    ran = counts["tests"] - counts["skipped"]
    return (ran == expected and counts["failures"] == 0 and counts["errors"] == 0
            and all(name == ANDROID_TRAILING_TEST.split("#")[1] for name in skipped))


class Android:
    """The tests on the emulator, through Gradle."""

    def __init__(self, args: argparse.Namespace):
        self.emulator = Emulator(args.avd) if args.avd else None
        self.env = {**os.environ, "ANDROID_HOME": str(sdk())}
        java_home = os.environ.get("JAVA_HOME")
        if java_home is None:
            jdks = sorted((Path.home() / ".local" / "share" / "jdks").glob("jdk-21*"))
            if jdks:
                self.env["JAVA_HOME"] = str(jdks[-1])

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
        say("building the engine for the emulator and its bindings")
        subprocess.run([sys.executable, str(ANDROID / "scripts" / "call_engine.py"), "--emulator-only"], cwd=REPO, check=True)

    def serial(self) -> str:
        """The emulator to drive, started here with `--avd`; a phone plugged in is never touched."""
        if self.emulator is not None:
            self.emulator.start()
            return self.emulator.serial
        running = emulators()
        if len(running) != 1:
            raise Failed(f"one running emulator is needed, found {running or 'none'}; start one, or give --avd")
        return running[0]

    def record_trailing(self, variables: dict[str, str]) -> Path:
        """Runs the trailing test alone, with the emulator's screen recorded around it, and pulls
        the recording here."""
        serial = self.serial()
        self.env["ANDROID_SERIAL"] = serial
        say(f"driving the emulator {serial}")
        remote = "/sdcard/aspen-trailing.mp4"
        recorder = None
        try:
            adb("shell", "rm", "-f", remote, serial=serial)
            recorder = subprocess.Popen([str(sdk() / "platform-tools" / "adb"), "-s", serial, "shell", "screenrecord",
                                         "--time-limit", "120", remote], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(2)
            say("recording the emulator while the hole moves")
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
        # Everything from here drives that emulator alone: a phone plugged in is never touched.
        self.env["ANDROID_SERIAL"] = serial
        say(f"driving the emulator {serial}")
        try:
            for port in (PORTS.api, PORTS.voice, HELPER_PORT):
                adb("reverse", f"tcp:{port}", f"tcp:{port}", serial=serial)
            adb("logcat", "-c", serial=serial)
            say(f"running {', '.join(test.rsplit('.', 1)[1] for test in ANDROID_TESTS)} on the emulator through Gradle")
            log = WORK / "test.log"
            arguments = [f"-Pandroid.testInstrumentationRunnerArguments.{name}={value}" for name, value in variables.items()]
            code = self.gradle(":app:connectedDebugAndroidTest",
                               f"-Pandroid.testInstrumentationRunnerArguments.class={','.join(ANDROID_TESTS)}",
                               *arguments, log=log)
            logcat = adb("logcat", "-d", serial=serial, timeout=120)
            (WORK / "logcat.txt").write_text(logcat)
            printed = [line for line in logcat.splitlines() if "call-engine-test" in line or "call-plugin-test" in line]
            for line in printed:
                print(line, flush=True)
            results = ANDROID / "app" / "build" / "outputs" / "androidTest-results" / "connected"
            ran = results.exists() and tests_passed(results, len(ANDROID_TESTS))
            # Each test's own line says what it saw; a test that ran without one did not do its work.
            told = all(any(f"{line}:" in printed_line for printed_line in printed) for line in ("call-engine-test", "call-plugin-test"))
            if code != 0 or not ran or not told:
                raise Failed(f"the tests did not pass (exit {code}); see {log} and {results}")
        finally:
            for port in (PORTS.api, PORTS.voice, HELPER_PORT):
                adb("reverse", "--remove", f"tcp:{port}", serial=serial)
            if self.emulator is not None:
                self.emulator.stop()


# --- The check --------------------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(description="Check the call engine on a phone's simulator or emulator against built servers.")
    parser.add_argument("--platform", choices=["ios", "android"], required=True)
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--build", action="store_true", help="build the engine, its bindings, and (on iOS) the app and tests first")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    parser.add_argument("--record-trailing", action="store_true",
                        help="record the screen while a hole moves instead of running the checks (see the module's doc)")
    ios = parser.add_argument_group("ios")
    ios.add_argument("--mac", default="aspen-mac", help="the Mac, as ssh names it")
    ios.add_argument("--worktree", default="~/aspen-native-calls", help="this repository's worktree on the Mac")
    ios.add_argument("--derived", default="~/aspen-derived-native-calls", help="Xcode's derived data on the Mac")
    ios.add_argument("--simulator", default=SIMULATOR)
    ios.add_argument("--runner", choices=["xcodebuild", "simctl"], default="xcodebuild",
                     help="how the test is run on the simulator (see the module's doc)")
    android = parser.add_argument_group("android")
    android.add_argument("--avd", help="start this emulator here, headless")
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
        say("starting a database and a NATS of its own, the chat server, and the voice server")
        with Stack(args.bin.resolve(), "mobilecall", PORTS, web_client=PAGE) as stack:
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
            variables = {
                "ASPEN_CALL_API": stack.base,
                "ASPEN_CALL_CHANNEL": channel,
                "ASPEN_CALL_USER_ID": ann["userId"],
                "ASPEN_CALL_REFRESH_TOKEN": ann["refreshToken"],
                "ASPEN_CALL_HELPER": f"http://127.0.0.1:{HELPER_PORT}",
            }
            try:
                platform.run(variables)
            finally:
                second.leave()
                server.shutdown()
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


if __name__ == "__main__":
    main()
