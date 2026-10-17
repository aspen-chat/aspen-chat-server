"""Starting a box, bringing it in line with its `box.json`, and moving it to a new release."""

from __future__ import annotations

import argparse
import re
import time

from .common import BUCKET, HERE_VOICE, IMAGES, SLOTS, Box, Failed
from .database import external_database, require_postgres
from .render import render, voice_urls


def ready(store: Box, slot: str) -> bool:
    """Whether the API server in `slot` reaches everything it needs, asked from inside its own
    container."""
    return store.exec_ok(slot, "curl", "-fsS", "-o", "/dev/null", f"http://127.0.0.1:{SLOTS[slot]}/api/v1/health/ready")


def voice_answers(store: Box) -> bool:
    return store.exec_ok("voice", "curl", "-fsS", "-o", "/dev/null", "http://127.0.0.1:9000/health")


def wait_for(what: str, check, seconds: int = 120) -> None:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(2)
    raise Failed(f"{what} did not come up in {seconds} seconds: `aspen-box logs` shows why")


def release(store: Box, box: dict) -> None:
    """A release's one-off work, before any server of it starts: the tables it expects, and
    every voice server under the id its NATS user and its own configuration name."""
    store.compose("run", "--rm", "tool", "aspen-migrate", "up")
    urls = voice_urls(box)
    for server in box["voice_servers"]:
        store.compose(
            "run", "--rm", "tool", "aspen-chat-server", "voice-servers", "add", server["name"],
            "--url", urls[server["name"]], "--capacity", str(server["capacity"]), "--id", server["id"],
            capture=True,
        )  # fmt: skip


def converge(store: Box, box: dict) -> set:
    """Writes the configuration, starts what the servers need, and has each service that was
    already running take up a file of its that changed. Answers the files that changed."""
    was_running = store.running()
    changed = render(store, box)
    if external_database(box):
        # Before a release meets it: the one thing here the box did not start itself.
        require_postgres(store, box)
    store.compose("up", "-d", "--wait", *(() if external_database(box) else ("postgres",)), "valkey", "nats", "caddy")
    if changed & {"nats.conf", "nats-ca.pem"} and "nats" in was_running:
        store.compose("restart", "nats")
    if "Caddyfile" in changed and "caddy" in was_running:
        store.compose("exec", "-T", "caddy", "caddy", "reload", "--config", "/etc/caddy/Caddyfile")
    if box["storage"]["kind"] == "bundled":
        store.compose("up", "-d", "garage")
        if changed & {"garage.toml", "garage.env"} and "garage" in was_running:
            store.compose("restart", "garage")
        # The read path is Garage's web endpoint, which serves only buckets told to be served.
        wait_for("the bundled storage", lambda: store.exec_ok("garage", "/garage", "bucket", "website", "--allow", BUCKET))
    return changed


def roll(store: Box, box: dict, images: dict) -> None:
    """Moves the API to its other slot, on `images` and the configuration as written, without
    a moment in which neither answers; then saves the box as it now runs. The old server goes
    on serving if the new one does not come up."""
    old = box.get("active_slot", "api-a")
    new = next(slot for slot in SLOTS if slot != old)
    moved = {**box, "images": {**images, "api_idle": box["images"]["api"]}, "active_slot": new}
    render(store, moved)
    store.compose("up", "-d", "--force-recreate", new)
    try:
        wait_for("the new API server", lambda: ready(store, new))
    except Failed:
        store.compose("stop", new)
        render(store, box)
        raise Failed(
            f"the new API server did not come up, and the old one is still serving. `aspen-box logs {new}` "
            "shows why. The deployment is on the release it was on"
        ) from None
    store.save(moved)
    box.clear()
    box.update(moved)
    store.compose("stop", old)


def start_voice(store: Box, changed: set, was_running: set) -> None:
    """Starts the voice server, or restarts it when its configuration changed. Its calls end,
    and their clients rejoin on their own."""
    recreate = bool(changed & {"voice_server.toml", "nats-ca.pem"}) and "voice" in was_running
    store.compose("up", "-d", *(["--force-recreate"] if recreate else []), "voice")
    wait_for("the voice server", lambda: voice_answers(store))


def up(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if box.get("role") == "voice":
        return up_voice_only(store, box)
    was_running = store.running()
    changed = converge(store, box)
    release(store, box)
    slot = box.get("active_slot", "api-a")
    if slot in was_running and changed & {"aspen.toml", "database-ca.pem", "nats-ca.pem"}:
        roll(store, box, box["images"])
    else:
        store.compose("up", "-d", slot)
        wait_for("the API server", lambda: ready(store, slot))
    if any(server["here"] for server in box["voice_servers"]):
        start_voice(store, changed, was_running)
    print(f"\nhttps://{box['domain']} is up. `aspen-box doctor` checks what the internet sees of it.")


def up_voice_only(store: Box, box: dict) -> None:
    was_running = store.running()
    changed = render(store, box)
    store.compose("up", "-d", "--wait", "caddy")
    if "Caddyfile" in changed and "caddy" in was_running:
        store.compose("exec", "-T", "caddy", "caddy", "reload", "--config", "/etc/caddy/Caddyfile")
    start_voice(store, changed, was_running)
    print(f"\nhttps://{box['name']}.{box['domain']} is up. `aspen-box doctor` checks it.")


def retag(image: str, tag: str | None) -> str:
    return re.sub(r":[^:/]+$", f":{tag}", image) if tag else image


def published(tag: str) -> dict:
    return {target: f"{repository}:{tag}" for target, repository in IMAGES.items()}


def upgrade(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    current = box["images"]
    if box.get("role") == "voice":
        voice = opt.voice_image or retag(current["voice"], opt.tag)
        if voice == current["voice"]:
            raise Failed("give the release to move to: --tag, or --voice-image")
        box["images"] = {"voice": voice}
        store.save(box)
        render(store, box)
        store.compose("up", "-d", "voice")
        wait_for("the voice server", lambda: voice_answers(store))
        print(f"\nOn {voice}. Calls in progress here ended and rejoin.")
        return
    images = {"api": opt.api_image or retag(current["api"], opt.tag), "voice": opt.voice_image or retag(current["voice"], opt.tag)}
    if images["api"] == current["api"] and images["voice"] == current["voice"]:
        raise Failed("give the release to move to: --tag, or --api-image and --voice-image")
    converge(store, box)
    # The order docs/operators/installing/upgrading.md gives: the tables first, with the new
    # release's tools, which the old server goes on using while the new one starts.
    render(store, {**box, "images": {**current, "tool": images["api"]}})
    release(store, box)
    roll(store, box, images)
    if any(server["here"] for server in box["voice_servers"]):
        store.compose("up", "-d", "voice")
        wait_for("the voice server", lambda: voice_answers(store))
    remote = [server["name"] for server in box["voice_servers"] if not server["here"]]
    print(f"\nOn {images['api']}. The API did not stop; calls in progress here ended and rejoin.")
    if remote:
        print(f"Upgrade the voice servers elsewhere too, on their machines: {', '.join(remote)}.")


def here_voice(box: dict) -> dict | None:
    return next((server for server in box.get("voice_servers", []) if server["name"] == HERE_VOICE), None)
