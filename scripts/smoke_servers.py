#!/usr/bin/env python3
"""Checks that built Aspen servers start and serve, against the services in docker-compose.yaml.

    scripts/smoke_servers.py --bin target/release

It makes a database and a NATS of its own, runs the migrations, starts the chat server and the
voice server on ports of their own (so a development stack can keep running beside it), and then:

- asks the chat server whether it is up and ready (`GET /health`, `GET /health/ready`);
- registers a user, signs in, and reads the user back;
- uploads an SVG and waits for its preview, which the SVG renderer (`aspen-svg-render` from
  `--bin`, with the fonts `scripts/svg_fonts.py` builds, or those `ASPEN_SVG_FONTS` names) draws
  behind its sandbox, and checks its original is served to be saved;
- makes a community with a voice channel and joins its call, which needs the voice server to
  have registered and reported in over NATS;
- reads both servers' metrics, including the allocator's figures;
- runs the voice server's `estimate-capacity`, whose self-test forwards real media through a
  media thread of its own, and checks the media arrived;
- removes the voice server's registration while it runs, checks the dashboard's fleet lists it
  as reporting under an id that is not registered, and checks that started again it stops,
  saying its id is not registered.

With `--ul`, it checks aspen-ul from `--bin` instead (`stack.UlStack`), whose voice server is
within it: the call is joined the same way, the one metrics endpoint (`[server.metrics]`) holds
both servers' figures, `estimate-capacity`, a command of the voice server's own program alone, is
not run, and once the voice server's registration is removed while it runs the fleet lists it as
unregistered, and aspen-ul started again registers it anew and offers it for calls.

The deployment is `stack.py`'s: what it made is removed however it ends, and its logs are printed
if a check fails. Needs Python 3.10, and for the full deployment `docker compose` with the
database, valkey, and seaweedfs services up (`--start-services` starts them) and the NATS image the
stack runs its own of.
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

import stack as deployments
from stack import Deployment, Failed, Ports, UlStack, clean_env, wait_for

PORTS = Ports(nats=14222, api=18000, voice=19001, api_metrics=19464, voice_metrics=19465, rtc_min=45000, rtc_max=45199,
              transfer=13478, relay_min=46000, relay_max=46099)


def say(message: str) -> None:
    print(f"smoke: {message}", flush=True)


# An SVG of its own size, with text, which its preview must keep and draw.
SMOKE_SVG = (b'<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40">'
             b'<rect width="120" height="40" fill="teal"/>'
             b'<text x="8" y="28" font-size="20" fill="white">Smoke</text></svg>')


def svg_preview(stack: Deployment, token: str) -> None:
    """Uploads an SVG and waits for its preview: a WebP at the SVG's own size, which only a
    server whose renderer passed its check makes. Its original is served to be saved."""
    api = stack.api
    handle = api("POST", "/attachments", {"fileName": "smoke.svg", "mimeType": "image/svg+xml",
                                          "byteSize": len(SMOKE_SVG)}, token)
    put = urllib.request.Request(handle["uploadUrl"], data=SMOKE_SVG, method="PUT",
                                 headers={"content-type": "application/octet-stream"})
    urllib.request.urlopen(put, timeout=15).close()
    confirmed = api("POST", f"/attachments/{handle['id']}/confirm", token=token)
    if confirmed.get("inline") is not False:
        raise Failed(f"an SVG's record says it is served inline: {confirmed}")
    with urllib.request.urlopen(confirmed["downloadUrl"], timeout=15) as original:
        served = original.headers.get_content_type()
        if served != "application/octet-stream" or original.read() != SMOKE_SVG:
            raise Failed(f"an SVG's original is served as {served}, not to be saved")
    record: dict = {}

    def previewed() -> bool:
        nonlocal record
        record = api("GET", f"/attachments/{handle['id']}", token=token)
        return record.get("preview") is not None

    wait_for("the SVG's preview", previewed, 60)
    preview = record["preview"]
    if (preview["width"], preview["height"], preview["mimeType"]) != (120, 40, "image/webp"):
        raise Failed(f"an SVG's preview is {preview}, not a 120 by 40 WebP")
    with urllib.request.urlopen(preview["url"], timeout=15) as response:
        webp = response.read()
    if webp[:4] != b"RIFF" or webp[8:12] != b"WEBP":
        raise Failed(f"an SVG's preview is not WebP: {webp[:16]!r}")


def smoke(stack: Deployment) -> None:
    api = stack.api
    say("asking whether the chat server is up and ready")
    for path in ("/health", "/health/ready"):
        if (status := stack.status("GET", path)) != 204:
            raise Failed(f"GET {path} answered {status}, not 204")

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
    general = next(c for c in api("GET", f"/communities/{community_id}/channels", token=token)["data"] if c["ty"] == "text")
    posted = api("POST", f"/channels/{general['id']}/messages", {"content": "Smoke signals from the Tuesday picnic", "attachments": []}, token)
    api("POST", f"/channels/{general['id']}/messages", {"content": "Nothing about lunch here", "attachments": []}, token)
    found = api("GET", "/messages?filter[text]=PICNIC%20tuesday&include=channels", token=token)
    if [m["id"] for m in found["data"]] != [posted["id"]] or found["included"]["channels"][0]["id"] != general["id"]:
        raise Failed(f"searching for the picnic found {found}")
    left_out = api("GET", "/messages?filter[text]=smoke%20-picnic", token=token)
    if left_out["data"]:
        raise Failed(f"searching for smoke without the picnic found {left_out}")

    say("drawing an SVG's preview, which runs the SVG renderer behind its sandbox")
    svg_preview(stack, token)

    say("joining a call, which needs the voice server to have reported in")
    joined_call(stack, community_id, "Lounge", token)

    say("reading both servers' metrics")
    # aspen-ul's one endpoint holds both servers' figures.
    endpoints = ({PORTS.api_metrics: ["aspen_db_pool_connections", "aspen_voice_rooms"]} if stack.ul else
                 {PORTS.api_metrics: ["aspen_db_pool_connections"], PORTS.voice_metrics: ["aspen_voice_rooms"]})
    for port, series in endpoints.items():
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/metrics", timeout=10) as response:
            text = response.read().decode()
        for wanted in [*series, "aspen_memory_allocated_bytes"]:
            if wanted not in text:
                raise Failed(f"metrics on port {port} lack {wanted}")

    if isinstance(stack, UlStack):
        say("estimate-capacity: not for aspen-ul, a command of the voice server's own program alone")
        unregistered_ul(stack, name, token, community_id)
        return

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

    unregistered(stack, name, token)


def joined_call(stack: Deployment, community: str, name: str, token: str) -> None:
    """Joins a call in a new voice channel `name`, which the deployment's voice server must be
    offered for, once it has reported in."""
    channel = stack.api("POST", "/channels", {"community": community, "name": name, "sortIndex": 0, "ty": "voice"}, token)
    offer: dict = {}

    def joined() -> bool:
        nonlocal offer
        status, text = stack.request("POST", f"/channels/{channel['id']}/voice/join", {}, token)
        offer = json.loads(text) if status == 200 else {}
        return status == 200

    wait_for("a voice server offer", joined, 90)
    if not any(c.get("id") == stack.voice_id for c in offer.get("candidates", [])):
        raise Failed(f"the join offered {offer.get('candidates')}, not voice server {stack.voice_id}")


def unregistered(stack: Deployment, name: str, token: str) -> None:
    """A voice server reporting under an id that is not registered is shown, and refuses to start."""
    say("removing the voice server's registration while it runs")
    stack.command("admin", "grant", name)
    stack.command("voice-servers", "remove", stack.name)

    def listed() -> bool:
        fleet = stack.api("GET", "/admin/fleet", token=token)
        return any(s.get("id") == stack.voice_id for s in fleet.get("unregisteredVoiceServers", []))

    # Its next load report, within fifteen seconds, matches no registered server.
    wait_for("the fleet to list the unregistered voice server", listed, 45)

    say("starting the unregistered voice server again, which stops")
    restarted = stack.restart_voice()
    try:
        code = restarted.wait(timeout=60)
    except subprocess.TimeoutExpired:
        raise Failed("the unregistered voice server kept running") from None
    log = (stack.work / "voice-again.log").read_text()
    if code == 0 or "is not registered" not in log or stack.voice_id not in log:
        raise Failed(f"the unregistered voice server exited {code}, saying {log[-1500:]}")


def unregistered_ul(stack: UlStack, name: str, token: str, community: str) -> None:
    """aspen-ul whose voice server's registration is removed while it runs has the fleet list it
    as unregistered, as a voice server of its own would, and started again registers it anew and
    offers it for calls."""
    say("removing aspen-ul's voice server's registration while it runs")
    stack.command("admin", "grant", name)
    removed = stack.voice_id
    stack.command("voice-servers", "remove", "aspen-ul")

    def listed() -> bool:
        fleet = stack.api("GET", "/admin/fleet", token=token)
        return any(s.get("id") == removed for s in fleet.get("unregisteredVoiceServers", []))

    wait_for("the fleet to list the unregistered voice server", listed, 45)

    say("starting aspen-ul again, which registers its voice server anew")
    stack.restart("ul-again.log")
    if not stack.voice_id or stack.voice_id == removed:
        raise Failed(f"aspen-ul registered {stack.voice_id!r}, not a voice server of its own anew")
    joined_call(stack, community, "Again", token)


def main() -> None:
    parser = argparse.ArgumentParser(description="Check built Aspen servers start and serve.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    deployments.add_arguments(parser)
    args = parser.parse_args()
    if not args.ul and shutil.which("docker") is None:
        sys.exit("smoke: docker is required")
    # aspen-ul serves its metrics only when its settings say to, which a full deployment's
    # servers do on the ports the stack gives them.
    settings = f'[metrics]\nenabled = true\nlisten_addr = "127.0.0.1:{PORTS.api_metrics}"\n' if args.ul else ""
    try:
        deployments.prepare_services(args, args.start_services)
        say("starting aspen-ul" if args.ul else
            "starting a database and a NATS of its own, the chat server, and the voice server")
        with deployments.deployment(args, "smoke", PORTS, settings) as stack:
            smoke(stack)
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


if __name__ == "__main__":
    main()
