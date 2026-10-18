#!/usr/bin/env python3
"""Checks deploy/box against built images, on this machine:

    scripts/check_box.py --api-image aspen-api:check --voice-image aspen-voice:check

It runs, each in a network of its own that stands in for its machine's (a container every
service of the box shares, so it works under rootless Docker too, and leaves the machine's own
ports alone):

- a whole deployment, files and database on the machine: the API, the web client, and the voice
  server through Caddy; the read path's refusals; the build the event stream names; an upgrade
  under load that loses no request; a changed setting taking effect without a gap; a backup and
  a restore, with a session from before it still good; an export of the database and people's
  files, which stops it, and `up`, which starts it again;
- a second box holding a voice server of that deployment's, on "another machine": the
  deployment opens NATS over TLS for it, registers it, and gives it a NATS user of its own,
  which it reaches the deployment with; then its removal, which closes NATS again;
- a deployment whose database is elsewhere, over TLS from an authority of its own, and one that
  is too old;
- the tool itself on the oldest Python it supports.

Everything it starts is named check-box-*, and removed however it ends unless --keep is given.
Needs Python 3.10, Docker with Compose 2.20, and openssl.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BOX_TOOL = REPO / "deploy" / "box" / "aspen-box"
NETWORK = "check-box"
DOMAIN = "chat.localhost"
CURL = "curlimages/curl:8.11.1"
PYTHON = "python:3.12-slim"
OLDEST_PYTHON = "python:3.9-slim"
WORK = REPO / "target" / "check-box"


class Failed(Exception):
    pass


def say(message: str) -> None:
    print(f"check-box: {message}", flush=True)


def run(*command: str, check: bool = True, **how) -> subprocess.CompletedProcess:
    done = subprocess.run(command, text=True, capture_output=True, **how)
    if check and done.returncode != 0:
        raise Failed(f"`{' '.join(command[:6])}` failed:\n{done.stdout[-1500:]}\n{done.stderr[-1500:]}")
    return done


class Box:
    """One box, in a directory of its own under target/check-box, as Compose project `name`,
    whose services share the network of one container, known on the check's network by
    `aliases`."""

    def __init__(self, name: str, aliases: list[str]):
        self.name = name
        self.dir = WORK / name
        self.netns = f"{name}-netns-1"
        self.override = WORK / f"{name}.compose.yaml"
        services = ["postgres", "valkey", "nats", "garage", "caddy", "api-a", "api-b", "voice", "tool", "files", "extpg", "oldpg"]
        lines = [
            "services:",
            "  netns:",
            "    image: alpine:3",
            "    command: [\"sleep\", \"infinity\"]",
            f"    networks: {{ check: {{ aliases: {json.dumps(aliases)} }} }}",
        ]
        lines += [f"  {service}: {{ network_mode: \"service:netns\" }}" for service in services if service not in ("extpg", "oldpg")]
        lines += [
            "  extpg:",
            "    image: postgres:18-alpine",
            "    profiles: [\"outside-database\"]",
            "    network_mode: \"service:netns\"",
            "    environment: { POSTGRES_USER: outside, POSTGRES_PASSWORD: outsidepw, POSTGRES_DB: aspen }",
            f"    volumes: [\"{WORK / 'certs'}:/certs:ro\"]",
            "    entrypoint: [\"sh\", \"-c\", \"cp /certs/server.crt /certs/server.key /tmp/ && chown postgres /tmp/server.* && chmod 600 /tmp/server.key && exec docker-entrypoint.sh postgres -c port=5433 -c ssl=on -c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key\"]",
            "  oldpg:",
            "    image: postgres:17-alpine",
            "    profiles: [\"outside-database\"]",
            "    network_mode: \"service:netns\"",
            "    environment: { POSTGRES_USER: outside, POSTGRES_PASSWORD: outsidepw, POSTGRES_DB: aspen }",
            "    command: [\"postgres\", \"-c\", \"port=5434\"]",
            "networks:",
            f"  check: {{ name: {NETWORK}, external: true }}",
        ]
        self.override.write_text("\n".join(lines) + "\n")

    def env(self) -> dict:
        return {**os.environ, "ASPEN_BOX_DIR": str(self.dir), "ASPEN_BOX_PROJECT": self.name,
                "ASPEN_BOX_COMPOSE_FILES": str(self.override)}  # fmt: skip

    def tool(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        return run(sys.executable, str(BOX_TOOL), *args, check=check, env=self.env())

    def compose(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        return run("docker", "compose", "--project-name", self.name, "--project-directory", str(self.dir),
                   "--env-file", str(self.dir / ".env"), "-f", str(REPO / "deploy" / "box" / "compose.yaml"),
                   "-f", str(self.override), *args, check=check)  # fmt: skip

    def json(self) -> dict:
        return json.loads((self.dir / "box.json").read_text())

    def edit(self, change) -> None:
        box = self.json()
        change(box)
        (self.dir / "box.json").write_text(json.dumps(box, indent=2) + "\n")

    def curl(self, *args: str, resolve: tuple = (DOMAIN,)) -> str:
        names = [flag for name in resolve for flag in ("--resolve", f"{name}:443:127.0.0.1")]
        return run("docker", "run", "--rm", "--network", f"container:{self.netns}", CURL, "-sk", "-m", "10",
                   *names, *args, check=False).stdout  # fmt: skip

    def status(self, url: str, *args: str, resolve: tuple = (DOMAIN,)) -> int:
        return int(self.curl("-o", "/dev/null", "-w", "%{http_code}", *args, url, resolve=resolve) or 0)

    def logs(self, service: str) -> str:
        done = self.compose("logs", "--no-color", service, check=False)
        return done.stdout + done.stderr

    def down(self) -> None:
        self.compose("--profile", "main", "--profile", "api", "--profile", "bundled-storage", "--profile",
                     "bundled-database", "--profile", "tool", "--profile", "outside-database", "down", "-v",
                     "--remove-orphans", check=False)  # fmt: skip


def expect(what: str, got, want) -> None:
    if got != want:
        raise Failed(f"{what}: {got!r}, not {want!r}")
    say(f"  ok: {what}")


def sign_in(box: Box, name: str) -> str:
    body = json.dumps({"name": name, "password": "a box of one's own 2026"})
    expect(f"registering {name}", box.status(f"https://{DOMAIN}/api/v1/users", "-X", "POST", "-H", "content-type: application/json", "-d", body), 201)
    login = json.dumps({"username": name, "password": "a box of one's own 2026"})
    answer = box.curl("-X", "POST", "-H", "content-type: application/json", "-d", login, f"https://{DOMAIN}/api/v1/auth/login")
    return json.loads(answer)["sessionToken"]


def ready_frame(box: Box, token: str) -> dict:
    """The event stream's `ready` frame, asked of the running API server directly."""
    slot_port = 8080 if box.json()["active_slot"] == "api-a" else 8081
    script = (
        "import json, sys; sys.path.insert(0, '/scripts'); from stack import WebSocket\n"
        f"socket = WebSocket('ws://127.0.0.1:{slot_port}/api/v1/events')\n"
        f"socket.send({{'type': 'identify', 'sessionToken': {token!r}}})\n"
        "print(json.dumps(socket.receive(10)))\n"
    )
    done = run("docker", "run", "--rm", "--network", f"container:{box.netns}", "-v", f"{REPO / 'scripts'}:/scripts:ro",
               PYTHON, "python", "-c", script)  # fmt: skip
    return json.loads(done.stdout.strip().splitlines()[-1])


