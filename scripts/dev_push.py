#!/usr/bin/env python3
"""Checks that a deployment wakes phones as `spec/push.md` says, against the development
deployments of `dev_federation.py` (run `scripts/dev_federation.py up` first).

    scripts/dev_push.py

It stands in for a relay and a phone at once: an HTTPS push endpoint at `push.localhost`, with a
certificate from the development authority, that checks each push is signed with alpha's push
key (RFC 8292) and decrypts it with the phone's key (RFC 8291). Then it has one of alpha's users
message another and checks what reaches the phone, and what does not. The relay itself, between a
deployment and Apple or Google, is checked by the relay's own `scripts/e2e.py`. Needs Python's
`cryptography`.
"""

import base64
import json
import os
import socket
import ssl
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dev_federation import (  # noqa: E402
    ALPHA, PASSWORD, Failed, api, expect, issue_certificate, make_certificates, running_pid, say,
    sign_in,
)

HOST = "push.localhost"


def b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def unb64(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def hkdf(salt: bytes, ikm: bytes, info: bytes, length: int) -> bytes:
    return HKDF(algorithm=hashes.SHA256(), length=length, salt=salt, info=info).derive(ikm)


class Phone:
    """A phone's keys for one subscription, and what it makes of a push."""

    def __init__(self) -> None:
        self.key = ec.generate_private_key(ec.SECP256R1())
        self.public = self.key.public_key().public_bytes(
            serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
        self.auth = os.urandom(16)

    def decrypt(self, message: bytes) -> dict:
        salt, id_length = message[:16], message[20]
        as_public = message[21:21 + id_length]
        record = message[21 + id_length:]
        peer = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), as_public)
        secret = self.key.exchange(ec.ECDH(), peer)
        ikm = hkdf(self.auth, secret, b"WebPush: info\0" + self.public + as_public, 32)
        cek = hkdf(salt, ikm, b"Content-Encoding: aes128gcm\0", 16)
        nonce = hkdf(salt, ikm, b"Content-Encoding: nonce\0", 12)
        padded = AESGCM(cek).decrypt(nonce, record, None).rstrip(b"\0")
        if not padded.endswith(b"\x02"):
            raise Failed("a push's one record does not end with the last-record delimiter")
        return json.loads(padded[:-1])


class Endpoint(BaseHTTPRequestHandler):
    """The push endpoint: records each push and answers with `answer`."""
    pushes: list = []
    answer = 201

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.pushes.append({"path": self.path, "headers": {k.lower(): v for k, v in self.headers.items()},
                            "body": body})
        self.send_response(self.answer)
        self.end_headers()


def serve() -> str:
    cert, key = issue_certificate(HOST)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(str(cert), str(key))
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    server = ThreadingHTTPServer(("127.0.0.1", port), Endpoint)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"https://{HOST}:{port}"


def verify_signature(push: dict, origin: str, key: bytes) -> None:
    """RFC 8292: signed with the deployment's key, for this endpoint's origin."""
    scheme, _, rest = push["headers"]["authorization"].partition(" ")
    parts = dict(part.strip().split("=", 1) for part in rest.split(","))
    if scheme != "vapid" or unb64(parts["k"]) != key:
        raise Failed("a push is not signed with alpha's push key")
    signing_input, signature = parts["t"].rsplit(".", 1)
    raw = unb64(signature)
    public = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), key)
    public.verify(encode_dss_signature(int.from_bytes(raw[:32], "big"), int.from_bytes(raw[32:], "big")),
                  signing_input.encode(), ec.ECDSA(hashes.SHA256()))
    claims = json.loads(unb64(signing_input.split(".")[1]))
    if claims["aud"] != origin or claims["exp"] <= time.time():
        raise Failed(f"a push's token is for {claims['aud']}, or has expired")


def next_push(what: str, seconds: float = 15) -> dict:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if Endpoint.pushes:
            return Endpoint.pushes.pop(0)
        time.sleep(0.1)
    raise Failed(f"no push came for {what}")


