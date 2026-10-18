"""`aspen-box doctor`: what is wrong with a box, and what to do about it. Most of what goes wrong
with a first deployment is outside the servers (a record that points elsewhere, a port not
forwarded, a certificate not yet issued, storage that serves too much), so it checks each, and
what it cannot see from inside the network it says it has not checked, and how to."""

from __future__ import annotations

import argparse
import ipaddress
import json
import os
import pathlib
import re
import shutil
import socket
import subprocess
import urllib.error
import urllib.parse
import urllib.request

from .common import GIB, HERE_VOICE, SLOTS, Box, Failed, detect_address, memory_bytes
from .database import MINIMUM_POSTGRES, database_url, database_version, external_database
from .lifecycle import ready, voice_answers
from .render import nats_open


class Report:
    def __init__(self):
        self.failed = 0

    def line(self, mark: str, text: str, fix: str | None = None) -> None:
        print(f"  {mark}  {text}")
        if fix:
            print(f"        {fix}")

    def ok(self, text: str) -> None:
        self.line("ok  ", text)

    def note(self, text: str, fix: str | None = None) -> None:
        self.line("note", text, fix)

    def fail(self, text: str, fix: str) -> None:
        self.failed += 1
        self.line("FAIL", text, fix)


def resolve(name: str, family: int) -> set:
    try:
        infos = socket.getaddrinfo(name, 443, socket.AF_INET if family == 4 else socket.AF_INET6, socket.SOCK_STREAM)
    except socket.gaierror:
        return set()
    return {str(ipaddress.ip_address(info[4][0])) for info in infos}


def local_address(family: int) -> str | None:
    """The address this machine sends from, which is its public one only when no router
    translates it. Nothing is sent: a UDP socket only chooses a route."""
    target = ("1.1.1.1", 9) if family == 4 else ("2606:4700:4700::1111", 9)
    try:
        with socket.socket(socket.AF_INET if family == 4 else socket.AF_INET6, socket.SOCK_DGRAM) as probe:
            probe.connect(target)
            return probe.getsockname()[0]
    except OSError:
        return None


def header_of(url: str, name: str, origin: str) -> str | None:
    """The response header `name` of a GET of `url` sent from `origin`, whatever its status."""
    request = urllib.request.Request(url, headers={"Origin": origin})
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.headers.get(name)
    except urllib.error.HTTPError as error:
        return error.headers.get(name)
    except (OSError, ValueError):
        return None


def status_of(url: str, method: str = "GET") -> int | None:
    request = urllib.request.Request(url, method=method, data=b"x" if method == "PUT" else None)
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code
    except (OSError, ValueError):
        return None


def nats_says(host: str, port: int = 4222) -> dict | None:
    """What a NATS server says first to whoever connects (its `INFO`), before any TLS."""
    try:
        with socket.create_connection((host, port), timeout=8) as connection:
            line = connection.makefile("rb").readline(64 * 1024).decode(errors="replace")
    except OSError:
        return None
    return json.loads(line[len("INFO "):]) if line.startswith("INFO ") else None


def data_device(directory: pathlib.Path) -> str:
    """The device the box's data is on, as /proc/mounts names it."""
    best = ("", "")
    for line in pathlib.Path("/proc/mounts").read_text().splitlines():
        device, mount = line.split()[:2]
        if str(directory.resolve()).startswith(mount) and len(mount) > len(best[1]):
            best = (device, mount)
    return best[0]


def unready_reason(port: int) -> str | None:
    """What the API server at `port` says it cannot reach, when it is running and says so."""
    try:
        urllib.request.urlopen(f"http://127.0.0.1:{port}/api/v1/health/ready", timeout=5)
    except urllib.error.HTTPError as error:
        try:
            return json.loads(error.read()).get("detail")
        except ValueError:
            return None
    except OSError:
        return None
    return None


def check_machine(store: Box, report: Report) -> None:
    print("The machine")
    info = subprocess.run(["docker", "info", "--format", "{{json .SecurityOptions}}"], capture_output=True, text=True)
    if info.returncode != 0:
        report.fail("Docker does not answer", "Start Docker, and let this user run it.")
    elif "rootless" in info.stdout:
        report.fail(
            "Docker is running rootless",
            "Its containers are on a network of their own, so nothing would be reachable. Use the system's Docker.",
        )
    else:
        report.ok("Docker")
    compose = subprocess.run(["docker", "compose", "version", "--short"], capture_output=True, text=True)
    version = tuple(int(part) for part in re.findall(r"\d+", compose.stdout)[:2])
    if compose.returncode != 0 or version < (2, 20):
        report.fail(
            f"Docker Compose {compose.stdout.strip() or 'is missing'}",
            "The box needs Compose 2.20 or later, as Docker's own packages install it (the `docker-compose-plugin` package).",
        )
    memory = memory_bytes() / GIB
    if memory < 1.8:
        report.fail(f"{memory:.1f} GiB of memory", "The deployment idles at about 0.7 GiB and needs room to work: use a machine with 2 GiB or more.")
    else:
        report.ok(f"{memory:.1f} GiB of memory")
    free = shutil.disk_usage(store.dir).free / GIB
    (report.ok if free >= 5 else report.note)(f"{free:.0f} GiB free where the data is kept")
    device = data_device(store.dir)
    if "mmcblk" in device:
        report.note(
            f"The data is on an SD card ({device})",
            "SD cards wear out under a database and lose writes when power is cut. Keep the box's directory on an SSD, and run `aspen-box backup` daily.",
        )
    page = os.sysconf("SC_PAGE_SIZE")
    if page != 4096:
        report.note(f"The kernel's pages are {page // 1024} KiB", "The servers are built for it. If a service will not start, `aspen-box logs` says which.")


