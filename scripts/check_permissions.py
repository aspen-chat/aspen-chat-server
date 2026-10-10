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
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib
from pathlib import Path

from stack import REPO, Failed, Ports, Stack, WebSocket, compose, psql, start_services, wait_for, wait_for_services

PORTS = Ports(nats=14322, api=18100, voice=19101, api_metrics=19564, voice_metrics=19565, rtc_min=45200, rtc_max=45399,
              transfer=13578, relay_min=46100, relay_max=46199)
PASSWORD = "check-permissions-password"
# Scenarios make several accounts and many changes in moments, as no person would. Mail goes to
# an SMTP port nothing listens on, so what is queued stays among the jobs to be read.
SETTINGS = ("[rate_limits]\nenabled = false\n"
            "[email]\nsmtp_url = \"smtp://127.0.0.1:9\"\nfrom = \"Aspen <noreply@localhost>\"\n")


def eventually(probe, seconds: float = 15) -> bool:
    """Whether `probe` comes true within `seconds`, as what a job does after a request does."""
    try:
        wait_for("it", probe, seconds)
        return True
    except Failed:
        return False


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


def edits_after_send(world: World, check: Checks) -> None:
    say("editing a message once Send messages is taken away")
    stack, member = world.stack, world.member
    quiet = world.channel("quiet")
    said = stack.api("POST", f"/channels/{quiet}/messages", {"content": "before", "attachments": []},
                     member["token"])["id"]
    world.as_owner("PUT", f"/channels/{quiet}/overrides/{world.everyone}", {"allow": [], "deny": ["sendMessages"]})
    check("the member cannot put new words in their message",
          stack.status("PATCH", f"/messages/{said}", {"content": "after"}, member["token"]) == 403)
    check("but may clear what it said",
          stack.status("PATCH", f"/messages/{said}", {"content": ""}, member["token"]) == 200)
    world.as_owner("DELETE", f"/channels/{quiet}/overrides/{world.everyone}")
    check("and with Send messages back, edits it again",
          stack.status("PATCH", f"/messages/{said}", {"content": "again"}, member["token"]) == 200)


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


