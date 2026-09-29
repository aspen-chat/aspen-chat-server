#!/usr/bin/env python3
"""Measures how quickly a deployment wakes every phone for one message that tags everyone in a
large community, against alpha of `dev_federation.py` (run `scripts/dev_federation.py up`
first, with `--alpha-bin target/release` for numbers worth comparing).

    scripts/bench_push.py [--members 10000] [--phones 1]

It seeds a community of `--members` people with the benchmark seeder, gives each of them a
sign-in and `--phones` phones whose push endpoint is a stand-in push service here (as
`dev_push.py` does), has the community's owner post `@everyone`, and reports how long the
deployment took to hand the stand-in its first push, half of them, and the last, then checks a
sample decrypts and removes everything it made with `bench purge`. With `--everyone-all`, every
member also asks to hear of every message in the community, so each one's setting is read as
well. The stand-in answers each push at once over kept-alive connections. Needs Python's
`cryptography`.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dev_federation import (  # noqa: E402
    ALPHA, REPO, Failed, api, bin_of, clean_env, psql, running_pid, say,
)
from dev_push import Phone, serve  # noqa: E402


class Sink(BaseHTTPRequestHandler):
    """The stand-in push service: notes when each push came, and keeps a few whole."""
    protocol_version = "HTTP/1.1"
    arrivals: list[float] = []
    kept: list[dict] = []
    lock = threading.Lock()

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        with self.lock:
            self.arrivals.append(time.monotonic())
            if len(self.kept) < 20:
                self.kept.append({"headers": {k.lower(): v for k, v in self.headers.items()}, "body": body})
        self.send_response(201)
        self.send_header("Content-Length", "0")
        self.end_headers()


def profile(members: int, run: str) -> str:
    return f"""name = "push-fanout-{run}"
description = "One community of {members} for the push fan-out measurement."

[target]
api = "https://{ALPHA.domain}"

[population]
users = {members}
communities = 1
largest_community = {members}
smallest_community = {members}
text_channels = 1
voice_channels = 0
history_per_channel = 0

[load]
online = 0.0
ramp_up_seconds = 1
duration_seconds = 1