def check_services(store: Box, box: dict, report: Report) -> None:
    print("\nThe deployment" if box.get("role") != "voice" else "\nThe voice server")
    running = store.running()
    if box.get("role") == "voice":
        wanted = {"caddy", "voice"}
    else:
        slot = box.get("active_slot", "api-a")
        here = any(server["here"] for server in box["voice_servers"])
        wanted = {"valkey", "nats", "caddy", slot} | ({"voice"} if here else set())
        wanted |= {"garage"} if box["storage"]["kind"] == "bundled" else set()
        wanted |= set() if external_database(box) else {"postgres"}
        if external_database(box):
            check_external_database(store, box, report)
    missing = sorted(wanted - running)
    if missing:
        report.fail(f"Not running: {', '.join(missing)}", "`aspen-box up` starts them, and `aspen-box logs <name>` says why one stopped.")
        return
    report.ok("Every service is running")
    if box.get("role") != "voice":
        slot = box.get("active_slot", "api-a")
        if ready(store, slot):
            report.ok("The API server answers, and reaches its database, NATS, and Valkey")
        else:
            report.fail("The API server is not ready", unready_reason(SLOTS[slot]) or f"`aspen-box logs {slot}` says why.")
    if "voice" in wanted:
        if voice_answers(store):
            report.ok("The voice server answers")
        else:
            report.fail(
                "The voice server does not answer",
                "`aspen-box logs voice` says why. It stops when its id is not registered or it cannot reach NATS.",
            )


def check_external_database(store: Box, box: dict, report: Report) -> None:
    url = urllib.parse.urlsplit(database_url(box))
    try:
        version = database_version(store, box)
    except Failed as failure:
        report.fail(f"The database at {url.hostname}: {failure}", "Check it is running, that this machine may reach it, and database.url in box.json.")
    else:
        if version < MINIMUM_POSTGRES:
            report.fail(f"The database at {url.hostname} is PostgreSQL {version}", f"Aspen needs {MINIMUM_POSTGRES} or later.")
        else:
            report.ok(f"The database at {url.hostname} answers, and is PostgreSQL {version}")
    mode = dict(urllib.parse.parse_qsl(url.query)).get("sslmode")
    if mode not in ("verify-full", "verify-ca"):
        report.note(
            f"The database is reached with sslmode={mode}, which does not check who answers",
            "Anything between this machine and it can read every query. Unless that network is yours alone, take sslmode out of database.url, and give database.ca_file if its certificate is from its provider's own authority.",
        )


def names(box: dict) -> list:
    if box.get("role") == "voice":
        return [f"{box['name']}.{box['domain']}"]
    domain = box["domain"]
    here = [f"{HERE_VOICE}.{domain}"] if any(server["here"] for server in box["voice_servers"]) else []
    return [domain, *here, *([f"media.{domain}"] if box["storage"]["kind"] == "bundled" else [])]


def ports(box: dict) -> str:
    rtc = box["rtc"]
    voice = f", UDP 3478, and UDP and TCP {rtc['min_port']}-{rtc['max_port']}"
    if box.get("role") == "voice":
        return f"TCP 80 and 443{voice}"
    here = any(server["here"] for server in box["voice_servers"])
    return f"TCP 80 and 443{', TCP 4222 (NATS, for the voice servers elsewhere)' if nats_open(box) else ''}{voice if here else ''}"