def no_push(what: str, seconds: float = 4) -> None:
    time.sleep(seconds)
    expect(not Endpoint.pushes, what)
    Endpoint.pushes.clear()


def main() -> int:
    if running_pid(ALPHA) is None:
        say("alpha is not running; run `scripts/dev_federation.py up` first")
        return 1
    make_certificates()
    origin = serve()
    try:
        check(origin)
    except Failed as failure:
        say(f"FAILED: {failure}")
        return 1
    say("every push check passed")
    return 0


def check(origin: str) -> None:
    key = unb64(api(ALPHA, "GET", "/auth/methods")["push"]["applicationServerKey"])
    expect(len(key) == 65, "alpha advertises its push key")

    stamp = int(time.time())
    sender = sign_in(ALPHA, f"pushsender{stamp}")
    sign_in(ALPHA, f"pushreader{stamp}")
    # Signed in again, keeping the refresh token, to sign out with at the end.
    reader_session = api(ALPHA, "POST", "/auth/login", {"username": f"pushreader{stamp}", "password": PASSWORD})
    reader = reader_session["sessionToken"]
    reader_id = api(ALPHA, "GET", "/users/@me", token=reader)["id"]
    club = api(ALPHA, "POST", "/communities", {"name": f"Push {stamp}"}, token=sender)
    invite = api(ALPHA, "POST", f"/communities/{club['id']}/invites", {}, token=sender)
    api(ALPHA, "PUT", f"/communities/{club['id']}/members/@me", {"inviteCode": invite["code"]},
        token=reader, expect=(200, 201))
    general = next(c for c in api(ALPHA, "GET", f"/communities/{club['id']}/channels", token=sender)
                   if c["ty"] == "text")

    phone = Phone()
    endpoint = f"{origin}/v1/push/reader{stamp}"
    subscription = {"endpoint": endpoint, "p256dh": b64(phone.public), "auth": b64(phone.auth)}
    api(ALPHA, "POST", "/users/@me/push-subscriptions", {**subscription, "endpoint": "http://x"},
        token=reader, expect=(400,))
    expect(True, "an endpoint that is not HTTPS is refused")
    registered = api(ALPHA, "POST", "/users/@me/push-subscriptions", subscription, token=reader,
                     expect=(201,))
    expect(registered["endpoint"] == endpoint, "the reader's phone registers its endpoint")
    Endpoint.pushes.clear()

    dm = api(ALPHA, "POST", "/users/@me/dms", {"recipients": [reader_id]}, token=sender, expect=(200, 201))
    hello = api(ALPHA, "POST", f"/channels/{dm['id']}/messages", {"content": "Hello!", "attachments": []},
                token=sender, expect=(201,))
    push = next_push("a DM")
    verify_signature(push, origin, key)
    expect(push["headers"]["content-encoding"] == "aes128gcm" and push["headers"]["urgency"] == "high"
           and push["headers"]["topic"] == dm["id"].replace("-", "") and int(push["headers"]["ttl"]) > 0,
           "a DM wakes the reader's phone at once, signed with alpha's key")
    pointer = phone.decrypt(push["body"])
    expect(pointer == {"v": 1, "kind": "message", "channel": dm["id"], "message": hello["id"], "badge": 1},
           "the phone decrypts a pointer to the message, and nothing of what it says")

    api(ALPHA, "POST", f"/channels/{general['id']}/messages", {"content": "Morning, all", "attachments": []},
        token=sender, expect=(201,))
    no_push("a message that tags no one wakes no one")

    tagged = api(ALPHA, "POST", f"/channels/{general['id']}/messages",
                 {"content": f"<@{reader_id}> lunch?", "attachments": []}, token=sender, expect=(201,))
    pointer = phone.decrypt(next_push("a tag")["body"])
    expect(pointer == {"v": 1, "kind": "message", "channel": general["id"], "message": tagged["id"],
                       "badge": 2},
           "a message tagging the reader wakes them, counting the tag and the unread DM")

    api(ALPHA, "PUT", f"/channels/{dm['id']}/read-states/@me", {"lastRead": hello["id"]}, token=reader,
        expect=(204,))
    push = next_push("reading the DM")
    expect(push["headers"]["urgency"] == "low"
           and phone.decrypt(push["body"]) == {"v": 1, "kind": "read", "channel": dm["id"],
                                               "message": hello["id"], "badge": 1},
           "reading the DM elsewhere tells the phone, quietly, to take its notification down")

    api(ALPHA, "DELETE", f"/messages/{tagged['id']}", token=sender, expect=(204,))
    push = next_push("a deletion")
    expect(phone.decrypt(push["body"]) == {"v": 1, "kind": "deleted", "channel": general["id"],
                                           "message": tagged["id"]},
           "a deleted message's notification is taken down")

    api(ALPHA, "PUT", f"/communities/{club['id']}/notification-settings/@me", {"level": "all"}, token=reader,
        expect=(201,))
    chatter = api(ALPHA, "POST", f"/channels/{general['id']}/messages", {"content": "Nice weather", "attachments": []},
                  token=sender, expect=(201,))
    expect(phone.decrypt(next_push("a message in a community set to all")["body"])["message"] == chatter["id"],
           "a community the reader wants every message of wakes them for one that tags no one")
    api(ALPHA, "PUT", f"/channels/{general['id']}/notification-settings/@me", {"level": "nothing"}, token=reader,
        expect=(201,))
    api(ALPHA, "POST", f"/channels/{general['id']}/messages", {"content": f"<@{reader_id}> hello?", "attachments": []},
        token=sender, expect=(201,))
    no_push("a channel set to nothing outranks its community, even for a tag")
    api(ALPHA, "DELETE", f"/channels/{general['id']}/notification-settings/@me", token=reader, expect=(204,))
    api(ALPHA, "DELETE", f"/communities/{club['id']}/notification-settings/@me", token=reader, expect=(204,))
    api(ALPHA, "POST", f"/channels/{general['id']}/messages", {"content": "Back to normal", "attachments": []},
        token=sender, expect=(201,))
    no_push("with the settings removed, a community tells only of tags again")
    api(ALPHA, "PUT", f"/channels/{dm['id']}/notification-settings/@me", {"level": "tags"}, token=reader,
        expect=(201,))
    api(ALPHA, "POST", f"/channels/{dm['id']}/messages", {"content": "Just chatting", "attachments": []},
        token=sender, expect=(201,))
    no_push("a DM set to tags only is quiet for a message that tags no one")
    api(ALPHA, "DELETE", f"/channels/{dm['id']}/notification-settings/@me", token=reader, expect=(204,))
    Endpoint.pushes.clear()

    api(ALPHA, "PUT", f"/channels/{dm['id']}/mutes/@me", {}, token=reader, expect=(201,))
    api(ALPHA, "POST", f"/channels/{dm['id']}/messages", {"content": "Still there?", "attachments": []},
        token=sender, expect=(201,))
    no_push("a muted DM wakes no one")
    api(ALPHA, "DELETE", f"/channels/{dm['id']}/mutes/@me", token=reader, expect=(204,))

    Endpoint.answer = 410
    api(ALPHA, "POST", f"/channels/{dm['id']}/messages", {"content": "Hello?", "attachments": []},
        token=sender, expect=(201,))
    next_push("the last push to a gone phone")
    Endpoint.answer = 201
    api(ALPHA, "POST", f"/channels/{dm['id']}/messages", {"content": "Anyone?", "attachments": []},
        token=sender, expect=(201,))
    no_push("a phone its push service says is gone is not pushed to again")

    api(ALPHA, "POST", "/users/@me/push-subscriptions", subscription, token=reader, expect=(201,))
    api(ALPHA, "POST", "/auth/logout", {"refreshToken": reader_session["refreshToken"]}, token=reader,
        expect=(200, 204))
    api(ALPHA, "POST", f"/channels/{dm['id']}/messages", {"content": "Goodbye", "attachments": []},
        token=sender, expect=(201,))
    no_push("signing out stops the phone being woken")


if __name__ == "__main__":
    sys.exit(main())
