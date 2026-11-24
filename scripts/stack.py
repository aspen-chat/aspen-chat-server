"""A throwaway Aspen deployment for the checks in this directory, and what talking to it takes,
the event stream's and the voice server's WebSockets included, with nothing beyond the Python
standard library. It is one of two kinds, each on ports of its own so a development stack can keep
running beside it:

- `Stack`, a full deployment: a database of its own, a NATS of its own (a development stack's
  servers on a shared one would take its voice reports), and the chat server and the voice server.
  Needs `docker compose` with the database, valkey, and seaweedfs services up (`start_services`
  starts them) and the NATS image it runs its own of.
- `UlStack`, Aspen Ultra Lite: aspen-ul with a data directory of its own, its PostgreSQL the
  installation `scripts/package_ul.py --postgres-only` builds. It needs no service.

    parser.add_argument("--bin", ...)
    stack.add_arguments(parser)  # --ul and --ul-postgres
    with stack.deployment(args, "smoke", SMOKE_PORTS) as deployment:
        deployment.api("POST", "/users", {...})

What the deployment made is removed however the block ends; `Failed` raised inside it prints its
servers' logs. Needs Python 3.10.
"""

from __future__ import annotations

import base64
import json
import os
import re
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
import zlib
from dataclasses import dataclass
from pathlib import Path

import device_credential
from web_client import stand_in

REPO = Path(__file__).resolve().parent.parent
# The NATS docker-compose.yaml runs, which a stack runs one of its own of.
NATS_IMAGE = "nats:2.11-alpine"
# The Valkey docker-compose.yaml runs, unless `VALKEY_URL` names another, as the Rust tests read it.
VALKEY_URL = os.environ.get("VALKEY_URL", "redis://127.0.0.1:6379")
# The fonts SVG previews' text is drawn in (`scripts/svg_fonts.py`), unless `ASPEN_SVG_FONTS` names
# others, as the SVG renderer's tests read them. A stack draws SVGs with the `aspen-svg-render` of
# its `bins` and these, and makes no previews of SVGs when either is missing.
SVG_FONTS = Path(os.environ.get("ASPEN_SVG_FONTS", REPO / "target" / "svg-fonts"))
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


def wait_for_services(media: str = "s3", database: bool = True) -> None:
    """Waits for the services a stack keeping its files in `media` needs: PostgreSQL unless
    `database` is off, and SeaweedFS unless the files are in a directory."""
    if database:
        wait_for("PostgreSQL", lambda: compose("exec", "-T", "database", "pg_isready", "-U", "postgres", check=False).returncode == 0, 90)
    if media == "s3":
        wait_for("SeaweedFS's S3", lambda: http_status("http://127.0.0.1:8333/") in (200, 403, 404), 90)



def valkey_integer(*command: str) -> int:
    """What the Valkey a stack uses (`VALKEY_URL`) answers `command` with, a command whose answer
    is an integer (`EXISTS`, `DEL`), spoken in RESP over a socket so no client is needed."""
    url = urllib.parse.urlsplit(VALKEY_URL)
    if url.scheme != "redis" or url.password:
        raise Failed(f"VALKEY_URL {VALKEY_URL!r}: only redis:// without a password is spoken here")
    request = f"*{len(command)}\r\n".encode() + b"".join(
        f"${len(part.encode())}\r\n".encode() + part.encode() + b"\r\n" for part in command
    )
    with socket.create_connection((url.hostname or "127.0.0.1", url.port or 6379), timeout=5) as conn:
        conn.sendall(request)
        reply = conn.makefile("rb").readline()
    if not reply.startswith(b":"):
        raise Failed(f"Valkey answered {' '.join(command)} with {reply!r}, not an integer")
    return int(reply[1:].strip())

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


