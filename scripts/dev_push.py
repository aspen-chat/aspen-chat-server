#!/usr/bin/env python3
"""Checks that a deployment wakes phones as `spec/push.md` says, against the development
deployments of `dev_federation.py` (run `scripts/dev_federation.py up` first).

    scripts/dev_push.py

It stands in for a relay and a phone at once: an HTTPS push endpoint at `push.localhost`, with a
certificate from the development authority, that checks each push is signed with alpha's push
key (RFC 8292) and decrypts it with the phone's key (RFC 8291). Then it has one of alpha's users
message another and checks what reaches the phone, and what does not; and, running a voice server
of its own beside alpha for the while, has one call the other in a DM and checks the `call` push
that rings the phone, marked for the relay, and that nothing follows when the ring ends. The relay
itself, between a deployment and Apple or Google, is checked by the relay's own `scripts/e2e.py`.
Needs Python's `cryptography`.
"""

import base64
import json
import os
import re
import socket
import ssl
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dev_ca import issue  # noqa: E402
from dev_federation import (  # noqa: E402
    ALPHA, PASSWORD, Failed, api, bin_of, clean_env, expect, running_pid, say, sign_in, terminal, tls_context,
)
from stack import WebSocket  # noqa: E402

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


def serve(handler: type[BaseHTTPRequestHandler] = Endpoint) -> str:
    """Serves `handler` over HTTPS at `push.localhost` on a free port, answering its origin."""
    cert, key = issue(HOST)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(str(cert), str(key))
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    server = ThreadingHTTPServer(("127.0.0.1", port), handler)
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


def large_community_members() -> str:
    """Alpha's threshold of members at which a community is large, as `settings show` prints it."""
    for line in terminal(ALPHA, "settings", "show").splitlines():
        name, _, value = line.partition(": ")
        if name == "large-community-members":
            return value
    raise Failed("`settings show` on alpha does not name large_community_members")


def set_large_community_members(members: str, club: str, token: str, small: bool) -> None:
    """Sets alpha's threshold and waits until `club`, read as `token`, is small or not as
    `small` says, which is when every server has the new setting."""
    terminal(ALPHA, "settings", "set", "--large-community-members", members)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if api(ALPHA, "GET", f"/communities/{club}", token=token)["data"].get("small") is small:
            return
        time.sleep(0.2)
    raise Failed(f"the community did not become {'small' if small else 'large'} after setting the threshold")