def check_network(store: Box, box: dict, report: Report) -> dict:
    print("\nThe network")
    config = store.dir / "config" / "voice_server.toml"
    announced = []
    if config.exists():
        found = re.search(r"^announced_address = (.*)$", config.read_text(), re.M)
        announced = json.loads(found.group(1)) if found else []
    public = {}
    for family in (4, 6):
        public[family] = detect_address(family)
        mine = [address for address in announced if (":" in address) == (family == 6)]
        if box[f"address{family}"] == "none":
            public[family] = None
            report.note(f"IPv{family} is turned off for this box")
        elif public[family] is None:
            report.note(f"No public IPv{family} address found", None if family == 6 else "The machine did not reach the internet over IPv4, or the service that tells its address.")
        elif announced and public[family] not in mine:
            report.fail(
                f"The voice server announces {', '.join(mine) or 'no IPv' + str(family) + ' address'}, and this machine's is {public[family]}",
                "`aspen-box refresh-address` follows a changed address; run it on a schedule if yours changes.",
            )
        else:
            report.ok(f"IPv{family} address {public[family]}")
    if public[4] and local_address(4) != public[4]:
        report.note(
            f"This machine is behind a router (its own address is {local_address(4)})",
            f"The router must forward to it: {ports(box)}. If the router's own outside address is not the one above, "
            "the provider shares addresses between customers and nothing can be forwarded.",
        )
    dns_ready = True
    for name in names(box):
        for family, kind in ((4, "A"), (6, "AAAA")):
            found = resolve(name, family)
            if not public[family]:
                if found:
                    # Whoever is given it tries an address nothing answers at.
                    report.fail(f"{name} has {kind} {', '.join(sorted(found))}, and this box has no IPv{family} address", "Remove that record at your DNS provider.")
                continue
            if public[family] in found:
                report.ok(f"{name} {kind} {public[family]}")
            else:
                dns_ready = False
                report.fail(
                    f"{name} has {'no ' + kind + ' record' if not found else kind + ' ' + ', '.join(sorted(found))}",
                    f"In your domain's DNS: {name}.  {kind}  {public[family]}. A record just written can take some minutes to be seen.",
                )
    return {"dns_ready": dns_ready}


def check_from_outside(box: dict, report: Report, network: dict) -> None:
    print("\nFrom the internet")
    if not network["dns_ready"]:
        report.note("Not checked until the names above point here")
        return
    if box.get("role") == "voice":
        checks = [(f"{box['name']}.{box['domain']}", "/health", 204)]
    else:
        checks = [(box["domain"], "/api/v1/health", 204)]
        if any(server["here"] for server in box["voice_servers"]):
            checks.append((f"{HERE_VOICE}.{box['domain']}", "/health", 204))
    for name, path, want in checks:
        got = status_of(f"https://{name}{path}")
        if got == want:
            report.ok(f"https://{name} answers, with a certificate browsers accept")
        else:
            report.fail(
                f"https://{name}{path} {'did not answer' if got is None else 'answered ' + str(got)}",
                "Caddy gets its certificate once ports 80 and 443 reach this machine: check the router and any firewall, then `aspen-box logs caddy`. "
                "A router that cannot reach its own outside address makes this fail from inside the network alone: try from a phone off the Wi-Fi.",
            )
    if box.get("role") == "voice":
        host = urllib.parse.urlsplit(box["join"]["nats_url"]).hostname
        said = nats_says(host)
        if said is None:
            report.fail(
                f"NATS at {host}:4222 does not answer",
                "The deployment's own box must forward TCP 4222 to its machine; `aspen-box doctor` there says whether NATS is open.",
            )
        elif not said.get("tls_required"):
            report.fail(f"NATS at {host}:4222 answers without TLS", "Run `aspen-box up` on the deployment's own box, which opens NATS over TLS.")
        else:
            report.ok(f"NATS at {host}:4222 answers, over TLS")
    else:
        check_read_path(box, report)
    report.note(
        "Not checked: whether the internet reaches the voice server's media ports",
        "Join a call from a phone off the Wi-Fi. If it connects and no one is heard, those ports are not forwarded.",
    )


def check_read_path(box: dict, report: Report) -> None:
    base = f"https://media.{box['domain']}" if box["storage"]["kind"] == "bundled" else box["storage"]["public_base_url"].rstrip("/")
    # docs/operators/installing/storage-read-path.md: what the read path must refuse.
    refusals = (
        ("lists the bucket", "GET", "/?list-type=2", "Anyone can see every file ever posted."),
        ("takes writes", "PUT", "/attachments/aspen-box-write-check", "Anyone can put files at your media address."),
        ("serves deleted messages' files", "GET", "/evidence/aspen-box-read-check", None),
        ("serves unconfirmed uploads", "GET", "/uploads/aspen-box-read-check", None),
    )
    for what, method, path, consequence in refusals:
        got = status_of(base + path, method)
        if got is None:
            report.fail(f"{base} did not answer", "The read path must be reachable by everyone's apps.")
            return
        if got in (200, 201, 204):
            report.fail(f"The read path {what}", (consequence + " " if consequence else "") + "See docs/operators/installing/storage-read-path.md.")
            return
    report.ok("The read path refuses listing, writes, and what it must not serve")
    allowed = header_of(base + "/attachments/aspen-box-cors-check", "Access-Control-Allow-Origin", f"https://{box['domain']}")
    if allowed not in ("*", f"https://{box['domain']}"):
        report.fail(
            "The read path does not let the apps read files",
            "Saving or copying a picture will fail. It must send Access-Control-Allow-Origin; see docs/operators/installing/storage-read-path.md.",
        )
    else:
        report.ok("The read path lets the apps read files")


def doctor(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    report = Report()
    check_machine(store, report)
    check_services(store, box, report)
    network = check_network(store, box, report)
    check_from_outside(box, report, network)
    print()
    if report.failed:
        raise Failed(f"{report.failed} thing{'s' if report.failed > 1 else ''} to fix, above")
    print("Nothing to fix.")