class Deployment:
    """A deployment named `name` on `ports`, running from the binaries in `bins`, with `settings`
    added to its aspen.toml: what every check talks to, whichever way it runs (`Stack`, the
    servers and services of a full deployment; `UlStack`, aspen-ul). `deployment` makes the one
    a check's arguments ask for."""

    # Whether this is aspen-ul, for the few checks of what only one kind of deployment has.
    ul = False

    def __init__(self, bins: Path, name: str, ports: Ports, settings: str = "",
                 public_host: str = "127.0.0.1", web_client: Path | None = None):
        """`public_host` is `public_url`'s host: `localhost` offers passkeys, at that domain,
        which an address does not. `web_client` is the directory served as the web client, a
        stand-in when none is given."""
        self.bins = bins
        self.name = name
        self.ports = ports
        self.settings = settings
        self.public_host = public_host
        self.web_client = web_client
        self.work = Path(tempfile.mkdtemp(prefix=f"aspen-{name}-"))
        self.processes: list[tuple[subprocess.Popen, Path]] = []
        self.voice_id = ""
        self.devices = device_credential.Devices()

    def __enter__(self) -> "Deployment":
        try:
            self._start()
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise
        return self

    def __exit__(self, kind, error, trace) -> None:
        # Any error, not only a check's: a request refused by a server that has died is one too.
        if error is not None:
            for process, log in self.processes:
                ended = process.poll()
                # A negative code is the signal that killed it: -11 a segmentation fault, -9 the
                # kernel's OOM killer, neither of which the process gets to log.
                state = "still running" if ended is None else f"exited with {ended}"
                print(f"----- {log.name} ({state}) -----\n{log.read_text()[-4000:]}", flush=True)
        for process, _ in self.processes:
            process.terminate()
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill()
        self._stop()
        shutil.rmtree(self.work, ignore_errors=True)

    def _start(self) -> None:
        raise NotImplementedError

    def _stop(self) -> None:
        """Removes what the deployment keeps outside its work directory."""
        raise NotImplementedError

    def _spawn(self, args: list[str], log_name: str) -> subprocess.Popen:
        log = open(self.work / log_name, "w")
        process = subprocess.Popen(args, cwd=self.work, env=clean_env(), stdout=log, stderr=subprocess.STDOUT)
        self.processes.append((process, self.work / log_name))
        return process

    def _wait_serving(self, what: str, url: str, process: subprocess.Popen, seconds: float) -> None:
        """Waits for `url` to answer 204, failing at once should `process` end first."""
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise Failed(f"{what} exited with {process.returncode} before it served")
            try:
                if http_status(url) == 204:
                    return
            except OSError:
                pass
            time.sleep(0.25)
        raise Failed(f"{what} did not serve within {seconds:.0f} s")

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.ports.api}"

    @property
    def voice_socket(self) -> str:
        """The voice server's signalling WebSocket."""
        raise NotImplementedError

    def request(self, method: str, path: str, body: dict | None = None, token: str | None = None,
                language: str | None = None, device: bool = True) -> tuple[int, str]:
        """`path` under `/api/v1`, or a whole URL. Without `token`, it presents the stack's device
        credential unless `device` is off."""
        url = path if path.startswith("http") else f"{self.base}/api/v1{path}"
        headers = {"content-type": "application/json"}
        if token:
            headers["authorization"] = f"Bearer {token}"
        elif device:
            headers[device_credential.HEADER] = self.devices.credential(
                self.base, lambda method, path, body: self.request(method, path, body, device=False))
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
        """Runs an operator command against this deployment, returning what it printed."""
        done = subprocess.run(
            [*self._command_line(), *args],
            cwd=self.work, env=clean_env(), capture_output=True, text=True, timeout=120,
        )
        if done.returncode != 0:
            raise Failed(f"`{' '.join(args)}` failed: {done.stderr[-1000:]}")
        return done.stdout

    def _command_line(self) -> list[str]:
        """What an operator command's arguments follow."""
        raise NotImplementedError

    def sql(self, statement: str) -> str:
        """What the deployment's database answers `statement` with, unaligned, one row a line."""
        raise NotImplementedError

    def events(self, token: str) -> "EventStream":
        return EventStream(self, token)