def check_oldest_python() -> None:
    say(f"the tool on {OLDEST_PYTHON}")
    script = (
        "import sys; sys.path.insert(0, '/box'); import compileall\n"
        "assert compileall.compile_dir('/box/aspen_box', quiet=1, legacy=True)\n"
        "import aspen_box.cli; aspen_box.cli.arguments().parse_args(['up'])\n"
    )
    run("docker", "run", "--rm", "-v", f"{REPO / 'deploy'}:/deploy:ro", "-e", "PYTHONDONTWRITEBYTECODE=1", OLDEST_PYTHON,
        "sh", "-c", f"cp -r /deploy/box /box && python -c \"{script}\"")  # fmt: skip
    say("  ok: it compiles and reads its arguments")


def check_main(images: dict) -> Box:
    say("a whole deployment on one machine")
    box = Box("check-box-main", [DOMAIN])
    box.tool("init", "--domain", DOMAIN, "--address4", "203.0.113.10", "--address6", "none", "--acme-email", "-",
             "--storage", "machine", "--api-image", images["api"], "--voice-image", images["voice"])  # fmt: skip
    box.tool("up")
    resolve = (DOMAIN, f"voice.{DOMAIN}", f"media.{DOMAIN}")
    expect("the API is up", box.status(f"https://{DOMAIN}/api/v1/health"), 204)
    expect("the API is ready", box.status(f"https://{DOMAIN}/api/v1/health/ready"), 204)
    expect("the web client is served", box.status(f"https://{DOMAIN}/"), 200)
    expect("the voice server answers", box.status(f"https://voice.{DOMAIN}/health", resolve=resolve), 204)
    media = f"https://media.{DOMAIN}"
    for path, method in (("/", "GET"), ("/?list-type=2", "GET"), ("/evidence/x", "GET"), ("/attachments/a/b", "GET"), ("/attachments/w", "PUT")):
        expect(f"the read path refuses {method} {path}", box.status(media + path, "-X", method, resolve=resolve) in (403, 404, 405), True)
    expect("the S3 API refuses an unsigned read", box.status(f"{media}/aspen-media/evidence/x", resolve=resolve), 403)
    allowed = box.curl("-D", "-", "-o", "/dev/null", "-H", f"Origin: https://{DOMAIN}", f"{media}/attachments/x", resolve=resolve)
    expect("the read path lets any origin read", "access-control-allow-origin: *" in allowed.lower(), True)
    token = sign_in(box, "boxer")
    page = box.curl(f"https://{DOMAIN}/")
    build = page.split('name="aspen-web-client-build" content="', 1)[1].split('"', 1)[0]
    expect("the event stream names the build the page is", ready_frame(box, token).get("webClientBuild"), build)

    say("an upgrade under load")
    for target in ("api", "voice"):
        run("docker", "tag", images[target], f"{images[target]}-next")
    poll = f"{box.name}-poll"
    run("docker", "run", "-d", "--name", poll, "--network", f"container:{box.netns}", "--entrypoint", "sh", CURL, "-c",
        f"while true; do curl -sk -m 5 -o /dev/null -w '%{{http_code}}\\n' --resolve {DOMAIN}:443:127.0.0.1 https://{DOMAIN}/api/v1/health; sleep 0.1; done")  # fmt: skip
    try:
        time.sleep(2)
        box.tool("upgrade", "--api-image", f"{images['api']}-next", "--voice-image", f"{images['voice']}-next")
        time.sleep(2)
        answers = run("docker", "logs", poll).stdout.split()
    finally:
        run("docker", "rm", "-f", poll, check=False)
    expect(f"every one of {len(answers)} requests during the upgrade answered", sorted(set(answers)), ["204"])
    expect("the API moved to the other place", box.json()["active_slot"], "api-b")

    say("a changed setting")
    box.edit(lambda b: b.update(email={"smtp_url": "smtps://mail:pw@smtp.example.org", "from": "Box <box@chat.localhost>"}))
    box.tool("up")
    expect("it took effect", "[email]" in (box.dir / "config" / "aspen.toml").read_text(), True)
    expect("by moving the API again", box.json()["active_slot"], "api-a")
    expect("and the API answers", box.status(f"https://{DOMAIN}/api/v1/health/ready"), 204)

    say("a backup and a restore")
    box.tool("backup")
    me = f"https://{DOMAIN}/api/v1/users/@me"
    made = box.status(f"https://{DOMAIN}/api/v1/communities", "-X", "POST", "-H", "content-type: application/json",
                      "-H", f"authorization: Bearer {token}", "-d", json.dumps({"name": "After the backup"}))  # fmt: skip
    expect("a community made after the backup", made, 201)
    dump = sorted((box.dir / "backups").glob("*.dump"))[-1]
    box.tool("restore", "--yes", str(dump))
    expect("the session from before the backup still signs in", box.status(me, "-H", f"authorization: Bearer {token}"), 200)
    communities = json.loads(box.curl("-H", f"authorization: Bearer {token}", f"https://{DOMAIN}/api/v1/users/@me/communities"))
    expect("the community made after it is gone", len(communities["data"]), 0)
    return box


