"""A throwaway Aspen deployment for the checks in this directory: a database of its own, a NATS
of its own (a development stack's servers on a shared one would take its voice reports), and
the chat server and the voice server on ports of their own (so a development stack can keep
running beside it), and what talking to them takes, the event stream's and the voice server's WebSockets
included, with nothing beyond the Python standard library.

    with Stack(bins, name="smoke", ports=SMOKE_PORTS) as stack:
        stack.api("POST", "/users", {...})

The database is dropped and the servers and NATS stopped however the block ends; `Failed` raised inside
it prints both servers' logs. Needs Python 3.10, and `docker compose` with the database, valkey,
and seaweedfs services up (`start_services` starts them) and the NATS image it runs its own of.
"""

from __future__ import annotations

import base64
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path

from web_client import stand_in

REPO = Path(__file__).resolve().parent.parent
# The NATS docker-compose.yaml runs, which a stack runs one of its own of.
NATS_IMAGE = "nats:2.11-alpine"
TOKEN_SECRET = "throwaway-stack-voice-secret-at-least-32-bytes"
# The S3 credentials docker-compose.yaml gives SeaweedFS.
S3_ACCESS_KEY = "GK484e56c38fb7e14b182bf47a"
S3_SECRET_KEY = "6b49da9e42f7959cc946d7987a504763f6ec405b88abeeec08aa926b61316027"


class Failed(Exception):
    pass


@dataclass(frozen=True)
class Ports:
    nats: int
    api: int
    voice: int
    api_metrics: int
    voice_metrics: int
    rtc_min: int
    rtc_max: int
    # STUN and TURN for file transfers, and the ports relayed transfers take.
    transfer: int
    relay_min: int
    relay_max: int


def compose(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["docker", "compose", *args], cwd=REPO, capture_output=True, text=True, check=check)


def psql(sql: str, database: str | None = None) -> str:
    where = ["-d", database] if database else []
    return compose("exec", "-T", "database", "psql", "-U", "postgres", *where, "-tAc", sql).stdout.strip()


def wait_for(what: str, probe, seconds: float) -> None:
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            if probe():
                return
        except Exception as error:  # noqa: BLE001 - any failure is a reason to keep waiting
            last = error
        time.sleep(0.25)
    raise Failed(f"{what} did not happen within {seconds:.0f} s{f' ({last})' if last else ''}")


def http_status(url: str) -> int:
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


def start_services() -> None:
    compose("up", "-d", "database", "valkey", "seaweedfs")


def wait_for_services() -> None:
    wait_for("PostgreSQL", lambda: compose("exec", "-T", "database", "pg_isready", "-U", "postgres", check=False).returncode == 0, 90)
    wait_for("SeaweedFS's S3", lambda: http_status("http://127.0.0.1:8333/") in (200, 403, 404), 90)


def clean_env() -> dict[str, str]:
    """This environment without ASPEN_ variables, which would override the configs written."""
    return {k: v for k, v in os.environ.items() if not k.startswith("ASPEN_")}


