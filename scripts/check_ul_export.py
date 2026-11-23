#!/usr/bin/env python3
"""Checks that a deployment leaves Ultra Lite whole: runs `aspen-ul` with data of many kinds,
exports it, takes the export into a full deployment the way docs/operators/ultra-lite/leaving.md
says, and compares the two. Every table must hold the same rows, every file be in object storage
at its size and type, a session from before the move still sign in, and a message and its
attachment read back.

    scripts/check_ul_export.py --ul target/ul/x86_64/aspen-ul --bin target/debug
    scripts/check_ul_export.py --ul target/debug/aspen-ul --runtime DIR --bin target/debug
    scripts/check_ul_export.py --ul target/debug/aspen-ul --ul-postgres DIR --bin target/debug

`--ul` is aspen-ul: one scripts/package_ul.py made, or a build with `--runtime`, a directory
holding the bundled PostgreSQL (`pgsql/`) and a web client (`web-client/`), or with
`--ul-postgres`, the installation `scripts/package_ul.py --postgres-only` built, which the check
makes such a directory of with a stand-in web client. `--bin` holds
aspen-chat-server and aspen-migrate. The full deployment's PostgreSQL, NATS, Valkey, and
SeaweedFS (for S3) are containers of its own, on ports from `--ports` up, so a development stack
keeps running beside it. Needs Python 3.10 and Docker.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from stack import Failed, Reached, clean_env, fetch, png, put, wait_for  # noqa: E402
from web_client import stand_in  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
IMAGES = {
    "postgres": "postgres:18-alpine",
    "nats": "nats:2.11-alpine",
    "valkey": "valkey/valkey:alpine",
    "seaweedfs": "chrislusf/seaweedfs:4.22",
}
BUCKET = "aspen-media"
# Ultra Lite's own, for `move-to-s3`, so neither move finds the other's files already there.
UL_BUCKET = "aspen-ul-media"
S3_ACCESS_KEY = "GKexportcheck0000000000000"
S3_SECRET_KEY = "export-check-secret-key-0000000000000000"
PASSWORD = "export-check-password-1"


def say(message: str) -> None:
    print(f"check_ul_export: {message}", flush=True)


def free(port: int) -> None:
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", port)) == 0:
            raise Failed(f"port {port} is taken; give --ports another first port")


class Check:
    def __init__(self, opt: argparse.Namespace):
        self.opt = opt
        # Short, since PostgreSQL's socket in the data directory must be (a Unix socket's path has
        # at most 107 bytes), and on disk rather than in memory.
        self.work = Path("/var/tmp") / f"aspen-ul-export-{os.getpid()}"
        self.containers: list[str] = []
        self.processes: list[tuple[subprocess.Popen, Path]] = []
        first = opt.ports
        self.port = first
        self.voice_ports = (first + 1, first + 2, first + 3, first + 9)
        self.pg, self.nats, self.valkey = first + 20, first + 21, first + 22
        self.s3, self.master, self.volume, self.filer = first + 23, first + 24, first + 25, first + 26
        self.base = f"http://localhost:{self.port}"
        self.data = self.work / "ul"
        self.export = self.work / "export"
        self.full = self.work / "full"

    # --- processes and containers -----------------------------------------------------------

    def run(self, args: list, cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
        done = subprocess.run([str(a) for a in args], cwd=cwd, env=clean_env(), capture_output=True, text=True)
        if check and done.returncode != 0:
            raise Failed(f"{' '.join(str(a) for a in args[:3])} failed ({done.returncode}): {done.stderr[-2000:]}")
        return done

    def spawn(self, args: list, log: str, cwd: Path) -> subprocess.Popen:
        out = open(self.work / log, "w")
        process = subprocess.Popen([str(a) for a in args], cwd=cwd, env=clean_env(), stdout=out, stderr=subprocess.STDOUT)
        self.processes.append((process, self.work / log))
        return process

    def until(self, what: str, probe, process: subprocess.Popen, seconds: float) -> None:
        """Waits for `probe` to answer true, failing at once should `process` end."""
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise Failed(f"{what} stopped ({process.returncode}) before it was ready")
            try:
                if probe():
                    return
            except OSError:
                pass
            time.sleep(0.5)
        raise Failed(f"{what} was not ready within {seconds:.0f} s")

    def stop(self, process: subprocess.Popen) -> None:
        process.send_signal(2)
        try:
            process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            process.kill()
            raise Failed("a server did not stop within a minute")

    def container(self, name: str, args: list[str]) -> None:
        full = f"aspen-ul-export-{os.getpid()}-{name}"
        subprocess.run(["docker", "run", "-d", "--rm", "--name", full, *args], check=True, capture_output=True)
        self.containers.append(full)

    def close(self, failed: bool) -> None:
        if failed:
            for process, log in self.processes:
                print(f"----- {log.name} -----\n{log.read_text()[-4000:]}", flush=True)
        for process, _ in self.processes:
            if process.poll() is None:
                process.kill()
        for name in self.containers:
            subprocess.run(["docker", "rm", "-f", name], capture_output=True)
        if not self.opt.keep:
            shutil.rmtree(self.work, ignore_errors=True)

    # --- Ultra Lite --------------------------------------------------------------------------

    def ul(self, *args: str) -> list:
        runtime = ["--runtime", self.opt.runtime] if self.opt.runtime else []
        return [self.opt.ul.resolve(), "--data", self.data, *runtime, *args]

    def pg_bin(self) -> Path:
        if self.opt.runtime:
            return self.opt.runtime.resolve() / "pgsql" / "bin"
        current = (self.data / "runtime" / "current").read_text().strip()
        return self.data / "runtime" / current / "pgsql" / "bin"

    def write_ul_config(self) -> None:
        media, transfer, relay_min, relay_max = self.voice_ports
        self.data.mkdir(parents=True)
        (self.data / "aspen-ul.toml").write_text(
            f'public_url = "{self.base}"\n'
            f'media_url = "http://media.localhost:{self.port}"\n'
            'listen = ["127.0.0.1"]\n'
            f"port = {self.port}\n"
            '[tls]\nmode = "off"\n'
            f"[voice]\nmedia_port = {media}\ntransfer_port = {transfer}\nrelay_ports = [{relay_min}, {relay_max}]\n"
        )

    def seed(self, client: Reached) -> dict:
        """Data of many kinds, and what is read back after the move."""
        tokens = {}
        users = {}
        for name in ("alice", "bob"):
            users[name] = client.api("POST", "/users", {"name": name, "password": PASSWORD})
            tokens[name] = client.api("POST", "/auth/login", {"username": name, "password": PASSWORD})["sessionToken"]
        alice, bob = tokens["alice"], tokens["bob"]
        self.run(self.ul("admin", "grant", "alice"))
        self.run(self.ul("settings", "set", "--display-name", "Export Check"))
        made = client.api("POST", "/communities", {"name": "Leaving"}, alice)
        community = made.get("id") or made["data"]["id"]
        invite = client.api("POST", f"/communities/{community}/invites", {}, alice)
        client.api("PUT", f"/communities/{community}/members/@me", {"inviteCode": invite.get("code") or invite.get("id")}, bob)
        channels = client.api("GET", f"/communities/{community}/channels", token=alice)["data"]
        general = next(c for c in channels if c["ty"] == "text")["id"]

        note = b"what a deployment keeps, through its move"
        handle = client.api("POST", "/attachments", {"fileName": "note.txt", "mimeType": "text/plain", "byteSize": len(note)}, alice)
        put(handle["uploadUrl"], note, "text/plain")
        client.api("POST", f"/attachments/{handle['id']}/confirm", token=alice)
        picture = png()
        shot = client.api("POST", "/attachments", {"fileName": "shot.png", "mimeType": "image/png", "byteSize": len(picture)}, alice)
        put(shot["uploadUrl"], picture, "image/png")
        client.api("POST", f"/attachments/{shot['id']}/confirm", token=alice)
        message = client.api("POST", f"/channels/{general}/messages",
                             {"content": "Files that move with us", "attachments": [handle["id"], shot["id"]]}, alice)
        client.api("PUT", f"/messages/{message['id']}/reactions/%F0%9F%91%8D/@me", token=bob)
        client.api("POST", f"/messages/{message['id']}/thread/messages",
                   {"content": "a reply in its thread", "attachments": [], "echoToParent": False}, bob)
        poll = client.api("POST", f"/channels/{general}/polls", {
            "question": "Move?", "options": [{"label": "yes"}, {"label": "no"}], "multipleChoice": False,
            "allowWriteIns": False, "anonymous": False, "durationSeconds": 3600}, alice)["id"]
        client.api("PUT", f"/polls/{poll}/votes/0/@me", token=bob)
        client.api("PUT", f"/users/@me/saved-messages/{message['id']}", token=bob)
        dm = client.api("POST", "/users/@me/dms", {"recipients": [users["bob"]["id"]]}, alice)
        dm = dm.get("id") or dm["data"]["id"]
        client.api("POST", f"/channels/{dm}/messages", {"content": "a word in private", "attachments": []}, alice)
        icon_png = png(128, 128)
        icon = client.api("POST", "/icons", {"mimeType": "image/png", "byteSize": len(icon_png)}, alice)
        put(icon["uploadUrl"], icon_png, "image/png")
        client.api("POST", f"/icons/{icon['id']}/confirm", token=alice)
        client.api("PATCH", f"/communities/{community}", {"icon": icon["id"]}, alice)
        client.api("PUT", f"/channels/{general}/read-states/@me", {"lastRead": message["id"]}, bob)
        # Previews of the picture are made by a job; the message is held until they are.
        wait_for("the picture's preview", lambda: self.preview_made(client, general, alice, message["id"]), 60)
        return {"token": alice, "channel": general, "message": message["id"], "note": note}

    def preview_made(self, client: Reached, channel: str, token: str, message: str) -> bool:
        window = client.api("GET", f"/channels/{channel}/messages?around={message}&include=attachments", token=token)
        attachments = window.get("included", {}).get("attachments", [])
        return any(a.get("previewUrl") or a.get("preview") for a in attachments)

    # --- comparing databases -----------------------------------------------------------------

    def tables(self, psql: list) -> dict[str, tuple[int, str]]:
        """Each table's row count and a digest of its rows as text, which both clusters write
        alike only under the same time zone and output settings, so those are given."""
        os.environ["PGOPTIONS"] = "-c TimeZone=UTC -c DateStyle=ISO -c IntervalStyle=postgres -c extra_float_digits=3"
        names = self.run([*psql, "-Atc", "SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY 1"]).stdout.split()
        found = {}
        for name in names:
            row = self.run([*psql, "-Atc",
                            f"SELECT count(*), md5(coalesce(string_agg(t::text, '|' ORDER BY t::text), '')) FROM \"{name}\" t"]).stdout.strip()
            count, digest = row.split("|")
            found[name] = (int(count), digest)
        return found

    def ul_tables(self) -> dict[str, tuple[int, str]]:
        """The tables of Ultra Lite's cluster, read by its own PostgreSQL started on a socket of
        the check's, with nothing else running."""
        socket_dir = self.work / "pg-socket"
        socket_dir.mkdir()
        cluster = next(p for p in (self.data / "postgres").iterdir() if (p / "PG_VERSION").exists())
        server = self.spawn([self.pg_bin() / "postgres", "-D", cluster, "-c", "listen_addresses=",
                             "-c", f"unix_socket_directories={socket_dir}"], "ul-postgres.log", self.work)
        psql = [self.pg_bin() / "psql", "-h", socket_dir, "-d", "aspen", "-X"]
        self.until("Ultra Lite's PostgreSQL", lambda: self.run([*psql, "-Atc", "SELECT 1"], check=False).returncode == 0, server, 60)
        try:
            return self.tables(psql)
        finally:
            self.stop(server)

    # --- the check ---------------------------------------------------------------------------

    def main(self) -> None:
        for port in (self.port, *self.voice_ports, self.pg, self.nats, self.valkey, self.s3, self.master, self.volume, self.filer):
            free(port)
        self.work.mkdir(parents=True)
        if self.opt.ul_postgres:
            # A runtime of the PostgreSQL given and a stand-in web client, as `--runtime` takes it.
            runtime = self.work / "runtime"
            runtime.mkdir()
            (runtime / "pgsql").symlink_to(self.opt.ul_postgres.resolve())
            (runtime / "web-client").symlink_to(stand_in(self.work / "ul-web-client").resolve())
            self.opt.runtime = runtime
        self.write_ul_config()
        server = self.spawn(self.ul(), "ul.log", self.work)
        client = Reached(self.base)
        self.until("aspen-ul", lambda: client.status("GET", "/health") == 204, server, 120)
        seeded = self.seed(client)
        say("seeded Ultra Lite: accounts, a community, messages with a file and a picture, a reaction, a thread, a poll, a DM, an icon")
        before = client.api("GET", f"/channels/{seeded['channel']}/messages?around={seeded['message']}&include=attachments",
                            token=seeded["token"])
        served = {}
        for attachment in before["included"]["attachments"]:
            served[attachment["id"]] = fetch(attachment["downloadUrl"])
        self.stop(server)

        self.run(self.ul("export", self.export))
        manifest = json.loads((self.export / "export.json").read_text())
        if manifest["publicUrl"] != self.base or not manifest["voiceServer"] or manifest["files"] != "directory":
            raise Failed(f"export.json says {manifest}")
        say(f"exported, at schema {manifest['schema']}")
        ul_tables = self.ul_tables()

        self.container("postgres", ["-p", f"127.0.0.1:{self.pg}:5432", "-e", "POSTGRES_PASSWORD=export", IMAGES["postgres"]])
        self.container("nats", ["-p", f"127.0.0.1:{self.nats}:4222", IMAGES["nats"], "--jetstream", "--auth", "export"])
        self.container("valkey", ["-p", f"127.0.0.1:{self.valkey}:6379", IMAGES["valkey"]])
        self.container("seaweedfs", [
            "--network", "host", "-e", f"AWS_ACCESS_KEY_ID={S3_ACCESS_KEY}", "-e", f"AWS_SECRET_ACCESS_KEY={S3_SECRET_KEY}",
            "-e", f"S3_BUCKET={BUCKET}", IMAGES["seaweedfs"], "server", "-s3", "-ip", "127.0.0.1", "-ip.bind", "127.0.0.1",
            f"-s3.port={self.s3}", f"-master.port={self.master}", f"-volume.port={self.volume}", f"-filer.port={self.filer}"])
        psql = [self.pg_bin() / "psql", "-h", "127.0.0.1", "-p", self.pg, "-U", "postgres", "-X"]
        os.environ["PGPASSWORD"] = "export"
        wait_for("PostgreSQL", lambda: self.run([*psql, "-Atc", "SELECT 1"], check=False).returncode == 0, 90)
        self.run([*psql, "-c", "CREATE DATABASE aspen"])
        # As leaving.md says: the dump into an empty database, then the full deployment's migrations.
        self.run([self.pg_bin() / "pg_restore", "--no-owner", "--no-privileges", "--exit-on-error", "--single-transaction",
                  "-h", "127.0.0.1", "-p", self.pg, "-U", "postgres", "-d", "aspen", self.export / "database.dump"])
        database_url = f"postgres://postgres:export@127.0.0.1:{self.pg}/aspen"
        self.run([self.opt.bin / "aspen-migrate", "up", "--database-url", database_url])
        full_tables = self.tables([*psql, "-d", "aspen"])
        differing = sorted(t for t in set(ul_tables) | set(full_tables) if ul_tables.get(t) != full_tables.get(t))
        if differing:
            raise Failed(f"tables differ after the move: {', '.join(differing)}")
        say(f"all {len(full_tables)} tables hold the same rows, {sum(c for c, _ in full_tables.values())} in all")

        self.full.mkdir()
        (self.full / "aspen.toml").write_text(
            f'public_url = "{manifest["publicUrl"]}"\n'
            f'database_url = "{database_url}"\n'
            f'nats_url = "nats://127.0.0.1:{self.nats}"\nnats_auth_token = "export"\n'
            f'valkey_url = "redis://127.0.0.1:{self.valkey}"\n'
            "[media.s3]\n"
            f'endpoint = "http://127.0.0.1:{self.s3}"\nregion = "seaweed"\nbucket = "{BUCKET}"\n'
            f'access_key = "{S3_ACCESS_KEY}"\nsecret_key = "{S3_SECRET_KEY}"\n'
            f'public_base_url = "http://127.0.0.1:{self.filer}/buckets/{BUCKET}"\n'
            "[metrics]\nenabled = false\n"
            f"[web_client]\ndir = {json.dumps(str(stand_in(self.work / 'web-client')))}\n"
        )
        wait_for("SeaweedFS's S3", lambda: self.s3_answers(), 90)
        imported = self.run([self.opt.bin / "aspen-chat-server", "storage", "import", self.export], cwd=self.full)
        say(imported.stdout.strip())
        # Run again, as after an import that stopped: what is there is left as it is.
        again = self.run([self.opt.bin / "aspen-chat-server", "storage", "import", self.export], cwd=self.full)
        if not again.stdout.startswith("uploaded 0 files"):
            raise Failed(f"importing again uploaded files again: {again.stdout}")
        self.run([self.opt.bin / "aspen-chat-server", "voice-servers", "remove", manifest["voiceServer"]["name"]], cwd=self.full)
        self.check_files()

        full = self.spawn([self.opt.bin / "aspen-chat-server", "--no-https", "--port", self.port, "--listen-addr", "127.0.0.1"],
                          "full.log", self.full)
        self.until("the full deployment", lambda: client.status("GET", "/health") == 204, full, 90)
        me = client.api("GET", "/users/@me", token=seeded["token"])
        if me["name"] != "alice":
            raise Failed(f"the session from before the move reads {me}")
        say("a session from before the move signs in")
        after = client.api("GET", f"/channels/{seeded['channel']}/messages?around={seeded['message']}&include=attachments",
                           token=seeded["token"])
        if [m["content"] for m in after["data"]] != [m["content"] for m in before["data"]]:
            raise Failed("the channel reads otherwise after the move")
        for attachment in after["included"]["attachments"]:
            body, kind = fetch(attachment["downloadUrl"])
            was_body, was_kind = served[attachment["id"]]
            if body != was_body or kind.split(";")[0] != was_kind.split(";")[0]:
                raise Failed(f"{attachment['downloadUrl']} served {kind} {len(body)} bytes, not {was_kind} {len(was_body)}")
        say("the channel and its files read back from object storage as they did from Ultra Lite")
        self.stop(full)
        self.check_move_to_s3(client, seeded, before, served)
        say("passed")

    def check_move_to_s3(self, client: Reached, seeded: dict, before: dict, served: dict) -> None:
        """Ultra Lite keeping its files in object storage instead: refused to start while the data
        directory still holds them, then moved, then serving them from there."""
        urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{self.filer}/buckets/{UL_BUCKET}/", method="POST"),
                               timeout=15).close()
        with (self.data / "aspen-ul.toml").open("a") as config:
            config.write(
                "[server.media.s3]\n"
                f'endpoint = "http://127.0.0.1:{self.s3}"\nregion = "seaweed"\nbucket = "{UL_BUCKET}"\n'
                f'access_key = "{S3_ACCESS_KEY}"\nsecret_key = "{S3_SECRET_KEY}"\n'
                f'public_base_url = "http://127.0.0.1:{self.filer}/buckets/{UL_BUCKET}"\n'
            )
        refused = self.run(self.ul(), check=False)
        if refused.returncode == 0 or "move-to-s3" not in refused.stderr + refused.stdout:
            raise Failed(f"aspen-ul started with its files still in the data directory: {refused.stderr[-500:]}")
        moved = self.run(self.ul("move-to-s3"))
        if not moved.stdout.startswith("uploaded ") or "(0 MiB; 0 were already there)" not in moved.stdout and "; 0 were already there" not in moved.stdout:
            raise Failed(f"move-to-s3 said {moved.stdout}")
        say(moved.stdout.strip().split(". ")[0])
        if (self.data / "media").exists() or not list(self.data.glob("media.moved-*")):
            raise Failed("move-to-s3 left media/ in place")
        server = self.spawn(self.ul(), "ul-s3.log", self.work)
        self.until("aspen-ul on object storage", lambda: client.status("GET", "/health") == 204, server, 120)
        after = client.api("GET", f"/channels/{seeded['channel']}/messages?around={seeded['message']}&include=attachments",
                           token=seeded["token"])
        for attachment in after["included"]["attachments"]:
            if f"/buckets/{UL_BUCKET}/" not in attachment["downloadUrl"]:
                raise Failed(f"{attachment['downloadUrl']} is not the bucket's")
            body, kind = fetch(attachment["downloadUrl"])
            was_body, was_kind = served[attachment["id"]]
            if body != was_body or kind.split(";")[0] != was_kind.split(";")[0]:
                raise Failed(f"{attachment['downloadUrl']} served {kind} {len(body)} bytes, not {was_kind} {len(was_body)}")
        self.stop(server)
        say("Ultra Lite refused to start with its files still at hand, moved them, and serves them from object storage")

    def s3_answers(self) -> bool:
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{self.s3}/", timeout=2)
        except urllib.error.HTTPError:
            return True
        except OSError:
            return False
        return True

    def check_files(self) -> None:
        """Every file of the export is in the bucket at its size, served as its record says."""
        objects = self.export / "media" / "objects"
        count = 0
        for path in sorted(p for p in objects.rglob("*") if p.is_file()):
            key = path.relative_to(objects).as_posix()
            if key.startswith("uploads/"):
                continue
            record = json.loads((self.export / "media" / "meta" / f"{key}.json").read_text())
            request = urllib.request.Request(f"http://127.0.0.1:{self.filer}/buckets/{BUCKET}/{key}", method="HEAD")
            with urllib.request.urlopen(request, timeout=15) as response:
                size = int(response.headers["content-length"])
                kind = response.headers.get("content-type", "")
            if size != path.stat().st_size or kind.split(";")[0] != record["contentType"]:
                raise Failed(f"{key} is {size} bytes of {kind} in the bucket, not {path.stat().st_size} of {record['contentType']}")
            count += 1
        if count == 0:
            raise Failed("the export holds no files, so moving them was not checked")
        say(f"all {count} files are in object storage at their size and type")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ul", type=Path, required=True)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--ul-postgres", type=Path,
                        help="a PostgreSQL installation (scripts/package_ul.py --postgres-only) to run --ul with, "
                             "with a stand-in web client, in place of --runtime")
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--ports", type=int, default=19100, help="the first of the 27 ports it takes (19100)")
    parser.add_argument("--keep", action="store_true", help="keep the work directory, /var/tmp/aspen-ul-export-<pid>")
    opt = parser.parse_args()
    if opt.runtime and opt.ul_postgres:
        parser.error("give --runtime or --ul-postgres, not both")
    opt.bin = opt.bin.resolve()
    check = Check(opt)
    failed = True
    try:
        check.main()
        failed = False
    except Failed as e:
        print(f"check_ul_export: FAILED: {e}", file=sys.stderr, flush=True)
        sys.exit(1)
    finally:
        check.close(failed)


if __name__ == "__main__":
    main()
