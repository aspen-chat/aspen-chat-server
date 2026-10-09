"""The checks `scripts/dev_federation.py check` runs against the two deployments `up` started:
the directory of other deployments, keys and their handovers, signing in abroad, DMs across
deployments, and foreign users' standing."""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import struct
import sys
import time
import urllib.parse
import urllib.request

from dev_federation import (
    ALPHA, BETA, DEPLOYMENTS, PASSWORD, Deployment, Failed, api, bin_of, clean_env, expect, request, restart, run,
    running_pid, say, sign_in, terminal,
)


def check(_args: argparse.Namespace) -> None:
    for deployment in DEPLOYMENTS:
        if running_pid(deployment) is None:
            raise Failed(f"{deployment.name} is not running; run `{sys.argv[0]} up` first")
    beta_path = f"/admin/federation/deployments/{urllib.parse.quote(BETA.domain, safe='')}"

    status, text = request("GET", f"https://{BETA.domain}/.well-known/aspen")
    document = json.loads(text)
    expect(status == 200 and document["domain"] == BETA.domain and document["keys"][0]["retiredAt"] is None,
           "beta publishes its document, its current key first")
    expect(document["users"] == {"emigration": "blockList", "immigration": "blockList"},
           "beta's document carries its gates")

    admin = sign_in(ALPHA, "alphaadmin")
    terminal(ALPHA, "admin", "grant", "alphaadmin")
    stranger = sign_in(ALPHA, "alphastranger")
    api(ALPHA, "GET", "/admin/federation", token=stranger, expect=(403,))
    expect(True, "someone without Manage federation is refused")

    # Start from nothing known, so the check can run again.
    request("DELETE", f"https://{ALPHA.domain}/api/v1{beta_path}", token=admin)
    overview = api(ALPHA, "GET", "/admin/federation", token=admin)
    expect(overview["domain"] == ALPHA.domain and overview["keyFingerprint"].startswith("SHA256:"),
           "alpha's overview names its domain and key")
    expect(overview["listsInForce"] == ["usersEmigrationAllow", "botsImmigrationBlock"],
           "alpha reads the lists its gates name")

    added = api(ALPHA, "POST", "/admin/federation/deployments", {"domain": f"Beta.Localhost:{BETA.port}", "note": "dev"},
                token=admin, expect=(201,))
    expect(added["domain"] == BETA.domain and added["publicKey"] is None, "alpha adds beta, not yet contacted")
    api(ALPHA, "POST", "/admin/federation/deployments", {"domain": BETA.domain}, token=admin, expect=(409,))
    api(ALPHA, "POST", "/admin/federation/deployments", {"domain": ALPHA.domain}, token=admin, expect=(400,))
    expect(True, "adding beta again, or alpha itself, is refused")
    nowhere = "nowhere.localhost:9999"
    nowhere_path = f"/admin/federation/deployments/{urllib.parse.quote(nowhere, safe='')}"
    request("DELETE", f"https://{ALPHA.domain}/api/v1{nowhere_path}", token=admin)
    api(ALPHA, "POST", "/admin/federation/deployments", {"domain": nowhere}, token=admin, expect=(201,))
    status, text = request("POST", f"https://{ALPHA.domain}/api/v1{nowhere_path}/contact", token=admin)
    api(ALPHA, "DELETE", nowhere_path, token=admin, expect=(204,))
    expect(status == 502 and f"{nowhere} refused the connection" in json.loads(text)["detail"],
           "contacting a deployment nothing serves says the connection was refused")
    expect(added["admission"] == {"usersEmigration": False, "usersImmigration": True,
                                  "botsEmigration": False, "botsImmigration": True},
           "beta's admission follows alpha's gates before any list")

    beta_status = terminal(BETA, "federation", "status")
    beta_key = next(line.split()[1] for line in beta_status.splitlines() if line.startswith("key:"))
    contacted = api(ALPHA, "POST", f"{beta_path}/contact", token=admin)
    expect(contacted["outcome"] == "pinned" and contacted["deployment"]["publicKeyFingerprint"] == beta_key,
           "contacting beta over TLS pins the key beta reports")
    again = api(ALPHA, "POST", f"{beta_path}/contact", token=admin)
    expect(again["outcome"] == "confirmed", "contacting it again confirms the pinned key")
    peer = again["deployment"]
    expect(peer["protocol"]["version"] == 1 and peer["software"]["name"] == "aspen" and peer["compatible"],
           "alpha records the protocol and software beta says it runs")

    listed = api(ALPHA, "PUT", f"{beta_path}/lists/usersEmigrationAllow", token=admin, expect=(201,))
    expect(listed["admission"]["usersEmigration"] and listed["lists"] == ["usersEmigrationAllow"],
           "on alpha's allow list, beta may take alpha's users")
    api(ALPHA, "PUT", f"{beta_path}/lists/usersEmigrationAllow", token=admin, expect=(200,))
    blocked = api(ALPHA, "PUT", f"{beta_path}/lists/botsImmigrationBlock", token=admin, expect=(201,))
    expect(not blocked["admission"]["botsImmigration"], "on alpha's block list, beta's bots may not come")
    api(ALPHA, "DELETE", f"{beta_path}/lists/botsImmigrationBlock", token=admin, expect=(204,))
    found = api(ALPHA, "GET", f"/admin/federation/deployments?filter[name]={urllib.parse.quote(BETA.domain)}", token=admin)
    expect([d["domain"] for d in found] == [BETA.domain] and found[0]["lists"] == ["usersEmigrationAllow"],
           "the directory finds beta by name with its lists")

    new_key = terminal(BETA, "federation", "rotate-key", "--compromised").split()[-1]
    changed = api(ALPHA, "POST", f"{beta_path}/contact", token=admin)
    expect(changed["outcome"] == "keyChanged"
           and changed["deployment"]["publicKeyFingerprint"] == beta_key
           and changed["deployment"]["offeredKeyFingerprint"] == new_key,
           "after beta replaces its key, alpha refuses the new one and keeps it as offered")
    api(ALPHA, "PUT", f"{beta_path}/key", {"publicKey": changed["deployment"]["publicKey"]}, token=admin,
        expect=(409,))
    expect(True, "accepting a key other than the offered one is refused")
    accepted = api(ALPHA, "PUT", f"{beta_path}/key", {"publicKey": changed["deployment"]["offeredKey"]}, token=admin)
    expect(accepted["publicKeyFingerprint"] == new_key and accepted["offeredKey"] is None,
           "accepting the offered key pins it")
    expect(api(ALPHA, "POST", f"{beta_path}/contact", token=admin)["outcome"] == "confirmed",
           "beta's new key is confirmed from then on")

    # The terminal reaches the same directory, the other way round.
    if ALPHA.domain in terminal(BETA, "federation", "list"):
        terminal(BETA, "federation", "list-remove", ALPHA.domain, "usersSharedBlock")
        terminal(BETA, "federation", "remove", ALPHA.domain)
    output = terminal(BETA, "federation", "add", ALPHA.domain, "--note", "from the terminal")
    expect("pinned" in output, "beta's terminal adds alpha and pins its key")
    output = terminal(BETA, "federation", "list-add", ALPHA.domain, "usersSharedBlock")
    expect("admits: no one" in output, "on beta's shared block list, alpha is admitted neither way")
    terminal(BETA, "federation", "list-remove", ALPHA.domain, "usersSharedBlock")
    check_abroad(admin)
    say("every check passed")