class Stack(Deployment):
    """A full deployment: a database of its own in docker-compose.yaml's PostgreSQL, a NATS of
    its own, and the chat server and the voice server, keeping its files in `media`."""

    def __init__(self, bins: Path, name: str, ports: Ports, settings: str = "",
                 public_host: str = "127.0.0.1", web_client: Path | None = None,
                 media: str = "s3"):
        """`media` is where files are kept: `s3`, the SeaweedFS of docker-compose.yaml, or
        `directory`, a directory of the stack's own that the chat server serves at
        `media.localhost` on its port (`[media.directory]`)."""
        if media not in ("s3", "directory"):
            raise Failed(f"no media store {media!r}: s3 or directory")
        super().__init__(bins, name, ports, settings, public_host, web_client)
        self.media = media
        self.database = f"aspen_{name}_{os.getpid()}"
        self.database_url = f"postgres://postgres:aspen_test@127.0.0.1:5432/{self.database}"
        self.nats = f"aspen-{name}-{os.getpid()}-nats"

    def _stop(self) -> None:
        subprocess.run(["docker", "rm", "-f", self.nats], capture_output=True)
        psql(f"DROP DATABASE IF EXISTS {self.database} WITH (FORCE)")

    def _write_configs(self) -> None:
        p = self.ports
        (self.work / "aspen.toml").write_text(
            f'public_url = "http://{self.public_host}:{p.api}"\n'
            f'database_url = "{self.database_url}"\n'
            f'nats_url = "nats://127.0.0.1:{p.nats}"\n'
            'nats_auth_token = "aspen_test"\n'
            f'valkey_url = "{VALKEY_URL}"\n'
            f"{self.settings}"
            f"{media_settings(self.media, self.work, p.api)}"
            "[media.previews]\n"
            f"svg_renderer = {json.dumps(str(self.bins / 'aspen-svg-render'))}\n"
            f"svg_fonts = {json.dumps(str(SVG_FONTS.resolve()))}\n"
            "[metrics]\n"
            f'listen_addr = "127.0.0.1:{p.api_metrics}"\n'
            "[web_client]\n"
            f"dir = {json.dumps(str(self.web_client or stand_in(self.work / 'web-client')))}\n"
        )
        if self.voice_id:
            (self.work / "voice_server.toml").write_text(
                f'id = "{self.voice_id}"\n'
                f'nats_url = "nats://127.0.0.1:{p.nats}"\n'
                'nats_auth_token = "aspen_test"\n'
                f'listen_addr = "127.0.0.1:{p.voice}"\n'
                "media_threads = 2\n"
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

    def nats_arguments(self) -> list[str]:
        """What `docker run` takes after the container's name and its client port: the image and
        how NATS is started, after any options of the container's own."""
        return [NATS_IMAGE, "--jetstream", "--auth", "aspen_test"]

    def start_nats(self) -> None:
        subprocess.run(
            ["docker", "run", "-d", "--rm", "--name", self.nats, "-p", f"127.0.0.1:{self.ports.nats}:4222",
             *self.nats_arguments()],
            check=True, capture_output=True,
        )
        wait_for("NATS", lambda: socket.create_connection(("127.0.0.1", self.ports.nats), timeout=1).close() is None, 30)

    def _start(self) -> None:
        for binary in ["aspen-chat-server", "voice_server", "aspen-migrate"]:
            if not (self.bins / binary).exists():
                raise Failed(f"{self.bins / binary} does not exist")
        psql(f"CREATE DATABASE {self.database}")
        self.start_nats()
        self._write_configs()
        migrated = subprocess.run(
            [str(self.bins / "aspen-migrate"), "up", "--database-url", self.database_url],
            cwd=self.work, env=clean_env(), capture_output=True, text=True,
        )
        if migrated.returncode != 0:
            raise Failed(f"migrations failed: {migrated.stderr[-1000:]}")
        added = self.command("voice-servers", "add", self.name, "--url", f"http://127.0.0.1:{self.ports.voice}",
                             "--capacity", "50")
        # The id is taken from what `add` prints, as an operator takes it.
        given = re.search(r'^\s*id = "([0-9a-f-]+)"$', added, re.MULTILINE)
        if not given:
            raise Failed(f"`voice-servers add` printed no id: {added}")
        api = self._spawn(
            [str(self.bins / "aspen-chat-server"), "--no-https", "--port", str(self.ports.api), "--listen-addr", "127.0.0.1"],
            "api.log",
        )
        self._wait_serving("the chat server", f"{self.base}/api/v1/health", api, 60)
        self.voice_id = given.group(1)
        if self.sql(f"SELECT id FROM voice_server WHERE name = '{self.name}'") != self.voice_id:
            raise Failed(f"`voice-servers add` printed {self.voice_id}, not the id it registered")
        self._write_configs()
        voice = self._spawn([str(self.bins / "voice_server")], "voice.log")
        self._wait_serving("the voice server", f"http://127.0.0.1:{self.ports.voice}/health", voice, 60)

    def restart_voice(self) -> subprocess.Popen:
        """Stops the voice server and starts it again as it was, logging to `voice-again.log`."""
        for process, log in self.processes:
            if log.name == "voice.log" and process.poll() is None:
                process.terminate()
                process.wait(timeout=30)
        return self._spawn([str(self.bins / "voice_server")], "voice-again.log")

    @property
    def voice_socket(self) -> str:
        return f"ws://127.0.0.1:{self.ports.voice}/ws"

    def _command_line(self) -> list[str]:
        return [str(self.bins / "aspen-chat-server")]

    def sql(self, statement: str) -> str:
        return psql(statement, self.database)


class UlStack(Deployment):
    """Aspen Ultra Lite: `aspen-ul` from `bins`, with a data directory of its own, run with the
    PostgreSQL installation `postgres` (`scripts/package_ul.py --postgres-only` builds one), the
    web client, and the `aspen-svg-render` of `bins` with `SVG_FONTS` as its runtime (`--runtime`),
    its voice server within it. Files are kept in its
    data directory, or with `media` `s3` in docker-compose.yaml's SeaweedFS
    (`[server.media.s3]`). `ports.api` is the port it serves at, `rtc_min` its calls' media port,
    `transfer` and `relay_min` to `relay_max` the file transfers', and `api_metrics` its metrics'
    when `settings` turn them on; it uses no other."""

    ul = True

    def __init__(self, bins: Path, name: str, ports: Ports, settings: str = "",
                 public_host: str = "127.0.0.1", web_client: Path | None = None,
                 media: str = "directory", postgres: Path | None = None):
        if media not in ("s3", "directory"):
            raise Failed(f"no media store {media!r}: s3 or directory")
        super().__init__(bins, name, ports, settings, public_host, web_client)
        self.media = media
        self.postgres = (postgres or ul_postgres()).resolve()
        self.data = self.work / "data"
        self.runtime = self.work / "runtime"

    def _stop(self) -> None:
        pass

    def _start(self) -> None:
        if not (self.bins / "aspen-ul").exists():
            raise Failed(f"{self.bins / 'aspen-ul'} does not exist")
        if not (self.postgres / "bin" / "postgres").exists():
            raise Failed(f"{self.postgres} holds no PostgreSQL: build one with "
                         "scripts/package_ul.py --arch <arch> --postgres-only, or name one with --ul-postgres")
        self.runtime.mkdir()
        (self.runtime / "pgsql").symlink_to(self.postgres)
        (self.runtime / "web-client").symlink_to(
            (self.web_client or stand_in(self.work / "web-client")).resolve())
        # Where aspen-ul looks for them in its runtime (`server/ul/src/bundle.rs`).
        (self.runtime / "svg-render").mkdir()
        (self.runtime / "svg-render" / "aspen-svg-render").symlink_to(self.bins / "aspen-svg-render")
        (self.runtime / "svg-render" / "fonts").symlink_to(SVG_FONTS.resolve())
        self.data.mkdir(mode=0o750)
        p = self.ports
        media = "" if self.media == "directory" else media_settings("s3", self.work, p.api, under="server.")
        (self.data / "aspen-ul.toml").write_text(
            f'public_url = "http://{self.public_host}:{p.api}"\n'
            f'media_url = "http://media.localhost:{p.api}"\n'
            'listen = ["127.0.0.1"]\n'
            f"port = {p.api}\n"
            # What the machines it is meant for have at most, so the database, its pool, and the
            # in-process bus are sized as they are there.
            "memory_mib = 2048\n"
            '[tls]\nmode = "off"\n'
            f"[voice]\nmedia_port = {p.rtc_min}\ntransfer_port = {p.transfer}\n"
            f"relay_ports = [{p.relay_min}, {p.relay_max}]\ncapacity = 50\n"
            "[server]\n"
            f"{under_server(self.settings)}"
            f"{media}"
        )
        served = self._spawn([*self._command_line(), "serve"], "ul.log")
        # Its first start makes the database and runs every migration, which a debug build takes
        # a while over.
        self._wait_serving("aspen-ul", f"{self.base}/api/v1/health", served, 180)
        self.voice_id = self.sql("SELECT id FROM voice_server WHERE name = 'aspen-ul'")
        if not self.voice_id:
            raise Failed("aspen-ul registered no voice server of its own")

    def restart(self, log_name: str) -> None:
        """Stops aspen-ul if it runs and starts it again as it was, logging to `log_name`, and
        reads which voice server it registered."""
        for process, _ in self.processes:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=60)
        served = self._spawn([*self._command_line(), "serve"], log_name)
        self._wait_serving("aspen-ul", f"{self.base}/api/v1/health", served, 120)
        self.voice_id = self.sql("SELECT id FROM voice_server WHERE name = 'aspen-ul'")

    @property
    def voice_socket(self) -> str:
        return f"ws://127.0.0.1:{self.ports.api}/voice-server/ws"

    def _command_line(self) -> list[str]:
        return [str(self.bins / "aspen-ul"), "--data", str(self.data), "--runtime", str(self.runtime)]

    def sql(self, statement: str) -> str:
        done = subprocess.run(
            [str(self.postgres / "bin" / "psql"), "-X", "-h", str(self.data / "run"), "-d", "aspen",
             "-v", "ON_ERROR_STOP=1", "-tAc", statement],
            env={**clean_env(), "LC_ALL": "C"}, capture_output=True, text=True, timeout=60,
        )
        if done.returncode != 0:
            raise Failed(f"psql: {done.stderr.strip()}")
        return done.stdout.strip()