def check_voice_elsewhere(main: Box, images: dict) -> None:
    say("a voice server on another machine")
    main.tool("voice", "add", "voice-2", "--capacity", "30")
    join = main.dir / "voice-voice-2.join"
    expect("its join file is the owner's alone", oct(join.stat().st_mode & 0o777), "0o600")
    expect("NATS listens beyond loopback", '"0.0.0.0:4222"' in (main.dir / "config" / "nats.conf").read_text(), True)
    expect("the API is ready again, reaching NATS over TLS", main.status(f"https://{DOMAIN}/api/v1/health/ready"), 204)
    voice = Box("check-box-voice", [f"voice-2.{DOMAIN}"])
    voice.tool("init-voice", "--join", str(join), "--address4", "203.0.113.20", "--address6", "none", "--acme-email", "-",
               "--voice-image", images["voice"])  # fmt: skip
    try:
        voice.tool("up")
        expect("it answers through its own Caddy", voice.status(f"https://voice-2.{DOMAIN}/health", resolve=(f"voice-2.{DOMAIN}",)), 204)
        expect("it took the join token key over NATS", "holding the join token key" in voice.logs("voice"), True)
        listed = main.tool("run", "--", "aspen-chat-server", "voice-servers", "list").stdout
        expect("the deployment lists it", "voice-2" in listed and f"https://voice-2.{DOMAIN}" in listed, True)
        expect("NATS refused it nothing", "Permissions Violation" in main.logs("nats"), False)
    finally:
        voice.down()
    say("its removal")
    main.tool("voice", "remove", "voice-2")
    expect("NATS keeps to loopback again", '"127.0.0.1:4222"' in (main.dir / "config" / "nats.conf").read_text(), True)
    expect("the API is ready", main.status(f"https://{DOMAIN}/api/v1/health/ready"), 204)


