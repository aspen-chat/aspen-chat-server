#!/usr/bin/env python3
"""Checks that taking access away, or giving it, reaches everything already open.

    scripts/check_permissions.py --bin target/debug

Every change to who may see or do what is tried against a deployment of its own (`stack.Stack`),
watching each place someone on either side of the change could notice it: REST reads, an open
event stream, a call in progress on the voice server, the operator commands, and attachments.
`AGENTS.md`'s revocation checklist names what each feature must answer; this is where the
answers are checked. Add a scenario here with each feature that grants or shows something.

Needs Python 3.10 and nothing beyond its standard library, and `docker compose` with the
database, valkey, and seaweedfs services up (`--start-services` starts them) and the NATS image
the stack runs its own of.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import shutil
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
import zlib
from pathlib import Path

from stack import REPO, Failed, Ports, Stack, WebSocket, psql, start_services, wait_for, wait_for_services

PORTS = Ports(nats=14322, api=18100, voice=19101, api_metrics=19564, voice_metrics=19565, rtc_min=45200, rtc_max=45399,
              transfer=13578, relay_min=46100, relay_max=46199)
PASSWORD = "check-permissions-password"
# Scenarios make several accounts and many changes in moments, as no person would. Mail goes to
# an SMTP port nothing listens on, so what is queued stays in the outbox to be read.
SETTINGS = ("[rate_limits]\nenabled = false\n"
            "[email]\nsmtp_url = \"smtp://127.0.0.1:9\"\nfrom = \"Aspen <noreply@localhost>\"\n")


def say(message: str) -> None:
    print(f"permissions: {message}", flush=True)


class Checks:
    def __init__(self) -> None:
        self.failed: list[str] = []
        self.passed = 0

    def __call__(self, what: str, ok: bool, detail: object = "") -> None:
        if ok:
            self.passed += 1
            print(f"  ok    {what}", flush=True)
        else:
            self.failed.append(what)
            print(f"  FAIL  {what}{f'  ({detail})' if detail != '' else ''}", flush=True)


def soon(condition, seconds: float = 5) -> bool:
    """Whether `condition` holds within `seconds`."""
    deadline = time.monotonic() + seconds
    while not condition():
        if time.monotonic() > deadline:
            return False
        time.sleep(0.05)
    return True


def of(events: list[dict], kind: str, **fields) -> list[dict]:
    return [e for e in events if e.get("serverEvent") == kind and all(e.get(k) == v for k, v in fields.items())]


class World:
    """An owner and a member of one community, each signed in, the member's stream open."""

    def __init__(self, stack: Stack, run: str):
        self.stack = stack
        self.run = run
        self.owner = self.account("owner")
        self.member = self.account("member")
        made = stack.api("POST", "/communities", {"name": f"Permissions {run}"}, self.owner["token"])
        self.community = made.get("id") or made["data"]["id"]
        invite = stack.api("POST", f"/communities/{self.community}/invites", {}, self.owner["token"])
        stack.api("PUT", f"/communities/{self.community}/members/@me",
                  {"inviteCode": invite.get("code") or invite.get("id")}, self.member["token"])
        roles = stack.api("GET", f"/communities/{self.community}/roles", token=self.owner["token"])
        self.everyone = next(r["id"] for r in roles if r["everyone"])
        self.stream = stack.events(self.member["token"])
        self.stream.gather(0.5)

    def account(self, role: str) -> dict:
        name = f"{role}{self.run}"
        user = self.stack.api("POST", "/users", {"name": name, "password": PASSWORD})
        return {"id": user["id"], "name": name, **self.sign_in(name)}

    def sign_in(self, name: str, password: str = PASSWORD) -> dict:
        signed = self.stack.api("POST", "/auth/login", {"username": name, "password": password})
        return {"token": signed["sessionToken"], "refresh": signed["refreshToken"]}

    def as_owner(self, method: str, path: str, body: dict | None = None):
        return self.stack.api(method, path, body, self.owner["token"])

    def channel(self, name: str, ty: str = "text", overrides: list | None = None, category: str | None = None) -> str:
        body = {"name": name, "ty": ty, "community": self.community, "sortIndex": 1}
        if overrides is not None:
            body["overrides"] = overrides
        if category is not None:
            body["parentCategory"] = category
        return self.as_owner("POST", "/channels", body)["id"]

    def role(self, name: str, permissions: list[str] | None = None) -> str:
        return self.as_owner("POST", f"/communities/{self.community}/roles",
                             {"name": name, "permissions": permissions or []})["id"]

    def give(self, role: str) -> None:
        self.as_owner("PUT", f"/communities/{self.community}/members/{self.member['id']}/roles/{role}")

    def post(self, channel: str, content: str) -> str:
        return self.as_owner("POST", f"/channels/{channel}/messages", {"content": content, "attachments": []})["id"]

    def member_sees(self, channel: str) -> bool:
        return self.stack.status("GET", f"/channels/{channel}", token=self.member["token"]) == 200

    def listed(self, channel: str) -> bool:
        read = self.stack.api("GET", f"/communities/{self.community}?include=channels,roles",
                              token=self.member["token"])
        included = read.get("included", {})
        return any(c["id"] == channel for c in included.get("channels", [])) or any(
            o["channel"] == channel for o in included.get("channelOverrides", []))

    def hears_message(self, channel: str) -> bool:
        posted = self.post(channel, "can you hear this?")
        return bool(of(self.stream.gather(1.0), "message", id=posted))