def check_dms_abroad(traveller: str) -> None:
    """DMs on beta with alpha's users: beta hosts one only with one of its own in it, and tells
    alpha when one of alpha's users is put in one. That alpha passes the notice on to its user's
    devices is checked in a browser; here, that it takes it and refuses a forged one."""
    stamp = int(time.time())
    host = sign_in(BETA, f"betahost{stamp}")
    club = api(BETA, "POST", "/communities", {"name": f"Beta DMs {stamp}"}, token=host)
    invite = api(BETA, "POST", f"/communities/{club['id']}/invites", {}, token=host)
    other = sign_in(ALPHA, f"alphaother{stamp}")
    abroad = {}
    for name, token in [("traveller", traveller), ("other", other)]:
        status, session = sign_in_abroad(assertion_for(token))
        if status != 200:
            raise Failed(f"signing {name} in at beta answered {status}")
        api(BETA, "PUT", f"/communities/{club['id']}/members/@me", {"inviteCode": invite["code"]},
            token=session["sessionToken"], expect=(200, 201))
        abroad[name] = (session["sessionToken"], session["userId"])
    status, text = request("POST", f"https://{BETA.domain}/api/v1/users/@me/dms",
                           {"recipients": [abroad["other"][1]]}, abroad["traveller"][0])
    expect(status == 403 and json.loads(text)["code"] == "federationRefused",
           "beta hosts no DM between two of alpha's users")
    host_id = api(BETA, "GET", "/users/@me", token=host)["id"]
    dm = api(BETA, "POST", "/users/@me/dms", {"recipients": [abroad["traveller"][1]]}, token=host,
             expect=(200, 201))
    expect(dm["ty"] == "dm" and host_id in dm["recipients"], "a beta user starts a DM with alpha's user")
    status, text = request("POST", f"https://{ALPHA.domain}/api/v1/federation/notices", {"notice": "a.b.c"})
    expect(status == 401 and json.loads(text)["code"] == "assertionInvalid", "alpha refuses a forged notice")
    # Delivered in the background; a refusal or failure is logged at once.
    time.sleep(2)
    log = (BETA.dir / "server.log").read_text()
    expect("refused a notice" not in log and "gave up delivering a notice" not in log,
           "beta delivered its notice to alpha")