def media_settings(media: str, work: Path, port: int, under: str = "") -> str:
    """aspen.toml's settings for keeping files in `media`, its tables under `under`."""
    if media == "directory":
        return (
            f"[{under}media.directory]\n"
            f"path = {json.dumps(str(work / 'media'))}\n"
            f'public_base_url = "http://media.localhost:{port}"\n'
        )
    return (
        f"[{under}media.s3]\n"
        'endpoint = "http://127.0.0.1:8333"\n'
        'region = "seaweed"\n'
        'bucket = "aspen-media"\n'
        f'access_key = "{S3_ACCESS_KEY}"\n'
        f'secret_key = "{S3_SECRET_KEY}"\n'
        'public_base_url = "http://127.0.0.1:8888/buckets/aspen-media"\n'
    )


def under_server(settings: str) -> str:
    """`settings`, written as aspen.toml's, as aspen-ul.toml's `[server]` holds them: each table's
    header under `server.`, the keys before the first table left where they are, under `[server]`
    itself. Every table header is a line of its own starting with `[`, as in each check's."""
    return re.sub(r"^\[(\[?)\s*", r"[\1server.", settings, flags=re.MULTILINE)


def ul_postgres() -> Path:
    """Where `scripts/package_ul.py --postgres-only` builds this machine's PostgreSQL."""
    # Imported here, since only a run of aspen-ul needs it and what it imports.
    import package_ul
    return package_ul.postgres_prefix(os.uname().machine, package_ul.POSTGRES_VERSION)