class WebSocket:
    """A WebSocket client of the standard library, enough for the event stream and the voice
    server's signalling: text frames each way, pings answered, and the close code kept."""

    def __init__(self, url: str, timeout: float = 15):
        parts = urllib.parse.urlsplit(url)
        self.sock = socket.create_connection((parts.hostname, parts.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        path = parts.path + (f"?{parts.query}" if parts.query else "")
        self.sock.sendall(
            f"GET {path} HTTP/1.1\r\nHost: {parts.hostname}:{parts.port}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode()
        )
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(1)
            if not chunk:
                raise Failed(f"{url} closed during the handshake")
            head += chunk
        status_line = head.split(b"\r\n", 1)[0].decode()
        if " 101 " not in status_line:
            raise Failed(f"{url} refused the upgrade: {status_line}")
        self.closed: int | None = None

    def send(self, message: dict) -> None:
        payload = json.dumps(message).encode()
        mask = os.urandom(4)
        header = bytes([0x81])
        if len(payload) < 126:
            header += bytes([0x80 | len(payload)])
        elif len(payload) < 65536:
            header += bytes([0x80 | 126]) + struct.pack(">H", len(payload))
        else:
            header += bytes([0x80 | 127]) + struct.pack(">Q", len(payload))
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(header + mask + masked)

    def _exactly(self, n: int) -> bytes:
        data = b""
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise ConnectionError("closed")
            data += chunk
        return data

    def receive(self, timeout: float) -> dict | None:
        """The next text frame as JSON, or `None` when none arrives within `timeout` or the
        server closed (its code then in `closed`)."""
        if self.closed is not None:
            return None
        self.sock.settimeout(timeout)
        try:
            while True:
                first, second = self._exactly(2)
                length = second & 0x7F
                if length == 126:
                    length = struct.unpack(">H", self._exactly(2))[0]
                elif length == 127:
                    length = struct.unpack(">Q", self._exactly(8))[0]
                payload = self._exactly(length)
                opcode = first & 0x0F
                if opcode == 0x1:
                    return json.loads(payload)
                if opcode == 0x8:
                    self.closed = struct.unpack(">H", payload[:2])[0] if len(payload) >= 2 else 1005
                    return None
                if opcode == 0x9:
                    # A pong, masked with a zero key, which leaves the payload as it is.
                    self.sock.sendall(bytes([0x8A, 0x80 | len(payload)]) + bytes(4) + payload)
        except (socket.timeout, TimeoutError):
            return None
        except ConnectionError:
            self.closed = self.closed or 1006
            return None

    def close(self) -> None:
        try:
            if self.closed is None:
                # A close frame with code 1000, masked with a zero key.
                self.sock.sendall(bytes([0x88, 0x82]) + bytes(4) + struct.pack(">H", 1000))
            self.sock.close()
        except OSError:
            pass


class Stack:
    """A deployment of `bins` named `name`, on `ports`, with `settings` added to its aspen.toml."""

    def __init__(self, bins: Path, name: str, ports: Ports, settings: str = ""):
        self.bins = bins
        self.name = name
        self.ports = ports
        self.settings = settings
        self.database = f"aspen_{name}_{os.getpid()}"
        self.database_url = f"postgres://postgres:aspen_test@127.0.0.1:5432/{self.database}"
        self.work = Path(tempfile.mkdtemp(prefix=f"aspen-{name}-"))
        self.processes: list[tuple[subprocess.Popen, Path]] = []
        self.voice_id = ""
        self.nats = f"aspen-{name}-{os.getpid()}-nats"

    def __enter__(self) -> "Stack":
        try:
            self._start()
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise
        return self

    def __exit__(self, kind, error, trace) -> None:
        if isinstance(error, Failed):
            for _, log in self.processes:
                print(f"----- {log.name} -----\n{log.read_text()[-4000:]}", flush=True)
        for process, _ in self.processes:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        subprocess.run(["docker", "rm", "-f", self.nats], capture_output=True)
        psql(f"DROP DATABASE IF EXISTS {self.database} WITH (FORCE)")
        shutil.rmtree(self.work, ignore_errors=True)

    def _write_configs(self) -> None:
        p = self.ports
        (self.work / "aspen.toml").write_text(
            f'public_url = "http://127.0.0.1:{p.api}"\n'
            f'database_url = "{self.database_url}"\n'
            f'nats_url = "nats://127.0.0.1:{p.nats}"\n'
            'nats_auth_token = "aspen_test"\n'
            'valkey_url = "redis://127.0.0.1:6379"\n'
            f"{self.settings}"
            "[media.s3]\n"
            'endpoint = "http://127.0.0.1:8333"\n'
            'region = "seaweed"\n'
            'bucket = "aspen-media"\n'
            f'access_key = "{S3_ACCESS_KEY}"\n'
            f'secret_key = "{S3_SECRET_KEY}"\n'
            'public_base_url = "http://127.0.0.1:8888/buckets/aspen-media"\n'
            "[metrics]\n"
            f'listen_addr = "127.0.0.1:{p.api_metrics}"\n'
            "[web_client]\n"
            f"dir = {json.dumps(str(stand_in(self.work / 'web-client')))}\n"
            "[voice]\n"
            f'token_secret = "{TOKEN_SECRET}"\n'
        )
        if self.voice_id:
            (self.work / "voice_server.toml").write_text(
                f'id = "{self.voice_id}"\n'
                f'token_secret = "{TOKEN_SECRET}"\n'
                f'nats_url = "nats://127.0.0.1:{p.nats}"\n'
                'nats_auth_token = "aspen_test"\n'
                f'listen_addr = "127.0.0.1:{p.voice}"\n'
                "workers = 2\n"
                "[rtc]\n"
                f"min_port = {p.rtc_min}\n"
                f"max_port = {p.rtc_max}\n"
                "[transfer]\n"
                f"port = {p.transfer}\n"
                f"relay_min_port = {p.relay_min}\n"
                f"relay_max_port = {p.relay_max}\n"
                "[metrics]\n"
                f'listen_addr = "127.0.0.1:{p.voice_metrics}"\n'
            )

    def _spawn(self, args: list[str], log_name: str) -> None:
        log = open(self.work / log_name, "w")
        process = subprocess.Popen(args, cwd=self.work, env=clean_env(), stdout=log, stderr=subprocess.STDOUT)
        self.processes.append((process, self.work / log_name))

    def _start(self) -> None:
        for binary in ["aspen-chat-server", "voice_server", "aspen-migrate"]:
            if not (self.bins / binary).exists():
                raise Failed(f"{self.bins / binary} does not exist")
        psql(f"CREATE DATABASE {self.database}")
        subprocess.run(
            ["docker", "run", "-d", "--rm", "--name", self.nats, "-p", f"127.0.0.1:{self.ports.nats}:4222",
             NATS_IMAGE, "--jetstream", "--auth", "aspen_test"],
            check=True, capture_output=True,
        )
        wait_for("NATS", lambda: socket.create_connection(("127.0.0.1", self.ports.nats), timeout=1).close() is None, 30)
        self._write_configs()
        migrated = subprocess.run(
            [str(self.bins / "aspen-migrate"), "up", "--database-url", self.database_url],
            cwd=self.work, env=clean_env(), capture_output=True, text=True,
        )
        if migrated.returncode != 0:
            raise Failed(f"migrations failed: {migrated.stderr[-1000:]}")
        self.command("voice-servers", "add", self.name, "--url", f"http://127.0.0.1:{self.ports.voice}",
                     "--capacity", "50")
        self._spawn(
            [str(self.bins / "aspen-chat-server"), "--no-https", "--port", str(self.ports.api), "--listen-addr", "127.0.0.1"],
            "api.log",
        )
        wait_for("the chat server", lambda: http_status(f"{self.base}/api/v1/auth/methods") == 200, 60)
        self.voice_id = psql(f"SELECT id FROM voice_server WHERE name = '{self.name}'", self.database)
        if not self.voice_id:
            raise Failed("the voice server was not registered")
        self._write_configs()
        self._spawn([str(self.bins / "voice_server")], "voice.log")
        wait_for("the voice server", lambda: http_status(f"http://127.0.0.1:{self.ports.voice}/health") == 204, 60)

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.ports.api}"

    def request(self, method: str, path: str, body: dict | None = None, token: str | None = None,
                language: str | None = None) -> tuple[int, str]:
        """`path` under `/api/v1`, or a whole URL."""
        url = path if path.startswith("http") else f"{self.base}/api/v1{path}"
        headers = {"content-type": "application/json"}
        if token:
            headers["authorization"] = f"Bearer {token}"
        if language:
            headers["accept-language"] = language
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(url, method=method, data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=15) as response:
                return response.status, response.read().decode()
        except urllib.error.HTTPError as error:
            return error.code, error.read().decode()

    def api(self, method: str, path: str, body: dict | None = None, token: str | None = None,
            expect: tuple = (200, 201, 204)):
        status, text = self.request(method, path, body, token)
        if status not in expect:
            raise Failed(f"{method} {path}: {status} {text[:300]}")
        return json.loads(text) if text else {}

    def status(self, method: str, path: str, body: dict | None = None, token: str | None = None) -> int:
        return self.request(method, path, body, token)[0]

    def command(self, *args: str) -> str:
        """Runs an operator command of the chat server's against this deployment."""
        done = subprocess.run(
            [str(self.bins / "aspen-chat-server"), *args],
            cwd=self.work, env=clean_env(), capture_output=True, text=True, timeout=60,
        )
        if done.returncode != 0:
            raise Failed(f"`{' '.join(args)}` failed: {done.stderr[-1000:]}")
        return done.stdout

    def events(self, token: str) -> "EventStream":
        return EventStream(self, token)


class EventStream:
    """One identified event stream connection, gathering what it receives."""

    def __init__(self, stack: Stack, token: str):
        self.socket = WebSocket(f"ws://127.0.0.1:{stack.ports.api}/api/v1/events")
        self.socket.send({"type": "identify", "sessionToken": token})
        ready = self.socket.receive(10)
        if not ready or ready.get("type") != "ready":
            raise Failed(f"the event stream answered {ready}, closed {self.socket.closed}")
        self.events: list[dict] = []

    def gather(self, seconds: float = 1.0) -> list[dict]:
        """Everything that arrives within `seconds`, added to `events` and returned."""
        got = []
        deadline = time.monotonic() + seconds
        while (left := deadline - time.monotonic()) > 0:
            frame = self.socket.receive(left)
            if frame is None:
                if self.socket.closed is not None:
                    break
                continue
            if frame.get("type") == "event":
                got.append(frame["event"])
        self.events.extend(got)
        return got

    @property
    def closed(self) -> int | None:
        return self.socket.closed

    def close(self) -> None:
        self.socket.close()
