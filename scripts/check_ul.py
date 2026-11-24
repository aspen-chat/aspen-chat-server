#!/usr/bin/env python3
"""Checks Aspen Ultra Lite as an operator runs it, the way docs/operators/ultra-lite/ says: a
package scripts/package_ul.py made, installed on a Debian 12 machine under systemd (a container
of its own, `--privileged`, so systemd can run in it), and then:

- `install`: the user, the data directory (theirs, 0750), the binary, the unit enabled;
- `init` with a certificate in files (from scripts/dev_ca.py), and started: it serves over HTTPS
  at its domain and its files' domain, answers plain HTTP with a redirect there, and refuses the
  API over plain HTTP;
- a person registers, signs in, and posts a message with a file;
- restarted by systemd, and then killed and started again by it, what was posted and the session
  are still there, PostgreSQL having recovered on its own;
- backed up while it runs, a message posted after, and the backup restored with the deployment
  stopped: the earlier message and its file are there and the later one is not;
- upgraded by installing a build carrying another runtime over it, as a new release is: the new
  runtime serves, the previous is kept, and everything is still there;
- exported with the deployment stopped (docs/operators/ultra-lite/leaving.md), the export saying
  what it holds, and started again afterwards.

    scripts/check_ul.py --ul target/ul/x86_64/aspen-ul

The deployment is `chat.aspen-ul.test`, its files at `media.chat.aspen-ul.test`, which this
process alone resolves to the container: its ports 443 and 80 are published on this machine's
loopback address at free ports, and connections to either name at either port go there. Needs Python 3.10, Docker, `zstd`, and the
`openssl` command dev_ca.py issues the certificate with. A run takes a few minutes; the container
is removed however it ends unless `--keep` is given.
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import io
import json
import os
import socket
import ssl
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dev_ca  # noqa: E402
from package_ul import MAGIC  # noqa: E402
from stack import Failed, Reached, fetch, png, put, wait_for  # noqa: E402

DOMAIN = "chat.aspen-ul.test"
MEDIA = f"media.{DOMAIN}"
IMAGE = "aspen-ul-check:bookworm"
# A Debian 12 machine booting systemd, with what `install` and the commands need of it.
DOCKERFILE = """FROM debian:bookworm
RUN apt-get update -qq \\
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \\
      systemd systemd-sysv libssl3 ca-certificates >/dev/null \\
 && rm -rf /var/lib/apt/lists/*
ENV container=docker
STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
"""
DATA = "/var/lib/aspen-ul"
BACKUPS = "/var/backups/aspen-ul"
PASSWORD = "ultra-lite-check-password"
# What the upgrade's runtime carries that the first's does not, served from the web client.
UPGRADE_MARK = "upgrade-check.txt"


def say(message: str) -> None:
    print(f"check_ul: {message}", flush=True)


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def resolve_to(published: dict[int, int]) -> None:
    """Has this process find the deployment's names on this machine's loopback address, each of
    their ports at the one `published` maps it to, as DNS would point them at the machine."""
    real = socket.getaddrinfo

    def getaddrinfo(host, port, *rest, **named):
        if host in (DOMAIN, MEDIA):
            return real("127.0.0.1", published[int(port)], *rest, **named)
        return real(host, port, *rest, **named)

    socket.getaddrinfo = getaddrinfo


def with_other_runtime(package: Path, out: Path) -> None:
    """`package` carrying another runtime, its web client holding one file more, as a release
    carrying a new web client does: what `server/ul/src/bundle.rs` reads after the binary is
    `package_ul.py`'s archive, its SHA-256, its length, and the magic."""
    whole = package.read_bytes()
    if whole[-len(MAGIC):] != MAGIC:
        raise Failed(f"{package} carries no runtime; give a package scripts/package_ul.py made")
    (length,) = struct.unpack("<Q", whole[-len(MAGIC) - 8:-len(MAGIC)])
    trailer = 32 + 8 + len(MAGIC)
    binary, archive = whole[:-trailer - length], whole[-trailer - length:-trailer]
    plain = io.BytesIO(subprocess.run(["zstd", "-q", "-d", "-c"], input=archive, capture_output=True,
                                      check=True).stdout)
    mark = f"the runtime of {out.name}\n".encode()
    with tarfile.open(fileobj=plain, mode="a", format=tarfile.PAX_FORMAT) as tar:
        info = tarfile.TarInfo(f"web-client/{UPGRADE_MARK}")
        info.size, info.mode = len(mark), 0o644
        tar.addfile(info, io.BytesIO(mark))
    packed = subprocess.run(["zstd", "-q", "-3", "--long=27", "-T0", "-c"], input=plain.getvalue(),
                            capture_output=True, check=True).stdout
    out.write_bytes(binary + packed + hashlib.sha256(packed).digest() + struct.pack("<Q", len(packed)) + MAGIC)
    out.chmod(0o755)


class Machine:
    """The container standing in for the operator's machine."""

    def __init__(self, keep: bool):
        self.name = f"aspen-ul-check-{os.getpid()}"
        self.keep = keep
        # The machine's HTTPS and plain HTTP ports, published at these.
        self.published = {443: free_port(), 80: free_port()}

    def __enter__(self) -> "Machine":
        built = subprocess.run(["docker", "build", "-q", "-t", IMAGE, "-"], input=DOCKERFILE, text=True,
                               capture_output=True)
        if built.returncode != 0:
            raise Failed(f"the machine's image did not build: {built.stderr[-2000:]}")
        ports = [arg for inner, outer in self.published.items() for arg in ("-p", f"127.0.0.1:{outer}:{inner}")]
        subprocess.run(["docker", "run", "-d", "-t", "--privileged", "--name", self.name, "--hostname", "aspen-ul",
                        *ports, IMAGE], check=True, capture_output=True)
        wait_for("systemd to boot the machine",
                 lambda: self.run("systemctl", "is-system-running", check=False).stdout.strip() in ("running", "degraded"),
                 60)
        return self

    def __exit__(self, kind, error, trace) -> None:
        if error is not None:
            journal = self.run("journalctl", "-u", "aspen-ul", "--no-pager", "-n", "150", check=False)
            print(f"----- aspen-ul's journal -----\n{journal.stdout}", flush=True)
        if self.keep:
            say(f"kept the machine, {self.name}")
        else:
            subprocess.run(["docker", "rm", "-f", self.name], capture_output=True)

    def run(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        """Runs `args` on the machine as root."""
        done = subprocess.run(["docker", "exec", self.name, *args], capture_output=True, text=True)
        if check and done.returncode != 0:
            raise Failed(f"`{' '.join(args)}` failed ({done.returncode}): {done.stdout[-1500:]}{done.stderr[-1500:]}")
        return done

    def copy(self, source: Path, target: str) -> None:
        subprocess.run(["docker", "cp", str(source), f"{self.name}:{target}"], check=True, capture_output=True)


class Check:
    def __init__(self, machine: Machine):
        self.machine = machine
        self.client = Reached(f"https://{DOMAIN}")
        self.ul = ["aspen-ul"]

    def serving(self) -> None:
        """Waits for the deployment to answer as systemd keeps it."""
        def up() -> bool:
            state = self.machine.run("systemctl", "is-active", "aspen-ul", check=False).stdout.strip()
            if state == "failed":
                raise Failed("systemd gave aspen-ul up")
            return self.client.status("GET", "/health") == 204
        wait_for("aspen-ul to serve", up, 180)

    def stat(self, path: str) -> str:
        return self.machine.run("stat", "-c", "%U:%G %a", path).stdout.strip()

    def install(self, package: Path) -> None:
        say("installing it as the guide says: sudo ./aspen-ul install")
        self.machine.copy(package, "/root/aspen-ul")
        self.machine.run("/root/aspen-ul", "install")
        if (owner := self.stat(DATA)) != "aspen-ul:aspen-ul 750":
            raise Failed(f"{DATA} is {owner}, not aspen-ul's alone")
        if self.machine.run("systemctl", "is-enabled", "aspen-ul", check=False).stdout.strip() != "enabled":
            raise Failed("aspen-ul is not started at boot")
        if (owner := self.stat("/usr/local/bin/aspen-ul")) != "root:root 755":
            raise Failed(f"/usr/local/bin/aspen-ul is {owner}")

    def init(self) -> None:
        say(f"describing the deployment, {DOMAIN}, with a certificate in files, and starting it")
        cert, key = dev_ca.issue(DOMAIN, MEDIA)
        self.machine.run("install", "-d", "-m", "750", "-o", "root", "-g", "aspen-ul", "/etc/aspen-ul")
        for source, name, mode in ((cert, "cert.pem", "640"), (key, "key.pem", "640")):
            self.machine.copy(source, f"/etc/aspen-ul/{name}")
            self.machine.run("chown", "root:aspen-ul", f"/etc/aspen-ul/{name}")
            self.machine.run("chmod", mode, f"/etc/aspen-ul/{name}")
        self.machine.run(*self.ul, "init", "--domain", DOMAIN, "--tls", "files", "--cert", "/etc/aspen-ul/cert.pem",
                         "--key", "/etc/aspen-ul/key.pem", "--yes")
        self.machine.run("systemctl", "start", "aspen-ul")
        self.serving()

    def plain_http(self) -> None:
        say("plain HTTP is answered with the deployment's address, and the API refused over it")
        for host, path, location in ((DOMAIN, "/invite/x?y=1", f"https://{DOMAIN}/invite/x?y=1"),
                                     (MEDIA, "/attachments/a", f"https://{MEDIA}/attachments/a"),
                                     ("elsewhere.test", "/", f"https://{DOMAIN}/")):
            status, headers = self.plain(host, path)
            if status != 308 or headers.get("location") != location:
                raise Failed(f"http://{host}{path} answered {status} {headers.get('location')}, not 308 {location}")
        status, headers = self.plain(DOMAIN, "/api/v1/health")
        if status != 403 or "problem+json" not in headers.get("content-type", ""):
            raise Failed(f"the API over plain HTTP answered {status} {headers}, not a refusal")

    def plain(self, host: str, path: str) -> tuple[int, dict]:
        connection = http.client.HTTPConnection("127.0.0.1", self.machine.published[80], timeout=10)
        try:
            connection.request("GET", path, headers={"host": host})
            response = connection.getresponse()
            response.read()
            return response.status, {k.lower(): v for k, v in response.getheaders()}
        finally:
            connection.close()

    def seed(self) -> dict:
        say("a person registers, signs in, and posts a message with a file")
        self.client.api("POST", "/users", {"name": "operator", "password": PASSWORD})
        token = self.client.api("POST", "/auth/login", {"username": "operator", "password": PASSWORD})["sessionToken"]
        made = self.client.api("POST", "/communities", {"name": "Ultra Lite"}, token)
        community = made.get("id") or made["data"]["id"]
        channels = self.client.api("GET", f"/communities/{community}/channels", token=token)["data"]
        channel = next(c for c in channels if c["ty"] == "text")["id"]
        picture = png()
        handle = self.client.api("POST", "/attachments",
                                 {"fileName": "shot.png", "mimeType": "image/png", "byteSize": len(picture)}, token)
        if not handle["uploadUrl"].startswith(f"https://{MEDIA}/"):
            raise Failed(f"the upload goes to {handle['uploadUrl']}, not the files' domain")
        put(handle["uploadUrl"], picture, "image/png")
        self.client.api("POST", f"/attachments/{handle['id']}/confirm", token=token)
        message = self.client.api("POST", f"/channels/{channel}/messages",
                                  {"content": "kept through everything", "attachments": [handle["id"]]}, token)
        state = {"token": token, "channel": channel, "message": message["id"], "picture": picture,
                 "posted": {message["id"]}}
        # The message is held until its picture's preview is made.
        wait_for("the message to be posted", lambda: self.messages(state) == [message["id"]], 60)
        self.still_there(state, [message["id"]])
        return state

    def messages(self, state: dict) -> list[str]:
        """Which of the messages this check posted the channel holds."""
        window = self.client.api("GET", f"/channels/{state['channel']}/messages?limit=50", token=state["token"])
        return sorted(m["id"] for m in window["data"] if m["id"] in state["posted"])

    def still_there(self, state: dict, messages: list[str]) -> None:
        """The session signs in, the channel holds `messages` of those this check posted and no
        other, and the first message's file is served from the files' domain as it was uploaded."""
        if (status := self.client.status("GET", "/users/@me", token=state["token"])) != 200:
            raise Failed(f"the session from before answered {status}")
        if (found := self.messages(state)) != sorted(messages):
            raise Failed(f"the channel holds {found}, not {sorted(messages)}")
        window = self.client.api("GET", f"/channels/{state['channel']}/messages?around={state['message']}&include=attachments",
                                 token=state["token"])
        (attachment,) = window["included"]["attachments"]
        if not attachment["downloadUrl"].startswith(f"https://{MEDIA}/"):
            raise Failed(f"the file is served at {attachment['downloadUrl']}, not the files' domain")
        body, kind = fetch(attachment["downloadUrl"])
        if body != state["picture"] or kind.split(";")[0] != "image/png":
            raise Failed(f"the file came back as {kind}, {len(body)} bytes")

    def restarts(self, state: dict) -> None:
        say("restarted by systemd, everything is still there")
        self.machine.run("systemctl", "restart", "aspen-ul")
        self.serving()
        self.still_there(state, [state["message"]])
        say("killed, started again by systemd, PostgreSQL recovers and everything is still there")
        before = int(self.machine.run("systemctl", "show", "-P", "NRestarts", "aspen-ul").stdout)
        self.machine.run("systemctl", "kill", "--kill-whom=main", "--signal=SIGKILL", "aspen-ul")
        wait_for("systemd to start aspen-ul again",
                 lambda: int(self.machine.run("systemctl", "show", "-P", "NRestarts", "aspen-ul").stdout) > before, 60)
        self.serving()
        self.still_there(state, [state["message"]])

    def backup_and_restore(self, state: dict) -> None:
        say("backed up while it runs, as the guide says")
        self.machine.run("install", "-d", "-o", "aspen-ul", "-g", "aspen-ul", "-m", "700", BACKUPS)
        backup = f"{BACKUPS}/check"
        self.machine.run(*self.ul, "backup", backup)
        # Written last, so a directory holding it is a whole backup.
        self.machine.run("test", "-s", f"{backup}/backup.json")
        if (owner := self.stat(backup)) != "aspen-ul:aspen-ul 700":
            raise Failed(f"the backup is {owner}, not aspen-ul's alone")
        later = self.client.api("POST", f"/channels/{state['channel']}/messages",
                                {"content": "after the backup", "attachments": []}, state["token"])["id"]
        state["posted"].add(later)
        self.still_there(state, [state["message"], later])
        say("restoring it with the deployment stopped")
        self.machine.run("systemctl", "stop", "aspen-ul")
        self.machine.run(*self.ul, "restore", backup)
        replaced = self.machine.run("sh", "-c", f"ls -d {DATA}/replaced-*", check=False).stdout.split()
        if len(replaced) != 1:
            raise Failed(f"restoring kept {replaced}, not what it replaced")
        self.machine.run("systemctl", "start", "aspen-ul")
        self.serving()
        self.still_there(state, [state["message"]])

    def upgrade(self, package: Path, work: Path, state: dict) -> None:
        say("upgraded by installing a release carrying another runtime over it")
        current = self.machine.run("cat", f"{DATA}/runtime/current").stdout.strip()
        upgraded = work / "aspen-ul-upgraded"
        with_other_runtime(package, upgraded)
        self.machine.copy(upgraded, "/root/aspen-ul")
        self.machine.run("/root/aspen-ul", "install")
        self.serving()
        now = self.machine.run("cat", f"{DATA}/runtime/current").stdout.strip()
        kept = self.machine.run("ls", f"{DATA}/runtime").stdout.split()
        if now == current or sorted(kept) != sorted([current, now, "current"]):
            raise Failed(f"the runtime went from {current} to {now}, keeping {kept}")
        body, _ = fetch(f"https://{DOMAIN}/{UPGRADE_MARK}")
        if body != f"the runtime of {upgraded.name}\n".encode():
            raise Failed(f"the upgraded runtime's web client is not served: {body[:200]!r}")
        self.still_there(state, [state["message"]])

    def export(self, state: dict) -> None:
        say("exported with the deployment stopped, as leaving.md says, and started again")
        self.machine.run("systemctl", "stop", "aspen-ul")
        export = "/var/backups/aspen-ul/export"
        self.machine.run(*self.ul, "export", export)
        manifest = json.loads(self.machine.run("cat", f"{export}/export.json").stdout)
        if manifest.get("publicUrl") != f"https://{DOMAIN}" or manifest.get("voiceServer", {}).get("name") != "aspen-ul":
            raise Failed(f"the export says {manifest}")
        files = self.machine.run("find", f"{export}/media", "-type", "f").stdout.split()
        if not files:
            raise Failed("the export holds no files")
        self.machine.run("systemctl", "start", "aspen-ul")
        self.serving()
        self.still_there(state, [state["message"]])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ul", type=Path, required=True, help="a package scripts/package_ul.py made")
    parser.add_argument("--keep", action="store_true", help="keep the machine afterwards, for looking into it")
    args = parser.parse_args()
    package = args.ul.resolve()
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="aspen-ul-check-") as work, Machine(args.keep) as machine:
            resolve_to(machine.published)
            # The deployment's certificate is the development authority's, which this process trusts.
            urllib.request.install_opener(urllib.request.build_opener(
                urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(dev_ca.authority())))))
            check = Check(machine)
            check.install(package)
            check.init()
            check.plain_http()
            state = check.seed()
            check.restarts(state)
            check.backup_and_restore(state)
            check.upgrade(package, Path(work), state)
            check.export(state)
    except (Failed, subprocess.CalledProcessError) as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)
    say(f"passed in {time.monotonic() - started:.0f} s")


if __name__ == "__main__":
    main()
