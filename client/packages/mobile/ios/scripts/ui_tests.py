#!/usr/bin/env python3
"""Runs the iOS app's UI tests (`App/AppUITests`) on a simulator against a deployment, with
what they need, and fails unless every test ran and passed: one skipped fails the run too.

    client/packages/mobile/ios/scripts/ui_tests.py --server https://192.168.2.220:8443 \\
        --relay https://192.168.2.220:7788 [--api URL] [--build] [--away-after 15] \\
        [--trust ca.pem] [testName ...]

`--server` is the deployment as the simulator reaches it, over HTTPS, as the app reaches
everything (its storage and voice servers too), and `--api` as this Mac reaches it, for the
wrapper's own requests and for the extension's code run here, which trusts the system's
authorities alone; `--server` when not given. `--relay` is the app side of
`scripts/push_relay_stand_in.py`, served over HTTPS (`--app-host`) beside the deployment, which
trusts the development authority for the stand-in's push side and lets it be reached on
loopback (`[federation.development] extra_root_certificates` and `allow_private_addresses`),
and wakes a user `--away-after` seconds after they last used it (`[presence]
away_after_seconds`, which must match). `--trust` adds a certificate authority to the
simulator's trusted roots and to the wrapper's own, for a deployment, storage, voice server, or
relay with a certificate of its own. `--build` builds the web app (`VITE_SCROLL_DEBUG=1`, which
the read-back test needs) and the tests for the simulator, the app naming the relay;
dependencies are installed first (`pnpm install` in `client/`).

Everything it keeps is under `target/ios-ui-tests/`: the fixture it seeds on first use and
reuses while the deployment still has it (two users, the reader's community with history to
read back through, a voice channel and a scratch channel, and the sender's community the reader
is invited to), the build, and each run's log. Each run installs the app afresh, so no session
from an earlier run is kept.

While the tests run it answers them on loopback (`ASPEN_TEST_HELPER`): `POST /say` has the
sender write in the scratch channel; `POST /away` does that with the app frozen as iOS suspends
it (`SIGSTOP`, the app's process and its web view's), thawing it once the deployment has
dropped its connection; and `POST /notify` has the sender message the reader, waits
for the push to the app's own subscription at the stand-in, and shows the reader's notification
as the service extension would: the simulator never runs the extension, so a second subscription
of the reader's, kept here, is pushed to as well, and the extension's own code
(`check_notification.sh`) turns that push into the notification `xcrun simctl push` delivers.
Each answers with the text written.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import secrets
import signal
import ssl
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

IOS = Path(__file__).resolve().parent.parent
REPO = IOS.parents[3]
CLIENT = REPO / "client"
WORK = REPO / "target" / "ios-ui-tests"
BUNDLE = "com.aspen-chat.client"
PASSWORD = "hunter22!!"
HISTORY = 260
"""Messages in the read-back channel: pages enough to read back through for minutes."""
AWAY_SECONDS = 95
"""How long `/away` keeps the app frozen: the event stream pings every 30 seconds and closes a
connection at the third ping it has sent unanswered (`event_stream.rs`), so the deployment has
dropped the app's by then, as it drops a phone's that iOS has suspended."""
WORDS = ["aspen", "birch", "cedar", "dogwood", "elm", "fir", "ginkgo", "hazel", "ironwood",
         "juniper", "larch", "maple"]


TLS = ssl.create_default_context()
"""What the wrapper's own requests trust: the system's authorities, and `--trust`'s."""


class Failed(Exception):
    pass


def say(text: str) -> None:
    print(f"ui-tests: {text}", flush=True)


