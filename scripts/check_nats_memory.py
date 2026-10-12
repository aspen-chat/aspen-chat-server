#!/usr/bin/env python3
"""Checks that the deployment's in-memory NATS streams are sized to their settings and to the
memory NATS has, against the services in docker-compose.yaml.

    scripts/check_nats_memory.py --bin target/debug

It runs a deployment of `stack.Stack`'s whose NATS has a memory limit and a monitoring port,
which the chat server is told of (`[nats] monitor_url`), and then checks that:

- left automatic, the two streams share everything NATS has, and the fleet shows it;
- a size set from the dashboard is the stream's size by the time the request answers, and the
  automatic stream takes what it leaves;
- a size NATS has no room for is refused, saying how much there is, and changes nothing;
- the terminal sets a size and makes it automatic again;
- after NATS restarts with another limit and no streams, the server makes them again, sized to
  the new limit.

Needs what `stack.Stack` needs.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

from stack import NATS_IMAGE, Failed, Ports, Stack, start_services, wait_for, wait_for_services

PORTS = Ports(nats=14422, api=18300, voice=19301, api_metrics=19764, voice_metrics=19765, rtc_min=45400,
              rtc_max=45599, transfer=13678, relay_min=46200, relay_max=46299)
MONITOR_PORT = 18322
MIB = 1 << 20
PASSWORD = "nats-memory-check-password"


def say(message: str) -> None:
    print(f"nats memory: {message}", flush=True)


class LimitedStack(Stack):
    """A stack whose NATS may keep `limit_mib` of streams in memory and says so on a monitoring port."""

    limit_mib = 300

    def nats_arguments(self) -> list[str]:
        config = self.work / "nats"
        config.mkdir(exist_ok=True)
        (config / "nats.conf").write_text(
            'authorization { token: "aspen_test" }\n'
            "http_port: 8222\n"
            f"jetstream {{ max_memory_store: {self.limit_mib}MB }}\n"
        )
        return ["-p", f"127.0.0.1:{MONITOR_PORT}:8222", "-v", f"{config}:/etc/aspen-nats:ro",
                NATS_IMAGE, "-c", "/etc/aspen-nats/nats.conf"]

    def restart_nats(self, limit_mib: int) -> None:
        """Starts NATS afresh, with no streams, able to keep `limit_mib` of them."""
        self.limit_mib = limit_mib
        subprocess.run(["docker", "rm", "-f", self.nats], check=True, capture_output=True)
        self.start_nats()


def sizes(stack: Stack, token: str) -> tuple[int | None, int | None]:
    """What the fleet says each stream may hold, events then voice reports, in MiB."""
    memory = stack.api("GET", "/admin/fleet", token=token)["natsMemory"]
    return tuple(stream and stream["maxBytes"] // MIB
                 for stream in (memory["eventStream"], memory["voiceReportStream"]))


def expect(what: str, got, wanted) -> None:
    if got != wanted:
        raise Failed(f"{what}: got {got}, wanted {wanted}")
    say(f"ok: {what}")


def check(stack: LimitedStack) -> None:
    name = f"natsmemory{os.getpid()}"
    stack.api("POST", "/users", {"name": name, "password": PASSWORD})
    stack.command("admin", "grant", name)
    token = stack.api("POST", "/auth/login", {"username": name, "password": PASSWORD})["sessionToken"]

    memory = stack.api("GET", "/admin/fleet", token=token)["natsMemory"]
    expect("the fleet reads the NATS server's limit", (memory["scope"], memory["limitBytes"]), ("server", 300 * MIB))
    expect("automatic streams reserve everything NATS has", memory["reservedBytes"], 300 * MIB)
    expect("automatic streams share it, an eighth to voice reports", sizes(stack, token), (263, 37))
    expect("and are shown as automatic",
           (memory["eventStream"]["automatic"], memory["voiceReportStream"]["automatic"]), (True, True))

    stack.api("PATCH", "/admin/settings", {"eventStreamMib": 100}, token=token)
    expect("a size set in the dashboard is the stream's at once, and the other takes the rest",
           sizes(stack, token), (100, 200))

    status, text = stack.request("PATCH", "/admin/settings", {"eventStreamMib": 400}, token=token)
    detail = json.loads(text).get("detail", "")
    expect("a size NATS has no room for is refused", status, 400)
    if "300 MiB" not in detail:
        raise Failed(f"the refusal does not say how much NATS has: {detail}")
    expect("and changes nothing", sizes(stack, token), (100, 200))
    expect("nor the setting", stack.api("GET", "/admin/settings", token=token)["eventStreamMib"], 100)

    expect("a size under the least is refused",
           stack.status("PATCH", "/admin/settings", {"voiceReportStreamMib": 8}, token=token), 400)

    stack.api("PATCH", "/admin/settings", {"eventStreamMib": None, "voiceReportStreamMib": 64}, token=token)
    expect("emptied, a size is automatic again", sizes(stack, token), (236, 64))

    shown = stack.command("settings", "set", "--event-stream-mib", "128", "--voice-report-stream-mib", "auto")
    if "event-stream-mib: 128" not in shown or "voice-report-stream-mib: auto" not in shown:
        raise Failed(f"`settings set` printed {shown}")
    expect("the terminal sets a size and makes one automatic", sizes(stack, token), (128, 172))
    stack.command("settings", "set", "--event-stream-mib", "auto")

    say("restarting NATS with 500 MiB and no streams")
    stack.restart_nats(500)
    # `fitStreams` runs every fifteen seconds, once the server's connection to NATS is back.
    def refitted() -> bool:
        # Slower than `wait_for` asks, which would run into the fleet's rate limit.
        time.sleep(2)
        status, text = stack.request("GET", "/admin/fleet", token=token)
        memory = json.loads(text).get("natsMemory") if status == 200 else None
        return bool(memory) and all(
            stream and stream["maxBytes"] == size * MIB
            for stream, size in ((memory["eventStream"], 438), (memory["voiceReportStream"], 62)))

    wait_for("the streams to be made again at the new limit", refitted, 90)
    say("ok: after NATS restarts with another limit, the streams are made again to fit it")


def main() -> None:
    parser = argparse.ArgumentParser(description="Check the NATS streams are sized to their settings and NATS's memory.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    args = parser.parse_args()
    if shutil.which("docker") is None:
        sys.exit("nats memory: docker is required")
    if args.start_services:
        start_services()
    try:
        wait_for_services()
        settings = f'[nats]\nmonitor_url = "http://127.0.0.1:{MONITOR_PORT}"\n'
        with LimitedStack(args.bin.resolve(), "natsmemory", PORTS, settings) as stack:
            check(stack)
        say("passed")
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)


if __name__ == "__main__":
    main()
