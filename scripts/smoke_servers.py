#!/usr/bin/env python3
"""Checks that built Aspen servers start and serve, against the services in docker-compose.yaml.

    scripts/smoke_servers.py --bin target/release

It makes a database and a NATS of its own, runs the migrations, starts the chat server and the
voice server on ports of their own (so a development stack can keep running beside it), and then:

- registers a user, signs in, and reads the user back;
- makes a community with a voice channel and joins its call, which needs the voice server to
  have registered and reported in over NATS;
- reads both servers' metrics, including the allocator's figures;
- runs the voice server's `estimate-capacity`, whose self-test forwards real media through
  mediasoup, and checks the media arrived.

The deployment is `stack.Stack`'s: the database is dropped and the servers and NATS stopped
however it ends, and their logs are printed if a check fails. Needs Python 3.10, and `docker compose` with
the database, valkey, and seaweedfs services up (`--start-services` starts them) and the NATS
image the stack runs its own of.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

from stack import Failed, Ports, Stack, clean_env, start_services, wait_for, wait_for_services

PORTS = Ports(nats=14222, api=18000, voice=19001, api_metrics=19464, voice_metrics=19465, rtc_min=45000, rtc_max=45199,
              transfer=13478, relay_min=46000, relay_max=46099)


def say(message: str) -> None:
    print(f"smoke: {message}", flush=True)


def smoke(stack: Stack) -> None:
    api = stack.api
    say("registering, signing in, and reading the user back")
    name = f"smoke{os.getpid()}"
    api("POST", "/users", {"name": name, "password": "smoke-test-password"})
    token = api("POST", "/auth/login", {"username": name, "password": "smoke-test-password"})["sessionToken"]
    me = api("GET", "/users/@me", token=token)
    if me.get("name") != name:
        raise Failed(f"GET /users/@me answered {me}")

    say("refusing a wrong password in the language asked for")
    status, text = stack.request("POST", "/auth/login", {"username": name, "password": "wrong"},
                                 language="en-XA, en;q=0.5")
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
        status, text = stack.request("POST", f"/channels/{channel['id']}/voice/join", {}, token)
        offer = json.loads(text) if status == 200 else {}
        return status == 200

    wait_for("a voice server offer", joined, 90)
    if not any(c.get("id") == stack.voice_id for c in offer.get("candidates", [])):
        raise Failed(f"the join offered {offer.get('candidates')}, not the smoke voice server")

    say("reading both servers' metrics")
    for port, series in [(PORTS.api_metrics, "aspen_db_pool_connections"), (PORTS.voice_metrics, "aspen_voice_rooms")]:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/metrics", timeout=10) as response:
            text = response.read().decode()
        for wanted in [series, "aspen_memory_allocated_bytes"]:
            if wanted not in text:
                raise Failed(f"metrics on port {port} lack {wanted}")

    say("running estimate-capacity")
    estimated = subprocess.run(
        [str(stack.bins / "voice_server"), "estimate-capacity", "--calibration-seconds", "2", "--json"],
        cwd=stack.work, env=clean_env(), capture_output=True, text=True, timeout=300,
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
    if shutil.which("docker") is None:
        sys.exit("smoke: docker is required")
    if args.start_services:
        say("starting the database, Valkey, and SeaweedFS")
        start_services()
    try:
        wait_for_services()
        say("starting a database and a NATS of its own, the chat server, and the voice server")
        with Stack(args.bin.resolve(), "smoke", PORTS) as stack:
            smoke(stack)
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


if __name__ == "__main__":
    main()