class VoiceServer:
    """A voice server of this check's own beside alpha, registered with it for the while, so a
    call can start: a ring exists only once someone is in a call, which only a voice server
    reports. Alpha is served over HTTPS and registers only HTTPS voice servers, so a TLS front of
    this check's own, with a certificate from the development authority, stands before the voice
    server's plain listener, as a TLS proxy does in a deployment."""

    def __init__(self) -> None:
        self.name = f"dev-push-{int(time.time())}"
        self.host = f"voice-{self.name}.localhost"
        self.dir = ALPHA.dir / "voice-dev-push"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.front = socket.socket()
        self.front.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.front.bind(("127.0.0.1", 0))
        self.port = self.front.getsockname()[1]
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            self.plain_port = probe.getsockname()[1]
        self.process: subprocess.Popen | None = None

    @property
    def url(self) -> str:
        return f"https://{self.host}:{self.port}"

    def start(self) -> None:
        cert, key = issue(self.host)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(cert), str(key))
        self.front.listen()
        threading.Thread(target=self.relay, args=(context,), daemon=True).start()
        added = terminal(ALPHA, "voice-servers", "add", self.name, "--url", self.url, "--capacity", "50")
        given = re.search(r'^\s*id = "([0-9a-f-]+)"$', added, re.MULTILINE)
        if not given:
            raise Failed(f"`voice-servers add` printed no id: {added}")
        (self.dir / "voice_server.toml").write_text(
            f'id = "{given.group(1)}"\n'
            f'nats_url = "nats://127.0.0.1:{ALPHA.nats_port}"\n'
            'nats_auth_token = "aspen_test"\n'
            f'listen_addr = "127.0.0.1:{self.plain_port}"\n'
            "media_threads = 1\n"
            "[rtc]\n"
            f"min_port = {self.port + 1000}\n"
            f"max_port = {self.port + 1050}\n"
            "[transfer]\n"
            f"port = {self.port + 2000}\n"
            f"relay_min_port = {self.port + 2001}\n"
            f"relay_max_port = {self.port + 2050}\n"
            "[metrics]\n"
            "enabled = false\n"
        )
        log = open(self.dir / "voice.log", "w")
        self.process = subprocess.Popen([str(bin_of(ALPHA) / "voice_server")], cwd=self.dir, env=clean_env(),
                                        stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", self.plain_port), timeout=0.5):
                    return
            except OSError:
                if self.process.poll() is not None:
                    raise Failed(f"the voice server stopped: see {self.dir / 'voice.log'}")
                time.sleep(0.2)
        raise Failed("the voice server did not start listening")

    def relay(self, context: ssl.SSLContext) -> None:
        """Takes each TLS connection to the front and carries its bytes to and from the voice
        server's plain listener."""
        while True:
            try:
                client, _ = self.front.accept()
            except OSError:
                return
            threading.Thread(target=self.carry, args=(client, context), daemon=True).start()

    def carry(self, client: socket.socket, context: ssl.SSLContext) -> None:
        def pump(source: socket.socket, sink: socket.socket) -> None:
            try:
                while data := source.recv(65536):
                    sink.sendall(data)
            except OSError:
                pass
            finally:
                for end in (source, sink):
                    try:
                        end.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass

        try:
            with context.wrap_socket(client, server_side=True) as secure, \
                    socket.create_connection(("127.0.0.1", self.plain_port)) as plain:
                back = threading.Thread(target=pump, args=(plain, secure), daemon=True)
                back.start()
                pump(secure, plain)
                back.join()
        except OSError:
            pass

    def stop(self) -> None:
        self.front.close()
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        terminal(ALPHA, "voice-servers", "remove", self.name)


def main() -> int:
    if running_pid(ALPHA) is None:
        say("alpha is not running; run `scripts/dev_federation.py up` first")
        return 1
    origin = serve()
    try:
        # The check makes its community large by the threshold, which it leaves as it found it.
        threshold = large_community_members()
        try:
            check(origin)
        finally:
            terminal(ALPHA, "settings", "set", "--large-community-members", threshold)
        voice = VoiceServer()
        voice.start()
        try:
            check_rings(origin, voice)
        finally:
            voice.stop()
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
    general = next(c for c in api(ALPHA, "GET", f"/communities/{club['id']}/channels", token=sender)["data"]
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

    # Two members make a small community, whose channels tell of every message by default; with
    # the threshold at 0 no community is small, and a community's channels tell of tags alone.
    morning = api(ALPHA, "POST", f"/channels/{general['id']}/messages",
                  {"content": "Morning, all", "attachments": []}, token=sender, expect=(201,))
    pointer = phone.decrypt(next_push("a message in a small community")["body"])
    expect(pointer["message"] == morning["id"] and pointer["badge"] == 1,
           "a small community wakes the reader for a message that tags no one, which no badge counts")
    set_large_community_members("0", club["id"], sender, small=False)
    expect(True, "with the threshold at 0 the community is no longer small")
    api(ALPHA, "POST", f"/channels/{general['id']}/messages", {"content": "Morning again", "attachments": []},
        token=sender, expect=(201,))
    no_push("a message that tags no one wakes no one in a large community")

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


def join_call(channel: str, token: str, voice: VoiceServer) -> WebSocket:
    """Joins `channel`'s call on this check's voice server as the user `token` is, which is when
    the call starts and rings: the join offer, which must name that server, then `identify` on
    its socket, through the TLS front, until `ready`."""
    deadline = time.monotonic() + 90
    while True:
        status, text = request_status("POST", f"/channels/{channel}/voice/join", token)
        if status == 200:
            break
        if time.monotonic() > deadline:
            raise Failed(f"no voice server was offered: {status} {text[:300]}")
        time.sleep(0.5)
    offer = json.loads(text)
    candidates = [candidate["url"].rstrip("/") for candidate in offer["candidates"]]
    if voice.url not in candidates:
        raise Failed(f"the join offer names {candidates}, not this check's voice server {voice.url}")
    call = WebSocket(voice.url.replace("https://", "wss://") + "/ws", tls=tls_context())
    call.send({"type": "identify", "token": offer["token"]})
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        frame = call.receive(deadline - time.monotonic())
        if frame is None and call.closed is not None:
            raise Failed(f"the voice server closed the socket with {call.closed}")
        if frame is not None and frame.get("type") == "ready":
            return call
        if frame is not None and frame.get("type") == "error":
            raise Failed(f"the voice server refused the join: {frame}")
    raise Failed("the voice server never answered ready")


def request_status(method: str, path: str, token: str) -> tuple[int, str]:
    from dev_federation import request
    return request(method, f"https://{ALPHA.domain}/api/v1{path}", None, token)


def check_rings(origin: str, voice: VoiceServer) -> None:
    """A DM's call ringing a phone (`spec/push.md`, Calls): the `call` push, marked for the relay,
    its pointer, and the silence when the ring ends."""
    key = unb64(api(ALPHA, "GET", "/auth/methods")["push"]["applicationServerKey"])
    stamp = int(time.time())
    caller = sign_in(ALPHA, f"ringcaller{stamp}")
    api(ALPHA, "PATCH", "/users/@me", {"displayName": "Ring Caller"}, token=caller)
    caller_id = api(ALPHA, "GET", "/users/@me", token=caller)["id"]
    callee = sign_in(ALPHA, f"ringcallee{stamp}")
    callee_id = api(ALPHA, "GET", "/users/@me", token=callee)["id"]
    # A DM is only between people who share a community.
    club = api(ALPHA, "POST", "/communities", {"name": f"Rings {stamp}"}, token=caller)
    invite = api(ALPHA, "POST", f"/communities/{club['id']}/invites", {}, token=caller)
    api(ALPHA, "PUT", f"/communities/{club['id']}/members/@me", {"inviteCode": invite["code"]},
        token=callee, expect=(200, 201))
    phone = Phone()
    endpoint = f"{origin}/v1/push/ring{stamp}"
    api(ALPHA, "POST", "/users/@me/push-subscriptions",
        {"endpoint": endpoint, "p256dh": b64(phone.public), "auth": b64(phone.auth)}, token=callee, expect=(201,))
    dm = api(ALPHA, "POST", "/users/@me/dms", {"recipients": [callee_id]}, token=caller, expect=(200, 201))
    Endpoint.pushes.clear()

    call = join_call(dm["id"], caller, voice)
    try:
        push = next_push("a DM's call ringing the phone")
        verify_signature(push, origin, key)
        headers = push["headers"]
        session = api(ALPHA, "GET", f"/channels/{dm['id']}/voice", token=callee)["session"]
        expect(headers.get("aspen-push-kind") == "call" and headers["urgency"] == "high"
               and headers["topic"] == session["id"].replace("-", "") and 0 < int(headers["ttl"]) <= 30,
               "the ring wakes the phone with a push marked as a call, by its session, living as long as the ring")
        pointer = phone.decrypt(push["body"])
        # The server writes nanoseconds; Python reads at most microseconds.
        until = datetime.fromisoformat(re.sub(r"\.(\d{6})\d+", r".\1", pointer.pop("until")).replace("Z", "+00:00"))
        left = (until - datetime.now(timezone.utc)).total_seconds()
        expect(pointer == {"v": 1, "kind": "call", "channel": dm["id"], "session": session["id"],
                           "caller": caller_id, "callerName": "Ring Caller"} and 0 < left <= 30,
               "the phone decrypts the call's channel, session, and caller by name, and when the ring ends")
        rings = api(ALPHA, "GET", "/users/@me/dms?include=voice", token=callee).get("included", {}).get("voiceRings", [])
        expect(any(r["user"] == callee_id and r["session"] == session["id"] for r in rings),
               "the ring stands at the deployment meanwhile")
        api(ALPHA, "DELETE", f"/channels/{dm['id']}/voice/rings/@me", token=callee, expect=(204,))
        no_push("declining the ring pushes nothing: the phone hears of it through the event stream")
    finally:
        call.sock.close()
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if api(ALPHA, "GET", f"/channels/{dm['id']}/voice", token=callee).get("session") is None:
            break
        time.sleep(0.5)
    else:
        raise Failed("the call did not end once its caller left")
    no_push("the call's end pushes nothing either", seconds=3)
    # The message the call left wakes no one: a missed call is told of by the ring.
    expect(not Endpoint.pushes, "the missed call's message wakes no one")


if __name__ == "__main__":
    sys.exit(main())
