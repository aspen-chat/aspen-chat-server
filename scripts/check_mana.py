#!/usr/bin/env python3
"""Checks channel mana end to end against the services in docker-compose.yaml.

    scripts/check_mana.py --bin target/debug

It runs a deployment of `stack.Stack`'s whose pools hold 100 mana and recharge 10 a second, and
checks that:

- a message the pool can pay posts at once, and the next waits (`202`), behind anything waiting;
- a holder of Bypass channel mana posts at once;
- a viewer of the channel is told its level, maximum, and rate, and can read who spent it;
- what waits is posted as mana recharges, a member of higher rank before anything of a lower
  one's;
- an edit that lengthens waits and is then made, and one changing nothing is answered at once;
- a poll waits and is posted, and a thread reply posts while its echo waits in the parent;
- what waits two minutes is dropped, its author told why, and a viewer hears the priority queue
  come into force and end;
- a waiting message can be cancelled;
- past 50 waiting rows, sending is refused with `channelBusy`.

Needs what `stack.Stack` needs, and takes about four minutes, two of them the wait to be dropped.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
import time
from pathlib import Path

from stack import Failed, Ports, Stack, start_services, wait_for_services

PORTS = Ports(nats=14822, api=18800, voice=19801, api_metrics=19864, voice_metrics=19865, rtc_min=47300,
              rtc_max=47399, transfer=13878, relay_min=47400, relay_max=47499)
# The limits of one person's own would refuse the floods long before the channel's mana does.
SETTINGS = "[rate_limits]\nenabled = false\n"
PASSWORD = "channel-mana-check-password"
failed = False


def check(what, ok, detail=""):
    global failed
    print("channel mana: " + ("ok: " if ok else "FAIL: ") + what + (f": {detail}" if not ok else ""), flush=True)
    failed |= not ok


def until(what, probe, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        got = probe()
        if got:
            return got
        time.sleep(0.25)
    return None


def logs(stack):
    for _, log in stack.processes:
        if log.name == "api.log":
            print("----- api.log -----\n" + "\n".join(log.read_text().splitlines()[-60:]), flush=True)


def run(stack: Stack) -> None:
    suffix = str(os.getpid())

    def account(role):
        name = f"{role}{suffix}"
        stack.api("POST", "/users", {"name": name, "password": PASSWORD})
        signed = stack.api("POST", "/auth/login", {"username": name, "password": PASSWORD})
        return {"name": name, "token": signed["sessionToken"], "id": signed["userId"]}

    owner, m1, m2 = account("owner"), account("one"), account("two")
    made = stack.api("POST", "/communities", {"name": f"Mana {suffix}"}, owner["token"])
    community = made.get("id") or made["data"]["id"]
    invite = stack.api("POST", f"/communities/{community}/invites", {}, owner["token"])
    for member in (m1, m2):
        stack.api("PUT", f"/communities/{community}/members/@me", {"inviteCode": invite.get("code") or invite.get("id")},
                  member["token"])
    channel = stack.api("POST", "/channels", {"name": "busy", "ty": "text", "community": community, "sortIndex": 1},
                        owner["token"])["id"]
    stack.command("settings", "set", "--mana-max", "100", "--mana-max-recharge", "10")
    time.sleep(1.5)  # every server follows the settings through NATS

    def post(member, content, expect=(201, 202, 429)):
        status, text = stack.request("POST", f"/channels/{channel}/messages",
                                     {"content": content, "attachments": []}, member["token"])
        assert status in expect, (status, text)
        return status, json.loads(text) if text else {}

    def posted_texts():
        status, text = stack.request("GET", f"/channels/{channel}/messages?limit=100", token=owner["token"])
        body = json.loads(text)
        rows = body["data"] if isinstance(body, dict) else body
        return [m["content"] for m in sorted(rows, key=lambda m: m["id"])]

    def held(member):
        return stack.api("GET", "/users/@me/held-messages", token=member["token"])

    stream = stack.events(m2["token"])
    stream.gather(0.5)
    stream.send({"type": "viewing", "channelIds": [channel]})
    stream.gather(0.3)

    status, _ = post(m1, "a" * 90)
    check("a message the pool can pay posts at once", status == 201, status)
    status, waiting = post(m1, "b" * 50)
    check("one it cannot pay waits", status == 202 and waiting.get("waiting") == "mana", (status, waiting))
    status, waiting2 = post(m2, "c" * 20)
    check("anyone else waits behind it", status == 202, status)
    status, _ = post(owner, "owner speaks")
    check("the owner, holding Bypass, posts at once", status == 201, status)
    # Who spent: one 90-glyph message of the first member's, with its marker-free text, and the
    # owner's; the second member's waits, so has spent nothing yet.
    top = stack.api("GET", f"/channels/{channel}/mana/top-senders?include=users", token=m2["token"])
    spent = {row["user"]: row["mana"] for row in top["data"]}
    check("the top senders name who spent the channel's mana, and how much",
          spent.get(m1["id"]) == 90 and spent.get(owner["id"]) == len("owner speaks") and m2["id"] not in spent, spent)
    check("and sideload them", {u["id"] for u in top["included"].get("users", [])} >= {m1["id"], owner["id"]},
          top["included"])
    stream.gather(1.0)
    mana_frames = [e for e in stream.ephemeral if e.get("type") == "mana"]
    check("a viewer is told the channel's mana", bool(mana_frames), stream.ephemeral[-3:])
    if mana_frames:
        level = mana_frames[-1]
        check("the level names its maximum and rate", level["max"] == 100 and level["recharge"] == 10, level)

    got = until("the queue drains", lambda: {"b" * 50, "c" * 20} <= set(posted_texts()), 30)
    check("the drainer posts what waited as mana recharges", bool(got), posted_texts()[-4:])
    check("nothing is left waiting", not held(m1) and not held(m2), (held(m1), held(m2)))

    # Priority: one floods; another, given a role above everyone's, sends one after it, which
    # posts before any of the flood.
    regular = stack.api("POST", f"/communities/{community}/roles", {"name": "Regular", "permissions": []},
                        owner["token"])["id"]
    stack.api("PUT", f"/communities/{community}/members/{m2['id']}/roles/{regular}", token=owner["token"])
    post(m1, "x" * 100)  # empties the pool
    for i in range(6):
        post(m1, f"flood{i}" + "f" * 24)
    post(m2, "ranked" + "g" * 24)
    got = until("the ranked one posts", lambda: ("ranked" + "g" * 24) in posted_texts(), 60)
    texts = posted_texts()
    floods_before = sum(1 for t in texts[: texts.index("ranked" + "g" * 24)] if t.startswith("flood")) if got else -1
    check("a member of higher rank posts before anything of a lower one's waiting", got and floods_before == 0,
          floods_before)
    until("the flood drains", lambda: not held(m1), 60)

    # Edits: one lengthening waits while busy; one changing nothing is answered as it stands.
    time.sleep(11)  # the pool refills: 100 at 10 a second
    status, mine = post(m1, "short")
    check("a message posts once the channel is quiet again", status == 201, (status, mine))
    post(owner, "z" * 120)  # owner drains the pool into debt
    status, text = stack.request("PATCH", f"/messages/{mine['id']}", {"content": "short and then much longer " * 2},
                                 m1["token"])
    check("an edit that lengthens waits while the channel is busy", status == 202, (status, text[:200]))
    status, text = stack.request("PATCH", f"/messages/{mine['id']}", {"content": "short"}, m1["token"])
    check("an edit that changes nothing is answered with the message", status == 200, (status, text[:200]))

    got = until("the edit is made", lambda: any(t.startswith("short and then") for t in posted_texts()), 40)
    check("a waiting edit is made as mana recharges", bool(got), posted_texts()[-3:])

    # A poll waits as a message does, and posts.
    post(owner, "z" * 120)
    status, text = stack.request("POST", f"/channels/{channel}/polls", {
        "question": "Lunch?", "options": [{"label": "Yes"}, {"label": "No"}], "multipleChoice": False,
        "anonymous": False, "allowWriteIns": False, "durationSeconds": 3600}, m2["token"])
    check("a poll waits while the channel is busy", status == 202, (status, text[:200]))
    got = until("the poll posts", lambda: not held(m2), 40)
    check("and is posted as mana recharges", bool(got), held(m2))

    # An echo the parent cannot pay waits there, while its reply posts in the thread.
    time.sleep(11)
    starter = post(owner, "thread starter")[1]["id"]
    post(owner, "z" * 120)
    status, text = stack.request("POST", f"/messages/{starter}/thread/messages",
                                 {"content": "reply shown below too", "attachments": [], "echoToParent": True},
                                 m1["token"])
    check("a reply whose parent is busy posts in its thread", status == 201, (status, text[:200]))
    got = until("the echo is made", lambda: not held(m1), 40)
    check("and its echo waits in the parent, then is made", bool(got), held(m1))

    # What waits too long is dropped, its author told.
    slow = stack.api("POST", "/channels", {"name": "slow", "ty": "text", "community": community, "sortIndex": 2},
                     owner["token"])["id"]
    stack.api("PATCH", f"/channels/{slow}", {"manaMax": 100, "manaRecharge": 1}, owner["token"])
    stack.request("POST", f"/channels/{slow}/messages", {"content": "z" * 100, "attachments": []}, owner["token"])
    stack.request("POST", f"/channels/{slow}/messages", {"content": "z" * 100, "attachments": []}, owner["token"])
    status, text = stack.request("POST", f"/channels/{slow}/messages", {"content": "y" * 99, "attachments": []},
                                 m2["token"])
    check("a message waits in a slow channel", status == 202, (status, text[:200]))
    # A stream read throughout, so it answers the server's pings while the message waits; it
    # views the channel, so it hears the queue come into force and go.
    watching = stack.events(m2["token"])
    watching.send({"type": "viewing", "channelIds": [slow]})
    got = until("it is dropped", lambda: (watching.gather(0.25), not held(m2))[1], 150)
    watching.gather(2.0)
    failures = [e for e in watching.events if e.get("serverEvent") == "heldMessageFailed"]
    check("after two minutes it is dropped", bool(got), held(m2))
    check("and its author told why", bool(failures), "none")
    if failures:
        check("in words", "busy" in failures[-1]["detail"], failures[-1])
    levels = [e for e in watching.ephemeral if e.get("type") == "mana" and e.get("channelId") == slow]
    check("a viewer is told the priority queue is in force while a message waits",
          bool(levels) and levels[0].get("queued") is True, levels[:1])
    check("and that it is no longer once the queue empties", bool(levels) and levels[-1].get("queued") is False,
          levels[-1:])

    # Cancelling.
    post(owner, "z" * 150)
    status, queued = post(m2, "cancel me")
    if status == 202:
        status = stack.status("DELETE", f"/users/@me/held-messages/{queued['id']}", token=m2["token"])
        check("a waiting message can be cancelled", status == 204, status)
        check("and is gone", all(h["id"] != queued["id"] for h in held(m2)))
    else:
        check("a message waits to be cancelled", False, status)

    # One person's item is bounded.
    post(owner, "z" * 150)
    statuses = [post(m1, "q")[0] for _ in range(55)]
    check("past 50 waiting, sending is refused with channelBusy", 429 in statuses and statuses.count(202) <= 50,
          (statuses.count(202), statuses.count(429)))
    status, text = stack.request("POST", f"/channels/{channel}/messages", {"content": "q", "attachments": []},
                                 m1["token"])
    if status == 429:
        check("the refusal names channelBusy", json.loads(text)["code"] == "channelBusy", text[:200])


def main() -> None:
    parser = argparse.ArgumentParser(description="Check channel mana end to end.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    args = parser.parse_args()
    if shutil.which("docker") is None:
        sys.exit("channel mana: docker is required")
    if args.start_services:
        start_services()
    try:
        wait_for_services()
        with Stack(args.bin.resolve(), "mana", PORTS, SETTINGS) as stack:
            try:
                run(stack)
            except Exception:
                logs(stack)
                raise
    except Failed as failure:
        print(f"channel mana: FAILED: {failure}", flush=True)
        sys.exit(1)
    print("channel mana: " + ("FAILED" if failed else "passed"), flush=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