def abroad_alive(token: str) -> bool:
    return request("GET", f"https://{BETA.domain}/api/v1/users/@me", token=token)[0] == 200


def signed_out_soon(token: str, seconds: float = 10) -> bool:
    """Whether a session at beta ends within `seconds`: signing out the users of a deployment the
    gates no longer admit is a job (`shutOut`), which a server starts within moments."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not abroad_alive(token):
            return True
        time.sleep(0.25)
    return False


def wait_until(what: str, done, seconds: float = 30) -> None:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if done():
            return
        time.sleep(0.5)
    raise Failed(f"{what} did not happen within {seconds:.0f} s")


def check_standing() -> None:
    """Beta asks alpha about alpha's users signed in there every few seconds, as it would every
    hour: one who left beta loses their session there, an account deleted at alpha is retired at
    beta at once by alpha's notice, and beta's moderators ban and unban one of alpha's users."""
    restart(BETA, {"ASPEN_FEDERATION__STANDING_INTERVAL_SECONDS": "3"})
    stamp = int(time.time())
    leaver = sign_in(ALPHA, f"alphaleaver{stamp}")
    status, left = sign_in_abroad(assertion_for(leaver))
    expect(status == 200 and abroad_alive(left["sessionToken"]), "alpha's leaver signs in at beta")
    api(ALPHA, "DELETE", f"/users/@me/foreign-deployments/{urllib.parse.quote(BETA.domain, safe='')}",
        token=leaver, expect=(204,))
    wait_until("beta ending the leaver's session", lambda: not abroad_alive(left["sessionToken"]))
    expect(True, "once alpha no longer lets them be at beta, beta's next check ends their session")
    status, again = sign_in_abroad(assertion_for(leaver))
    expect(status == 200 and abroad_alive(again["sessionToken"]),
           "signing in at beta again is allowed, and lists beta again")

    deleter = sign_in(ALPHA, f"alphadeleter{stamp}")
    status, deleted = sign_in_abroad(assertion_for(deleter))
    deleter_at_beta = deleted["userId"]
    api(ALPHA, "DELETE", "/users/@me", token=deleter, expect=(204,))
    wait_until("beta retiring the deleted account", lambda: not abroad_alive(deleted["sessionToken"]), 10)
    expect(True, "an account deleted at alpha is gone from beta too")

    moderator = sign_in(BETA, f"betamod{stamp}")
    terminal(BETA, "admin", "grant", f"betamod{stamp}")
    terminal(BETA, "admin", "allow", "moderateCommunities")
    terminal(BETA, "admin", "allow", "banUsers")
    rogue = sign_in(ALPHA, f"alpharogue{stamp}")
    status, roguish = sign_in_abroad(assertion_for(rogue))
    api(BETA, "PUT", f"/admin/users/{roguish['userId']}/ban", {}, token=moderator, expect=(201,))
    expect(not abroad_alive(roguish["sessionToken"]), "a ban ends the user's sessions at beta")
    expect(problem(*sign_in_abroad(assertion_for(rogue))) == "403 federationRefused",
           "a banned user cannot sign in at beta again")
    api(BETA, "PUT", f"/admin/users/{deleter_at_beta}/ban", {}, token=moderator, expect=(404,))
    log = api(BETA, "GET", "/admin/moderation-log", token=moderator)
    entries = log if isinstance(log, list) else log.get("entries", log.get("data", []))
    expect(any(e.get("action") == "banUser" for e in entries), "the ban is in beta's moderation log")
    api(BETA, "DELETE", f"/admin/users/{roguish['userId']}/ban", token=moderator, expect=(204,))
    status, _ = sign_in_abroad(assertion_for(rogue))
    expect(status == 200, "once the ban is lifted they may sign in again")
    restart(BETA)