def add_arguments(parser) -> None:
    """The arguments `deployment` reads, beside `--bin`."""
    parser.add_argument("--ul", action="store_true",
                        help="run the deployment as Aspen Ultra Lite, aspen-ul from --bin")
    parser.add_argument("--ul-postgres", type=Path,
                        help="the PostgreSQL installation aspen-ul runs (what scripts/package_ul.py "
                             "--postgres-only builds for this machine)")


def deployment(args, name: str, ports: Ports, settings: str = "", public_host: str = "127.0.0.1",
               web_client: Path | None = None, media: str | None = None) -> Deployment:
    """The deployment `args` ask for (`add_arguments`), aspen-ul or the full one, keeping its files
    in `media`: each kind's own way when not given, a directory for aspen-ul and S3 for the other."""
    bins = args.bin.resolve()
    if args.ul:
        return UlStack(bins, name, ports, settings, public_host, web_client, media or "directory",
                       args.ul_postgres)
    return Stack(bins, name, ports, settings, public_host, web_client, media or "s3")


def prepare_services(args, start: bool, media: str | None = None) -> None:
    """Starts (when `start`) and waits for the services of docker-compose.yaml the deployment
    `args` ask for needs: none for aspen-ul keeping its files itself."""
    if args.ul and media in (None, "directory"):
        return
    if start:
        start_services()
    # aspen-ul's database is its own.
    wait_for_services(media or "s3", database=not args.ul)