def run(*args: str, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(list(args), capture_output=True, text=True, **kwargs)


class Api:
    """The deployment's REST API, as one user, waiting out rate limits."""

    def __init__(self, server: str, token: str | None = None) -> None:
        self.base = f"{server}/api/v1"
        self.token = token

    def __call__(self, method: str, path: str, body: dict | None = None, expect=(200, 201, 204)):
        while True:
            request = urllib.request.Request(
                f"{self.base}{path}", method=method,
                data=None if body is None else json.dumps(body).encode())
            request.add_header("content-type", "application/json")
            if self.token is not None:
                request.add_header("authorization", f"Bearer {self.token}")
            try:
                with urllib.request.urlopen(request, timeout=30, context=TLS) as response:
                    status, raw = response.status, response.read()
            except urllib.error.HTTPError as error:
                status, raw = error.code, error.read()
                if status == 429:
                    time.sleep(float(error.headers.get("Retry-After") or 1))
                    continue
            if status not in expect:
                raise Failed(f"{method} {path}: {status} {raw[:300].decode(errors='replace')}")
            return json.loads(raw) if raw else None

    def sign_in(self, name: str) -> dict:
        session = self("POST", "/auth/login", {"username": name, "password": PASSWORD})
        if session.get("status") != "signedIn":
            raise Failed(f"{name} did not sign in: {session}")
        self.token = session["sessionToken"]
        return session


def records(answer) -> list:
    return answer["data"] if isinstance(answer, dict) and "data" in answer else answer


def fixture(server: str) -> dict:
    """The fixture kept from an earlier run while the deployment still has it, else a new one."""
    path = WORK / "fixture.json"
    if path.exists():
        kept = json.loads(path.read_text())
        try:
            if kept["server"] == server:
                reader = Api(server)
                reader.sign_in(kept["reader"]["name"])
                reader("GET", f"/communities/{kept['community']}")
                Api(server).sign_in(kept["sender"]["name"])
                reader("GET", f"/invites/{kept['invite']['code']}")
                return kept
        except Failed as error:
            say(f"the kept fixture is gone ({error}); seeding another")
    made = seed(server)
    WORK.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(made, indent=2))
    return made


def seed(server: str) -> dict:
    suffix = secrets.token_hex(3)
    users = {}
    for role in ("reader", "sender"):
        name = f"ios{role}{suffix}"
        Api(server)("POST", "/users", {"name": name, "password": PASSWORD})
        api = Api(server)
        api.sign_in(name)
        users[role] = {"name": name, "id": api("GET", "/users/@me")["id"], "api": api}
    reader, sender = users["reader"]["api"], users["sender"]["api"]
    say(f"seeding: {users['reader']['name']} and {users['sender']['name']}")
    community = records(reader("POST", "/communities", {"name": "iOS tests"}))["id"]
    channels = records(reader("GET", f"/communities/{community}/channels"))
    general = next(c["id"] for c in channels if c["name"] == "general")
    made = {}
    for index, (name, kind) in enumerate([("probe-voice", "voice"), ("scratch", "text")], start=1):
        made[name] = records(reader("POST", "/channels", {
            "community": community, "name": name, "ty": kind, "sortIndex": index}))["id"]
    code = records(reader("POST", f"/communities/{community}/invites", {}))["code"]
    sender("PUT", f"/communities/{community}/members/@me", {"inviteCode": code})
    elsewhere = f"Invited place {suffix}"
    other = records(sender("POST", "/communities", {"name": elsewhere}))["id"]
    invite = records(sender("POST", f"/communities/{other}/invites", {}))["code"]
    say(f"seeding: {HISTORY} messages in general, paced by the deployment's rate limits")
    for n in range(1, HISTORY + 1):
        words = " ".join(WORDS[(n * 7 + i) % len(WORDS)] for i in range(3 + n % 9))
        author = reader if n % 3 else sender
        author("POST", f"/channels/{general}/messages", {"content": f"{n}: {words}", "attachments": []})
    return {
        "server": server,
        "reader": {k: v for k, v in users["reader"].items() if k != "api"},
        "sender": {k: v for k, v in users["sender"].items() if k != "api"},
        "community": community,
        "general": general,
        "voice": made["probe-voice"],
        "scratch": made["scratch"],
        "invite": {"code": invite, "community": elsewhere},
    }


def b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def p256_key() -> tuple[bytes, bytes]:
    """A new P-256 key pair from the system's `openssl`: the private scalar and the public point."""
    pem = run("openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", check=True).stdout
    text = run("openssl", "ec", "-text", "-noout", input=pem, check=True).stdout
    blocks = re.findall(r"(priv|pub):\s*\n((?:\s+[0-9a-f:]+\n)+)", text)
    values = {name: bytes.fromhex(re.sub(r"[\s:]", "", hexes)) for name, hexes in blocks}
    # The scalar is printed without its leading zero bytes, or with one more before a high bit.
    return values["priv"][-32:].rjust(32, b"\0"), values["pub"]