def private_channels(world: World, check: Checks) -> None:
    say("a channel made private")
    secret = world.channel("secret", overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    got = world.stream.gather(1.0)
    check("its creation and overrides do not reach the member",
          not of(got, "channel") and not of(got, "channelOverride"), [e["serverEvent"] for e in got])
    check("the member cannot read it", not world.member_sees(secret))
    check("nor find it, or its overrides, in the community's read", not world.listed(secret))
    check("nor read its read state",
          world.stack.status("GET", f"/channels/{secret}/read-states/@me", token=world.member["token"]) == 404)
    check("nor hear what is said in it", not world.hears_message(secret))


def granting_and_revoking(world: World, check: Checks) -> None:
    say("access given by a role and an override, then taken away")
    staff = world.role("Staff")
    secret = world.channel("staff-only", overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": ["viewChannel"], "deny": []})
    check("an override for a role the member lacks does not reach them",
          not of(world.stream.gather(0.8), "channelOverride"))
    world.give(staff)
    world.stream.gather(0.8)
    check("given the role, the member reads the channel", world.member_sees(secret))
    check("and finds it in the community's read", world.listed(secret))
    check("and hears what is said in it", world.hears_message(secret))
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": [], "deny": ["viewChannel"]})
    got = world.stream.gather(1.0)
    check("an override shutting the member out reaches them, so they can let it go",
          len(of(got, "channelOverride", channel=secret)) == 1, [e["serverEvent"] for e in got])
    check("and then they cannot read it", not world.member_sees(secret))
    check("nor hear it", not world.hears_message(secret))
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": ["viewChannel"], "deny": []})
    got = world.stream.gather(1.0)
    check("letting the role back in reaches the member, about a channel they did not have",
          len(of(got, "channelOverride", channel=secret)) == 1 and world.member_sees(secret))
    world.as_owner("DELETE", f"/roles/{staff}")
    got = world.stream.gather(1.0)
    check("deleting the role reaches the member", bool(of(got, "role", id=staff)))
    check("and with it they cannot read the channel", not world.member_sees(secret))
    check("nor hear it", not world.hears_message(secret))


def moves_and_categories(world: World, check: Checks) -> None:
    say("a channel moved into a hidden category and out, and the category deleted")
    hidden = world.as_owner("POST", f"/communities/{world.community}/categories", {"name": "Mods", "sortIndex": 9})["id"]
    world.as_owner("PUT", f"/categories/{hidden}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    lounge = world.channel("lounge")
    world.stream.gather(0.8)
    world.as_owner("PATCH", f"/channels/{lounge}", {"parentCategory": hidden})
    check("the move into the hidden category reaches the member once",
          len(of(world.stream.gather(1.0), "channel", id=lounge)) == 1)
    world.as_owner("PATCH", f"/channels/{lounge}", {"name": "lounge-renamed"})
    check("and nothing about it after", not of(world.stream.gather(0.8), "channel"))
    check("nor what is said in it", not world.hears_message(lounge))
    world.as_owner("PATCH", f"/channels/{lounge}", {"parentCategory": None})
    check("moving it out reaches the member again", len(of(world.stream.gather(1.0), "channel", id=lounge)) == 1)
    world.as_owner("PATCH", f"/channels/{lounge}", {"parentCategory": hidden})
    world.stream.gather(0.8)
    world.as_owner("DELETE", f"/categories/{hidden}")
    got = world.stream.gather(1.0)
    check("deleting the category moves its channel out, into the member's view",
          any(e.get("parentCategory", "unset") is None for e in of(got, "channel", id=lounge)) and world.member_sees(lounge))


def hidden_managers(world: World, check: Checks) -> None:
    say("managing channels and categories hidden from the manager")
    member = world.member["token"]
    world.give(world.role("Managers", ["manageChannels", "manageCategories"]))
    secret = world.channel("not-for-managers", overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    world.stream.gather(0.8)
    unhide = {"allow": ["viewChannel"], "deny": []}
    check("Manage channels does not reach an override of a channel its holder may not view",
          world.stack.status("PUT", f"/channels/{secret}/overrides/{world.everyone}", unhide, member) == 404)
    check("nor clear one",
          world.stack.status("DELETE", f"/channels/{secret}/overrides/{world.everyone}", token=member) == 404)
    check("nor rename, move, or delete the channel",
          world.stack.status("PATCH", f"/channels/{secret}", {"name": "found"}, member) == 404
          and world.stack.status("PATCH", f"/channels/{secret}", {"parentCategory": None}, member) == 404
          and world.stack.status("DELETE", f"/channels/{secret}", token=member) == 404)
    check("which stays hidden from them", not world.member_sees(secret))
    hidden = world.as_owner("POST", f"/communities/{world.community}/categories", {"name": "Hidden", "sortIndex": 9})["id"]
    world.as_owner("PUT", f"/categories/{hidden}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    inside = world.channel("inside-hidden", category=hidden)
    world.stream.gather(0.8)
    check("Manage categories does not reach the overrides of a category that hides its channels from its holder",
          world.stack.status("PUT", f"/categories/{hidden}/overrides/{world.everyone}", unhide, member) == 403
          and world.stack.status("DELETE", f"/categories/{hidden}/overrides/{world.everyone}", token=member) == 403)
    check("nor delete that category, which would move its channels into view",
          world.stack.status("DELETE", f"/categories/{hidden}", token=member) == 403)
    check("whose channels stay hidden from them", not world.member_sees(inside))
    open_channel = world.channel("for-managers")
    check("while a channel they may view is theirs to manage",
          world.stack.status("PATCH", f"/channels/{open_channel}", {"name": "managed"}, member) == 200)


def role_grants(world: World, check: Checks) -> None:
    say("giving a role whose permissions the giver lacks, in a community and the deployment")
    stack, member = world.stack, world.member
    world.give(world.role("Assigners", ["assignRoles"]))
    # Each role is made just above everyone's, so these two rank below Assigners.
    powerful = world.role("Powerful", ["manageChannels"])
    plain = world.role("Plain")
    world.stream.gather(0.5)
    mine = f"/communities/{world.community}/members/{member['id']}/roles"
    check("Assign roles does not give a role allowing what its holder lacks",
          stack.status("PUT", f"{mine}/{powerful}", token=member["token"]) == 403)
    check("but gives one allowing nothing more", stack.status("PUT", f"{mine}/{plain}", token=member["token"]) == 201)
    world.give(powerful)
    check("and takes one away by rank alone", stack.status("DELETE", f"{mine}/{powerful}", token=member["token"]) == 204)
    stack.command("admin", "grant", world.owner["name"])
    keepers = world.as_owner("POST", "/admin/roles", {"name": f"Keepers{world.run}",
                                                      "permissions": ["manageDeploymentRoles"]})["id"]
    settings = world.as_owner("POST", "/admin/roles", {"name": f"Settings{world.run}",
                                                       "permissions": ["manageDeploymentSettings"]})["id"]
    world.as_owner("PUT", f"/admin/users/{member['id']}/roles/{keepers}")
    theirs = f"/admin/users/{member['id']}/roles/{settings}"
    check("Manage deployment roles does not give a role allowing what its holder lacks",
          stack.status("PUT", theirs, token=member["token"]) == 403)
    world.as_owner("PUT", theirs)
    check("but takes one away by rank alone", stack.status("DELETE", theirs, token=member["token"]) == 204)
    world.as_owner("DELETE", f"/admin/roles/{settings}")
    world.as_owner("DELETE", f"/admin/roles/{keepers}")
    stack.command("admin", "revoke", world.owner["name"])


def poll_votes(world: World, check: Checks) -> None:
    say("a vote withdrawn after the poll's channel is hidden")
    stack, member = world.stack, world.member
    ballot = world.channel("ballot")
    poll = world.as_owner("POST", f"/channels/{ballot}/polls", {
        "question": "Which?", "options": [{"label": "this"}, {"label": "that"}], "multipleChoice": False,
        "allowWriteIns": False, "anonymous": False, "durationSeconds": 3600})["id"]
    vote = f"/polls/{poll}/votes/0/@me"
    check("the member votes", stack.status("PUT", vote, token=member["token"]) == 201)
    world.as_owner("PUT", f"/channels/{ballot}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    check("hidden from the poll's channel, they cannot withdraw their vote",
          stack.status("DELETE", vote, token=member["token"]) == 404)
    check("which still counts", world.as_owner("GET", f"/polls/{poll}")["data"]["results"][0]["count"] == 1)


def deleted_parents(world: World, check: Checks) -> None:
    say("a thread whose channel is deleted")
    stack, member = world.stack, world.member["token"]
    doomed = world.channel("doomed")
    starter = world.post(doomed, "start a thread here")
    thread = stack.api("PUT", f"/messages/{starter}/thread", token=member)["id"]
    reply = stack.api("POST", f"/channels/{thread}/messages", {"content": "a reply", "attachments": []}, member)["id"]
    world.as_owner("DELETE", f"/channels/{doomed}")
    world.stream.gather(0.8)
    check("goes with it: the thread is not found",
          stack.status("GET", f"/channels/{thread}", token=member) == 404
          and stack.status("GET", f"/channels/{thread}/messages", token=member) == 404)
    check("nor is a reply in it", stack.status("GET", f"/messages/{reply}", token=member) == 404)
    check("and nobody posts in it",
          stack.status("POST", f"/channels/{thread}/messages", {"content": "still here?", "attachments": []},
                       member) == 404)


def thread_echoes(world: World, check: Checks) -> None:
    say("a thread reply echoed to its channel after it was posted")
    member = world.member["token"]
    talk = world.channel("talk")
    starter = world.post(talk, "start a thread here")
    thread = world.stack.api("PUT", f"/messages/{starter}/thread", token=member)["id"]
    reply = world.stack.api("POST", f"/channels/{thread}/messages", {"content": "a reply", "attachments": []},
                            member)["id"]
    world.stream.gather(0.8)
    check("someone other than the reply's author cannot echo it",
          world.stack.status("PUT", f"/messages/{reply}/echo", token=world.owner["token"]) == 403)
    world.as_owner("PUT", f"/channels/{talk}/overrides/{world.everyone}", {"allow": [], "deny": ["sendMessages"]})
    world.stream.gather(0.8)
    check("nor can its author while they may not send messages in the channel",
          world.stack.status("PUT", f"/messages/{reply}/echo", token=member) == 403)
    world.as_owner("DELETE", f"/channels/{talk}/overrides/{world.everyone}")
    world.stream.gather(0.8)
    echo = world.stack.api("PUT", f"/messages/{reply}/echo", token=member)
    got = world.stream.gather(1.0)
    check("once they may, the echo is made in the channel",
          echo.get("kind") == "threadEcho" and echo.get("echoOf") == reply and echo.get("channelId") == talk)
    check("and its creation and the reply's new echo reach the stream",
          bool(of(got, "message", id=echo["id"], type="create"))
          and bool(of(got, "message", id=reply, type="update", echo=echo["id"])), got)
    check("echoing it again answers the same echo",
          world.stack.api("PUT", f"/messages/{reply}/echo", token=member).get("id") == echo["id"])
    world.as_owner("DELETE", f"/messages/{echo['id']}")
    got = world.stream.gather(1.0)
    check("a moderator deleting the echo alone frees the reply, announced",
          bool(of(got, "message", id=echo["id"], type="delete"))
          and bool(of(got, "message", id=reply, type="update", echo=None))
          and world.stack.api("GET", f"/messages/{reply}", token=member)["data"].get("echo") is None, got)
    again = world.stack.api("PUT", f"/messages/{reply}/echo", token=member)
    check("after which its author may echo it again", again.get("id") not in (None, echo["id"]))
    world.stream.gather(0.8)
    world.as_owner("DELETE", f"/messages/{reply}")
    got = world.stream.gather(1.0)
    check("deleting the reply takes its echo with it",
          bool(of(got, "message", id=again["id"], type="delete"))
          and world.stack.status("GET", f"/messages/{again['id']}", token=member) == 404)

def join(token: str) -> WebSocket:
    """Joins a call on the voice server with a join token."""
    socket = WebSocket(f"ws://127.0.0.1:{PORTS.voice}/ws")
    socket.send({"type": "identify", "token": token})
    return socket


def frame_of(socket: WebSocket, kind: str, seconds: float = 5) -> dict | None:
    """The next frame of `kind` the voice server sends within `seconds`."""
    deadline = time.monotonic() + seconds
    while (left := deadline - time.monotonic()) > 0:
        frame = socket.receive(left)
        if frame is None and socket.closed is not None:
            return None
        if frame is not None and frame.get("type") == kind:
            return frame
    return None


def in_call(world: World) -> bool:
    """Whether the community's record of its calls shows the member in one."""
    read = world.stack.api("GET", f"/communities/{world.community}?include=voice", token=world.owner["token"])
    return any(p["user"] == world.member["id"] for p in read.get("included", {}).get("voiceParticipants", []))


def calls(world: World, check: Checks) -> None:
    say("a call in progress as what the member may do there changes")
    stack = world.stack
    room = world.channel("call", ty="voice")
    world.stream.gather(0.5)

    def offer() -> dict:
        return stack.api("POST", f"/channels/{room}/voice/join", {}, world.member["token"])

    # The voice server is offered once it has reported in.
    wait_for("a voice server offer",
             lambda: stack.status("POST", f"/channels/{room}/voice/join", {}, world.member["token"]) == 200, 90)

    first = offer()
    server = first["candidates"][0]["id"]
    stranger = stack.api("POST", f"/voice-servers/{server}/failures", None, world.owner["token"])
    check("a failure report from someone no offer sent to the voice server counts for nothing",
          stranger.get("counted") is False and stranger.get("failures") == 0, stranger)
    offered = stack.api("POST", f"/voice-servers/{server}/failures", None, world.member["token"])
    check("one from someone an offer sent there counts", offered.get("counted") is True, offered)
    call = join(first["token"])
    check("the member joins the call", frame_of(call, "ready") is not None)
    wait_for("the call's record to show the member", lambda: in_call(world), 30)
    stack.api("PATCH", f"/channels/{room}/voice/participants/{world.member['id']}", {"muted": True},
              world.owner["token"], expect=(202,))
    muted = frame_of(call, "participantState")
    check("a moderator's mute reaches the call", muted is not None and muted["muted"] is True, muted)
    call.send({"type": "setState", "muted": False, "deafened": False})
    still = frame_of(call, "participantState")
    check("and the member cannot unmute themself", still is not None and still["muted"] is True, still)
    stack.api("PATCH", f"/channels/{room}/voice/participants/{world.member['id']}", {"muted": False},
              world.owner["token"], expect=(202,))
    lifted = frame_of(call, "participantState")
    check("until the moderator unmutes them", lifted is not None and lifted["muted"] is False, lifted)
    world.as_owner("PUT", f"/channels/{room}/overrides/{world.everyone}", {"allow": [], "deny": ["speak"]})
    changed = frame_of(call, "grantsChanged")
    check("taking Speak away reaches the call at once", changed is not None and not changed["grants"]["speak"], changed)
    replay = join(first["token"])
    refused = frame_of(replay, "error")
    check("the token they joined with admits no second connection",
          refused is not None and refused.get("fatal") is True, refused)
    replay.close()
    world.as_owner("PUT", f"/channels/{room}/overrides/{world.everyone}", {"allow": [], "deny": ["joinVoice"]})
    kicked = frame_of(call, "kicked")
    check("taking Join voice away removes them, saying why",
          kicked is not None and kicked.get("reason") == "accessLost", kicked)
    check("and closes their socket, so it can act in the call no more",
          soon(lambda: call.receive(0.2) is None and call.closed is not None, 10), call.closed)
    call.close()
    # A token issued before a change is brought in line once its join is recorded.
    world.as_owner("DELETE", f"/channels/{room}/overrides/{world.everyone}")
    early = offer()
    world.as_owner("PUT", f"/channels/{room}/overrides/{world.everyone}", {"allow": [], "deny": ["speak"]})
    late = join(early["token"])
    check("a join on a token from before the change succeeds", frame_of(late, "ready") is not None)
    changed = frame_of(late, "grantsChanged", 15)
    check("and is then told it may not speak", changed is not None and not changed["grants"]["speak"], changed)
    world.as_owner("DELETE", f"/channels/{room}/overrides/{world.everyone}")
    frame_of(late, "grantsChanged", 5)
    world.as_owner("PUT", f"/communities/{world.community}/bans/{world.member['id']}", {})
    kicked = frame_of(late, "kicked", 10)
    check("a ban from the community removes them from its calls",
          kicked is not None and kicked.get("reason") == "accessLost", kicked)
    late.close()
    world.as_owner("DELETE", f"/communities/{world.community}/bans/{world.member['id']}")


def attachments(world: World, check: Checks) -> None:
    say("an attachment, before it is sent and after")
    stack = world.stack
    handle = world.as_owner("POST", "/attachments", {"fileName": "notes.txt", "mimeType": "text/plain"})
    put = urllib.request.Request(handle["uploadUrl"], data=b"secret notes", method="PUT",
                                 headers={"content-type": "text/plain"})
    urllib.request.urlopen(put, timeout=15).close()
    confirmed = world.as_owner("POST", f"/attachments/{handle['id']}/confirm")
    path = f"/attachments/{handle['id']}"
    again = urllib.request.Request(handle["uploadUrl"], data=b"swapped notes", method="PUT",
                                   headers={"content-type": "text/plain"})
    urllib.request.urlopen(again, timeout=15).close()
    with urllib.request.urlopen(confirmed["downloadUrl"], timeout=15) as read:
        check("its upload link, used again once it is confirmed, changes nothing anyone reads",
              read.read() == b"secret notes")
    check("its uploader reads it", stack.status("GET", path, token=world.owner["token"]) == 200)
    check("nobody else reads it before it is sent", stack.status("GET", path, token=world.member["token"]) == 404)
    check("nor deletes it", stack.status("DELETE", path, token=world.member["token"]) == 404)
    check("nor describes it",
          stack.status("PATCH", path, {"description": "theirs"}, world.member["token"]) == 404)
    check("its uploader describes it",
          stack.status("PATCH", path, {"description": "my notes"}, world.owner["token"]) == 200)
    general = world.channel("pictures")
    posting = {"content": "mine now", "attachments": [handle["id"]]}
    check("nor sends it as their own",
          stack.status("POST", f"/channels/{general}/messages", posting, world.member["token"]) == 400)
    world.as_owner("POST", f"/channels/{general}/messages", {"content": "notes", "attachments": [handle["id"]]})
    check("once sent, whoever may view the channel reads it",
          stack.status("GET", path, token=world.member["token"]) == 200)
    check("and its description stays as it was sent",
          stack.status("PATCH", path, {"description": "changed"}, world.owner["token"]) == 404)
    world.as_owner("PUT", f"/channels/{general}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    check("and nobody once they may not", stack.status("GET", path, token=world.member["token"]) == 404)


def uploads(world: World, check: Checks) -> None:
    say("uploads: their size, and what a browser opening one may run")
    stack = world.stack
    page = b"<html><script>alert(document.domain)</script></html>"
    handle = world.as_owner("POST", "/attachments",
                            {"fileName": "page.html", "mimeType": "text/html", "byteSize": len(page)})
    check("a page is uploaded as a file to save", handle.get("contentType") == "application/octet-stream", handle)

    def put(data: bytes, content_type: str) -> int:
        request = urllib.request.Request(handle["uploadUrl"], data=data, method="PUT",
                                         headers={"content-type": content_type})
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                return response.status
        except urllib.error.HTTPError as error:
            return error.code
    check("storage refuses it sent as a page", put(page, "text/html") == 403)
    check("or longer than declared", put(page + b" ", "application/octet-stream") == 403)
    check("and takes it as declared", put(page, "application/octet-stream") == 200)
    confirmed = world.as_owner("POST", f"/attachments/{handle['id']}/confirm")
    check("its record keeps the type its sender gave", confirmed["mimeType"] == "text/html")
    with urllib.request.urlopen(confirmed["downloadUrl"], timeout=15) as read:
        check("and it is served as a file, not a page",
              read.headers.get_content_type() == "application/octet-stream")
    check("a file over the deployment's limit is refused before it is sent",
          stack.status("POST", "/attachments", {"fileName": "huge.bin", "mimeType": "application/zip",
                                                "byteSize": 1 << 40}, world.owner["token"]) == 400)
    check("as is an icon over 8 MiB",
          stack.status("POST", "/icons", {"mimeType": "image/png", "byteSize": 9 << 20},
                       world.owner["token"]) == 400)


def picture(width: int = 1600, height: int = 1200) -> bytes:
    """A PNG with detail everywhere, as a photo has, large enough that its preview is kept."""
    rows = b"".join(
        b"\x00" + bytes(((x * 7 + y * 3) & 0xFF, (x ^ y) & 0xFF, (x * y >> 4) & 0xFF)[i % 3]
                         for x in range(width) for i in range(3))
        for y in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows, 6))
            + chunk(b"IEND", b""))


def upload_picture(world: World, token: str, png: bytes) -> str:
    handle = world.stack.api("POST", "/attachments", {"fileName": "photo.png", "mimeType": "image/png"}, token)
    put = urllib.request.Request(handle["uploadUrl"], data=png, method="PUT", headers={"content-type": "image/png"})
    urllib.request.urlopen(put, timeout=15).close()
    world.stack.api("POST", f"/attachments/{handle['id']}/confirm", token=token)
    return handle["id"]


def previews(world: World, check: Checks) -> None:
    say("previews of pictures, and messages held for them")
    stack = world.stack
    png = picture()
    secret = world.channel("secret previews",
                           overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    hidden = upload_picture(world, world.owner["token"], png)
    world.as_owner("POST", f"/channels/{secret}/messages", {"content": "hidden", "attachments": [hidden]})
    made = soon(lambda: "preview" in stack.api("GET", f"/attachments/{hidden}", token=world.owner["token"]), 30)
    check("a preview is made of a picture sent", made)
    got = world.stream.gather(1.0)
    check("its event does not reach a member who may not view the channel",
          not of(got, "attachmentPreviewed", attachment=hidden), [e["serverEvent"] for e in got])
    check("nor may they read it", stack.status("GET", f"/attachments/{hidden}", token=world.member["token"]) == 404)

    shared = world.channel("shared previews")
    world.stream.gather(0.5)
    mine = upload_picture(world, world.owner["token"], png)
    status, _ = stack.request("POST", f"/channels/{shared}/messages",
                              {"content": "held", "attachments": [mine], "mayHold": True}, world.owner["token"])
    check("a message sent while its picture's preview is made is held", status == 202, status)
    first = []
    soon(lambda: bool(first.extend(world.stream.gather(0.2)) or of(first, "message", content="held")), 30)
    check("nobody else hears of it until it is posted",
          not of(first, "heldMessagePosted") and not of(first, "heldMessageFailed"),
          [e["serverEvent"] for e in first])
    check("and then they hear of it, with its preview on the attachment",
          bool(of(first, "message", content="held"))
          and "preview" in stack.api("GET", f"/attachments/{mine}", token=world.member["token"]))

    theirs = upload_picture(world, world.member["token"], png)
    status, body = stack.request("POST", f"/channels/{shared}/messages",
                                 {"content": "not allowed", "attachments": [theirs], "mayHold": True},
                                 world.member["token"])
    held = json.loads(body) if status == 202 else {}
    check("the member's own is held too", status == 202, status)
    world.as_owner("PUT", f"/channels/{shared}/overrides/{world.everyone}", {"allow": [], "deny": ["sendMessages"]})
    owner_stream = stack.events(world.owner["token"])
    seen = []
    dropped = soon(lambda: bool(seen.extend(world.stream.gather(0.2))
                                or of(seen, "heldMessageFailed", held=held.get("id"))), 30)
    check("losing the right to post drops it, and its author is told why",
          dropped and bool(of(seen, "heldMessageFailed", held=held.get("id"))[0].get("detail")),
          [e["serverEvent"] for e in seen])
    check("and nobody else ever sees it", not of(owner_stream.gather(1.0), "message", content="not allowed"))
    owner_stream.close()
    check("nor is it in the channel",
          all(m["content"] != "not allowed" for m in
              stack.api("GET", f"/channels/{shared}/messages", token=world.owner["token"])["data"]))


def upload_icon(world: World, token: str, png: bytes) -> str:
    handle = world.stack.api("POST", "/icons", {"mimeType": "image/png"}, token)
    put = urllib.request.Request(handle["uploadUrl"], data=png, method="PUT", headers={"content-type": "image/png"})
    urllib.request.urlopen(put, timeout=15).close()
    world.stack.api("POST", f"/icons/{handle['id']}/confirm", token=token)
    return handle["id"]


def icons(world: World, check: Checks) -> None:
    say("icons: only pictures, and only the uploader's to give")
    stack = world.stack
    png = picture(16, 16)
    check("an icon that is not a picture every browser shows safely is refused",
          stack.status("POST", "/icons", {"mimeType": "image/svg+xml"}, world.owner["token"]) == 400
          and stack.status("POST", "/icons", {"mimeType": "text/html"}, world.owner["token"]) == 400)
    handle = world.as_owner("POST", "/icons", {"mimeType": "image/png"})
    put = urllib.request.Request(handle["uploadUrl"], data=png, method="PUT", headers={"content-type": "image/png"})
    urllib.request.urlopen(put, timeout=15).close()
    check("nobody else confirms another's upload",
          stack.status("POST", f"/icons/{handle['id']}/confirm", token=world.member["token"]) == 404)
    world.as_owner("POST", f"/icons/{handle['id']}/confirm")
    icon = handle["id"]
    check("nor gives it to their own profile",
          stack.status("PATCH", "/users/@me", {"icon": icon}, world.member["token"]) == 400)
    check("nor to a community they make",
          stack.status("POST", "/communities", {"name": "Not mine", "icon": icon}, world.member["token"]) == 400)
    manager = world.role("icon managers", ["manageCommunity"])
    world.give(manager)
    check("nor, managing the community, to it",
          stack.status("PATCH", f"/communities/{world.community}", {"icon": icon}, world.member["token"]) == 400)
    world.as_owner("PATCH", f"/communities/{world.community}", {"icon": icon})
    check("though a manager may keep the icon another gave it",
          stack.status("PATCH", f"/communities/{world.community}", {"icon": icon, "name": "Kept"},
                       world.member["token"]) == 200)
    check("its uploader gives it to their own profile",
          stack.status("PATCH", "/users/@me", {"icon": icon}, world.owner["token"]) == 200)
    own = upload_icon(world, world.member["token"], png)
    check("and a manager their own upload to the community",
          stack.status("PATCH", f"/communities/{world.community}", {"icon": own}, world.member["token"]) == 200)
    with urllib.request.urlopen(f"{stack.base}/api/v1/federation/icons/{icon}", timeout=15) as avatar:
        check("an avatar served from the web client's origin can run nothing there",
              avatar.headers.get("x-content-type-options") == "nosniff"
              and "sandbox" in (avatar.headers.get("content-security-policy") or "")
              and avatar.headers.get("content-type") == "image/png")


def operators(world: World, check: Checks) -> None:
    say("the operator commands")
    stack = world.stack
    hidden = world.channel("hidden-from-all", overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    world.stream.gather(0.5)
    stack.command("admin", "grant", world.member["name"])
    stack.command("admin", "allow", "moderateCommunities")
    got = world.stream.gather(1.0)
    check("granting moderation from the terminal reaches the member",
          any("moderateCommunities" in e.get("permissions", []) for e in of(got, "deploymentAccessChanged")), got)
    check("who then hears even hidden channels", world.hears_message(hidden))
    stack.command("admin", "revoke", world.member["name"])
    got = world.stream.gather(1.0)
    check("revoking it from the terminal reaches them", bool(of(got, "deploymentAccessChanged")))
    check("who then no longer hears them", not world.hears_message(hidden))
    stack.command("communities", "set-owner", world.community, world.member["name"])
    got = world.stream.gather(1.0)
    check("naming an owner from the terminal is announced",
          bool(of(got, "community", id=world.community, owner=world.member["id"])))
    check("and the new owner reads hidden channels", world.member_sees(hidden))
    stack.command("communities", "set-owner", world.community, world.owner["name"])
    world.stream.gather(0.5)


def deployment_settings(world: World, check: Checks) -> None:
    say("the deployment's settings, changed while people use it")
    stack = world.stack
    check("changing them takes Manage deployment settings",
          stack.status("PATCH", "/admin/settings", {"fileTransfers": False}, world.owner["token"]) == 403)
    stack.command("admin", "grant", world.owner["name"])
    room = world.channel("files", ty="voice")
    wait_for("a voice server offer",
             lambda: stack.status("POST", f"/channels/{room}/voice/join", {}, world.member["token"]) == 200, 90)
    call = join(stack.api("POST", f"/channels/{room}/voice/join", {}, world.member["token"])["token"])
    check("the member joins a call", frame_of(call, "ready") is not None)
    wait_for("the call's record to show the member", lambda: in_call(world), 30)
    world.as_owner("PATCH", "/admin/settings", {"fileTransfers": False})
    changed = frame_of(call, "grantsChanged", 10)
    check("turning file transfers off reaches a call at once",
          changed is not None and not changed["grants"]["transferFiles"], changed)
    check("and join offers grant them no more",
          not stack.api("POST", f"/channels/{room}/voice/join", {}, world.member["token"])["transferFiles"])
    world.as_owner("PATCH", "/admin/settings", {"fileTransfers": True})
    changed = frame_of(call, "grantsChanged", 10)
    check("turning them on again reaches it too", changed is not None and changed["grants"]["transferFiles"], changed)
    call.close()
    stack.command("settings", "set", "--require-two-factor", "true")
    world.stream.gather(3.0)
    check("requiring two factors from the terminal closes the stream of a member without one",
          world.stream.closed == 4403, world.stream.closed)
    check("whose requests are refused until they add one",
          stack.status("GET", "/users/@me", token=world.member["token"]) == 403)
    stack.command("settings", "set", "--require-two-factor", "false")
    # Every server reads a change as it commits, a moment after the command returns.
    check("and once it is lifted their session works again",
          soon(lambda: stack.status("GET", "/users/@me", token=world.member["token"]) == 200))
    stack.command("admin", "revoke", world.owner["name"])


def sign_ins(world: World, check: Checks) -> None:
    say("sign-ins ending while their streams are open")
    stack = world.stack
    second = world.sign_in(world.member["name"])
    other = stack.events(second["token"])
    stack.api("POST", "/auth/logout", {"refreshToken": second["refresh"]}, second["token"])
    other.gather(1.0)
    world.stream.gather(0.2)
    check("signing out closes that sign-in's stream as unauthorized", other.closed == 4401, other.closed)
    check("and leaves the member's other one open", world.stream.closed is None, world.stream.closed)
    third = world.sign_in(world.member["name"])
    another = stack.events(third["token"])
    stack.api("PUT", "/users/@me/password", {"oldPassword": PASSWORD, "newPassword": PASSWORD + "!"},
              world.member["token"])
    another.gather(1.0)
    world.stream.gather(0.2)
    check("a password change closes every other sign-in's stream", another.closed == 4401, another.closed)
    check("but not the one that changed it", world.stream.closed is None, world.stream.closed)
    stack.api("PUT", "/users/@me/password", {"oldPassword": PASSWORD + "!", "newPassword": PASSWORD},
              world.member["token"])


def removal(world: World, check: Checks) -> None:
    say("the member removed")
    open_channel = world.channel("after-removal")
    world.stream.gather(0.5)
    world.as_owner("DELETE", f"/communities/{world.community}/members/{world.member['id']}")
    check("removal reaches the member",
          bool(of(world.stream.gather(1.0), "userCommunity", type="delete", user=world.member["id"])))
    check("who then cannot read the community's channels", not world.member_sees(open_channel))
    check("nor hear them", not world.hears_message(open_channel))


def dual_invites(world: World, check: Checks) -> None:
    say("dual invites: an account made and joined at once, and both parts revoked")
    stack = world.stack
    check("making one takes Manage registration invites",
          stack.status("POST", "/admin/registration-invites", {"community": world.community},
                       world.member["token"]) == 403)
    stack.command("admin", "grant", world.owner["name"])
    stranger = world.account("stranger")
    stack.command("admin", "grant", stranger["name"])
    # Refused as not found, or, where the administrators also moderate, for lacking Create invites.
    check("and a community its maker may invite to",
          stack.status("POST", "/admin/registration-invites", {"community": world.community},
                       stranger["token"]) in (403, 404))
    dual = world.as_owner("POST", "/admin/registration-invites", {"maxUses": 2, "community": world.community})
    invite = dual["community"]["invite"]
    world.stream.gather(0.5)
    newcomer = f"new{world.run}"
    stack.api("POST", "/users", {"name": newcomer, "password": PASSWORD, "inviteCode": dual["code"]})
    joined = world.sign_in(newcomer)
    mine = stack.api("GET", "/users/@me/communities", token=joined["token"])
    ids = [c["id"] for c in (mine["data"] if isinstance(mine, dict) else mine)]
    check("the account it makes is in the community", world.community in ids, ids)
    check("and members hear it join", bool(of(world.stream.gather(1.0), "userCommunity", type="create")))
    world.as_owner("DELETE", f"/invites/{invite}")
    late = f"late{world.run}"
    stack.api("POST", "/users", {"name": late, "password": PASSWORD, "inviteCode": dual["code"]})
    mine = stack.api("GET", "/users/@me/communities", token=world.sign_in(late)["token"])
    ids = [c["id"] for c in (mine["data"] if isinstance(mine, dict) else mine)]
    check("its community invite revoked, it makes accounts that join nothing", world.community not in ids, ids)
    again = world.as_owner("POST", "/admin/registration-invites", {"community": world.community})
    # Invites are announced to those who manage them, the owner among them.
    owner_stream = stack.events(world.owner["token"])
    owner_stream.gather(0.5)
    world.as_owner("DELETE", f"/admin/registration-invites/{again['code']}")
    check("revoking a dual invite revokes its community invite",
          stack.status("GET", f"/invites/{again['community']['invite']}", token=world.owner["token"]) == 404)
    check("and the community's invite managers hear that invite go",
          bool(of(owner_stream.gather(1.0), "invite", type="delete", code=again["community"]["invite"])))
    check("the registration link stops working",
          stack.status("GET", f"/registration-invites/{again['code']}") == 404)
    stack.command("admin", "revoke", world.owner["name"])
    stack.command("admin", "revoke", stranger["name"])


def device_links(world: World, check: Checks) -> None:
    say("sign-in codes: the giving sign-in ending before the code is claimed")
    stack = world.stack
    verifier = "v" * 43
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()

    def offered(giver: dict) -> str:
        link = stack.api("POST", "/auth/device-links", {}, giver["token"])["id"]
        stack.api("POST", f"/auth/device-links/{link}/scan", {"deviceName": "Phone", "codeChallenge": challenge})
        stack.api("PUT", f"/auth/device-links/{link}/approval", token=giver["token"])
        return link

    def claim(link: str) -> int:
        return stack.status("POST", f"/auth/device-links/{link}/claim", {"codeVerifier": verifier})

    giver = world.sign_in(world.member["name"])
    link = offered(giver)
    stack.api("POST", "/auth/logout", {"refreshToken": giver["refresh"]}, giver["token"])
    check("a code approved by a sign-in that then signs out signs nothing in", claim(link) == 404)
    giver = world.sign_in(world.member["name"])
    link = offered(giver)
    stack.api("PUT", "/users/@me/password", {"oldPassword": PASSWORD, "newPassword": PASSWORD + "!"},
              world.member["token"])
    check("nor one approved before a password change elsewhere", claim(link) == 404)
    stack.api("PUT", "/users/@me/password", {"oldPassword": PASSWORD + "!", "newPassword": PASSWORD},
              world.member["token"])
    link = offered(world.member)
    got = stack.api("POST", f"/auth/device-links/{link}/claim", {"codeVerifier": verifier})
    check("one whose giver stands signs in", got.get("status") == "signedIn", got)
    check("as the giver", got.get("userId") == world.member["id"], got)
    check("and once", claim(link) == 404)

def name_colours(world: World, check: Checks) -> None:
    say("name colours, from a community role and a deployment role")
    stack = world.stack
    outsider = world.account("outsider")
    tinted = world.as_owner("POST", f"/communities/{world.community}/roles",
                            {"name": "Tinted", "permissions": [], "hue": 200, "hoist": True})
    world.give(tinted["id"])
    got = world.stream.gather(1.0)
    check("a community role's hue reaches its members", bool(of(got, "role", id=tinted["id"], hue=200)), got)
    check("but no one outside the community reads it",
          stack.status("GET", f"/communities/{world.community}/roles", token=outsider["token"]) == 404)
    check("nor does it colour the member's name anywhere else",
          stack.api("GET", f"/users/{world.member['id']}", token=outsider["token"]).get("nameHue") is None)
    stack.command("admin", "grant", world.member["name"])
    world.stream.gather(0.5)
    staff = stack.api("POST", "/admin/roles", {"name": f"Staff{world.run}", "permissions": [], "hue": 120},
                      world.member["token"])["id"]
    stack.api("PUT", f"/admin/users/{world.owner['id']}/roles/{staff}", token=world.member["token"])
    got = world.stream.gather(1.0)
    check("giving a deployment role with a hue announces the holder's new name colour",
          bool(of(got, "user", id=world.owner["id"], nameHue=120)), got)
    check("which shows to someone who shares nothing with them",
          stack.api("GET", f"/users/{world.owner['id']}", token=outsider["token"]).get("nameHue") == 120)
    stack.api("DELETE", f"/admin/roles/{staff}", token=world.member["token"])
    got = world.stream.gather(1.0)
    check("deleting the role takes the colour away, announced",
          any("nameHue" in e and e["nameHue"] is None for e in of(got, "user", id=world.owner["id"]))
          and stack.api("GET", f"/users/{world.owner['id']}", token=outsider["token"]).get("nameHue") is None, got)
    stack.command("admin", "revoke", world.member["name"])
    world.stream.gather(0.5)

def nicknames(world: World, check: Checks) -> None:
    say("nicknames: choosing, losing Change nickname, clearing, and reporting")
    stack = world.stack
    community, member = world.community, world.member
    mine = f"/communities/{community}/members/@me"
    owner_stream = stack.events(world.owner["token"])
    owner_stream.gather(0.5)
    stack.api("PATCH", mine, {"nickname": "Aster"}, member["token"])
    got = owner_stream.gather(1.0)
    heard = of(got, "userCommunity", type="update", user=member["id"])
    check("a nickname chosen reaches the rest of the community",
          any(e.get("nickname") == "Aster" for e in heard), got)
    check("without the member's own list position", all("sortIndex" not in e for e in heard), heard)
    check("and reads back from the member's record",
          world.as_owner("GET", f"/communities/{community}/members/{member['id']}").get("nickname") == "Aster")
    stack.api("PATCH", mine, {"sortIndex": 7}, member["token"])
    check("a reorder alone tells the community nothing",
          not of(owner_stream.gather(1.0), "userCommunity", user=member["id"]))
    everyone = next(r for r in world.as_owner("GET", f"/communities/{community}/roles") if r["everyone"])
    kept = [p for p in everyone["permissions"] if p != "changeNickname"]
    world.as_owner("PATCH", f"/roles/{world.everyone}", {"permissions": kept})
    check("without Change nickname, a member cannot choose another",
          stack.status("PATCH", mine, {"nickname": "Bramble"}, member["token"]) == 403)
    check("but keeps the one they had",
          world.as_owner("GET", f"/communities/{community}/members/{member['id']}").get("nickname") == "Aster")
    check("and may clear their own",
          stack.status("DELETE", f"{mine}/nickname", token=member["token"]) == 204)
    check("which the community hears", any(
        "nickname" in e and e["nickname"] is None
        for e in of(owner_stream.gather(1.0), "userCommunity", type="update", user=member["id"])))
    world.as_owner("PATCH", f"/roles/{world.everyone}", {"permissions": kept + ["changeNickname"]})
    stack.api("PATCH", mine, {"nickname": "Aster"}, member["token"])
    world.as_owner("PATCH", mine, {"nickname": "Captain"})

    moderator = world.account("nickmod")
    invite = world.as_owner("POST", f"/communities/{community}/invites", {})
    stack.api("PUT", mine, {"inviteCode": invite.get("code") or invite.get("id")}, moderator["token"])
    clearers = world.role("Clearers", ["manageNicknames"])
    world.as_owner("PUT", f"/communities/{community}/members/{moderator['id']}/roles/{clearers}")
    check("without Manage nicknames, nobody clears another's",
          stack.status("DELETE", f"/communities/{community}/members/{moderator['id']}/nickname",
                       token=member["token"]) == 403)
    check("with it, a moderator clears a member's below them",
          stack.status("DELETE", f"/communities/{community}/members/{member['id']}/nickname",
                       token=moderator["token"]) == 204)
    check("and the member hears it", any(
        "nickname" in e and e["nickname"] is None
        for e in of(world.stream.gather(1.0), "userCommunity", type="update", user=member["id"])))
    check("but never the owner's",
          stack.status("DELETE", f"/communities/{community}/members/{world.owner['id']}/nickname",
                       token=moderator["token"]) == 403)
    check("nor may they choose one for someone else",
          stack.status("PATCH", f"/communities/{community}/members/{member['id']}", {"nickname": "X"},
                       token=moderator["token"]) in (404, 405))

    say("nicknames: reports and their review")
    stack.api("PATCH", mine, {"nickname": "Aster"}, member["token"])
    category = stack.api("GET", "/report-categories", token=world.owner["token"])[0]["id"]
    reports = f"/communities/{community}/members/{member['id']}/nickname/reports"
    outsider = world.account("nickout")
    check("someone outside the community cannot report a nickname in it",
          stack.status("POST", reports, {"category": category}, outsider["token"]) == 404)
    check("nor can anyone report a member with none",
          stack.status("POST", f"/communities/{community}/members/{moderator['id']}/nickname/reports",
                       {"category": category}, world.owner["token"]) == 400)
    check("nor their own", stack.status("POST", reports, {"category": category}, member["token"]) == 400)
    check("a member reports another's", stack.status("POST", reports, {"category": category},
                                                     world.owner["token"]) == 201)
    check("once", stack.status("POST", reports, {"category": category}, world.owner["token"]) == 409)
    stack.command("admin", "grant", world.owner["name"])
    for permission in REVIEWING:
        stack.command("admin", "allow", permission)
    cases = world.as_owner("GET", "/admin/reports")
    case = next((c for c in cases["cases"] if c["kind"] == "nickname" and c["subject"] == member["id"]), None)
    check("the reviewer finds a nickname case", case is not None, cases["cases"])
    if case is None:
        return stop_reviewing(world)
    check("naming the community, which the page includes",
          case["community"] == community and any(c["id"] == community for c in cases["communities"]), case)
    check("and the nickname as reported", case["reports"][0].get("nickname") == "Aster", case["reports"])
    stack.api("PATCH", mine, {"nickname": "Aster Again"}, member["token"])
    world.stream.gather(0.5)
    check("clearing is for nickname cases alone",
          stack.status("POST", f"/admin/reports/{case['id']}/resolution", {"deleteMessage": True},
                       world.owner["token"]) == 400)
    resolved = world.as_owner("POST", f"/admin/reports/{case['id']}/resolution",
                              {"clearNickname": True, "warn": "Please choose a kinder nickname."})
    outcome = resolved["cases"][0]
    check("resolving it clears the nickname standing now",
          outcome["resolution"]["clearedNickname"] and stack.api(
              "GET", f"/communities/{community}/members/{member['id']}", token=member["token"]
          ).get("nickname") is None, outcome)
    check("which the member hears", any(
        "nickname" in e and e["nickname"] is None
        for e in of(world.stream.gather(1.0), "userCommunity", type="update", user=member["id"])))
    dms = stack.api("GET", "/users/@me/dms", token=member["token"])
    warned = []
    for dm in (dms["data"] if isinstance(dms, dict) else dms):
        read = stack.api("GET", f"/channels/{dm['id']}/messages", token=member["token"])
        warned += [m for m in (read["data"] if isinstance(read, dict) else read) if m.get("kind") == "warning"]
    check("and the warning names the nickname and its community",
          any((m.get("warning") or {}).get("nickname", {}).get("nickname") == "Aster" and
              m["warning"]["nickname"]["community"] == community for m in warned), warned)
    check("coming from the system account, not the reviewer",
          bool(warned) and all(m["author"] != world.owner["id"] for m in warned), warned)
    stop_reviewing(world)


# What reviewing a nickname case and acting on it take, beyond the top deployment role.
REVIEWING = ["reviewReports", "removeContent"]

# The deployment permissions given deliberately, which `admin grant` alone never gives. Moderate
# any community comes first, since denying Remove content while it is allowed is refused.
MODERATION = ["moderateCommunities", "reviewReports", "removeContent", "banUsers", "messageAnyUser"]


def review_powers(world: World, check: Checks) -> None:
    say("reviewing reports: what each deployment permission allows on its own")
    stack = world.stack
    reported = world.channel("reported")
    category = stack.api("GET", "/report-categories", token=world.owner["token"])[0]["id"]

    def report(content: str) -> str:
        posted = stack.api("POST", f"/channels/{reported}/messages", {"content": content, "attachments": []},
                           world.member["token"])["id"]
        stack.api("POST", f"/messages/{posted}/reports", {"category": category}, world.owner["token"])
        return posted

    first, second = report("first reportable thing"), report("second reportable thing")
    reviewer = world.account("reviewer")
    for permission in MODERATION:
        stack.command("admin", "deny", permission)
    stack.command("admin", "grant", reviewer["name"])
    stack.command("admin", "allow", "reviewReports")

    def status(method: str, path: str, body: dict | None = None) -> int:
        return stack.status(method, path, body, reviewer["token"])

    def case_of(message: str) -> str | None:
        cases = stack.api("GET", "/admin/reports", token=reviewer["token"])["cases"]
        return next((c["id"] for c in cases if c.get("message") == message), None)

    case = case_of(first)
    check("Review reports alone reads the cases", case is not None)
    if case is None:
        return stop_review_powers(world, reviewer)
    resolution = f"/admin/reports/{case}/resolution"
    check("but deletes nothing without Remove content",
          status("POST", resolution, {"deleteMessage": True}) == 403)
    check("nor bans without Ban users", status("POST", resolution, {"ban": {}}) == 403)
    check("yet resolves a case with a warning",
          status("POST", resolution, {"warn": "Please keep it civil."}) == 200)
    dms = stack.api("GET", "/users/@me/dms", token=world.member["token"])
    warned = [m for dm in (dms["data"] if isinstance(dms, dict) else dms)
              for m in stack.api("GET", f"/channels/{dm['id']}/messages", token=world.member["token"])["data"]
              if m.get("kind") == "warning"]
    check("which reaches the person reported from the system account",
          bool(warned) and all(m["author"] != reviewer["id"] for m in warned), warned)
    check("and opens no DM of the reviewer's",
          not stack.api("GET", "/users/@me/dms", token=reviewer["token"])["data"])

    stack.command("admin", "allow", "removeContent")
    case = case_of(second)
    check("the reviewer cannot read the reported channel",
          status("GET", f"/channels/{reported}/messages") in (403, 404))
    world.stream.gather(0.5)
    check("yet Remove content deletes the reported message there",
          case is not None and status("POST", f"/admin/reports/{case}/resolution", {"deleteMessage": True}) == 200)
    check("which the channel hears", bool(of(world.stream.gather(1.0), "message", type="delete", id=second)))

    stack.command("admin", "deny", "removeContent")
    stack.command("admin", "allow", "banUsers")
    check("a ban that deletes messages takes Remove content besides Ban users",
          status("PUT", f"/admin/users/{world.member['id']}/ban", {"deleteMessagesSeconds": 3600}) == 403)

    check("View dashboard alone does not read the record of files sent in calls",
          status("GET", "/admin/file-transfers") == 403)
    stack.command("admin", "allow", "moderateCommunities")
    check("Moderate any community does", status("GET", "/admin/file-transfers") == 200)
    access = stack.api("GET", "/users/@me/admin", token=reviewer["token"])
    check("and includes Remove content, which the caller is told",
          "removeContent" in access["permissions"] and any(
              i["permission"] == "moderateCommunities" and "removeContent" in i["includes"]
              for i in access["inclusions"]), access)
    try:
        stack.command("admin", "deny", "removeContent")
        refused = False
    except Failed:
        refused = True
    check("the terminal refuses to deny what an allowed permission includes", refused)
    stop_review_powers(world, reviewer)


def ban_ranks(world: World, check: Checks) -> None:
    say("lifting and replacing a deployment ban ranks as banning does")
    stack, member = world.stack, world.member
    stack.command("admin", "grant", world.owner["name"])
    stack.command("admin", "allow", "banUsers")
    # Each role is made below the others, so Senior outranks Banners.
    senior_role = world.as_owner("POST", "/admin/roles", {"name": f"Senior{world.run}", "permissions": []})["id"]
    banners = world.as_owner("POST", "/admin/roles", {"name": f"Banners{world.run}", "permissions": ["banUsers"]})["id"]
    world.as_owner("PUT", f"/admin/users/{member['id']}/roles/{banners}")
    senior, plain = world.account("senior"), world.account("plain")
    world.as_owner("PUT", f"/admin/users/{senior['id']}/roles/{senior_role}")
    world.as_owner("PUT", f"/admin/users/{senior['id']}/ban", {"reason": "checking ranks"})
    world.as_owner("PUT", f"/admin/users/{plain['id']}/ban", {"reason": "checking ranks"})
    check("Ban users does not lift the ban of someone who outranks its holder",
          stack.status("DELETE", f"/admin/users/{senior['id']}/ban", token=member["token"]) == 403)
    check("nor replace it", stack.status("PUT", f"/admin/users/{senior['id']}/ban", {}, member["token"]) == 403)
    check("whose ban stands", stack.status("POST", "/auth/login",
                                           {"username": senior["name"], "password": PASSWORD}) == 403)
    check("but lifts one of someone ranked below",
          stack.status("DELETE", f"/admin/users/{plain['id']}/ban", token=member["token"]) == 204)
    world.as_owner("DELETE", f"/admin/users/{senior['id']}/ban")
    world.as_owner("DELETE", f"/admin/roles/{banners}")
    world.as_owner("DELETE", f"/admin/roles/{senior_role}")
    stack.command("admin", "deny", "banUsers")
    stack.command("admin", "revoke", world.owner["name"])


def dm_reads(world: World, check: Checks) -> None:
    say("a deployment moderator's every read of a DM they are not in is logged")
    stack, member = world.stack, world.member
    made = stack.api("POST", "/users/@me/dms", {"recipients": [world.owner["id"]]}, member["token"])
    dm = made.get("id") or made["data"]["id"]
    secret = stack.api("POST", f"/channels/{dm}/messages", {"content": "between us", "attachments": []},
                       member["token"])["id"]
    thumbs = "%F0%9F%91%8D"
    stack.api("PUT", f"/messages/{secret}/reactions/{thumbs}/@me", token=member["token"])
    links = world.channel("links")
    world.post(links, f"see https://localhost/dms/{dm}/messages/{secret}")
    watcher = world.account("watcher")
    stack.command("admin", "grant", watcher["name"])
    stack.command("admin", "allow", "moderateCommunities")

    def logged() -> int:
        entries = stack.api("GET", "/admin/moderation-log?limit=100", token=watcher["token"])
        return sum(1 for e in entries if e.get("action") == "readDm" and e.get("channel") == dm)

    before = logged()
    read = stack.api("GET", f"/channels/{links}/messages?include=linked", token=watcher["token"])
    check("a DM message sideloaded by a link reaches the moderator",
          any(m["id"] == secret for m in read.get("included", {}).get("messages", [])), read.get("included"))
    check("and the reading is logged, once", logged() == before + 1)
    status = stack.status("GET", f"/messages/{secret}/reactions/{thumbs}", token=watcher["token"])
    check("reading who reacted in it is logged too", status == 200 and logged() == before + 2, status)
    check("while what reads no content finds no DM",
          stack.status("GET", f"/channels/{dm}/read-states/@me", token=watcher["token"]) == 404
          and stack.status("GET", f"/channels/{dm}/presence", token=watcher["token"]) == 404)
    stack.command("admin", "deny", "moderateCommunities")
    stack.command("admin", "revoke", watcher["name"])


def group_dm_moderators(world: World, check: Checks) -> None:
    say("a deployment moderator reads a group DM and does not join or reshape it")
    stack, member = world.stack, world.member
    third = world.account("third")
    invite = world.as_owner("POST", f"/communities/{world.community}/invites", {})
    stack.api("PUT", f"/communities/{world.community}/members/@me",
              {"inviteCode": invite.get("code") or invite.get("id")}, third["token"])
    made = stack.api("POST", "/users/@me/dms", {"recipients": [world.owner["id"], third["id"]]}, member["token"])
    group = made.get("id") or made["data"]["id"]
    watcher = world.account("watcher")
    stack.command("admin", "grant", watcher["name"])
    stack.command("admin", "allow", "moderateCommunities")
    check("the moderator reads the group's messages",
          stack.status("GET", f"/channels/{group}/messages", token=watcher["token"]) == 200)
    check("but does not add themselves to it",
          stack.status("PUT", f"/channels/{group}/recipients/{watcher['id']}", token=watcher["token"]) == 404)
    check("nor anyone else",
          stack.status("PUT", f"/channels/{group}/recipients/{world.owner['id']}", token=watcher["token"]) == 404)
    check("nor post in it",
          stack.status("POST", f"/channels/{group}/messages", {"content": "hello", "attachments": []},
                       watcher["token"]) == 404)
    check("whose people stay as they were",
          len(stack.api("GET", f"/channels/{group}", token=member["token"])["recipients"]) == 3)
    stack.command("admin", "deny", "moderateCommunities")
    stack.command("admin", "revoke", watcher["name"])


def stop_review_powers(world: World, reviewer: dict) -> None:
    for permission in MODERATION:
        world.stack.command("admin", "deny", permission)
    world.stack.command("admin", "revoke", reviewer["name"])


def stop_reviewing(world: World) -> None:
    for permission in MODERATION:
        world.stack.command("admin", "deny", permission)
    world.stack.command("admin", "revoke", world.owner["name"])


WORD_FILTER_ID = "org.aspenchat.wordfilter"
CALENDAR_ID = "org.aspenchat.calendar"
BLACKJACK_ID = "org.aspenchat.blackjack"


def example(name: str) -> Path:
    """An example plugin's manifest, its component built for `wasm32-wasip2` first."""
    directory = REPO / "plugins" / name
    built = subprocess.run(
        ["cargo", "build", "--release", "--target", "wasm32-wasip2"],
        cwd=directory, capture_output=True, text=True,
    )
    if built.returncode != 0:
        raise Failed(f"the example plugin {name} did not build (rustup target add wasm32-wasip2?): {built.stderr[-800:]}")
    return directory / "aspen-plugin.json"


def word_filter() -> Path:
    return example("word_filter")


def running(world: World, plugin: str) -> None:
    """Waits for the chat server to run `plugin`, which it does once it has compiled its
    component: a while on a debug build, and longer for a larger plugin."""
    wait_for(f"{plugin} running", lambda: any(
        p["id"] == plugin for p in world.stack.api("GET", "/plugins", token=world.owner["token"])), 60)


def plugins(world: World, check: Checks) -> None:
    say("a plugin's notes, events, routes, settings, and account")
    stack = world.stack
    stack.command("plugins", "install", str(word_filter()), "--yes")
    stack.command("plugins", "enable", WORD_FILTER_ID)
    running(world, WORD_FILTER_ID)
    watched = world.channel("plugin-watched")
    world.stream.gather(0.5)
    turned_on = world.as_owner("PUT", f"/communities/{world.community}/plugins/{WORD_FILTER_ID}",
                               {"settings": {"watchWords": ["pineapple"]}, "grant": ["viewChannel", "sendMessages"]})
    check("the owner turns it on", turned_on.get("enabled") is True, turned_on)
    got = world.stream.gather(1.0)
    check("its settings there do not reach a member without Manage plugins", not of(got, "communityPlugin"),
          [e["serverEvent"] for e in got])
    check("nor may the member read them",
          stack.status("GET", f"/communities/{world.community}/plugins", token=world.member["token"]) == 403)
    principal = next(p["principal"] for p in stack.api("GET", "/plugins", token=world.member["token"])
                     if p["id"] == WORD_FILTER_ID)
    check("its account joined the community",
          stack.status("GET", f"/communities/{world.community}/members/{principal}", token=world.owner["token"]) == 200)

    posted = world.post(watched, "pineapple on pizza")
    got = world.stream.gather(3.0)
    check("its note on a message reaches the member who reads it", bool(of(got, "messageAnnotation", message=posted)),
          [e["serverEvent"] for e in got])
    check("as does its event in the channel", bool(of(got, "pluginEvent", channel=watched)))
    count = f"/plugins/{WORD_FILTER_ID}/routes/channels/{watched}/count"
    check("its route answers the member about the channel", stack.status("GET", count, token=world.member["token"]) == 200)

    world.as_owner("PUT", f"/channels/{watched}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(1.0)
    hidden = world.post(watched, "pineapple again")
    got = world.stream.gather(3.0)
    check("once the member loses the channel, its notes there stop reaching them",
          not of(got, "messageAnnotation", message=hidden), [e["serverEvent"] for e in got])
    check("and its events there", not of(got, "pluginEvent", channel=watched))
    check("its route no longer answers them about the channel",
          stack.status("GET", count, token=world.member["token"]) == 404)
    check("but still answers the owner", stack.status("GET", count, token=world.owner["token"]) == 200)
    check("nor can they read the note through the message",
          stack.status("GET", f"/messages/{hidden}?include=annotations", token=world.member["token"]) in (403, 404))

    managers = world.role("Plugin managers", ["managePlugins"])
    world.give(managers)
    world.stream.gather(1.0)
    world.as_owner("PATCH", f"/communities/{world.community}/plugins/{WORD_FILTER_ID}", {"watchWords": ["kiwi"]})
    got = world.stream.gather(1.0)
    check("given Manage plugins, the member hears its settings change",
          bool(of(got, "communityPlugin", plugin=WORD_FILTER_ID)), [e["serverEvent"] for e in got])
    check("and may read them",
          stack.status("GET", f"/communities/{world.community}/plugins", token=world.member["token"]) == 200)

    world.as_owner("DELETE", f"/communities/{world.community}/plugins/{WORD_FILTER_ID}")
    got = world.stream.gather(1.0)
    check("turning it off takes its account out",
          stack.status("GET", f"/communities/{world.community}/members/{principal}", token=world.owner["token"]) == 404)
    check("which the member hears", bool(of(got, "userCommunity", user=principal)), [e["serverEvent"] for e in got])
    stack.command("plugins", "disable", WORD_FILTER_ID)


def profile_annotations(world: World, check: Checks) -> None:
    say("a plugin's annotation of a person reaches only those who share a community with them")
    stack, member = world.stack, world.member
    if psql(f"SELECT count(*) FROM plugin WHERE id = '{WORD_FILTER_ID}'", stack.database) != "1":
        check("the word filter is installed, to annotate with", False, "the plugins scenario installs it")
        return
    # No example plugin annotates people, so the row is written as a plugin's would be.
    psql("INSERT INTO user_annotation (id, plugin, \"user\", kind, severity, label) "
         f"VALUES (gen_random_uuid(), '{WORD_FILTER_ID}', '{member['id']}', 'checked', 'info', "
         "'{\"key\": \"checked\"}')", stack.database)
    outsider = world.account("outsider")
    path = f"/users/{member['id']}/annotations"
    check("the person reads their own", len(stack.api("GET", path, token=member["token"])) == 1)
    check("and so does someone who shares a community with them",
          len(stack.api("GET", path, token=world.owner["token"])) == 1)
    check("but nobody else", stack.api("GET", path, token=outsider["token"]) == [])
    world.as_owner("DELETE", f"/communities/{world.community}/members/{member['id']}")
    check("nor anyone they no longer share a community with",
          stack.api("GET", path, token=world.owner["token"]) == [])
    psql(f"DELETE FROM user_annotation WHERE \"user\" = '{member['id']}'", stack.database)


def calendar_channels(world: World, check: Checks) -> None:
    say("a plugin's channel, its notices, its cards, and a private URL")
    stack = world.stack
    stack.command("plugins", "install", str(example("calendar")), "--yes")
    stack.command("plugins", "enable", CALENDAR_ID)
    running(world, CALENDAR_ID)
    announce = world.channel("announcements")
    world.as_owner("PUT", f"/communities/{world.community}/plugins/{CALENDAR_ID}",
                   {"settings": {"announceChannel": announce}, "grant": ["viewChannel", "sendMessages"]})
    calendar = world.as_owner("POST", "/channels", {
        "name": "events", "ty": "plugin", "pluginType": f"{CALENDAR_ID}:calendar",
        "community": world.community, "sortIndex": 3})["id"]
    events = f"/plugins/{CALENDAR_ID}/routes/calendars/{calendar}/events"
    check("a member reads the plugin's channel through its routes",
          stack.status("GET", events, token=world.member["token"]) == 200)
    feed = stack.api("POST", f"/plugins/{CALENDAR_ID}/routes/calendars/{calendar}/feed", None,
                     world.member["token"])["path"]
    check("and follows their private URL without signing in",
          stack.status("GET", f"{stack.base}{feed}") == 200)
    soon = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 3600))
    world.as_owner("POST", events, {"title": "Planning", "start": soon})
    time.sleep(1.0)
    card = stack.api("GET", f"/channels/{announce}/messages?limit=1", token=world.owner["token"])["data"][0]
    press = f"/messages/{card['id']}/card/buttons/rsvp"
    check("the event's card is posted by the plugin's account", card.get("card") is not None, card)
    check("a member who reads it may press its button", stack.status("POST", press, token=world.member["token"]) == 200)

    # An event whose reminder falls due in a few seconds, which the member says they go to.
    later = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 600 + 8))
    world.as_owner("POST", events, {"title": "Soon", "start": later})
    time.sleep(0.5)
    soon_card = stack.api("GET", f"/channels/{announce}/messages?limit=1", token=world.owner["token"])["data"][0]
    stack.api("POST", f"/messages/{soon_card['id']}/card/buttons/rsvp", None, world.member["token"])
    owner_stream = stack.events(world.owner["token"])
    owner_stream.gather(0.2)

    world.as_owner("PUT", f"/channels/{calendar}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.as_owner("PUT", f"/channels/{announce}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.5)
    check("once they lose the plugin's channel, its routes answer them nothing of it",
          stack.status("GET", events, token=world.member["token"]) == 404)
    check("nor does their private URL", stack.status("GET", f"{stack.base}{feed}") == 404)
    check("nor may they press a card they can no longer read",
          stack.status("POST", press, token=world.member["token"]) in (403, 404))
    got = world.stream.gather(12.0)
    check("nor do its notices reach them, though they said they would go",
          not of(got, "pluginNotice"), [e["serverEvent"] for e in got])
    check("while someone who may still view it is told",
          bool(of(owner_stream.gather(1.0), "pluginNotice", channel=calendar)))
    world.as_owner("DELETE", f"/channels/{calendar}/overrides/{world.everyone}")
    world.as_owner("DELETE", f"/channels/{announce}/overrides/{world.everyone}")
    world.stream.gather(0.5)
    check("given the channel back, their private URL answers again",
          stack.status("GET", f"{stack.base}{feed}") == 200)
    stack.command("admin", "grant", world.owner["name"])
    stack.command("admin", "allow", "banUsers")
    world.as_owner("PUT", f"/admin/users/{world.member['id']}/ban", {})
    check("banned from the deployment, their private URL answers nothing",
          stack.status("GET", f"{stack.base}{feed}") == 404)
    world.as_owner("DELETE", f"/admin/users/{world.member['id']}/ban")
    stack.command("admin", "deny", "banUsers")
    stack.command("admin", "revoke", world.owner["name"])
    stack.command("plugins", "disable", CALENDAR_ID)


def email(world: World, check: Checks) -> None:
    say("email: a shown address, the verification gate, and what a digest tells of")
    stack = world.stack
    member = world.member
    owner_stream = stack.events(world.owner["token"])
    owner_stream.gather(0.5)
    outsider = world.account("outsider")
    address = f"member.{world.run}@example.org"
    check("a fresh sign-in gives an address",
          stack.status("PUT", "/users/@me/email/address", {"address": address}, member["token"]) == 200)
    stack.api("PATCH", "/users/@me/email", {"shown": True}, member["token"])
    check("an unverified address is not shown, though the member chose to",
          stack.api("GET", f"/users/{member['id']}", token=outsider["token"]).get("publicEmail") is None)
    # The code goes to a mailbox no one reads here; the database stands in for typing it.
    psql(f"UPDATE user_email SET verified_at = now() WHERE \"user\" = '{member['id']}'", stack.database)
    stack.api("PATCH", "/users/@me/email", {"shown": False}, member["token"])
    owner_stream.gather(0.5)
    stack.api("PATCH", "/users/@me/email", {"shown": True}, member["token"])
    got = owner_stream.gather(1.0)
    check("showing a verified address announces it to those who share a community",
          bool(of(got, "user", id=member["id"], publicEmail=address)), got)
    check("and anyone reading the user sees it",
          stack.api("GET", f"/users/{member['id']}", token=outsider["token"]).get("publicEmail") == address)
    stack.api("PATCH", "/users/@me/email", {"shown": False}, member["token"])
    got = owner_stream.gather(1.0)
    check("hiding it announces that it is gone",
          any("publicEmail" in e and e["publicEmail"] is None for e in of(got, "user", id=member["id"])), got)
    check("and no read shows it",
          stack.api("GET", f"/users/{member['id']}", token=outsider["token"]).get("publicEmail") is None)
    check("someone else's address is not theirs to read",
          stack.status("GET", f"/users/{member['id']}/email", token=outsider["token"]) == 403)
    owner_stream.close()

    stack.api("PUT", "/users/@me/email/address", {"address": f"other.{world.run}@example.org"}, member["token"])
    world.stream.gather(0.5)
    stack.command("settings", "set", "--email-verification-required", "true")
    world.stream.gather(3.0)
    check("requiring verified addresses closes the stream of a member whose address is not",
          world.stream.closed == 4428, world.stream.closed)
    check("whose requests are refused but for verifying it",
          stack.status("GET", "/users/@me", token=member["token"]) == 403
          and stack.status("GET", "/users/@me/email", token=member["token"]) == 200)
    psql(f"UPDATE user_email SET verified_at = now() WHERE \"user\" = '{member['id']}'", stack.database)
    check("and once it is verified their session works again",
          stack.status("GET", "/users/@me", token=member["token"]) == 200)
    stream = stack.events(member["token"])
    stream.gather(0.5)
    stack.api("PUT", "/users/@me/email/address", {"address": f"third.{world.run}@example.org"}, member["token"])
    stream.gather(2.0)
    check("changing to an unverified address while verified ones are required closes their stream",
          stream.closed == 4428, stream.closed)
    psql(f"UPDATE user_email SET verified_at = now() WHERE \"user\" = '{member['id']}'", stack.database)
    stack.command("settings", "set", "--email-verification-required", "false")

    seen = world.channel("seen")
    hidden = world.channel("hidden")
    world.post(seen, "for the digest")
    world.post(hidden, "not for the member")
    world.as_owner("PUT", f"/channels/{hidden}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    stack.api("PATCH", "/users/@me/email", {"digest": True}, member["token"])
    psql(f"UPDATE user_email SET digest_since = now() - interval '1 hour', digest_next_at = now() "
         f"WHERE \"user\" = '{member['id']}'", stack.database)

    def digest() -> str:
        return psql(f"SELECT mail FROM email_outbox WHERE \"user\" = '{member['id']}' "
                    f"AND mail->>'kind' = 'digest'", stack.database)

    check("a digest is made when it is due", soon(lambda: digest() != "", 75))
    made = digest()
    check("it tells of what the member may read", "for the digest" in made, made)
    check("and nothing of a channel they lost view of", "not for the member" not in made and hidden not in made, made)


def invite_previews(world: World, check: Checks) -> None:
    say("an invite's link preview, as the invite and its community change")
    stack = world.stack

    def preview(code: str) -> str:
        status, page = stack.request("GET", f"{stack.base}/invite/{code}")
        if status != 200:
            raise Failed(f"the page of invite {code}: {status} {page[:300]}")
        return page

    name = f"Previewed {world.run}"
    world.as_owner("PATCH", f"/communities/{world.community}", {"name": name})
    code = world.as_owner("POST", f"/communities/{world.community}/invites", {})["code"]
    check("an invite's page previews as its community", f'og:title" content="{name}"' in preview(code))
    renamed = f"Renamed {world.run}"
    world.as_owner("PATCH", f"/communities/{world.community}", {"name": renamed})
    page = preview(code)
    check("a renamed community previews by its new name at once", renamed in page and name not in page)
    world.as_owner("DELETE", f"/invites/{code}")
    check("a revoked invite's page shows nothing of its community", renamed not in preview(code))
    expiring = world.as_owner("POST", f"/communities/{world.community}/invites",
                              {"expiresAt": "2000-01-01T00:00:00Z"})
    expired = expiring["code"]
    check("nor does an expired one's", renamed not in preview(expired))
    later = world.as_owner("POST", f"/communities/{world.community}/invites", {})["code"]
    world.as_owner("DELETE", f"/communities/{world.community}")
    check("nor, once the community is deleted, a working one's", renamed not in preview(later))


def blackjack_tables(world: World, check: Checks) -> None:
    say("a game's table: who may watch and play, and chips kept straight under simultaneous requests")
    from concurrent.futures import ThreadPoolExecutor

    stack, member = world.stack, world.member
    stack.command("plugins", "install", str(example("blackjack")), "--yes")
    stack.command("plugins", "enable", BLACKJACK_ID)
    running(world, BLACKJACK_ID)
    world.as_owner("PUT", f"/communities/{world.community}/plugins/{BLACKJACK_ID}", {"settings": {}})
    table = world.as_owner("POST", "/channels", {
        "name": "blackjack", "ty": "plugin", "pluginType": f"{BLACKJACK_ID}:table",
        "community": world.community, "sortIndex": 4})["id"]
    base = f"/plugins/{BLACKJACK_ID}/routes/tables/{table}"
    seen = stack.api("GET", base, token=member["token"])
    check("a member sits down to a table with 1,000 chips", seen.get("chips") == 1000, seen)

    # The member's bet sent eight times at once, and the owner's beside them: each write to the
    # table reads it first, so without storage-swap one would overwrite another.
    bets = [(member["token"], 100)] * 8 + [(world.owner["token"], 50)]
    with ThreadPoolExecutor(len(bets)) as pool:
        statuses = list(pool.map(
            lambda bet: stack.status("POST", f"{base}/bet", {"amount": bet[1]}, bet[0]), bets))
    check("of eight bets a member sends at once, one is taken", statuses[:8].count(200) == 1, statuses)
    after = stack.api("GET", base, token=member["token"])
    check("and its chips taken once", after.get("chips") == 900, after)
    check("while another player's bet made at the same moment keeps its seat",
          statuses[8] == 200 and {s["user"] for s in after["table"]["seats"]} == {member["id"], world.owner["id"]},
          after["table"]["seats"])
    got = world.stream.gather(1.0)
    check("everyone viewing the table hears of it", bool(of(got, "pluginEvent", channel=table)),
          [e["serverEvent"] for e in got])

    for who in (member, world.owner):
        stack.api("POST", f"{base}/ready", None, who["token"])
    tokens = {member["id"]: member["token"], world.owner["id"]: world.owner["token"]}
    decided_twice = None
    deadline = time.monotonic() + 30
    while True:
        now = stack.api("GET", base, token=world.owner["token"])["table"]
        phase = now["phase"]
        if phase["name"] == "settled" or time.monotonic() > deadline:
            break
        if phase["name"] == "playing":
            token = tokens[now["seats"][phase["seat"]]["user"]]
            decision = {"action": "stand", "version": now["version"]}
            stack.api("POST", f"{base}/actions", decision, token)
            if decided_twice is None:
                decided_twice = stack.request("POST", f"{base}/actions", decision, token)
        time.sleep(0.2)
    check("the timer deals, and the round is played to its end", phase["name"] == "settled", phase)
    if decided_twice is not None:
        check("a decision sent twice counts once", decided_twice[0] == 409 and "stale" in decided_twice[1],
              decided_twice)
    hand = next(s for s in now["seats"] if s["user"] == member["id"])["hands"][0]
    won = {"blackjack": 250, "win": 200, "push": 100}.get(hand.get("outcome"), 0)
    check(f"the member's payout ({hand.get('outcome')}) reaches their chips",
          soon(lambda: stack.api("GET", base, token=member["token"])["chips"] == 900 + won, 5),
          stack.api("GET", base, token=member["token"]))

    world.as_owner("PUT", f"/channels/{table}/overrides/{world.everyone}", {"allow": [], "deny": ["sendMessages"]})
    check("without Send messages, a member may watch",
          stack.status("GET", base, token=member["token"]) == 200)
    refused = stack.request("POST", f"{base}/bet", {"amount": 10}, member["token"])
    check("but not play", refused[0] == 403 and "cannotPlay" in refused[1], refused)
    world.as_owner("PUT", f"/channels/{table}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.5)
    stack.api("POST", f"{base}/bet", {"amount": 10}, world.owner["token"])
    got = world.stream.gather(1.5)
    check("once they lose the channel, its table's events stop reaching them",
          not of(got, "pluginEvent", channel=table), [e["serverEvent"] for e in got])
    check("and its routes answer them nothing of it", stack.status("GET", base, token=member["token"]) == 404)
    stack.command("plugins", "disable", BLACKJACK_ID)


SCENARIOS = [private_channels, granting_and_revoking, moves_and_categories, hidden_managers, role_grants,
             poll_votes, deleted_parents, thread_echoes, calls, attachments,
             operators, deployment_settings, sign_ins, removal, name_colours, dual_invites, device_links,
             nicknames, review_powers, ban_ranks, dm_reads,
             group_dm_moderators, plugins, profile_annotations, calendar_channels, blackjack_tables, email, invite_previews, previews, icons, uploads]


def main() -> None:
    parser = argparse.ArgumentParser(description="Check that changes to access reach everything already open.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    args = parser.parse_args()
    if shutil.which("docker") is None:
        sys.exit("permissions: docker is required")
    if args.start_services:
        say("starting the database, Valkey, and SeaweedFS")
        start_services()
    wait_for_services()
    check = Checks()
    try:
        say("starting a database and a NATS of its own, the chat server, and the voice server")
        with Stack(args.bin.resolve(), "permissions", PORTS, SETTINGS) as stack:
            run = format(int(time.time() * 1000) % 36**6, "x")
            for scenario in SCENARIOS:
                # Each starts from a community of its own, so one that fails, or stops on a
                # request refused, leaves the rest fair.
                try:
                    scenario(World(stack, f"{run}{scenario.__name__[:4]}"), check)
                except Failed as stopped:
                    check(f"{scenario.__name__} ran to its end", False, stopped)
            if check.failed:
                raise Failed(f"{len(check.failed)} of {check.passed + len(check.failed)} checks failed")
        say(f"passed: {check.passed} checks")
    except Failed as failure:
        say(f"FAILED: {failure}")
        for what in check.failed:
            say(f"  {what}")
        sys.exit(1)


if __name__ == "__main__":
    main()