def check_export(box: Box) -> None:
    say("an export, for a deployment that replaces the box")
    secret = box.json()["secrets"]
    stored = "kept through the export"
    run("docker", "run", "--rm", "--network", f"container:{box.netns}", CURL, "-sf", "-X", "PUT", "--data-binary", stored,
        "--aws-sigv4", "aws:amz:garage:s3", "--user", f"{secret['s3_access_key']}:{secret['s3_secret_key']}",
        "http://127.0.0.1:3900/aspen-media/attachments/check/file.txt")  # fmt: skip
    out = WORK / "export"
    box.tool("export", "--yes", str(out))
    expect("the deployment is stopped", box.status(f"https://{DOMAIN}/api/v1/health") == 204, False)
    expect("the database is dumped", (out / "database.dump").stat().st_size > 0, True)
    expect("people's files are copied", (out / "files" / "attachments" / "check" / "file.txt").read_text(), stored)
    expect("it names the voice servers to unregister", json.loads((out / "export.json").read_text())["voice_servers"], ["voice"])
    expect("a second export refuses the same directory", box.tool("export", "--yes", str(out), check=False).returncode, 1)
    restore_as_the_fleet_does(out / "database.dump")
    box.tool("up")
    expect("`up` starts it again", box.status(f"https://{DOMAIN}/api/v1/health/ready"), 204)


def restore_as_the_fleet_does(dump: Path) -> None:
    """Restores an export's dump into an empty PostgreSQL with the options of the larger AWS
    configuration's restore task (deploy/aws/migration.tf), as a deployment the box moves to
    would."""
    options = re.search(r'restore_command = (\[.*\])', (REPO / "deploy" / "aws" / "migration.tf").read_text()).group(1)
    name = "check-box-restore"
    run("docker", "run", "-d", "--name", name, "-e", "POSTGRES_USER=aspen", "-e", "POSTGRES_PASSWORD=pw",
        "-e", "POSTGRES_DB=aspen", "-v", f"{dump}:/dump/database.dump:ro", "postgres:18-alpine")  # fmt: skip
    try:
        # Over TCP: the image first runs a server on its socket alone to make the database, and
        # stops it before starting the one that stays.
        for _ in range(60):
            if run("docker", "exec", name, "pg_isready", "-h", "127.0.0.1", "-U", "aspen", "-d", "aspen", check=False).returncode == 0:
                break
            time.sleep(1)
        run("docker", "exec", name, *json.loads(options), "--username=aspen", "--dbname=aspen", "/dump/database.dump")
        say("  ok: the larger configuration's restore takes the dump")
        users = run("docker", "exec", name, "psql", "-U", "aspen", "-d", "aspen", "-tAc", "SELECT name FROM \"user\" WHERE name = 'boxer'")
        expect("with the accounts in it", users.stdout.strip(), "boxer")
    finally:
        run("docker", "rm", "-f", name, check=False)