class HostPhone:
    """A subscription of the reader's kept on this Mac, as the app keeps one, for the
    extension's code to read a push with: the simulator's own keys are in its keychain."""

    def __init__(self, server: str, relay: str, reader: dict) -> None:
        self.api = Api(server)
        session = self.api.sign_in(reader["name"])
        self.refresh_token = session["refreshToken"]
        server_key = Api(server)("GET", "/auth/methods")["push"]["applicationServerKey"]
        private, public = p256_key()
        auth = os.urandom(16)
        device = relay_call(relay, "POST", "/v1/devices", {
            "platform": "apns", "app": BUNDLE, "environment": "sandbox", "token": "host"})
        made = relay_call(relay, "POST", f"/v1/devices/{device['device']}/subscriptions",
                          {"applicationServerKey": server_key}, device["secret"])
        self.subscription = made["subscription"]
        self.api("POST", "/users/@me/push-subscriptions",
                 {"endpoint": made["endpoint"], "p256dh": b64(public), "auth": b64(auth)})
        self.state = WORK / "host-state.json"
        self.state.write_text(json.dumps({"version": 1, "accounts": [{
            "subscription": self.subscription,
            "origin": server,
            "userId": reader["id"],
            "refreshToken": session["refreshToken"],
            "sessionToken": session["sessionToken"],
            "keys": {"privateKey": {"d": b64(private)}, "publicKey": b64(public), "auth": b64(auth)},
        }]}))

    def close(self) -> None:
        """Signs its sign-in out, which ends its subscription."""
        self.api("POST", "/auth/logout", {"refreshToken": self.refresh_token}, expect=(200, 204, 401))


def relay_call(relay: str, method: str, path: str, body: dict | None = None, secret: str | None = None):
    request = urllib.request.Request(f"{relay}{path}", method=method,
                                     data=None if body is None else json.dumps(body).encode())
    request.add_header("content-type", "application/json")
    if secret is not None:
        request.add_header("authorization", f"Bearer {secret}")
    with urllib.request.urlopen(request, timeout=10, context=TLS) as response:
        raw = response.read()
        return json.loads(raw) if raw else None


class Helper:
    """What the tests ask for while they run (`ASPEN_TEST_HELPER`)."""

    def __init__(self, api: str, origin: str, relay: str, kept: dict, host: HostPhone, udid: str,
                 away_after: int) -> None:
        self.origin = origin
        self.relay = relay
        self.kept = kept
        self.host = host
        self.udid = udid
        self.away_after = away_after
        self.sender = Api(api)
        self.sender.sign_in(kept["sender"]["name"])
        self.dm: str | None = None
        helper = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args) -> None:
                pass

            def do_POST(self) -> None:
                actions = {"/say": helper.say, "/away": helper.away, "/notify": helper.notify}
                if self.path not in actions:
                    self.answer(404, {"error": f"no {self.path}"})
                    return
                try:
                    self.answer(200, {"text": actions[self.path]()})
                except Exception as error:  # noqa: BLE001 - the test reads why
                    say(f"{self.path} failed: {error}")
                    self.answer(500, {"error": str(error)})

            def answer(self, status: int, payload: dict) -> None:
                data = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def say(self) -> str:
        text = f"Said while you were away {secrets.token_hex(3)}"
        self.sender("POST", f"/channels/{self.kept['scratch']}/messages", {"content": text, "attachments": []})
        return text

    def away(self) -> str:
        """Freezes the app as iOS suspends it, has the sender write meanwhile, and thaws it once
        the deployment has dropped its connection, which the app finds out only on waking."""
        frozen = app_processes(self.udid)
        if not frozen:
            raise Failed("the app is not running")
        for pid in frozen:
            os.kill(pid, signal.SIGSTOP)
        try:
            text = self.say()
            time.sleep(AWAY_SECONDS)
        finally:
            for pid in frozen:
                os.kill(pid, signal.SIGCONT)
        return text

    def notify(self) -> str:
        """Messages the reader until a push wakes them, which waits out the activity that keeps a
        user who has just used Aspen from being woken, then shows the notification."""
        if self.dm is None:
            self.dm = records(self.sender("POST", "/users/@me/dms", {"recipients": [self.kept["reader"]["id"]]}))["id"]
        seen = len(relay_call(self.relay, "GET", "/stand-in/pushes")["pushes"])
        deadline = time.time() + self.away_after + 60
        while time.time() < deadline:
            text = f"Are you coming tonight {secrets.token_hex(3)}"
            self.sender("POST", f"/channels/{self.dm}/messages", {"content": text, "attachments": []})
            host_push, app_woken = None, False
            until = time.time() + 8
            while time.time() < until and not (host_push and app_woken):
                time.sleep(0.5)
                pushes = relay_call(self.relay, "GET", f"/stand-in/pushes?after={seen}")["pushes"]
                for push in pushes:
                    if push["subscription"] == self.host.subscription:
                        host_push = push["payload"]
                    elif push["device"]["token"] != "host":
                        app_woken = True
            seen += len(relay_call(self.relay, "GET", f"/stand-in/pushes?after={seen}")["pushes"])
            if host_push is None:
                continue
            if not app_woken:
                raise Failed("the reader was woken, but not on the app's own subscription: "
                             "did the app register with the relay and subscribe?")
            return self.show(host_push, text)
        raise Failed(f"no push came in {self.away_after + 60} seconds: is the deployment "
                     f"waking users {self.away_after} seconds after they last used it?")

    def show(self, push: dict, text: str) -> str:
        payload, shown = WORK / "push.json", WORK / "shown.json"
        payload.write_text(json.dumps(push))
        checked = run(str(IOS / "scripts" / "check_notification.sh"), str(self.host.state), str(payload), str(shown))
        if checked.returncode != 0:
            raise Failed(f"the extension's code did not read the push: {checked.stdout}{checked.stderr}")
        notification = json.loads(shown.read_text())
        if text not in notification["aps"]["alert"]["body"]:
            raise Failed(f"the notification does not say what was written: {shown.read_text()}")
        # The host phone's account names the deployment as this Mac reaches it (`--api`); the
        # app's own names it as the app does, which a tap is matched against.
        notification["origin"] = self.origin
        shown.write_text(json.dumps(notification))
        run("xcrun", "simctl", "push", self.udid, BUNDLE, str(shown), check=True)
        return text

    def close(self) -> None:
        self.server.shutdown()