def check_gates_live(traveller: str) -> None:
    """Beta's administrator closes beta's immigration gate from the dashboard while one of alpha's
    users is signed in there: their session ends at once, signing in is refused, and opening the
    gate again lets them back, all without a restart."""
    stamp = int(time.time())
    beta_admin = sign_in(BETA, f"betagates{stamp}")
    terminal(BETA, "admin", "grant", f"betagates{stamp}")
    status, visit = sign_in_abroad(assertion_for(traveller))
    expect(status == 200 and abroad_alive(visit["sessionToken"]), "alpha's traveller is signed in at beta")
    closed = api(BETA, "PATCH", "/admin/federation", {"usersImmigration": "closed", "usersSharedList": False},
                 token=beta_admin)
    expect(closed["users"]["immigration"] == "closed", "beta's administrator closes beta's immigration gate")
    expect(signed_out_soon(visit["sessionToken"]), "closing the gate ends the traveller's session at beta within moments")
    expect(problem(*sign_in_abroad(assertion_for(traveller))) == "403 federationRefused",
           "and beta refuses them while it is closed")
    api(BETA, "PATCH", "/admin/federation", {"usersImmigration": "blockList", "usersSharedList": True},
        token=beta_admin)
    status, _ = sign_in_abroad(assertion_for(traveller))
    expect(status == 200, "once the gate opens again they sign in at beta")


def totp(secret: str, step_offset: int = 0) -> str:
    """The RFC 6238 code for `secret` (base32) at the current step plus `step_offset`."""
    key = base64.b32decode(secret + "=" * (-len(secret) % 8))
    counter = int(time.time()) // 30 + step_offset
    digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    offset = digest[-1] & 0x0F
    value = struct.unpack(">I", digest[offset:offset + 4])[0] & 0x7FFFFFFF
    return f"{value % 1_000_000:06d}"


def assertion_for(token: str, audience: Deployment = BETA, home: Deployment = ALPHA) -> str:
    return api(home, "POST", "/auth/assertions", {"audience": audience.domain}, token=token)["assertion"]


def sign_in_abroad(assertion: str, invite: str | None = None, at: Deployment = BETA) -> tuple[int, dict]:
    body = {"assertion": assertion, **({"inviteCode": invite} if invite else {})}
    status, text = request("POST", f"https://{at.domain}/api/v1/auth/federated-sign-in", body)
    return status, json.loads(text) if text else {}


def subprocess_status(deployment: Deployment, *args: str) -> tuple[int, str]:
    """Runs an operator command on `deployment` that may fail, and returns its exit status and
    everything it printed."""
    done = run(str(bin_of(deployment) / "aspen-chat-server"), *args, cwd=deployment.dir, env=clean_env())
    return done.returncode, done.stdout + done.stderr


def problem(status: int, body: dict) -> str:
    return f"{status} {body.get('code')}"


