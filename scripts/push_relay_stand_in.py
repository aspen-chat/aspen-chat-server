#!/usr/bin/env python3
"""Stands in for the push relay (`spec/push.md`, The relay's API) for an app on a simulator or
emulator and a deployment on this machine, as `client/packages/mobile/ios/scripts/ui_tests.py`
uses it.

    scripts/push_relay_stand_in.py [--app-port 7788] [--push-port 7789] [--app-host HOST]

Two listeners. The app side, on every interface (`--app-port`), takes what the app sends a
relay (registering its device, subscribing for each deployment, updating its token, ending
either) with CORS for the web view, and answers `GET /stand-in/pushes?after=<n>` with every push
since the `n`th, for a test on another machine to read. It is plain HTTP, which only a
development build of the app reaches, and only at this machine; with `--app-host`, the name or
address the app reaches it by, it is HTTPS with a certificate for that host from
the development authority (`dev_ca.py`), which the app's device must trust. The push side,
HTTPS on loopback as `push.localhost` (`--push-port`) with a certificate from the same
authority, takes what a deployment sends; the deployment trusts that authority through
`[federation.development] extra_root_certificates` (`target/dev-ca/ca.pem`) and reaches
loopback with `allow_private_addresses`. A push is kept as the APNs payload the relay would
send, which only the device it is for could decrypt; nothing is sent onward.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import secrets
import ssl
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dev_ca import issue  # noqa: E402

HOST = "push.localhost"

devices: dict[str, dict] = {}
subscriptions: dict[str, str] = {}
"""Each subscription's device."""
pushes: list[dict] = []
lock = threading.Lock()


def say(text: str) -> None:
    print(f"push-relay-stand-in: {text}", flush=True)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args) -> None:
        pass

    def body(self) -> dict:
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        return json.loads(raw) if raw else {}

    def answer(self, status: int, payload: dict | None = None) -> None:
        self.send_response(status)
        self.send_header("Access-Control-Allow-Origin", "*")
        data = b"" if payload is None else json.dumps(payload).encode()
        if payload is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class AppSide(Handler):
    """What the app calls, and what a test reads."""

    push_origin = ""

    def do_OPTIONS(self) -> None:
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "authorization, content-type")
        self.send_header("Access-Control-Max-Age", "600")
        self.end_headers()

    def holds_secret(self, device: str) -> bool:
        return device in devices and self.headers.get("Authorization") == f"Bearer {devices[device]['secret']}"

    def do_GET(self) -> None:
        url = urllib.parse.urlsplit(self.path)
        if url.path != "/stand-in/pushes":
            self.answer(404, {"code": "notFound"})
            return
        after = int(urllib.parse.parse_qs(url.query).get("after", ["0"])[0])
        with lock:
            self.answer(200, {"pushes": pushes[after:]})

    def do_POST(self) -> None:
        parts = self.path.strip("/").split("/")
        if parts == ["v1", "devices"]:
            device = self.body()
            id = secrets.token_hex(8)
            with lock:
                devices[id] = {"secret": secrets.token_urlsafe(24), **device}
            say(f"device {id} registered: {device.get('platform')} {device.get('app')} {device.get('environment')}")
            self.answer(201, {"device": id, "secret": devices[id]["secret"]})
        elif len(parts) == 4 and parts[:2] == ["v1", "devices"] and parts[3] == "subscriptions":
            if not self.holds_secret(parts[2]):
                self.answer(401, {"code": "unauthorized"})
                return
            id = secrets.token_hex(8)
            with lock:
                subscriptions[id] = parts[2]
            say(f"subscription {id} for device {parts[2]}")
            self.answer(201, {"subscription": id, "endpoint": f"{self.push_origin}/v1/push/{id}"})
        else:
            self.answer(404, {"code": "notFound"})

    def do_PATCH(self) -> None:
        parts = self.path.strip("/").split("/")
        if len(parts) == 3 and parts[:2] == ["v1", "devices"] and self.holds_secret(parts[2]):
            with lock:
                devices[parts[2]]["token"] = self.body().get("token")
            self.answer(204)
        else:
            self.answer(404, {"code": "notFound"})

    def do_DELETE(self) -> None:
        parts = self.path.strip("/").split("/")
        if len(parts) == 3 and parts[:2] == ["v1", "devices"] and self.holds_secret(parts[2]):
            with lock:
                devices.pop(parts[2], None)
            self.answer(204)
        elif (len(parts) == 5 and parts[:2] == ["v1", "devices"] and parts[3] == "subscriptions"
              and self.holds_secret(parts[2])):
            with lock:
                subscriptions.pop(parts[4], None)
            self.answer(204)
        else:
            self.answer(404, {"code": "notFound"})


class PushSide(Handler):
    """What a deployment pushes to."""

    def do_POST(self) -> None:
        parts = self.path.strip("/").split("/")
        if len(parts) != 3 or parts[:2] != ["v1", "push"] or parts[2] not in subscriptions:
            self.answer(404, {"code": "notFound"})
            return
        subscription = parts[2]
        ciphertext = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        with lock:
            device = devices.get(subscriptions[subscription], {})
            pushes.append({
                "subscription": subscription,
                "device": {"platform": device.get("platform"), "token": device.get("token")},
                "payload": {
                    "aps": {
                        "alert": {"loc-key": "notification.placeholder"},
                        "mutable-content": 1,
                        "sound": "default",
                    },
                    "aspen": {
                        "s": subscription,
                        "c": base64.urlsafe_b64encode(ciphertext).rstrip(b"=").decode(),
                    },
                },
            })
            n = len(pushes)
        say(f"push {n} to subscription {subscription}, urgency {self.headers.get('Urgency')}")
        self.answer(201)


def tls(cert: Path, key: Path) -> ssl.SSLContext:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(str(cert), str(key))
    return context


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--app-port", type=int, default=7788)
    parser.add_argument("--push-port", type=int, default=7789)
    parser.add_argument("--app-host", help="serve the app side over HTTPS as this name or address")
    arguments = parser.parse_args()
    AppSide.push_origin = f"https://{HOST}:{arguments.push_port}"
    app = ThreadingHTTPServer(("0.0.0.0", arguments.app_port), AppSide)
    app_origin = f"http://0.0.0.0:{arguments.app_port}"
    if arguments.app_host is not None:
        app.socket = tls(*issue(arguments.app_host)).wrap_socket(app.socket, server_side=True)
        app_origin = f"https://{arguments.app_host}:{arguments.app_port}"
    push = ThreadingHTTPServer(("127.0.0.1", arguments.push_port), PushSide)
    push.socket = tls(*issue(HOST)).wrap_socket(push.socket, server_side=True)
    threading.Thread(target=push.serve_forever, daemon=True).start()
    say(f"app side {app_origin}, push side {AppSide.push_origin}")
    try:
        app.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
