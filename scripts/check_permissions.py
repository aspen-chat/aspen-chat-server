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
import shutil
import sys
import time
import urllib.request
from pathlib import Path

from stack import Failed, Ports, Stack, WebSocket, start_services, wait_for, wait_for_services

PORTS = Ports(nats=14322, api=18100, voice=19101, api_metrics=19564, voice_metrics=19565, rtc_min=45200, rtc_max=45399,
              transfer=13578, relay_min=46100, relay_max=46199)
PASSWORD = "check-permissions-password"
# Scenarios make several accounts and many changes in moments, as no person would.
SETTINGS = "[rate_limits]\nenabled = false\n"


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

    def join(token: str) -> WebSocket:
        socket = WebSocket(f"ws://127.0.0.1:{PORTS.voice}/ws")
        socket.send({"type": "identify", "token": token})
        return socket

    def frame_of(socket: WebSocket, kind: str, seconds: float = 5) -> dict | None:
        deadline = time.monotonic() + seconds
        while (left := deadline - time.monotonic()) > 0:
            frame = socket.receive(left)
            if frame is None and socket.closed is not None:
                return None
            if frame is not None and frame.get("type") == kind:
                return frame
        return None

    def recorded() -> bool:
        read = stack.api("GET", f"/communities/{world.community}?include=voice", token=world.owner["token"])
        return any(p["user"] == world.member["id"] for p in read.get("included", {}).get("voiceParticipants", []))

    first = offer()
    call = join(first["token"])
    check("the member joins the call", frame_of(call, "ready") is not None)
    wait_for("the call's record to show the member", recorded, 30)
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
    world.as_owner("POST", f"/attachments/{handle['id']}/confirm")
    path = f"/attachments/{handle['id']}"
    check("its uploader reads it", stack.status("GET", path, token=world.owner["token"]) == 200)
    check("nobody else reads it before it is sent", stack.status("GET", path, token=world.member["token"]) == 404)
    check("nor deletes it", stack.status("DELETE", path, token=world.member["token"]) == 404)
    general = world.channel("pictures")
    posting = {"content": "mine now", "attachments": [handle["id"]]}
    check("nor sends it as their own",
          stack.status("POST", f"/channels/{general}/messages", posting, world.member["token"]) == 400)
    world.as_owner("POST", f"/channels/{general}/messages", {"content": "notes", "attachments": [handle["id"]]})
    check("once sent, whoever may view the channel reads it",
          stack.status("GET", path, token=world.member["token"]) == 200)
    world.as_owner("PUT", f"/channels/{general}/overrides/{world.everyone}", {"allow": [], "deny": ["viewChannel"]})
    check("and nobody once they may not", stack.status("GET", path, token=world.member["token"]) == 404)


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

SCENARIOS = [private_channels, granting_and_revoking, moves_and_categories, calls, attachments, operators,
             sign_ins, removal, name_colours, dual_invites, device_links]

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