def make_certificates() -> None:
    certs = WORK / "certs"
    certs.mkdir(parents=True, exist_ok=True)
    if (certs / "server.crt").exists():
        return
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=check-box authority",
        "-keyout", str(certs / "ca.key"), "-out", str(certs / "ca.pem"))  # fmt: skip
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", str(certs / "server.key"),
        "-out", str(certs / "server.csr"))  # fmt: skip
    (certs / "san.ext").write_text("subjectAltName=DNS:localhost\n")
    run("openssl", "x509", "-req", "-in", str(certs / "server.csr"), "-CA", str(certs / "ca.pem"), "-CAkey", str(certs / "ca.key"),
        "-CAcreateserial", "-days", "2", "-extfile", str(certs / "san.ext"), "-out", str(certs / "server.crt"))  # fmt: skip
    for name in ("server.key", "server.crt"):
        (certs / name).chmod(0o644)


def check_database_elsewhere(images: dict) -> None:
    say("a deployment whose database is elsewhere")
    make_certificates()
    box = Box("check-box-outside", [])
    try:
        box.tool("init", "--domain", DOMAIN, "--address4", "203.0.113.30", "--address6", "none", "--acme-email", "-",
                 "--storage", "machine", "--database-url", "postgres://outside:outsidepw@localhost:5433/aspen",
                 "--database-ca", str(WORK / "certs" / "ca.pem"),
                 "--api-image", images["api"], "--voice-image", images["voice"])  # fmt: skip
        box.compose("--profile", "outside-database", "up", "-d", "extpg", "oldpg")
        time.sleep(6)
        box.tool("up")
        expect("the API is ready", box.status(f"https://{DOMAIN}/api/v1/health/ready"), 204)
        expect("the box runs no PostgreSQL of its own", "postgres" in box.compose("ps", "--services").stdout.split(), False)
        tables = run("docker", "exec", f"{box.name}-extpg-1", "psql", "-p", "5433", "-U", "outside", "-d", "aspen", "-Atc",
                     "select count(*) > 50 from information_schema.tables where table_schema = 'public'").stdout.strip()  # fmt: skip
        expect("the migrations ran there", tables, "t")
        encrypted = run("docker", "exec", f"{box.name}-extpg-1", "psql", "-p", "5433", "-U", "outside", "-d", "aspen", "-Atc",
                        "select bool_and(ssl) from pg_stat_ssl join pg_stat_activity using (pid) where usename = 'outside' and client_addr is not null").stdout.strip()  # fmt: skip
        expect("every connection from the servers is over TLS", encrypted, "t")
        box.edit(lambda b: b.update(database={"kind": "external", "url": "postgres://outside:outsidepw@localhost:5434/aspen?sslmode=disable", "ca_file": None}))
        refused = box.tool("up", check=False)
        expect("a PostgreSQL too old is refused", refused.returncode != 0 and "PostgreSQL 17" in refused.stderr, True)
    finally:
        box.down()


def clear() -> None:
    """Takes away what an earlier run left: its containers, network, and files."""
    names = run("docker", "ps", "-aq", "--filter", "name=check-box-", check=False).stdout.split()
    if names:
        run("docker", "rm", "-f", *names, check=False)
    run("docker", "network", "rm", NETWORK, check=False)
    if WORK.exists():
        # Some of what the services wrote is their users', not this one's.
        run("docker", "run", "--rm", "-v", f"{WORK.parent}:/work", "alpine:3", "rm", "-rf", f"/work/{WORK.name}", check=False)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--api-image", required=True)
    parser.add_argument("--voice-image", required=True)
    parser.add_argument("--keep", action="store_true", help="leave everything running, for looking at it")
    opt = parser.parse_args()
    images = {"api": opt.api_image, "voice": opt.voice_image}
    clear()
    WORK.mkdir(parents=True, exist_ok=True)
    run("docker", "network", "create", NETWORK, check=False)
    boxes = []
    try:
        check_oldest_python()
        main_box = check_main(images)
        boxes.append(main_box)
        check_voice_elsewhere(main_box, images)
        check_export(main_box)
        check_database_elsewhere(images)
        say("every check passed")
    finally:
        if not opt.keep:
            for box in boxes:
                box.down()
            run("docker", "network", "rm", NETWORK, check=False)
            for target in ("api", "voice"):
                run("docker", "rmi", f"{images[target]}-next", check=False)
            clear()


if __name__ == "__main__":
    try:
        main()
    except Failed as failure:
        print(f"check-box: FAILED: {failure}", file=sys.stderr)
        sys.exit(1)