def app_processes(udid: str) -> list[int]:
    """The app's process on the simulator and its web view's (WebKit's content, networking, and
    GPU processes): those the simulator's `launchd` started beside it, on a simulator where no
    other app uses WebKit."""
    listed = run("xcrun", "simctl", "spawn", udid, "launchctl", "list", check=True).stdout
    app = next((int(line.split()[0]) for line in listed.splitlines()
                if f"UIKitApplication:{BUNDLE}[" in line and line.split()[0].isdigit()), None)
    if app is None:
        return []
    table = [line.split(None, 2) for line in run("ps", "-axo", "pid=,ppid=,command=", check=True).stdout.splitlines()]
    parent = next(int(ppid) for pid, ppid, _ in table if int(pid) == app)
    return [app, *(int(pid) for pid, ppid, command in table
                   if int(ppid) == parent and "/com.apple.WebKit." in command)]


def picture() -> Path:
    """A picture for the photo library: a gradient, so the picker shows something."""
    path = WORK / "picture.png"
    width, height = 640, 480
    rows = b"".join(
        b"\x00" + bytes(v for x in range(width) for v in (x * 255 // width, y * 255 // height, 160))
        for y in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
    return path


def build(relay: str, derived: Path, device: str) -> None:
    mobile = CLIENT / "packages" / "mobile"
    log = WORK / "build.log"
    with log.open("w") as out:
        for args, cwd, env in [
            (["pnpm", "codegen"], CLIENT, {}),
            (["pnpm", "build"], mobile, {"VITE_SCROLL_DEBUG": "1"}),
            (["npx", "cap", "sync", "ios"], mobile, {}),
            (["xcodebuild", "-project", "App.xcodeproj", "-scheme", "App", "-sdk", "iphonesimulator",
              "-destination", f"platform=iOS Simulator,name={device}", "-derivedDataPath", str(derived),
              f"ASPEN_PUSH_RELAY={relay}", "build-for-testing"], IOS / "App", {}),
        ]:
            say(f"building: {' '.join(args[:3])}")
            if subprocess.run(args, cwd=cwd, env={**os.environ, **env}, stdout=out, stderr=subprocess.STDOUT).returncode:
                raise Failed(f"{' '.join(args)} failed; see {log}")


def simulator(device: str, authorities: list[str]) -> str:
    """The named simulator, booted, with the software keyboard (WebKit's fields take typing from
    the keyboard on screen) and `authorities` among its trusted roots. Roots added to a running
    simulator keep a test runner from starting until it boots again, so it is restarted then."""
    listed = json.loads(run("xcrun", "simctl", "list", "devices", "available", "-j", check=True).stdout)
    udid = next((d["udid"] for ds in listed["devices"].values() for d in ds if d["name"] == device), None)
    if udid is None:
        raise Failed(f"no simulator named {device}")
    run("defaults", "write", "com.apple.iphonesimulator", "ConnectHardwareKeyboard", "-bool", "false")
    run("xcrun", "simctl", "boot", udid)
    run("xcrun", "simctl", "bootstatus", udid, "-b", check=True)
    if authorities:
        for authority in authorities:
            run("xcrun", "simctl", "keychain", udid, "add-root-cert", authority, check=True)
        run("xcrun", "simctl", "shutdown", udid, check=True)
        run("xcrun", "simctl", "boot", udid, check=True)
        run("xcrun", "simctl", "bootstatus", udid, "-b", check=True)
    return udid


def install(udid: str, derived: Path) -> None:
    app = derived / "Build" / "Products" / "Debug-iphonesimulator" / "App.app"
    if not app.exists():
        raise Failed(f"no build at {app}; run with --build")
    run("xcrun", "simctl", "terminate", udid, BUNDLE)
    run("xcrun", "simctl", "uninstall", udid, BUNDLE)
    run("xcrun", "simctl", "install", udid, str(app), check=True)
    run("xcrun", "simctl", "addmedia", udid, str(picture()), check=True)


def test(udid: str, derived: Path, env: dict[str, str], names: list[str]) -> int:
    log = WORK / "test.log"
    only = [f"-only-testing:AppUITests/AppUITests/{name}" for name in names]
    say(f"testing; the log is {log}")
    with log.open("w") as out:
        process = subprocess.Popen(
            ["xcodebuild", "-project", "App.xcodeproj", "-scheme", "App", "-sdk", "iphonesimulator",
             "-destination", f"id={udid}", "-derivedDataPath", str(derived), "test-without-building", *only],
            cwd=IOS / "App", env={**os.environ, **{f"TEST_RUNNER_{k}": v for k, v in env.items()}},
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        assert process.stdout is not None
        outcomes: dict[str, str] = {}
        for line in process.stdout:
            out.write(line)
            found = re.search(r"Test Case '-\[AppUITests\.AppUITests (\w+)\]' (passed|failed|skipped)", line)
            if found:
                outcomes[found[1]] = found[2]
                say(f"{found[1]} {found[2]}")
            elif ": error: " in line or "Test skipped" in line:
                print(line.rstrip(), flush=True)
        code = process.wait()
    not_passed = {name: outcome for name, outcome in outcomes.items() if outcome != "passed"}
    if not outcomes:
        say(f"no test ran (xcodebuild exited {code}); see {log}")
        return 1
    if not_passed or code != 0:
        say(f"{len(outcomes) - len(not_passed)} of {len(outcomes)} passed; not passed: {not_passed or 'none'}"
            f" (xcodebuild exited {code})")
        return 1
    say(f"all {len(outcomes)} passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--server", required=True, help="the deployment, as the simulator reaches it")
    parser.add_argument("--api", help="the deployment, as this Mac reaches it; --server when not given")
    parser.add_argument("--relay", required=True, help="the stand-in relay's app side")
    parser.add_argument("--away-after", type=int, default=15,
                        help="the deployment's [presence] away_after_seconds")
    parser.add_argument("--device", default="iPhone 18 Pro")
    parser.add_argument("--build", action="store_true", help="build the app and its tests first")
    parser.add_argument("--trust", action="append", default=[], metavar="CA_PEM",
                        help="a certificate authority for the simulator to trust; repeatable")
    parser.add_argument("tests", nargs="*", help="test names; every test when none")
    arguments = parser.parse_args()
    api = arguments.api or arguments.server
    for authority in arguments.trust:
        TLS.load_verify_locations(authority)
    WORK.mkdir(parents=True, exist_ok=True)
    derived = WORK / "derived"
    host = helper = None
    try:
        if arguments.build:
            build(arguments.relay, derived, arguments.device)
        kept = fixture(api)
        udid = simulator(arguments.device, arguments.trust)
        install(udid, derived)
        host = HostPhone(api, arguments.relay, kept["reader"])
        helper = Helper(api, arguments.server, arguments.relay, kept, host, udid, arguments.away_after)
        return test(udid, derived, {
            "ASPEN_TEST_SERVER": arguments.server,
            "ASPEN_TEST_USER": kept["reader"]["name"],
            "ASPEN_TEST_PASSWORD": PASSWORD,
            "ASPEN_TEST_CHANNEL": "general",
            "ASPEN_TEST_VOICE_CHANNEL": "probe-voice",
            "ASPEN_TEST_SCRATCH_CHANNEL": "scratch",
            "ASPEN_TEST_INVITE": f"aspen://app/invite/{kept['invite']['code']}",
            "ASPEN_TEST_INVITE_COMMUNITY": kept["invite"]["community"],
            "ASPEN_TEST_HELPER": helper.url,
        }, arguments.tests)
    except Failed as error:
        say(f"failed: {error}")
        return 1
    finally:
        if helper is not None:
            helper.close()
        if host is not None:
            host.close()


if __name__ == "__main__":
    sys.exit(main())
