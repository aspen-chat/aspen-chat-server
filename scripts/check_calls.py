#!/usr/bin/env python3
"""Check real calls in real browsers against built servers.

Runs a deployment of its own (`stack.py`), whose web client is a page made of the client's own
call code (`client/packages/protocol/calls/`), and drives two people's browsers through it with
Playwright (`calls/drive.ts`): Ann in Chromium, Bob in Chromium and then in Firefox. Each hears
the other's microphone on an audio slot; a camera reaches Bob only once he shows video; a shared
screen and the camera arrive and decode side by side; and Ann leaving takes them away.

    scripts/check_calls.py --bin target/debug

It builds the page first, and needs the client's dependencies installed, Playwright's Chromium
and Firefox (`pnpm --filter @aspen/app exec playwright install chromium firefox`), and a sound
server (PulseAudio, or PipeWire's), without which Firefox plays and records only silence.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

from stack import Failed, Ports, Stack, clean_env, start_services, wait_for, wait_for_services
from web_client import PREVIEW_PICTURE

REPO = Path(__file__).resolve().parent.parent
PROTOCOL = REPO / "client" / "packages" / "protocol"
PAGE = PROTOCOL / "calls" / "dist"
PORTS = Ports(nats=14922, api=18900, voice=19901, api_metrics=19964, voice_metrics=19965, rtc_min=47600,
              rtc_max=47699, transfer=13978, relay_min=47700, relay_max=47799)
PASSWORD = "call-check-password"


def say(message: str) -> None:
    print(f"calls: {message}", flush=True)


def build_page() -> None:
    say("building the call page from the client's call code")
    built = subprocess.run(["pnpm", "build:calls"], cwd=PROTOCOL, capture_output=True, text=True)
    if built.returncode != 0:
        raise Failed(f"the call page did not build: {built.stdout[-2000:]}{built.stderr[-2000:]}")
    # A server serves a web client with the picture its link previews name.
    shutil.copyfile(PREVIEW_PICTURE, PAGE / "open-graph.png")


def person(stack: Stack, name: str) -> dict:
    stack.api("POST", "/users", {"name": name, "password": PASSWORD})
    signed = stack.api("POST", "/auth/login", {"username": name, "password": PASSWORD})
    return {key: signed[key] for key in ("userId", "sessionToken", "sessionTokenExpires", "refreshToken")}


def calls(stack: Stack) -> None:
    run = os.getpid()
    ann = person(stack, f"ann{run}")
    bob = person(stack, f"bob{run}")
    made = stack.api("POST", "/communities", {"name": "Calls"}, ann["sessionToken"])
    community = made.get("id") or made["data"]["id"]
    invite = stack.api("POST", f"/communities/{community}/invites", {}, ann["sessionToken"])
    stack.api("PUT", f"/communities/{community}/members/@me",
              {"inviteCode": invite.get("code") or invite.get("id")}, bob["sessionToken"])
    channel = stack.api("POST", "/channels", {"community": community, "name": "Lounge", "sortIndex": 0, "ty": "voice"},
                        ann["sessionToken"])["id"]
    wait_for("a voice server to be offered",
             lambda: stack.status("POST", f"/channels/{channel}/voice/join", {}, ann["sessionToken"]) == 200, 90)
    say("driving calls between browsers")
    check = json.dumps({"base": stack.base, "channel": channel, "ann": ann, "bob": bob})
    driven = subprocess.run(["pnpm", "calls"], cwd=PROTOCOL, env={**clean_env(), "ASPEN_CALL_CHECK": check}, text=True)
    if driven.returncode != 0:
        raise Failed("a call between browsers did not work as it should")


def main() -> None:
    parser = argparse.ArgumentParser(description="Check real calls in real browsers.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    args = parser.parse_args()
    if shutil.which("docker") is None:
        sys.exit("calls: docker is required")
    if args.start_services:
        say("starting the database, Valkey, and SeaweedFS")
        start_services()
    try:
        build_page()
        wait_for_services()
        say("starting a database and a NATS of its own, the chat server, and the voice server")
        with Stack(args.bin.resolve(), "calls", PORTS, web_client=PAGE) as stack:
            calls(stack)
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


if __name__ == "__main__":
    main()