def hidden_categories(world: World, check: Checks) -> None:
    say("a category hidden by its own overrides, then shown, then hidden again")
    stack, member = world.stack, world.member["token"]

    def listed(category: str) -> bool:
        read = stack.api("GET", f"/communities/{world.community}?include=categories,roles", token=member)
        included = read.get("included", {})
        return any(c["id"] == category for c in included.get("categories", [])) or any(
            o["category"] == category for o in included.get("categoryOverrides", []))

    hidden = world.as_owner("POST", f"/communities/{world.community}/categories", {"name": "Secret", "sortIndex": 9})["id"]
    world.stream.gather(0.5)
    world.as_owner("PUT", f"/categories/{hidden}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    got = world.stream.gather(1.0)
    check("hiding it reaches the member once, so they can let it go",
          len(of(got, "categoryOverride", category=hidden)) == 1, [e["serverEvent"] for e in got])
    world.as_owner("PATCH", f"/categories/{hidden}", {"name": "Secret plans"})
    check("and nothing about it after", not of(world.stream.gather(0.8), "category"))
    check("they do not find it, or its overrides, in the community's read", not listed(hidden))
    check("nor read it, its channels, or fold it",
          stack.status("GET", f"/categories/{hidden}", token=member) == 404
          and stack.status("GET", f"/categories/{hidden}/channels", token=member) == 404
          and stack.status("PUT", f"/categories/{hidden}/collapses/@me", token=member) == 404)
    inside = world.channel("let-in", category=hidden,
                           overrides=[{"role": world.everyone, "allow": ["viewChannel"], "deny": []}])
    world.stream.gather(0.8)
    check("a channel in it whose own override lets them view it is theirs, the category still not",
          world.member_sees(inside) and not listed(hidden))
    world.as_owner("DELETE", f"/categories/{hidden}/overrides/{world.everyone}")
    got = world.stream.gather(1.0)
    check("showing it reaches the member, about a category they did not have",
          len(of(got, "categoryOverride", category=hidden)) == 1 and listed(hidden))
    world.as_owner("PUT", f"/categories/{hidden}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.8)
    world.as_owner("DELETE", f"/categories/{hidden}")
    got = world.stream.gather(1.0)
    check("deleting it while hidden does not tell them of it", not of(got, "category", id=hidden))


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
    check("Manage categories does not reach the overrides of a category that hides itself from its holder",
          world.stack.status("PUT", f"/categories/{hidden}/overrides/{world.everyone}", unhide, member) == 404
          and world.stack.status("DELETE", f"/categories/{hidden}/overrides/{world.everyone}", token=member) == 404)
    check("nor delete that category, which would move its channels into view",
          world.stack.status("DELETE", f"/categories/{hidden}", token=member) == 404)
    check("whose channels stay hidden from them", not world.member_sees(inside))
    open_channel = world.channel("for-managers")
    check("while a channel they may view is theirs to manage",
          world.stack.status("PATCH", f"/channels/{open_channel}", {"name": "managed"}, member) == 200)
    check("but not to file in a category hidden from them",
          world.stack.status("PATCH", f"/channels/{open_channel}", {"parentCategory": hidden}, member) == 404)
    check("nor is a channel made there",
          world.stack.status("POST", "/channels", {"name": "smuggled", "ty": "text", "community": world.community,
                                                   "sortIndex": 1, "parentCategory": hidden}, member) == 404)


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
    stack.command("admin", "allow", "moderateCommunities")
    mods = world.as_owner("POST", "/admin/roles", {"name": f"Mods{world.run}",
                                                   "permissions": ["moderateCommunities"]})["id"]
    world.as_owner("PUT", f"/admin/users/{member['id']}/roles/{mods}")
    deleters = world.role("Deleters", ["manageMessages"])
    check("moderating the deployment hands on none of its powers through a role",
          stack.status("PUT", f"{mine}/{deleters}", token=member["token"]) == 403)
    third = world.account("third")
    invite = world.as_owner("POST", f"/communities/{world.community}/invites", {})
    stack.api("PUT", f"/communities/{world.community}/members/@me",
              {"inviteCode": invite.get("code") or invite.get("id")}, third["token"])
    roles = world.as_owner("GET", f"/communities/{world.community}/roles")
    moderator = next(r["id"] for r in roles if r["name"] == "Moderator")
    world.as_owner("PUT", f"/communities/{world.community}/members/{third['id']}/roles/{moderator}")
    check("nor ranks its holder above roles for taking them away",
          stack.status("DELETE", f"/communities/{world.community}/members/{third['id']}/roles/{moderator}",
                       token=member["token"]) == 403)
    world.as_owner("DELETE", f"/admin/users/{member['id']}/roles/{mods}")
    world.as_owner("DELETE", f"/admin/roles/{mods}")
    stack.command("admin", "deny", "moderateCommunities")
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


def poll_write_ins(world: World, check: Checks) -> None:
    say("an answer written in takes posting in the poll's channel")
    stack, member = world.stack, world.member
    ballot = world.channel("write-ins")
    poll = world.as_owner("POST", f"/channels/{ballot}/polls", {
        "question": "Which?", "options": [{"label": "this"}, {"label": "that"}], "multipleChoice": False,
        "allowWriteIns": True, "anonymous": False, "durationSeconds": 3600})["id"]
    world.as_owner("PUT", f"/channels/{ballot}/overrides/{world.everyone}", {"allow": [], "deny": ["sendMessages"]})
    check("without Send messages, the member cannot write an answer in",
          stack.status("POST", f"/polls/{poll}/write-ins", {"label": "another"}, member["token"]) == 403)
    check("though they may still vote", stack.status("PUT", f"/polls/{poll}/votes/0/@me", token=member["token"]) == 201)
    world.as_owner("PUT", f"/channels/{ballot}/overrides/{world.everyone}", {"allow": [], "deny": []})
    check("given it back, they may",
          stack.status("POST", f"/polls/{poll}/write-ins", {"label": "another"}, member["token"]) == 201)


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


def first_replies(world: World, check: Checks) -> None:
    say("a thread made by its first reply")
    stack, member = world.stack, world.member["token"]
    talk = world.channel("first-replies")
    starter = world.post(talk, "start a thread by replying")

    def reply(content: str, echo: bool = False) -> int:
        return stack.status("POST", f"/messages/{starter}/thread/messages",
                            {"content": content, "attachments": [], "echoToParent": echo}, member)

    def thread_of(message: str) -> str | None:
        return world.as_owner("GET", f"/messages/{message}")["data"]["thread"]

    def thread_of_starter() -> str | None:
        return thread_of(starter)

    world.as_owner("PUT", f"/channels/{talk}/overrides/{world.everyone}", {"allow": [], "deny": ["startThreads"]})
    world.stream.gather(0.8)
    check("without Start threads, the first reply is refused and makes no thread",
          reply("refused") == 403 and thread_of_starter() is None)
    world.as_owner("PUT", f"/channels/{talk}/overrides/{world.everyone}", {"allow": [], "deny": ["sendInThreads"]})
    world.stream.gather(0.8)
    check("nor without Send in threads", reply("refused") == 403 and thread_of_starter() is None)
    world.as_owner("PUT", f"/channels/{talk}/overrides/{world.everyone}", {"allow": [], "deny": ["sendMessages"]})
    world.stream.gather(0.8)
    check("nor echoed to a channel they may not send in",
          reply("refused", echo=True) == 403 and thread_of_starter() is None)
    world.as_owner("DELETE", f"/channels/{talk}/overrides/{world.everyone}")
    world.stream.gather(0.8)
    posted = stack.api("POST", f"/messages/{starter}/thread/messages",
                       {"content": "the first reply", "attachments": []}, member)
    thread = posted["channelId"]
    got = world.stream.gather(1.0)
    made = [e for e in got if e.get("serverEvent") in ("channel", "message") and e.get("type") == "create"]
    check("given them, the reply makes its thread, heard as the thread and then the reply",
          thread_of_starter() == thread
          and [(e["serverEvent"], e["id"]) for e in made] == [("channel", thread), ("message", posted["id"])], got)
    again = stack.api("POST", f"/messages/{starter}/thread/messages",
                      {"content": "a second reply", "attachments": []}, member)
    check("a later reply goes to the same thread",
          again["channelId"] == thread and stack.api("GET", f"/channels/{thread}", token=member)["replyCount"] == 2)
    world.as_owner("PUT", f"/channels/{talk}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.8)
    other = world.post(talk, "out of sight")
    check("a message the member may not see starts no thread for them",
          stack.status("POST", f"/messages/{other}/thread/messages",
                       {"content": "hidden", "attachments": []}, member) == 404
          and thread_of(other) is None)
    world.as_owner("DELETE", f"/channels/{talk}/overrides/{world.everyone}")
    world.stream.gather(0.8)


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
    # The join is noted just after its record is made; the failure reports are limited, so wait
    # once rather than asking over and over.
    time.sleep(1)
    joined = stack.api("POST", f"/voice-servers/{server}/failures", None, world.member["token"])
    check("a failure report from someone who joined a call there counts for nothing",
          joined.get("counted") is False, joined)
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
    # A moderator's mute is the community's: it outlasts the call until a moderator lifts it.
    stack.api("PATCH", f"/channels/{room}/voice/participants/{world.member['id']}", {"muted": True},
              world.owner["token"], expect=(202,))
    frame_of(call, "participantState")
    call.close()
    wait_for("the member's leaving to be recorded", lambda: not in_call(world), 30)
    muted_offer = offer()
    check("the next join offer says the mute stands", muted_offer.get("serverMuted") is True, muted_offer)
    call = join(muted_offer["token"])
    check("the member joins again", frame_of(call, "ready") is not None)

    def recorded_muted() -> bool:
        read = stack.api("GET", f"/communities/{world.community}?include=voice", token=world.owner["token"])
        return any(p["user"] == world.member["id"] and p["muted"]
                   for p in read.get("included", {}).get("voiceParticipants", []))

    check("and is recorded muted from the start", soon(recorded_muted, 15))
    mutes = f"/communities/{world.community}/voice-mutes"
    check("the community's mutes are not the member's to list",
          stack.status("GET", mutes, token=world.member["token"]) == 403)
    listed = stack.api("GET", mutes, token=world.owner["token"])
    check("its call moderators see the member among them",
          any(mute["user"] == world.member["id"] for mute in listed), listed)
    check("a member cannot lift their own mute",
          stack.status("DELETE", f"{mutes}/{world.member['id']}", token=world.member["token"]) == 403)
    stack.api("DELETE", f"{mutes}/{world.member['id']}", token=world.owner["token"], expect=(204,))
    lifted = frame_of(call, "participantState", 10)
    check("lifting it from the community's list reaches the call",
          lifted is not None and lifted["muted"] is False, lifted)
    callers = world.role("Call moderators", ["manageCalls"])
    world.give(callers)
    owner_seat = f"/channels/{room}/voice/participants/{world.owner['id']}"
    check("Manage calls does not let a member mute the owner",
          stack.status("PATCH", owner_seat, {"muted": True}, token=world.member["token"]) == 403)
    check("nor remove the owner from a call",
          stack.status("DELETE", owner_seat, token=world.member["token"]) == 403)
    world.as_owner("DELETE", f"/roles/{callers}")
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
    # A sign-in ending takes the participant that joined on its token out of the call.
    late.close()
    second = world.sign_in(world.member["name"])
    stale = stack.api("POST", f"/channels/{room}/voice/join", {}, second["token"])
    elsewhere = join(stack.api("POST", f"/channels/{room}/voice/join", {}, second["token"])["token"])
    check("another sign-in of the member joins the call", frame_of(elsewhere, "ready") is not None)
    stack.api("POST", "/auth/logout", {"refreshToken": second["refresh"]}, second["token"])
    kicked = frame_of(elsewhere, "kicked", 10)
    check("signing out takes that sign-in out of the call, saying why",
          kicked is not None and kicked.get("reason") == "signedOut", kicked)
    elsewhere.close()
    after = join(stale["token"])
    check("a token issued to the sign-in before it ended still joins", frame_of(after, "ready") is not None)
    kicked = frame_of(after, "kicked", 15)
    check("but is taken out once its join is recorded",
          kicked is not None and kicked.get("reason") == "signedOut", kicked)
    after.close()
    late = join(offer()["token"])
    check("the member's own sign-in joins again", frame_of(late, "ready") is not None)
    world.as_owner("PUT", f"/communities/{world.community}/bans/{world.member['id']}", {})
    kicked = frame_of(late, "kicked", 10)
    check("a ban from the community removes them from its calls",
          kicked is not None and kicked.get("reason") == "accessLost", kicked)
    late.close()
    world.as_owner("DELETE", f"/communities/{world.community}/bans/{world.member['id']}")


def attachments(world: World, check: Checks) -> None:
    say("an attachment, before it is sent and after")
    stack = world.stack
    handle = world.as_owner("POST", "/attachments", {"fileName": "notes.txt", "mimeType": "text/plain",
                                                     "byteSize": len(b"secret notes")})
    put = urllib.request.Request(handle["uploadUrl"], data=b"secret notes", method="PUT",
                                 headers={"content-type": "text/plain"})
    urllib.request.urlopen(put, timeout=15).close()
    confirmed = world.as_owner("POST", f"/attachments/{handle['id']}/confirm")
    path = f"/attachments/{handle['id']}"
    again = urllib.request.Request(handle["uploadUrl"], data=b"swapped note", method="PUT",
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
    check("a type that would read as a page too is refused",
          stack.status("POST", "/attachments", {"fileName": "a.txt", "mimeType": "text/plain;x=,text/html",
                                                "byteSize": 1}, world.owner["token"]) == 400)
    text = world.as_owner("POST", "/attachments", {"fileName": "a.txt", "byteSize": 1,
                                                   "mimeType": "text/plain; charset=utf-8; name=a.html"})
    check("and plain text is sent as plain text alone",
          text.get("contentType") == "text/plain; charset=utf-8", text)
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
    handle = world.stack.api("POST", "/attachments",
                             {"fileName": "photo.png", "mimeType": "image/png", "byteSize": len(png)}, token)
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

    starter = world.post(shared, "a thread whose first reply waits")
    waiting = upload_picture(world, world.owner["token"], png)
    status, body = stack.request("POST", f"/messages/{starter}/thread/messages",
                                 {"content": "held first reply", "attachments": [waiting], "mayHold": True},
                                 world.owner["token"])
    thread = json.loads(body).get("channelId") if status == 202 else None
    starter_thread = stack.api("GET", f"/messages/{starter}", token=world.member["token"])["data"]["thread"]
    check("a first reply held for its preview makes its thread, and waits in it",
          status == 202 and thread is not None and starter_thread == thread, (status, body))
    replies = []
    soon(lambda: bool(replies.extend(world.stream.gather(0.2))
                      or of(replies, "message", content="held first reply")), 30)
    check("and is posted there once the preview is made",
          bool(of(replies, "message", content="held first reply", channelId=thread)),
          [e["serverEvent"] for e in replies])

    theirs = upload_picture(world, world.member["token"], png)
    status, body = stack.request("POST", f"/channels/{shared}/messages",
                                 {"content": "not allowed", "attachments": [theirs], "mayHold": True},
                                 world.member["token"])
    held = json.loads(body) if status == 202 else {}
    check("the member's own is held too", status == 202, status)
    # A first reply held, which makes its thread; and a reply held in a thread that has one.
    lone_starter = world.post(shared, "a thread only a held reply is in")
    status, body = stack.request("POST", f"/messages/{lone_starter}/thread/messages",
                                 {"content": "lone reply", "attachments": [upload_picture(world, world.member["token"], png)],
                                  "mayHold": True}, world.member["token"])
    lone_thread = json.loads(body).get("channelId") if status == 202 else None
    kept_starter = world.post(shared, "a thread with a reply in it")
    kept_thread = world.as_owner("POST", f"/messages/{kept_starter}/thread/messages",
                                 {"content": "posted reply", "attachments": []})["channelId"]
    status, body = stack.request("POST", f"/channels/{kept_thread}/messages",
                                 {"content": "second reply", "attachments": [upload_picture(world, world.member["token"], png)],
                                  "mayHold": True}, world.member["token"])
    kept_held = json.loads(body) if status == 202 else {}
    check("held first replies make their threads", lone_thread is not None and kept_held != {}, body)
    world.as_owner("PUT", f"/channels/{shared}/overrides/{world.everyone}",
                   {"allow": [], "deny": ["sendMessages", "sendInThreads"]})
    owner_stream = stack.events(world.owner["token"])
    seen = []
    dropped = soon(lambda: bool(seen.extend(world.stream.gather(0.2))
                                or (of(seen, "heldMessageFailed", held=held.get("id"))
                                    and len(of(seen, "heldMessageFailed")) >= 3)), 30)
    check("losing the right to post drops it, and its author is told why",
          dropped and bool(of(seen, "heldMessageFailed", held=held.get("id"))[0].get("detail")),
          [e["serverEvent"] for e in seen])
    check("a dropped first reply takes the thread it made, and every client hears so",
          bool(of(seen, "channel", type="delete", id=lone_thread))
          and bool(of(seen, "message", type="update", id=lone_starter, thread=None))
          and stack.status("GET", f"/channels/{lone_thread}", token=world.owner["token"]) == 404
          and world.as_owner("GET", f"/messages/{lone_starter}")["data"]["thread"] is None,
          [(e["serverEvent"], e.get("type")) for e in seen])
    check("a dropped reply leaves a thread that has another",
          not of(seen, "channel", type="delete", id=kept_thread)
          and stack.status("GET", f"/channels/{kept_thread}", token=world.owner["token"]) == 200)
    check("and nobody else ever sees it", not of(owner_stream.gather(1.0), "message", content="not allowed"))
    owner_stream.close()
    check("nor is it in the channel",
          all(m["content"] != "not allowed" for m in
              stack.api("GET", f"/channels/{shared}/messages", token=world.owner["token"])["data"]))


def upload_icon(world: World, token: str, png: bytes) -> str:
    handle = world.stack.api("POST", "/icons", {"mimeType": "image/png", "byteSize": len(png)}, token)
    put = urllib.request.Request(handle["uploadUrl"], data=png, method="PUT", headers={"content-type": "image/png"})
    urllib.request.urlopen(put, timeout=15).close()
    world.stack.api("POST", f"/icons/{handle['id']}/confirm", token=token)
    return handle["id"]


def icons(world: World, check: Checks) -> None:
    say("icons: only pictures, and only the uploader's to give")
    stack = world.stack
    png = picture(16, 16)
    check("an icon that is not a picture every browser shows safely is refused",
          stack.status("POST", "/icons", {"mimeType": "image/svg+xml", "byteSize": 1}, world.owner["token"]) == 400
          and stack.status("POST", "/icons", {"mimeType": "text/html", "byteSize": 1}, world.owner["token"]) == 400)
    check("an upload whose size is not given is refused",
          stack.status("POST", "/icons", {"mimeType": "image/png"}, world.owner["token"]) == 400
          and stack.status("POST", "/attachments", {"fileName": "a.png", "mimeType": "image/png"},
                           world.owner["token"]) == 400)
    handle = world.as_owner("POST", "/icons", {"mimeType": "image/png", "byteSize": len(png)})
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
    # The owner joins too and takes a file the member offers.
    owners_call = join(stack.api("POST", f"/channels/{room}/voice/join", {}, world.owner["token"])["token"])
    check("the owner joins the same call", frame_of(owners_call, "ready") is not None)
    offer = str(uuid.uuid4())
    call.send({"type": "offerFile", "offer": offer, "name": "notes.txt", "size": 5, "allowDirect": True,
               "validForSeconds": 60})
    check("the member's offer reaches the owner", frame_of(owners_call, "fileOffered") is not None)
    owners_call.send({"type": "acceptFile", "offer": offer, "mode": "directPreferred"})
    check("and the owner's acceptance starts a transfer", frame_of(call, "transferStarting") is not None)
    world.as_owner("PATCH", "/admin/settings", {"fileTransfers": False})
    changed = frame_of(call, "grantsChanged", 10)
    check("turning file transfers off reaches a call at once",
          changed is not None and not changed["grants"]["transferFiles"], changed)
    ended = frame_of(call, "transferEnded", 10)
    check("and ends the transfers its people are sending, telling the sender",
          ended is not None and ended.get("reason") == "notPermitted", ended)
    ended = frame_of(owners_call, "transferEnded", 10)
    check("and the receiver", ended is not None and ended.get("reason") == "notPermitted", ended)
    owners_call.close()
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
    fourth = world.sign_in(world.member["name"])
    yet_another = stack.events(fourth["token"])
    stack.api("POST", "/auth/reauthenticate", {"method": "password", "secret": PASSWORD}, world.member["token"])
    stack.api("DELETE", "/users/@me/sign-ins", token=world.member["token"])
    yet_another.gather(1.0)
    world.stream.gather(0.2)
    check("signing out everywhere else closes every other sign-in's stream",
          yet_another.closed == 4401, yet_another.closed)
    check("and their refresh tokens give no new session",
          stack.status("POST", "/auth/token-refresh", {"refreshToken": fourth["refresh"]}) == 401)
    check("but leaves the caller's own sign-in", world.stream.closed is None, world.stream.closed)


def removal(world: World, check: Checks) -> None:
    say("the member removed")
    open_channel = world.channel("after-removal")
    world.stream.gather(0.5)
    world.as_owner("DELETE", f"/communities/{world.community}/members/{world.member['id']}")
    check("removal reaches the member",
          bool(of(world.stream.gather(1.0), "userCommunity", type="delete", user=world.member["id"])))
    check("who then cannot read the community's channels", not world.member_sees(open_channel))
    check("nor hear them", not world.hears_message(open_channel))


def presence(world: World, check: Checks) -> None:
    say("presence, told only to those who share a community and are not blocked")
    stranger = world.account("stranger")

    def status_of(token: str, user: str) -> str:
        return world.stack.api("GET", f"/users/statuses?ids={user}", token=token)[0]["onlineStatus"]

    def profile_status(token: str, user: str) -> str:
        return world.stack.api("GET", f"/users/{user}", token=token)["onlineStatus"]

    # The owner's requests mark them connected.
    world.as_owner("GET", "/users/@me")
    check("a member sees the owner connected",
          status_of(world.member["token"], world.owner["id"]) != "offline"
          and profile_status(world.member["token"], world.owner["id"]) != "offline")
    check("someone who shares nothing with them sees them offline",
          status_of(stranger["token"], world.owner["id"]) == "offline"
          and profile_status(stranger["token"], world.owner["id"]) == "offline")
    counted = world.channel("presence-count")
    made = world.stack.api("POST", "/users/@me/dms", {"recipients": [world.owner["id"]]}, world.member["token"])
    dm = made.get("id") or made["data"]["id"]

    def online_in(channel: str) -> int:
        return world.stack.api("GET", f"/channels/{channel}/presence", token=world.member["token"])["online"]

    world.as_owner("GET", "/users/@me")
    owner_counted = 1 if status_of(world.member["token"], world.owner["id"]) == "online" else 0
    in_channel, in_dm = online_in(counted), online_in(dm)
    world.as_owner("PUT", f"/users/@me/blocks/{world.member['id']}")
    check("once the owner blocks the member, the member sees them offline",
          status_of(world.member["token"], world.owner["id"]) == "offline")
    check("and counted in no channel's presence",
          online_in(counted) == in_channel - owner_counted and online_in(dm) == in_dm - owner_counted,
          (in_channel, in_dm, owner_counted, online_in(counted), online_in(dm)))
    world.as_owner("DELETE", f"/users/@me/blocks/{world.member['id']}")
    check("and unblocked, connected again", status_of(world.member["token"], world.owner["id"]) != "offline")
    # A DM between them shares their presence whatever their communities; the database stands in
    # for one they never had, so that the community alone is what the removal takes away.
    psql(f"DELETE FROM dm_recipient WHERE channel = '{dm}'", world.stack.database)
    world.as_owner("DELETE", f"/communities/{world.community}/members/{world.member['id']}")
    world.stream.gather(0.5)
    check("removed from the community, the member sees the owner offline",
          status_of(world.member["token"], world.owner["id"]) == "offline")


def lost_presence_keys(world: World, check: Checks) -> None:
    say("an invisible user whose presence keys Valkey lost, coming back, is never shown online")

    def status_of(token: str) -> str:
        return world.stack.api("GET", f"/users/statuses?ids={world.owner['id']}", token=token)[0]["onlineStatus"]

    world.as_owner("PUT", "/users/@me/presence-override", {"presenceOverride": "invisible"})
    keys = [f"user:{world.owner['id']}:{kind}" for kind in ("online", "active", "override", "listed")]
    compose("exec", "-T", "valkey", "valkey-cli", "DEL", *keys)
    # Past MARK_EVERY, so the server marks the owner online afresh when they come back.
    time.sleep(16)
    seen: list[str] = []
    stop = threading.Event()

    def poll() -> None:
        while not stop.is_set():
            seen.append(status_of(world.member["token"]))

    poller = threading.Thread(target=poll)
    poller.start()
    stream = world.stack.events(world.owner["token"])
    time.sleep(3)
    stop.set()
    poller.join()
    check("the member reads them as offline throughout, before and after the copy of what they chose",
          bool(seen) and all(s == "offline" for s in seen), {s: seen.count(s) for s in set(seen)})
    check("and they read themself as invisible once back", status_of(world.owner["token"]) == "invisible")
    stream.close()
    world.as_owner("DELETE", "/users/@me/presence-override")


def chosen_presence(world: World, check: Checks) -> None:
    say("a chosen status: what it was reaches only its chooser, and invisible is offline to the rest")

    def status_of(token: str, user: str) -> str:
        return world.stack.api("GET", f"/users/statuses?ids={user}", token=token)[0]["onlineStatus"]

    def chosen(body: dict | None) -> None:
        if body is None:
            world.as_owner("DELETE", "/users/@me/presence-override")
        else:
            world.as_owner("PUT", "/users/@me/presence-override", body)

    counted = world.channel("chosen-presence-count")

    def online_in() -> int:
        return world.stack.api("GET", f"/channels/{counted}/presence", token=world.member["token"])["online"]

    world.as_owner("GET", "/users/@me")
    world.stream.gather(0.5)

    def told(seconds: float = 2.5) -> str | None:
        """What the member's stream, watching the owner, is told of them next, within `seconds`."""
        seen = len(world.stream.ephemeral)
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            world.stream.gather(0.2)
            for event in world.stream.ephemeral[seen:]:
                for status in event.get("statuses", []) if event.get("type") == "presence" else []:
                    if status["id"] == world.owner["id"]:
                        return status["onlineStatus"]
        return None

    heard = len(world.stream.ephemeral)
    world.stream.send({"type": "watchPresence", "userIds": [world.owner["id"]]})
    check("a connection's first watch list is told only of changes, which its client reads whole",
          told() is None)
    check("and the server says when it took the list up",
          any(e.get("type") == "presenceWatching" for e in world.stream.ephemeral[heard:]))
    world.stream.send({"type": "watchPresence", "userIds": []})
    world.stream.gather(1.5)
    world.stream.send({"type": "watchPresence", "userIds": [world.owner["id"]]})
    first = told()
    check("a member whose watch list adds the owner is told their presence at once",
          first not in (None, "offline"), first)
    world.as_owner("PUT", "/users/@me/presence-override", {"presenceOverride": "doNotDisturb"})
    check("and of their choosing do not disturb within moments", told() == "doNotDisturb")
    world.as_owner("PUT", f"/users/@me/blocks/{world.member['id']}")
    check("once the owner blocks them, they are told the owner is offline", told() == "offline")
    world.as_owner("DELETE", f"/users/@me/blocks/{world.member['id']}")
    check("and unblocked, of their presence again", told() == "doNotDisturb")
    world.as_owner("PUT", "/users/@me/presence-override", {"presenceOverride": "invisible", "durationSeconds": 2})
    check("a timed status is told", told() == "offline")
    ended = told(5)
    check("and so is its running out", ended not in (None, "offline"), ended)
    world.as_owner("DELETE", "/users/@me/presence-override")
    told()
    before = online_in()
    owner_counted = 1 if status_of(world.member["token"], world.owner["id"]) == "online" else 0
    chosen({"presenceOverride": "invisible"})
    check("an invisible owner reads as offline to a member, alone and in their profile",
          status_of(world.member["token"], world.owner["id"]) == "offline"
          and world.stack.api("GET", f"/users/{world.owner['id']}", token=world.member["token"])["onlineStatus"]
          == "offline")
    check("and as invisible to themself", status_of(world.owner["token"], world.owner["id"]) == "invisible")
    check("and is counted online in no channel", online_in() == before - owner_counted,
          (before, owner_counted, online_in()))
    check("nor does the member's stream hear what they chose",
          not of(world.stream.gather(1.0), "presenceOverrideChanged"))
    chosen({"presenceOverride": "doNotDisturb"})
    check("in do not disturb, the member sees it", status_of(world.member["token"], world.owner["id"]) == "doNotDisturb")
    made = world.stack.api("POST", "/users/@me/dms", {"recipients": [world.owner["id"]]}, world.member["token"])
    dm = made.get("id") or made["data"]["id"]

    def call_rings_owner() -> bool:
        """Whether the member's starting a call in their DM rings the owner; the call then ends."""
        wait_for("a voice server offer", lambda: world.stack.status(
            "POST", f"/channels/{dm}/voice/join", {}, world.member["token"]) == 200, 90)
        world.stream.gather(0.5)
        call = join(world.stack.api("POST", f"/channels/{dm}/voice/join", {}, world.member["token"])["token"])
        frame_of(call, "ready")
        events = world.stream.gather(2.0)
        call.close()
        wait_for("the call to end", lambda: bool(of(world.stream.gather(1.0), "voiceSessionEnded")), 30)
        return bool(of(events, "voiceRing", type="create", user=world.owner["id"]))

    check("a DM call rings no one in do not disturb", not call_rings_owner())
    chosen(None)
    check("and rings them once it ends", call_rings_owner())
    chosen({"presenceOverride": "doNotDisturb"})
    chosen({"presenceOverride": "away", "durationSeconds": 1})
    check("a timed status shows while it lasts", status_of(world.member["token"], world.owner["id"]) == "away")
    world.as_owner("GET", "/users/@me")
    check("and ends by itself", eventually(
        lambda: status_of(world.member["token"], world.owner["id"]) in ("online", "away")
        and world.as_owner("GET", "/users/@me/presence-override")["presenceOverride"] is None, 5))
    check("thirty days is the longest asked for",
          world.stack.status("PUT", "/users/@me/presence-override",
                             {"presenceOverride": "away", "durationSeconds": 30 * 86400 + 1},
                             world.owner["token"]) == 400)
    chosen({"presenceOverride": "invisible"})
    chosen(None)
    check("ending it shows them connected again", status_of(world.member["token"], world.owner["id"]) != "offline")


def typing(world: World, check: Checks) -> None:
    say("typing, told only to those who may view the channel, and only by those who may send there")
    owner = world.stack.events(world.owner["token"])
    hidden = world.channel("typing-hidden", overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    quiet = world.channel("typing-quiet", overrides=[{"role": world.everyone, "allow": [], "deny": ["sendMessages"]}])
    general = world.channel("typing-open")
    world.stream.gather(0.5)
    owner.gather(0.2)

    def heard(stream, channel: str, user: str, seconds: float = 1.0) -> list[bool]:
        stream.ephemeral.clear()
        stream.gather(seconds)
        return [e["typing"] for e in stream.ephemeral
                if e.get("type") == "typing" and e.get("channelId") == channel and e.get("userId") == user]

    def type_in(stream, channel: str, typing: bool = True) -> None:
        stream.send({"type": "typing" if typing else "stoppedTyping", "channelId": channel})

    def view(stream, *channels: str) -> None:
        # A client hears typing only in the channels it says it has open.
        stream.send({"type": "viewing", "channelIds": list(channels)})
        stream.gather(0.2)

    view(world.stream, general, hidden, quiet)
    view(owner, general, hidden, quiet)
    type_in(owner, general)
    check("a member hears the owner typing", heard(world.stream, general, world.owner["id"]) == [True])
    check("the owner is not told of their own typing", heard(owner, general, world.owner["id"], 0.3) == [])
    type_in(owner, general, False)
    check("and hears them stop", heard(world.stream, general, world.owner["id"]) == [False])
    view(world.stream, hidden, quiet)
    type_in(owner, general)
    check("a member without the channel open hears nobody typing there",
          heard(world.stream, general, world.owner["id"]) == [])
    type_in(owner, general, False)
    view(world.stream, general, hidden, quiet)
    type_in(owner, hidden)
    check("typing in a channel the member may not view does not reach them",
          heard(world.stream, hidden, world.owner["id"]) == [])
    type_in(owner, hidden, False)
    type_in(world.stream, general)
    check("the owner hears the member typing where they may send",
          heard(owner, general, world.member["id"]) == [True])
    type_in(world.stream, general, False)
    type_in(world.stream, quiet)
    check("but typing where the member may not send is not passed on",
          heard(owner, quiet, world.member["id"]) == [])
    dm = world.as_owner("POST", "/users/@me/dms", {"recipients": [world.member["id"]]})
    dm = dm.get("id") or dm["data"]["id"]
    world.stream.gather(0.5)
    view(world.stream, general, dm)
    type_in(owner, dm)
    check("the other person of a DM hears the owner typing there", heard(world.stream, dm, world.owner["id"]) == [True])
    type_in(owner, dm, False)
    world.as_owner("PUT", f"/users/@me/blocks/{world.member['id']}")
    world.stream.gather(0.3)
    type_in(owner, general)
    check("someone the owner blocks does not hear them typing", heard(world.stream, general, world.owner["id"]) == [])
    type_in(owner, general, False)
    world.as_owner("DELETE", f"/users/@me/blocks/{world.member['id']}")
    owner.gather(0.3)
    type_in(owner, general)
    world.stream.gather(0.5)
    owner.close()
    check("the owner's connection closing says at once that they stopped",
          heard(world.stream, general, world.owner["id"], 2.0) == [False])
    again = world.stack.events(world.owner["token"])
    world.as_owner("DELETE", f"/communities/{world.community}/members/{world.member['id']}")
    world.stream.gather(0.5)
    type_in(again, general)
    check("removed from the community, the member no longer hears the owner typing",
          heard(world.stream, general, world.owner["id"]) == [])
    again.close()


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
    stale = world.sign_in(world.member["name"])
    psql(f"UPDATE refresh_token SET verified_at = now() - interval '1 day' WHERE \"user\" = '{world.member['id']}'",
         stack.database)
    refused = stack.request("POST", "/auth/device-links", {}, stale["token"])
    check("a sign-in not verified lately cannot offer itself to another device",
          refused[0] == 403 and "reauthenticationRequired" in refused[1], refused)
    asked = stack.api("POST", "/auth/device-links", {"deviceName": "Computer", "codeChallenge": challenge})["id"]
    scanned = stack.request("POST", f"/auth/device-links/{asked}/scan", {}, stale["token"])
    check("nor give itself to a computer that asks",
          scanned[0] == 403 and "reauthenticationRequired" in scanned[1], scanned)
    stack.api("POST", "/auth/reauthenticate", {"method": "password", "secret": PASSWORD}, world.member["token"])
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


def read_url(url: str) -> tuple[int, bytes]:
    """Reads `url` without credentials, as anyone holding it could."""
    try:
        with urllib.request.urlopen(url, timeout=15) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, b""


def evidence(world: World, check: Checks) -> None:
    say("the files of deleted messages and removed attachments are kept for reviewers alone")
    stack, member = world.stack, world.member
    channel = world.channel("evidence")
    category = stack.api("GET", "/report-categories", token=world.owner["token"])[0]["id"]

    def upload(body: bytes) -> dict:
        handle = stack.api("POST", "/attachments", {"fileName": "kept.txt", "mimeType": "text/plain",
                                                    "byteSize": len(body)}, member["token"])
        put = urllib.request.Request(handle["uploadUrl"], data=body, method="PUT",
                                     headers={"content-type": "text/plain"})
        urllib.request.urlopen(put, timeout=15).close()
        return stack.api("POST", f"/attachments/{handle['id']}/confirm", token=member["token"])

    def post(attachments: list[str]) -> str:
        posted = stack.api("POST", f"/channels/{channel}/messages",
                           {"content": "kept", "attachments": attachments}, member["token"])["id"]
        stack.api("POST", f"/messages/{posted}/reports", {"category": category}, world.owner["token"])
        return posted

    deleted_file = upload(b"deleted message's file")
    deleted = post([deleted_file["id"]])
    removed_file, kept_file = upload(b"removed file"), upload(b"file that stays")
    edited = post([removed_file["id"], kept_file["id"]])
    stack.api("DELETE", f"/messages/{deleted}", token=member["token"])
    stack.api("DELETE", f"/messages/{edited}/attachments/{removed_file['id']}", token=member["token"])
    check("its uploader no longer reads a deleted message's file",
          stack.status("GET", f"/attachments/{deleted_file['id']}", token=member["token"]) == 404)
    check("nor one they took off its message",
          stack.status("GET", f"/attachments/{removed_file['id']}", token=member["token"]) == 404)
    check("the public read path soon stops serving either",
          soon(lambda: read_url(deleted_file["downloadUrl"])[0] in (403, 404)
               and read_url(removed_file["downloadUrl"])[0] in (403, 404), 20))
    check("while the file that stays is still served", read_url(kept_file["downloadUrl"]) == (200, b"file that stays"))
    check("nor may a new message take kept evidence up",
          stack.status("POST", f"/channels/{channel}/messages",
                       {"content": "again", "attachments": [deleted_file["id"]]}, member["token"]) == 400)

    reviewer = world.account("evidencereviewer")
    for permission in MODERATION:
        stack.command("admin", "deny", permission)
    stack.command("admin", "grant", reviewer["name"])
    stack.command("admin", "allow", "reviewReports")
    try:
        page = stack.api("GET", "/admin/reports?limit=50", token=reviewer["token"])
        files = {a["id"]: a for a in page["attachments"]}
        messages = {m["message"]["id"]: m for m in page["messages"]}
        check("a reviewer reads the deleted message's file at a signed URL",
              deleted_file["id"] in files
              and read_url(files[deleted_file["id"]]["downloadUrl"]) == (200, b"deleted message's file"), files)
        check("and the file taken off the reported message, which the case names",
              removed_file["id"] in messages.get(edited, {}).get("removedAttachments", [])
              and removed_file["id"] in files
              and read_url(files[removed_file["id"]]["downloadUrl"]) == (200, b"removed file"), messages.get(edited))
        signed = files.get(deleted_file["id"], {}).get("downloadUrl", "")
        stack.command("attachments", "purge", "--message", deleted)
        page = stack.api("GET", "/admin/reports?limit=50", token=reviewer["token"])
        check("purging from the terminal deletes it outright",
              deleted_file["id"] not in {a["id"] for a in page["attachments"]}
              and (not signed or read_url(signed)[0] in (403, 404)))
        log = stack.api("GET", "/admin/moderation-log", token=reviewer["token"])
        check("and the moderation log records it, with no account as its actor",
              any(e["action"] == "purgeAttachment" and e.get("actor") is None for e in log), log[:3])
        try:
            stack.command("attachments", "purge", "--attachment", kept_file["id"])
            refused = False
        except Failed:
            refused = True
        check("the terminal refuses to purge a file a message still holds", refused)
    finally:
        stop_review_powers(world, reviewer)


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
    stack.api("POST", f"/users/{reviewer['id']}/reports", {"category": category, "aspects": ["username"]},
              world.member["token"])
    cases = stack.api("GET", "/admin/reports", token=reviewer["token"])["cases"]
    check("a case about the reviewer is not among those they read",
          not any(c["subject"] == reviewer["id"] for c in cases), cases)
    bot = stack.api("POST", "/users/@me/bots", {"name": f"rbot{world.run}"}, reviewer["token"])["bot"]
    stack.api("POST", f"/users/{bot['id']}/reports", {"category": category, "aspects": ["username"]},
              world.member["token"])
    cases = stack.api("GET", "/admin/reports", token=reviewer["token"])["cases"]
    check("nor a case about the reviewer's own bot",
          not any(c["subject"] == bot["id"] for c in cases), cases)
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


def banned_owners_bots(world: World, check: Checks) -> None:
    say("a ban from the deployment shuts out the banned person's bots until it is lifted")
    stack, member = world.stack, world.member
    stack.command("admin", "grant", world.owner["name"])
    stack.command("admin", "allow", "banUsers")
    bot_token = stack.api("POST", "/users/@me/bots", {"name": f"bot{world.run}"}, member["token"])["token"]
    check("the member's bot works", stack.status("GET", "/users/@me", token=bot_token) == 200)
    bot_stream = stack.events(bot_token)
    bot_stream.gather(0.5)
    world.as_owner("PUT", f"/admin/users/{member['id']}/ban", {"reason": "checking bots"})
    bot_stream.gather(1.0)
    check("banning its owner closes the bot's stream as banned", bot_stream.closed == 4410, bot_stream.closed)
    check("and refuses its token", stack.status("GET", "/users/@me", token=bot_token) == 401)
    world.as_owner("DELETE", f"/admin/users/{member['id']}/ban")
    check("lifting the ban restores it", stack.status("GET", "/users/@me", token=bot_token) == 200)
    bot_id = stack.api("GET", "/users/@me", token=bot_token)["id"]
    plain = world.as_owner("POST", "/admin/roles", {"name": f"Plain{world.run}", "permissions": []})["id"]
    check("a bot is given no deployment role",
          stack.status("PUT", f"/admin/users/{bot_id}/roles/{plain}", token=world.owner["token"]) == 400)
    check("and its token never opens the dashboard",
          stack.status("GET", "/admin/overview", token=bot_token) == 403)
    world.as_owner("DELETE", f"/admin/roles/{plain}")
    stack.command("admin", "deny", "banUsers")
    stack.command("admin", "revoke", world.owner["name"])


def bot_transfers(world: World, check: Checks) -> None:
    say("a bot changes hands only when its recipient accepts, and its old token ends then")
    stack = world.stack
    giver, heir = world.account("giver"), world.account("heir")
    made = stack.api("POST", "/users/@me/bots", {"name": f"tbot{world.run}"}, giver["token"])
    bot, old_token = made["bot"]["id"], made["token"]

    def stale() -> None:
        psql(f"UPDATE refresh_token SET verified_at = now() - interval '1 day' WHERE \"user\" = '{giver['id']}'",
             stack.database)

    stale()
    rotated = stack.request("POST", f"/bots/{bot}/token", token=giver["token"])
    check("issuing a bot a new token asks for a fresh verification",
          rotated[0] == 403 and "reauthenticationRequired" in rotated[1], rotated)
    offered = stack.request("PUT", f"/bots/{bot}/transfer", {"owner": heir["id"]}, giver["token"])
    check("and so does offering it", offered[0] == 403 and "reauthenticationRequired" in offered[1], offered)
    stack.api("POST", "/auth/reauthenticate", {"method": "password", "secret": PASSWORD}, giver["token"])
    stack.api("PUT", f"/bots/{bot}/transfer", {"owner": heir["id"]}, giver["token"])
    check("an offer leaves the bot with its owner",
          stack.api("GET", f"/users/{bot}", token=giver["token"])["botOwner"] == giver["id"])
    offers = stack.api("GET", "/users/@me/bot-transfers", token=heir["token"])
    check("the recipient finds the offer", any(o["bot"]["id"] == bot for o in offers), offers)
    stack.api("DELETE", f"/bots/{bot}/transfer", token=giver["token"])
    check("a withdrawn offer cannot be accepted",
          stack.status("POST", f"/bots/{bot}/transfer/acceptance", token=heir["token"]) == 404)
    stack.api("PUT", f"/bots/{bot}/transfer", {"owner": heir["id"]}, giver["token"])
    bot_stream = stack.events(old_token)
    bot_stream.gather(0.5)
    accepted = stack.api("POST", f"/bots/{bot}/transfer/acceptance", token=heir["token"])
    bot_stream.gather(1.0)
    check("accepting makes the recipient its owner", accepted["bot"]["botOwner"] == heir["id"], accepted["bot"])
    check("closes the stream its old token opened", bot_stream.closed == 4401, bot_stream.closed)
    check("refuses the old token", stack.status("GET", "/users/@me", token=old_token) == 401)
    check("and takes the new one", stack.status("GET", "/users/@me", token=accepted["token"]) == 200)
    check("and its old owner can no longer issue it a token",
          stack.status("POST", f"/bots/{bot}/token", token=giver["token"]) == 403)


def moderator_ranks(world: World, check: Checks) -> None:
    say("moderating the deployment reaches no one ranked at or above the moderator there")
    stack, member = world.stack, world.member
    stack.command("admin", "grant", world.owner["name"])
    stack.command("admin", "allow", "moderateCommunities")
    # Each role is made below the others, so Senior outranks Mods.
    senior_role = world.as_owner("POST", "/admin/roles", {"name": f"Senior{world.run}", "permissions": []})["id"]
    mods = world.as_owner("POST", "/admin/roles", {"name": f"Mods{world.run}",
                                                   "permissions": ["moderateCommunities"]})["id"]
    world.as_owner("PUT", f"/admin/users/{member['id']}/roles/{mods}")
    senior, plain = world.account("senior"), world.account("plain")
    for person in (senior, plain):
        invite = world.as_owner("POST", f"/communities/{world.community}/invites", {})
        stack.api("PUT", f"/communities/{world.community}/members/@me",
                  {"inviteCode": invite.get("code") or invite.get("id")}, person["token"])
    world.as_owner("PUT", f"/admin/users/{senior['id']}/roles/{senior_role}")
    channel = world.channel("ranked")

    def said(person: dict) -> str:
        return stack.api("POST", f"/channels/{channel}/messages", {"content": "hello", "attachments": []},
                         person["token"])["id"]

    check("a moderator does not delete the message of someone who outranks them on the deployment",
          stack.status("DELETE", f"/messages/{said(senior)}", token=member["token"]) == 403)
    check("nor remove them from a community",
          stack.status("DELETE", f"/communities/{world.community}/members/{senior['id']}",
                       token=member["token"]) == 403)
    check("but deletes the message of someone with no deployment role",
          stack.status("DELETE", f"/messages/{said(plain)}", token=member["token"]) == 204)
    world.as_owner("DELETE", f"/admin/users/{member['id']}/roles/{mods}")
    world.as_owner("DELETE", f"/admin/roles/{mods}")
    world.as_owner("DELETE", f"/admin/roles/{senior_role}")
    stack.command("admin", "deny", "moderateCommunities")
    stack.command("admin", "revoke", world.owner["name"])


def ban_deletions(world: World, check: Checks) -> None:
    say("a ban's deletion window reaches only channels the banner may view")
    stack, member = world.stack, world.member
    banners = world.role("Banners", ["banMembers", "manageMessages"])
    world.give(banners)
    target = world.account("target")
    invite = world.as_owner("POST", f"/communities/{world.community}/invites", {})
    stack.api("PUT", f"/communities/{world.community}/members/@me",
              {"inviteCode": invite.get("code") or invite.get("id")}, target["token"])
    hidden = world.channel("hidden-from-banners",
                           overrides=[{"role": banners, "allow": [], "deny": ["viewChannel"]}])
    shown = world.channel("seen-by-banners")

    def said(channel: str) -> str:
        return stack.api("POST", f"/channels/{channel}/messages",
                         {"content": "soon banned", "attachments": []}, target["token"])["id"]

    in_hidden, in_shown = said(hidden), said(shown)
    banned = stack.api("PUT", f"/communities/{world.community}/bans/{target['id']}",
                       {"deleteMessagesSeconds": 3600}, member["token"])
    check("the ban says how many of their messages go", banned.get("deletedMessages") == 1, banned)
    # The deletion is a job (`app::jobs`), done once the ban commits.
    check("the message where the banner may view is deleted, shortly after", eventually(
        lambda: stack.status("GET", f"/messages/{in_shown}", token=world.owner["token"]) == 404))
    check("the one in a channel hidden from the banner stays",
          stack.status("GET", f"/messages/{in_hidden}", token=world.owner["token"]) == 200)


def job_preview(world: World, check: Checks) -> None:
    say("the jobs preview takes View jobs")
    stack, member = world.stack, world.member
    check("a member without it is refused",
          stack.status("GET", "/admin/jobs", token=member["token"]) == 403)
    viewers = world.account("job-viewer")
    stack.command("admin", "grant", viewers["name"])
    shown = stack.api("GET", "/admin/jobs", token=viewers["token"])
    check("an administrator, who holds it, reads it", isinstance(shown.get("waitingCounts"), list), shown)
    check("and it names no job's payload",
          all("payload" not in job for job in shown.get("waiting", []) + shown.get("running", [])))
    stack.command("admin", "revoke", viewers["name"])
    check("once their role goes, they are refused again",
          stack.status("GET", "/admin/jobs", token=viewers["token"]) == 403)


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
    listed = stack.status("GET", f"/admin/users/{member['id']}/dms", token=watcher["token"])
    entries = stack.api("GET", "/admin/moderation-log?limit=100", token=watcher["token"])
    check("listing someone's DMs is logged, naming them", listed == 200 and any(
        e.get("action") == "listDms" and e.get("subject") == member["id"] for e in entries), listed)
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
                               {"settings": {"watchWords": ["pineapple"], "words": ["durian"]},
                                "grant": ["viewChannel", "sendMessages"]})
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
    ballot = {"question": "Fruit?", "options": [{"label": "apple"}, {"label": "durian"}], "multipleChoice": False,
              "allowWriteIns": True, "anonymous": False, "durationSeconds": 3600}
    check("it decides a poll's answers as it decides a message, refusing what it would mask",
          stack.status("POST", f"/channels/{watched}/polls", ballot, world.owner["token"]) == 422)
    ballot["options"][1]["label"] = "pear"
    poll = world.as_owner("POST", f"/channels/{watched}/polls", ballot)["id"]
    check("and an answer written in",
          stack.status("POST", f"/polls/{poll}/write-ins", {"label": "durian"}, world.member["token"]) == 422)
    first = stack.api("POST", f"/messages/{world.post(watched, 'a thread to start')}/thread/messages",
                      {"content": "durian in the thread", "attachments": []}, world.member["token"])
    check("it decides the reply that makes a thread, where the thread will be",
          "durian" not in first["content"] and first["alteredBy"] == [WORD_FILTER_ID]
          and stack.status("GET", f"/channels/{first['channelId']}", token=world.member["token"]) == 200, first)

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
    check("but a channel of another kind holds no calendar",
          stack.status("GET", f"/plugins/{CALENDAR_ID}/routes/calendars/{announce}/events",
                       token=world.member["token"]) == 404)
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
    # A route's actions reach only where its caller may look: hidden from the announcements, a
    # member's new event is kept, but the plugin's account posts no card there on their behalf.
    world.as_owner("PUT", f"/channels/{announce}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.5)
    latest = stack.api("GET", f"/channels/{announce}/messages?limit=1", token=world.owner["token"])["data"][0]["id"]
    added = stack.status("POST", events, {"title": "Unseen", "start": soon}, world.member["token"])
    time.sleep(0.5)
    still = stack.api("GET", f"/channels/{announce}/messages?limit=1", token=world.owner["token"])["data"][0]["id"]
    check("a member who cannot see where events are announced adds one, and no card is posted there for them",
          added == 201 and still == latest, added)
    world.as_owner("DELETE", f"/channels/{announce}/overrides/{world.everyone}")
    world.stream.gather(0.5)
    stack.command("admin", "grant", world.owner["name"])
    stack.command("admin", "allow", "banUsers")
    world.as_owner("PUT", f"/admin/users/{world.member['id']}/ban", {})
    check("banned from the deployment, their private URL answers nothing",
          stack.status("GET", f"{stack.base}{feed}") == 404)
    world.as_owner("DELETE", f"/admin/users/{world.member['id']}/ban")
    stack.command("admin", "deny", "banUsers")
    stack.command("admin", "revoke", world.owner["name"])

    # The ban ended the member's sign-ins; they sign in again to go on.
    member = world.sign_in(world.member["name"])["token"]
    # An event is deleted by whoever added it, or by someone who may manage messages.
    mine = stack.api("POST", events, {"title": "Mine", "start": soon}, member)["id"]
    owners = world.as_owner("POST", events, {"title": "Theirs", "start": soon})["id"]
    check("a member may not delete someone else's event",
          stack.status("DELETE", f"{events}/{owners}", token=member) == 403)
    check("but may delete their own",
          stack.status("DELETE", f"{events}/{mine}", token=member) == 204)
    check("and its reminder goes with it",
          psql(f"SELECT count(*) FROM job WHERE kind = 'firePluginTimer' AND key = '{CALENDAR_ID}/remind:{mine}'", stack.database) == "0")
    # Deleting the calendar deletes the reminders set in it, with its events.
    check("an event's reminder is kept in the calendar's scope",
          psql(f"SELECT count(*) FROM job WHERE kind = 'firePluginTimer' AND payload->>'scope' = '{calendar}'", stack.database) != "0")
    world.as_owner("DELETE", f"/channels/{calendar}")
    # What a plugin kept in a deleted channel is forgotten by a job (`forgetPluginScope`).
    check("deleting the calendar deletes its reminders, shortly after", eventually(
        lambda: psql(f"SELECT count(*) FROM job WHERE kind = 'firePluginTimer' AND payload->>'scope' = '{calendar}'", stack.database) == "0"))
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
        return psql(f"SELECT payload->'mail' FROM job WHERE kind = 'sendEmail' "
                    f"AND payload->>'user' = '{member['id']}' "
                    f"AND payload->'mail'->>'kind' = 'digest'", stack.database)

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


def deleted_communities(world: World, check: Checks) -> None:
    say("a deleted community and a deleted message: nothing found, shared, joined, or reacted to")
    stack, member = world.stack, world.member
    thumbs = "%F0%9F%91%8D"
    general = world.channel("doomed")
    gone = world.post(general, "soon deleted")
    world.as_owner("DELETE", f"/messages/{gone}")
    check("a deleted message takes no reaction",
          stack.status("PUT", f"/messages/{gone}/reactions/{thumbs}/@me", token=member["token"]) == 404)
    check("and lists no reactors",
          stack.status("GET", f"/messages/{gone}/reactions/{thumbs}", token=member["token"]) == 404)
    word = f"doomedword{world.run}"
    posted = world.post(general, f"{word} in a community about to go")
    code = world.as_owner("POST", f"/communities/{world.community}/invites", {})["code"]

    def found(query: str) -> bool:
        read = stack.api("GET", f"/messages?filter[text]={word}{query}", token=member["token"])
        rows = read["data"] if isinstance(read, dict) else read
        return any(m["id"] == posted for m in rows)

    def status_of(user: str) -> str:
        return stack.api("GET", f"/users/statuses?ids={user}", token=member["token"])[0]["onlineStatus"]

    check("a member finds its messages while it stands", soon(lambda: found("")))
    world.as_owner("GET", "/users/@me")
    check("and sees its owner connected", status_of(world.owner["id"]) != "offline")
    world.as_owner("DELETE", f"/communities/{world.community}")
    world.stream.gather(0.5)
    check("once deleted, its messages are found nowhere", not found(""))
    check("nor by naming it",
          stack.status("GET", f"/messages?filter[text]={word}&filter[community]={world.community}",
                       token=member["token"]) == 404)
    check("its members share nothing to start a DM over",
          stack.status("POST", "/users/@me/dms", {"recipients": [world.owner["id"]]}, member["token"]) == 400)
    world.as_owner("GET", "/users/@me")
    check("nor to see each other's presence", status_of(world.owner["id"]) == "offline")
    check("its invites read as not found", stack.status("GET", f"/invites/{code}", token=member["token"]) == 404)
    stranger = world.account("stranger")
    check("and join nobody to it",
          stack.status("PUT", f"/communities/{world.community}/members/@me", {"inviteCode": code},
                       stranger["token"]) in (400, 404))


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
    elsewhere = world.channel("not-a-table")
    check("a channel of another kind has no table",
          stack.status("GET", f"/plugins/{BLACKJACK_ID}/routes/tables/{elsewhere}", token=member["token"]) == 404)

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
        # The second is refused as made at a version the table has left, or, when the first ended
        # the players' turns, as coming when no one is to decide: the phase is looked at first.
        check("a decision sent twice counts once",
              decided_twice[0] == 409 and any(why in decided_twice[1] for why in ("stale", "notNow")),
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


def frequent_emoji(world: World, check: Checks) -> None:
    say("the emoji someone reacts with most are theirs alone, and follow their reactions")
    stack, member = world.stack, world.member
    channel = world.channel("quick")
    post = world.post(channel, "react to me")
    party = "%F0%9F%8E%89"
    mine = f"/users/@me/frequent-emoji?community={world.community}"

    def used() -> list[str]:
        return [f["emoji"] for f in stack.api("GET", mine, token=member["token"])]

    stack.api("PUT", f"/messages/{post}/reactions/{party}/@me", token=member["token"])
    check("a reaction counts toward its author's most used", used()[:1] == ["\U0001F389"], used())
    check("no one else may read them",
          stack.status("GET", f"/users/{member['id']}/frequent-emoji", token=world.owner["token"]) == 403)
    stack.api("DELETE", f"/messages/{post}/reactions/{party}/@me", token=member["token"])
    check("a reaction taken back no longer counts", "\U0001F389" not in used(), used())


def saved_messages(world: World, check: Checks) -> None:
    say("saved messages follow access to what they save, and are their saver's alone")
    stack, member = world.stack, world.member["token"]
    staff = world.role("Staff")
    world.give(staff)
    secret = world.channel("saved-secret", overrides=[{"role": world.everyone, "allow": [], "deny": ["viewChannel"]}])
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": ["viewChannel"], "deny": []})
    said = world.post(secret, "come back to this")
    world.stream.gather(0.8)

    def saves(token: str = member) -> list[str]:
        return [s["message"] for s in stack.api("GET", "/users/@me/saved-messages", token=token)]

    def saved_messages() -> list[str]:
        return [m["id"] for m in stack.api("GET", "/users/@me/saved-messages/messages", token=member)["data"]]

    check("the member saves a message they may read", stack.status("PUT", f"/users/@me/saved-messages/{said}",
                                                                   token=member) == 201)
    got = world.stream.gather(1.0)
    check("and their devices hear of it", len(of(got, "savedMessageChanged", message=said)) == 1, got)
    check("saving it again answers the same save",
          stack.status("PUT", f"/users/@me/saved-messages/{said}", token=member) == 200)
    check("it is listed, with its message", saves() == [said] and saved_messages() == [said])
    check("no one else sees it", said not in saves(world.owner["token"]))
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.8)
    check("once the member loses the channel, the save is unlisted", saves() == [] and saved_messages() == [])
    check("and nothing more of it may be saved",
          stack.status("PUT", f"/users/@me/saved-messages/{said}", token=member) == 404)
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": ["viewChannel"], "deny": []})
    world.stream.gather(0.8)
    check("given it back, the save is listed again", saves() == [said])
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.8)
    world.as_owner("DELETE", f"/messages/{said}")
    got = world.stream.gather(1.0)
    check("deleting the message, unseen, still tells its saver the save is gone",
          bool(of(got, "savedMessageChanged", message=said, saved=None)), got)
    world.as_owner("PUT", f"/channels/{secret}/overrides/{staff}", {"allow": ["viewChannel"], "deny": []})
    check("and it is gone for good", saves() == [])
    dm = world.as_owner("POST", "/users/@me/dms", {"recipients": [world.member["id"]]})
    dm_id = dm.get("id") or dm["data"]["id"]
    private = world.post(dm_id, "between us")
    outsider = world.account("outsider")
    check("someone outside a DM cannot save its messages",
          stack.status("PUT", f"/users/@me/saved-messages/{private}", token=outsider["token"]) == 404)
    stack.api("PUT", f"/users/@me/saved-messages/{private}", token=member)
    world.stream.gather(0.5)
    stack.api("DELETE", f"/users/@me/saved-messages/{private}", token=member)
    got = world.stream.gather(1.0)
    check("unsaving reaches the saver's devices",
          bool(of(got, "savedMessageChanged", message=private, saved=None)) and saves() == [], got)


def thread_follows(world: World, check: Checks) -> None:
    say("following threads: by taking part, by hand, and as access changes")
    stack, member = world.stack, world.member["token"]
    talk = world.channel("follow-talk")
    starter = world.post(talk, "replies welcome")
    thread = stack.api("PUT", f"/messages/{starter}/thread", token=member)["id"]

    def follows(token: str = member) -> list[str]:
        return [f["thread"] for f in stack.api("GET", "/users/@me/thread-follows", token=token)]

    def feed(query: str = "") -> list[str]:
        return [m["id"] for m in stack.api("GET", f"/users/@me/activity{query}", token=member)["data"]]

    check("the starter's author follows the thread once it is made", follows(world.owner["token"]) == [thread])
    check("opening it follows nothing", follows() == [])
    world.stream.gather(0.5)
    stack.api("POST", f"/channels/{thread}/messages", {"content": "me too", "attachments": []}, member)
    got = world.stream.gather(1.0)
    check("posting in it follows it, heard by the poster's devices",
          follows() == [thread] and bool(of(got, "threadFollowChanged", thread=thread, following=True)), got)
    reply = world.post(thread, "a reply no one is tagged in")
    check("a followed thread's untagged replies are in the follower's feed", reply in feed())
    stack.api("DELETE", f"/channels/{thread}/follows/@me", token=member)
    got = world.stream.gather(1.0)
    check("unfollowing is heard", bool(of(got, "threadFollowChanged", thread=thread, following=False)), got)
    check("and the replies leave the feed", reply not in feed())
    tagging = world.post(thread, f"<@{world.member['id']}> look")
    check("a reply tagging them follows it again", follows() == [thread] and tagging in feed())
    latest = stack.api("GET", f"/channels/{thread}/messages", token=member)
    newest = max(m["id"] for m in (latest.get("data") if isinstance(latest, dict) else latest))
    check("a thread keeps its reader's position",
          stack.status("PUT", f"/channels/{thread}/read-states/@me", {"lastRead": newest}, member) == 204)
    check("and what is read there leaves the unread feed", tagging not in feed("?filter[unread]=true")
          and tagging in feed())
    world.as_owner("PUT", f"/channels/{talk}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.8)
    check("once the channel is lost, its thread's follow is unlisted", follows() == [])
    check("and its replies leave the feed", tagging not in feed() and reply not in feed())
    check("and it cannot be followed",
          stack.status("PUT", f"/channels/{thread}/follows/@me", token=member) == 404)
    world.as_owner("DELETE", f"/channels/{talk}/overrides/{world.everyone}")
    world.stream.gather(0.8)
    check("given it back, the follow returns", follows() == [thread])
    check("only threads are followed", stack.status("PUT", f"/channels/{talk}/follows/@me", token=member) == 400)


def activity_feed(world: World, check: Checks) -> None:
    say("the activity feed holds what notifies its reader, as they may read it now")
    stack, member = world.stack, world.member["token"]
    room = world.channel("activity-room")

    def feed(query: str = "") -> list[str]:
        return [m["id"] for m in stack.api("GET", f"/users/@me/activity{query}", token=member)["data"]]

    plain = world.post(room, "nothing for you")
    tagged = world.post(room, f"<@{world.member['id']}> this is for you")
    own = stack.api("POST", f"/channels/{room}/messages", {"content": "my own", "attachments": []}, member)["id"]
    check("at the default level only tags are in it, never the reader's own",
          tagged in feed() and plain not in feed() and own not in feed(), feed())
    stack.api("PUT", f"/communities/{world.community}/notification-settings/@me", {"level": "all"}, member)
    check("at every message, every message is", plain in feed() and tagged in feed())
    stack.api("PUT", f"/channels/{room}/mutes/@me", {}, member)
    check("a muted channel gives nothing", plain not in feed() and tagged not in feed())
    stack.api("DELETE", f"/channels/{room}/mutes/@me", token=member)
    check("filtered to another community, it holds none of this one",
          feed(f"?filter[community]={world.everyone}") == [])
    dm = world.as_owner("POST", "/users/@me/dms", {"recipients": [world.member["id"]]})
    dm_id = dm.get("id") or dm["data"]["id"]
    whisper = world.post(dm_id, "psst")
    check("a DM's messages are in it", whisper in feed())
    check("unless DMs are filtered out", whisper not in feed("?filter[dms]=false") and plain in feed("?filter[dms]=false"))
    world.as_owner("PUT", f"/channels/{room}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    world.stream.gather(0.8)
    check("once the channel is lost, nothing of it is in the feed", plain not in feed() and tagged not in feed())
    world.as_owner("DELETE", f"/channels/{room}/overrides/{world.everyone}")
    stack.api("PUT", f"/channels/{room}/read-states/@me", {"lastRead": tagged}, member)
    check("what is read leaves the unread feed", tagged not in feed("?filter[unread]=true") and tagged in feed())
    outsider = world.account("feedless")
    check("nothing of a community reaches the feed of someone outside it",
          not ({plain, tagged, whisper} & set(m["id"] for m in stack.api(
              "GET", "/users/@me/activity", token=outsider["token"])["data"])))


def emoji_deletions(world: World, check: Checks) -> None:
    say("a deleted custom emoji is gone at once, and its reactions by a job soon after")
    stack, member = world.stack, world.member
    icon = upload_icon(world, world.owner["token"], picture(32, 32))
    emoji = world.as_owner("POST", f"/communities/{world.community}/emoji", {"name": "soon_gone", "icon": icon})["id"]
    channel = world.channel("emoji")
    post = world.post(channel, "react with it")
    key = urllib.parse.quote(f"<:{emoji}>", safe="")
    stack.api("PUT", f"/messages/{post}/reactions/{key}/@me", token=member["token"])
    world.stream.gather(0.5)
    world.as_owner("DELETE", f"/emoji/{emoji}")
    got = world.stream.gather(1.0)
    check("deleting it reaches the member at once", bool(of(got, "customEmoji", type="delete", id=emoji)),
          [e["serverEvent"] for e in got])
    check("and it is listed no more",
          all(e["id"] != emoji for e in stack.api("GET", f"/communities/{world.community}/emoji",
                                                    token=member["token"])))
    check("nobody may react with it after",
          stack.status("PUT", f"/messages/{post}/reactions/{key}/@me", token=world.owner["token"]) in (400, 404))
    # The reactions go by a job (`purgeCustomEmoji`), and then the emoji and its picture.
    check("its reactions are taken off, shortly after", eventually(
        lambda: psql(f"SELECT count(*) FROM react WHERE custom_emoji = '{emoji}'", stack.database) == "0"))
    check("and then it goes, with its picture", eventually(
        lambda: psql(f"SELECT count(*) FROM custom_emoji WHERE id = '{emoji}'", stack.database) == "0"
        and psql(f"SELECT count(*) FROM icon WHERE id = '{icon}'", stack.database) == "0"))


def plugin_removal(world: World, check: Checks) -> None:
    say("a removed plugin stops at once, and its account, timers, and storage go by jobs")
    stack, member = world.stack, world.member
    stack.command("plugins", "enable", CALENDAR_ID)
    running(world, CALENDAR_ID)
    world.as_owner("PUT", f"/communities/{world.community}/plugins/{CALENDAR_ID}",
                   {"settings": {}, "grant": ["viewChannel", "sendMessages"]})
    principal = next(p["principal"] for p in stack.api("GET", "/plugins", token=member["token"])
                     if p["id"] == CALENDAR_ID)
    calendar = world.as_owner("POST", "/channels", {
        "name": "removed", "ty": "plugin", "pluginType": f"{CALENDAR_ID}:calendar",
        "community": world.community, "sortIndex": 3})["id"]
    events = f"/plugins/{CALENDAR_ID}/routes/calendars/{calendar}/events"
    later = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 7200))
    world.as_owner("POST", events, {"title": "Never", "start": later})
    timers = f"SELECT count(*) FROM job WHERE kind = 'firePluginTimer' AND payload->>'plugin' = '{CALENDAR_ID}'"
    check("its event set a timer", psql(timers, stack.database) != "0")
    world.stream.gather(0.5)
    stack.command("plugins", "remove", CALENDAR_ID, "--yes")
    check("removed, it is listed nowhere", eventually(
        lambda: all(p["id"] != CALENDAR_ID for p in stack.api("GET", "/plugins", token=member["token"]))))
    check("and its routes answer nobody", eventually(
        lambda: stack.status("GET", events, token=world.owner["token"]) == 404))
    # Its account leaves each community by a job (`retirePlugin`), announced as any leaving is.
    heard: list[dict] = []
    check("its account leaves the community, which the member hears", eventually(
        lambda: heard.extend(world.stream.gather(0.5)) or bool(of(heard, "userCommunity", user=principal))))
    check("and the owner finds it gone",
          stack.status("GET", f"/communities/{world.community}/members/{principal}", token=world.owner["token"]) == 404)
    stack.command("plugins", "purge", CALENDAR_ID, "--yes")
    check("purging it deletes its timers and what it kept, shortly after", eventually(
        lambda: psql(timers, stack.database) == "0"
        and psql(f"SELECT count(*) FROM plugin_storage WHERE plugin = '{CALENDAR_ID}'", stack.database) == "0"))


SCENARIOS = [private_channels, granting_and_revoking, edits_after_send, moves_and_categories, hidden_categories, hidden_managers,
             role_grants,
             poll_votes, poll_write_ins, deleted_parents, first_replies, thread_echoes, calls, attachments,
             operators, deployment_settings, sign_ins, removal, presence, chosen_presence, lost_presence_keys, typing, name_colours, dual_invites, device_links,
             nicknames, review_powers, evidence, ban_ranks, banned_owners_bots, bot_transfers, moderator_ranks, ban_deletions, job_preview, dm_reads, frequent_emoji, emoji_deletions,
             group_dm_moderators, plugins, profile_annotations, calendar_channels, blackjack_tables, plugin_removal, email, invite_previews,
             deleted_communities, previews, icons, uploads, saved_messages, thread_follows, activity_feed]


def main() -> None:
    parser = argparse.ArgumentParser(description="Check that changes to access reach everything already open.")
    parser.add_argument("--bin", type=Path, required=True, help="the directory holding the binaries")
    parser.add_argument("--start-services", action="store_true", help="docker compose up the services first")
    parser.add_argument("--only", help="the scenarios to run, comma separated by name; every one when absent")
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
            only = None if args.only is None else set(args.only.split(","))
            for index, scenario in enumerate(SCENARIOS):
                if only is not None and scenario.__name__ not in only:
                    continue
                # Each starts from a community and accounts of its own, so one that fails, or stops
                # on a request refused, leaves the rest fair.
                try:
                    scenario(World(stack, f"{run}s{index:02d}"), check)
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