[behaviours.idle]
share = 1.0
"""


def seed(members: int, run: str, bench: Path, work: Path) -> dict:
    (work / "profile.toml").write_text(profile(members, run))
    subprocess.run([str(bench / "aspen-bench"), "plan", str(work / "profile.toml"), "--run", run,
                    "--out", str(work / "plan.json")], check=True, capture_output=True)
    seeded = subprocess.run([str(bin_of(ALPHA) / "aspen-chat-server"), "bench", "seed", "--plan",
                             str(work / "plan.json"), "--out", str(work / "manifest.json")],
                            cwd=ALPHA.dir, env=clean_env(), capture_output=True, text=True)
    if seeded.returncode != 0:
        raise Failed(f"seeding failed: {seeded.stderr[-800:]}")
    return json.loads((work / "manifest.json").read_text())


def purge(run: str) -> None:
    purged = subprocess.run([str(bin_of(ALPHA) / "aspen-chat-server"), "bench", "purge", "--run", run],
                            cwd=ALPHA.dir, env=clean_env(), capture_output=True, text=True)
    if purged.returncode != 0:
        raise Failed(f"purging {run} failed: {purged.stderr[-800:]}")


def give_phones(run: str, phones: int, endpoint: str, phone: Phone) -> int:
    """A sign-in for every user of the run, and `phones` subscriptions on it, all to the stand-in."""
    users = f"SELECT id FROM \"user\" WHERE name LIKE 'bench-{run}-%'"
    psql(f"INSERT INTO refresh_token (token, expires, \"user\", verified_at, method) "
         f"SELECT 'bench-push-' || id, now() + interval '1 day', id, now(), 'password' FROM ({users}) u",
         ALPHA.database)
    psql(f"INSERT INTO push_subscription (id, \"user\", refresh_token, endpoint, p256dh, auth, push_key) "
         f"SELECT gen_random_uuid(), u.id, 'bench-push-' || u.id, '{endpoint}/' || u.id || '/' || n, "
         f"'\\x{phone.public.hex()}', '\\x{phone.auth.hex()}', "
         f"(SELECT id FROM push_key WHERE retired_at IS NULL) "
         f"FROM ({users}) u CROSS JOIN generate_series(1, {phones}) n", ALPHA.database)
    return int(psql("SELECT count(*) FROM push_subscription WHERE refresh_token LIKE 'bench-push-%'",
                    ALPHA.database))


def percentile(times: list[float], share: float) -> float:
    return times[min(len(times) - 1, int(share * len(times)))]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--members", type=int, default=10000)
    parser.add_argument("--phones", type=int, default=1, help="phones per member")
    parser.add_argument("--bench-bin", default=str(REPO / "target" / "release"),
                        help="directory holding aspen-bench, for the seeding plan")
    parser.add_argument("--timeout", type=float, default=300, help="seconds to wait for every push")
    parser.add_argument("--everyone-all", action="store_true",
                        help="give every member an every-message setting for the community")
    args = parser.parse_args()
    endpoint = serve(Sink)
    if running_pid(ALPHA) is None:
        say("alpha is not running; run `scripts/dev_federation.py up` first")
        return 1
    run = f"push{os.getpid()}"
    phone = Phone()
    with tempfile.TemporaryDirectory() as work:
        try:
            say(f"seeding a community of {args.members}")
            manifest = seed(args.members, run, Path(args.bench_bin), Path(work))
            expected = give_phones(run, args.phones, endpoint, phone)
            say(f"{expected} phones subscribed")
            community = manifest["communities"][0]
            if args.everyone_all:
                psql("INSERT INTO notification_setting (id, \"user\", community, level) "
                     f"SELECT gen_random_uuid(), id, '{community['id']}', 'all' FROM \"user\" "
                     f"WHERE name LIKE 'bench-{run}-%'", ALPHA.database)
            owner = manifest["users"][community["members"][0]]
            token = api(ALPHA, "POST", "/auth/login", {"username": owner["name"], "password": manifest["password"]})["sessionToken"]
            channel = community["textChannels"][0]
            # The owner is not among those woken, having written it.
            expected -= args.phones
            Sink.arrivals.clear()
            Sink.kept.clear()
            posted = time.monotonic()
            api(ALPHA, "POST", f"/channels/{channel}/messages", {"content": "@everyone the fan-out", "attachments": []}, token)
            answered = time.monotonic() - posted
            deadline = posted + args.timeout
            while len(Sink.arrivals) < expected and time.monotonic() < deadline:
                time.sleep(0.05)
            time.sleep(1)
            arrivals = sorted(t - posted for t in Sink.arrivals)
            if not arrivals:
                raise Failed("no push came")
            say(f"posting answered in {answered * 1000:.0f} ms")
            say(f"{len(arrivals)} of {expected} pushes: first {arrivals[0]:.2f} s, half {percentile(arrivals, 0.5):.2f} s, "
                f"99% {percentile(arrivals, 0.99):.2f} s, last {arrivals[-1]:.2f} s "
                f"({len(arrivals) / max(arrivals[-1] - arrivals[0], 1e-3):.0f} a second after the first)")
            for push in Sink.kept:
                pointer = phone.decrypt(push["body"])
                if pointer.get("kind") != "message":
                    raise Failed(f"a push carried {pointer}")
            say(f"ok: the {len(Sink.kept)} pushes kept decrypt to message pointers")
            if len(arrivals) != expected:
                raise Failed(f"{expected - len(arrivals)} phones were not woken within {args.timeout:.0f} s")
        finally:
            say("purging the run")
            purge(run)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Failed as failure:
        say(f"FAILED: {failure}")
        sys.exit(1)
