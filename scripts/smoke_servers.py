#!/usr/bin/env python3
"""Checks that built Aspen servers start and serve, against the services in docker-compose.yaml.

    scripts/smoke_servers.py --bin target/release

It makes a database of its own, runs the migrations, starts the chat server and the voice server
on ports of their own (so a development stack can keep running beside it), and then:

- registers a user, signs in, and reads the user back;
- makes a community with a voice channel and joins its call, which needs the voice server to
  have registered and reported in over NATS;
- reads both servers' metrics, including the allocator's figures;
- runs the voice server's `estimate-capacity`, whose self-test forwards real media through
  mediasoup, and checks the media arrived.

The database is dropped and both servers stopped however it ends; their logs are printed if a
check fails. Needs Python 3.10, and `docker compose` with the database, nats, valkey, and
seaweedfs services up (`--start-services` starts them).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
API_PORT = 18000
VOICE_PORT = 19001
API_METRICS = 19464
VOICE_METRICS = 19465
TOKEN_SECRET = "smoke-test-voice-secret"
DATABASE = f"aspen_smoke_{os.getpid()}"
DATABASE_URL = f"postgres://postgres:aspen_test@127.0.0.1:5432/{DATABASE}"
# The S3 credentials docker-compose.yaml gives SeaweedFS.
S3_ACCESS_KEY = "GK484e56c38fb7e14b182bf47a"
S3_SECRET_KEY = "6b49da9e42f7959cc946d7987a504763f6ec405b88abeeec08aa926b61316027"


class Failed(Exception):
    pass


def say(message: str) -> None:
    print(f"smoke: {message}", flush=True)


def compose(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["docker", "compose", *args], cwd=REPO, capture_output=True, text=True, check=check)


def psql(sql: str) -> str:
    return compose("exec", "-T", "database", "psql", "-U", "postgres", "-tAc", sql).stdout.strip()


def request(method: str, url: str, body: dict | None = None, token: str | None = None,
            language: str | None = None) -> tuple[int, str]:
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


def api(method: str, path: str, body: dict | None = None, token: str | None = None, expect: tuple = (200, 201)) -> dict:
    status, text = request(method, f"http://127.0.0.1:{API_PORT}/api/v1{path}", body, token)
    if status not in expect:
        raise Failed(f"{method} {path}: {status} {text[:300]}")
    return json.loads(text) if text else {}


def wait_for(what: str, probe, seconds: float) -> None:
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            if probe():
                return
        except Exception as error:  # noqa: BLE001 - any failure is a reason to keep waiting
            last = error
        time.sleep(1)
    raise Failed(f"{what} did not come up within {seconds:.0f} s{f' ({last})' if last else ''}")


def http_status(url: str) -> int:
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


def start_services() -> None:
    say("starting the database, NATS, Valkey, and SeaweedFS")
    compose("up", "-d", "database", "nats", "valkey", "seaweedfs")


def wait_for_services() -> None:
    wait_for("PostgreSQL", lambda: compose("exec", "-T", "database", "pg_isready", "-U", "postgres", check=False).returncode == 0, 90)
    wait_for("SeaweedFS's S3", lambda: http_status("http://127.0.0.1:8333/") in (200, 403, 404), 90)


def write_configs(work: Path, voice_id: str | None) -> None:
    (work / "aspen.toml").write_text(
        f'database_url = "{DATABASE_URL}"\n'
        'nats_url = "nats://127.0.0.1:4222"\n'
        'nats_auth_token = "aspen_test"\n'
        'valkey_url = "redis://127.0.0.1:6379"\n'
        "[media.s3]\n"
        'endpoint = "http://127.0.0.1:8333"\n'
        'region = "seaweed"\n'
        'bucket = "aspen-media"\n'
        f'access_key = "{S3_ACCESS_KEY}"\n'
        f'secret_key = "{S3_SECRET_KEY}"\n'
        'public_base_url = "http://127.0.0.1:8888/buckets/aspen-media"\n'
        "[metrics]\n"
        f'listen_addr = "127.0.0.1:{API_METRICS}"\n'
        "[voice]\n"
        f'token_secret = "{TOKEN_SECRET}"\n'
        "[[voice.servers]]\n"
        'name = "smoke"\n'
        f'url = "http://127.0.0.1:{VOICE_PORT}"\n'
        "capacity = 50\n"
    )
    if voice_id:
        (work / "voice_server.toml").write_text(
            f'id = "{voice_id}"\n'
            f'token_secret = "{TOKEN_SECRET}"\n'
            'nats_url = "nats://127.0.0.1:4222"\n'
            'nats_auth_token = "aspen_test"\n'
            f'listen_addr = "127.0.0.1:{VOICE_PORT}"\n'
            "workers = 2\n"
            "[rtc]\n"
            "min_port = 45000\n"
            "max_port = 45199\n"
            "[metrics]\n"
            f'listen_addr = "127.0.0.1:{VOICE_METRICS}"\n'
        )


def clean_env() -> dict[str, str]:
    """This environment without ASPEN_ variables, which would override the configs written."""
    return {k: v for k, v in os.environ.items() if not k.startswith("ASPEN_")}


def smoke(bins: Path, work: Path, processes: list) -> None:
    for name in ["aspen-chat-server", "voice_server", "aspen-migrate"]:
        if not (bins / name).exists():
            raise Failed(f"{bins / name} does not exist")
    psql(f"CREATE DATABASE {DATABASE}")
    write_configs(work, None)
    say("running the migrations")
    migrated = subprocess.run(
        [str(bins / "aspen-migrate"), "up", "--database-url", DATABASE_URL],
        cwd=work, env=clean_env(), capture_output=True, text=True,
    )
    if migrated.returncode != 0:
        raise Failed(f"migrations failed: {migrated.stderr[-1000:]}")

    say("starting the chat server")
    api_log = open(work / "api.log", "w")
    processes.append((subprocess.Popen(
        [str(bins / "aspen-chat-server"), "--no-https", "--port", str(API_PORT), "--listen-addr", "127.0.0.1"],
        cwd=work, env=clean_env(), stdout=api_log, stderr=subprocess.STDOUT,
    ), work / "api.log"))
    wait_for("the chat server", lambda: http_status(f"http://127.0.0.1:{API_PORT}/api/v1/auth/methods") == 200, 60)

    voice_id = subprocess.run(
        ["docker", "compose", "exec", "-T", "database", "psql", "-U", "postgres", "-d", DATABASE, "-tAc",
         "SELECT id FROM voice_server WHERE name = 'smoke'"],
        cwd=REPO, capture_output=True, text=True, check=True,
    ).stdout.strip()
    if not voice_id:
        raise Failed("the chat server did not register the voice server from its config")
    write_configs(work, voice_id)
    say("starting the voice server")
    voice_log = open(work / "voice.log", "w")
    processes.append((subprocess.Popen(
        [str(bins / "voice_server")], cwd=work, env=clean_env(), stdout=voice_log, stderr=subprocess.STDOUT,
    ), work / "voice.log"))
    wait_for("the voice server", lambda: http_status(f"http://127.0.0.1:{VOICE_PORT}/health") == 204, 60)

    say("registering, signing in, and reading the user back")
    name = f"smoke{os.getpid()}"
    api("POST", "/users", {"name": name, "password": "smoke-test-password"})
    token = api("POST", "/auth/login", {"username": name, "password": "smoke-test-password"})["sessionToken"]
    me = api("GET", "/users/@me", token=token)
    if me.get("name") != name:
        raise Failed(f"GET /users/@me answered {me}")

    say("refusing a wrong password in the language asked for")
    status, text = request("POST", f"http://127.0.0.1:{API_PORT}/api/v1/auth/login",
                           {"username": name, "password": "wrong"}, language="en-XA, en;q=0.5")
    title = json.loads(text).get("title", "")
    if status != 401 or not (title.startswith("[") and title.endswith("]")):
        raise Failed(f"a refusal asked for in en-XA answered {status} {text[:300]}")

    community = api("POST", "/communities", {"name": "Smoke"}, token)
    community_id = community.get("id") or community.get("data", {}).get("id")

    say("finding a message by its words")
    general = next(c for c in api("GET", f"/communities/{community_id}/channels", token=token) if c["ty"] == "text")
    posted = api("POST", f"/channels/{general['id']}/messages", {"content": "Smoke signals from the Tuesday picnic", "attachments": []}, token)
    api("POST", f"/channels/{general['id']}/messages", {"content": "Nothing about lunch here", "attachments": []}, token)
    found = api("GET", "/messages?filter[text]=PICNIC%20tuesday&include=channels", token=token)
    if [m["id"] for m in found["data"]] != [posted["id"]] or found["included"]["channels"][0]["id"] != general["id"]:
        raise Failed(f"searching for the picnic found {found}")
    left_out = api("GET", "/messages?filter[text]=smoke%20-picnic", token=token)
    if left_out["data"]:
        raise Failed(f"searching for smoke without the picnic found {left_out}")

    say("joining a call, which needs the voice server to have reported in")
    channel = api("POST", "/channels", {"community": community_id, "name": "Lounge", "sortIndex": 0, "ty": "voice"}, token)
    offer: dict = {}

    def joined() -> bool:
        nonlocal offer
        status, text = request("POST", f"http://127.0.0.1:{API_PORT}/api/v1/channels/{channel['id']}/voice/join", {}, token)
        offer = json.loads(text) if status == 200 else {}
        return status == 200

    wait_for("a voice server offer", joined, 90)
    if not any(c.get("id") == voice_id for c in offer.get("candidates", [])):
        raise Failed(f"the join offered {offer.get('candidates')}, not the smoke voice server")

    say("reading both servers' metrics")
    for port, series in [(API_METRICS, "aspen_db_pool_connections"), (VOICE_METRICS, "aspen_voice_rooms")]:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/metrics", timeout=10) as response:
            text = response.read().decode()
        for wanted in [series, "aspen_memory_allocated_bytes"]:
            if wanted not in text:
                raise Failed(f"metrics on port {port} lack {wanted}")

    say("running estimate-capacity")
    estimated = subprocess.run(
        [str(bins / "voice_server"), "estimate-capacity", "--calibration-seconds", "2", "--json"],
        cwd=work, env=clean_env(), capture_output=True, text=True, timeout=300,
    )
    if estimated.returncode != 0:
        raise Failed(f"estimate-capacity failed: {estimated.stderr[-1000:]}")
    estimate = json.loads(estimated.stdout)
    delivered = {k: estimate[k]["delivered"] for k in ("audio_stream", "video_stream", "participant_in_calls")}
    if estimate["capacity"] <= 0 or min(delivered.values()) < 0.9:
        raise Failed(f"estimate-capacity: capacity {estimate['capacity']}, delivered {delivered}")
    say(f"estimate-capacity: capacity {estimate['capacity']}, media delivered {delivered}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Check built Aspen servers start and serve.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    args = parser.parse_args()
    bins = args.bin.resolve()
    if shutil.which("docker") is None:
        sys.exit("smoke: docker is required")
    if args.start_services:
        start_services()
    wait_for_services()
    processes: list = []
    work = Path(tempfile.mkdtemp(prefix="aspen-smoke-"))
    try:
        smoke(bins, work, processes)
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        for _, log in processes:
            print(f"----- {log.name} -----\n{log.read_text()[-4000:]}", flush=True)
        sys.exit(1)
    finally:
        for process, _ in processes:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        psql(f"DROP DATABASE IF EXISTS {DATABASE} WITH (FORCE)")
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