def check_abroad(admin: str) -> None:
    """Signing in abroad from alpha at beta: the assertion, the foreign user, profiles and
    avatars following home, both deployments' gates, key handovers and compromises, and a host
    that requires two factors and invites."""
    beta_path = f"/admin/federation/deployments/{urllib.parse.quote(BETA.domain, safe='')}"
    api(ALPHA, "PUT", f"{beta_path}/lists/usersEmigrationAllow", token=admin, expect=(200, 201))
    traveller = sign_in(ALPHA, "alphatraveller")
    api(ALPHA, "PATCH", "/users/@me", {"displayName": "Traveller"}, token=traveller)
    assertion = assertion_for(traveller)
    status, session = sign_in_abroad(assertion)
    expect(status == 200 and not session["twoFactorEnrollmentRequired"], "an alpha user signs in at beta")
    abroad = session["sessionToken"]
    me = api(BETA, "GET", "/users/@me", token=abroad)
    expect(me["homeDomain"] == ALPHA.domain and me["name"] == "alphatraveller" and me["displayName"] == "Traveller",
           "at beta they are alpha's alphatraveller, with their profile")
    expect(problem(*sign_in_abroad(assertion)) == "401 assertionInvalid", "an assertion is used once")
    expect(problem(*sign_in_abroad(assertion_for(traveller), at=ALPHA)) == "401 assertionInvalid",
           "an assertion is accepted only where it is for")
    visited = api(ALPHA, "GET", "/users/@me/foreign-deployments", token=traveller)
    expect([d["domain"] for d in visited] == [BETA.domain], "alpha remembers where its user signed in")

    api(BETA, "PATCH", "/users/@me", {"displayName": "Elsewhere"}, token=abroad, expect=(403,))
    api(BETA, "PATCH", "/users/@me", {"status": {"text": "visiting"}}, token=abroad)
    api(BETA, "POST", "/users/@me/bots", {"name": "travelbot"}, token=abroad, expect=(403,))
    api(BETA, "POST", "/auth/assertions", {"audience": ALPHA.domain}, token=abroad, expect=(403,))
    expect(True, "abroad, the profile is home's, bots are made at home, and travel starts at home")

    api(ALPHA, "PATCH", "/users/@me", {"displayName": "Traveller Two"}, token=traveller)
    png = base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
    )
    upload = api(ALPHA, "POST", "/icons", {"mimeType": "image/png", "byteSize": len(png)}, token=traveller)
    put = urllib.request.Request(upload["uploadUrl"], data=png, method="PUT", headers={"content-type": "image/png"})
    with urllib.request.urlopen(put, timeout=20) as response:
        if response.status not in (200, 201):
            raise Failed(f"uploading an avatar answered {response.status}")
    api(ALPHA, "POST", f"/icons/{upload['id']}/confirm", token=traveller)
    api(ALPHA, "PATCH", "/users/@me", {"icon": upload["id"]}, token=traveller)
    status, again = sign_in_abroad(assertion_for(traveller))
    abroad = again["sessionToken"]
    me2 = api(BETA, "GET", "/users/@me", token=abroad)
    expect(status == 200 and again["userId"] == session["userId"] and me2["displayName"] == "Traveller Two",
           "signing in again is the same user, with the profile as it is at home")
    for _ in range(40):
        me2 = api(BETA, "GET", "/users/@me", token=abroad)
        if me2.get("icon"):
            break
        time.sleep(0.25)
    copied = me2.get("icon")
    expect(copied is not None and copied != upload["id"], "their avatar is copied into beta's storage")
    expect(api(BETA, "GET", f"/icons/{copied}", token=abroad)["id"] == copied, "beta serves its copy")

    api(ALPHA, "DELETE", f"{beta_path}/lists/usersEmigrationAllow", token=admin, expect=(204,))
    status, text = request("POST", f"https://{ALPHA.domain}/api/v1/auth/assertions", {"audience": BETA.domain}, traveller)
    expect(status == 403 and json.loads(text)["code"] == "federationRefused",
           "off alpha's allow list, alpha signs nothing for beta")
    api(ALPHA, "PUT", f"{beta_path}/lists/usersEmigrationAllow", token=admin, expect=(201,))
    expect(abroad_alive(abroad), "the traveller is signed in at beta")
    terminal(BETA, "federation", "list-add", ALPHA.domain, "usersSharedBlock")
    expect(signed_out_soon(abroad), "putting alpha on beta's block list signs its users out of beta within moments")
    expect(problem(*sign_in_abroad(assertion_for(traveller))) == "403 federationRefused",
           "on beta's block list, alpha's users are turned away")
    status, text = subprocess_status(BETA, "federation", "remove", ALPHA.domain)
    expect(status != 0 and "block list" in text, "beta will not forget alpha while it is on a block list")
    terminal(BETA, "federation", "list-remove", ALPHA.domain, "usersSharedBlock")

    # A block on a name blocks every name under it and every port: alpha is alpha.localhost on
    # another port.
    stamp = int(time.time())
    beta_admin = sign_in(BETA, f"betablocks{stamp}")
    terminal(BETA, "admin", "grant", f"betablocks{stamp}")
    bare = "alpha.localhost"
    bare_path = f"/admin/federation/deployments/{bare}"
    request("DELETE", f"https://{BETA.domain}/api/v1{bare_path}/lists/usersSharedBlock", token=beta_admin)
    request("DELETE", f"https://{BETA.domain}/api/v1{bare_path}", token=beta_admin)
    api(BETA, "POST", "/admin/federation/deployments", {"domain": bare}, token=beta_admin, expect=(201,))
    status, visit = sign_in_abroad(assertion_for(traveller))
    expect(status == 200 and abroad_alive(visit["sessionToken"]), "the traveller is signed in at beta again")
    api(BETA, "PUT", f"{bare_path}/lists/usersSharedBlock", token=beta_admin, expect=(201,))
    expect(signed_out_soon(visit["sessionToken"]),
           f"blocking {bare} signs out the users of {ALPHA.domain}, on another port, within moments")
    expect(problem(*sign_in_abroad(assertion_for(traveller))) == "403 federationRefused",
           f"and {ALPHA.domain} stays blocked while {bare} is")
    api(BETA, "DELETE", bare_path, token=beta_admin, expect=(409,))
    api(BETA, "DELETE", f"{bare_path}/lists/usersSharedBlock", token=beta_admin, expect=(204,))
    api(BETA, "DELETE", bare_path, token=beta_admin, expect=(204,))

    planned = terminal(ALPHA, "federation", "rotate-key", "--planned").split()[-1]
    status, again = sign_in_abroad(assertion_for(traveller))
    expect(status == 200 and planned in terminal(BETA, "federation", "list"),
           "after alpha hands over to a new key, beta follows the handover on its own")
    abroad = again["sessionToken"]
    compromised = terminal(ALPHA, "federation", "rotate-key", "--compromised").split()[-1]
    # Beta contacts a statement's sender at most every fifteen seconds
    # (`received::CONTACT_INTERVAL_SECONDS`), and it just contacted alpha for the handover.
    time.sleep(16)
    expect(problem(*sign_in_abroad(assertion_for(traveller))) == "401 assertionInvalid"
           and f"offers a new key {compromised}" in terminal(BETA, "federation", "list"),
           "after alpha replaces a compromised key, beta refuses it and holds it as offered")
    expect(signed_out_soon(abroad), "and signs alpha's users out until the new key is accepted")
    terminal(BETA, "federation", "accept-key", ALPHA.domain, "--fingerprint", compromised)
    status, _ = sign_in_abroad(assertion_for(traveller))
    expect(status == 200, "once beta's operator accepts the new key, alpha's users sign in again")

    check_dms_abroad(traveller)
    check_standing()

    check_gates_live(traveller)

    # Settings changed from the terminal reach the running servers without a restart, a moment
    # after the command returns.
    terminal(BETA, "settings", "set", "--require-two-factor", "true", "--users-immigration-invite-required", "true")
    wait_until("beta reading its new settings", lambda: api(BETA, "GET", "/auth/methods")["twoFactorRequired"], 5)
    try:
        expect(problem(*sign_in_abroad(assertion_for(traveller))) == "403 strongerSignInRequired",
               "where beta requires two factors, a password sign-in at home is not enough")
        # A new account each run: the first arrival is what is checked.
        newcomer_name = f"alphanew{int(time.time())}"
        newcomer = sign_in(ALPHA, newcomer_name)
        enrollment = api(ALPHA, "POST", "/users/@me/totp", token=newcomer, expect=(201,))
        api(ALPHA, "POST", "/users/@me/totp/confirmation", {"code": totp(enrollment["secret"])}, token=newcomer)
        challenge = api(ALPHA, "POST", "/auth/login", {"username": newcomer_name, "password": PASSWORD})
        # A code counts once per step; the next step's is accepted as drift.
        strong = api(ALPHA, "POST", "/auth/login/second-factor",
                     {"ticket": challenge["ticket"], "method": "totp", "code": totp(enrollment["secret"], 1)})
        expect(problem(*sign_in_abroad(assertion_for(strong["sessionToken"])))
               == "403 registrationInviteRequired",
               "a first arrival needs an invite where beta asks one of accounts from elsewhere")
        invite = terminal(BETA, "invites", "create").strip()
        status, arrived = sign_in_abroad(assertion_for(strong["sessionToken"]), invite)
        expect(status == 200 and not arrived["twoFactorEnrollmentRequired"],
               "with a second factor at home and an invite, they arrive, owing beta no second factor")
        expect(api(BETA, "GET", "/users/@me", token=arrived["sessionToken"])["homeDomain"] == ALPHA.domain,
               "and their session works")
    finally:
        terminal(BETA, "settings", "set", "--require-two-factor", "false",
                 "--users-immigration-invite-required", "false")
        wait_until("beta reading its settings again",
                   lambda: not api(BETA, "GET", "/auth/methods")["twoFactorRequired"], 5)