class Reached:
    """A deployment already running at `base`, spoken to as a `Deployment` speaks to its own."""

    request = Deployment.request
    api = Deployment.api
    status = Deployment.status

    def __init__(self, base: str):
        self._base = base
        self.devices = device_credential.Devices()

    @property
    def base(self) -> str:
        return self._base


def png(width: int = 64, height: int = 48) -> bytes:
    """A picture, of `width` by `height` pixels in a pattern of colours."""
    rows = b"".join(
        b"\x00" + bytes(v for x in range(width) for v in (x * 4 & 0xFF, y * 5 & 0xFF, (x ^ y) & 0xFF))
        for y in range(height)
    )

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def put(url: str, body: bytes, content_type: str) -> None:
    """Uploads `body` to an upload URL the deployment handed out."""
    request = urllib.request.Request(url, data=body, method="PUT", headers={"content-type": content_type})
    urllib.request.urlopen(request, timeout=30).close()


def fetch(url: str) -> tuple[bytes, str]:
    """What `url` serves, and its content type."""
    with urllib.request.urlopen(url, timeout=30) as response:
        return response.read(), response.headers.get("content-type", "")


class EventStream:
    """One identified event stream connection, gathering what it receives."""

    def __init__(self, stack: Deployment, token: str):
        self.socket = WebSocket(f"ws://127.0.0.1:{stack.ports.api}/api/v1/events")
        self.socket.send({"type": "identify", "sessionToken": token})
        ready = self.socket.receive(10)
        if not ready or ready.get("type") != "ready":
            raise Failed(f"the event stream answered {ready}, closed {self.socket.closed}")
        self.events: list[dict] = []
        self.ephemeral: list[dict] = []

    def send(self, message: dict) -> None:
        self.socket.send(message)

    def gather(self, seconds: float = 1.0) -> list[dict]:
        """Every event that arrives within `seconds`, added to `events` and returned. What happens
        and is never kept (`ephemeral` frames: who is typing) is added to `ephemeral`."""
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
            elif frame.get("type") == "ephemeral":
                self.ephemeral.append(frame["event"])
        self.events.extend(got)
        return got

    @property
    def closed(self) -> int | None:
        return self.socket.closed

    def close(self) -> None:
        self.socket.close()
