#!/usr/bin/env python3
"""Two Aspen deployments on one machine, federating over real TLS, for developing federation.

    scripts/dev_federation.py up       # certificates, services, databases, and both servers
    scripts/dev_federation.py check    # exercises federation between them
    scripts/dev_federation.py down     # stops the servers and their services

The deployments are `alpha.localhost:8443` and `beta.localhost:8444`. Names under `localhost`
resolve to the loopback address on their own, and each server presents a certificate for its
name from a development certificate authority made here, which each trusts for calls to the
other through `[federation.development] extra_root_certificates`. To reach them from a browser
or another tool, trust `target/dev-federation/ca.pem` there. Nothing here serves plain HTTP.

Each deployment has a database of its own (`aspen_dev_alpha`, `aspen_dev_beta`) in the
docker-compose PostgreSQL, and a NATS and a Valkey of its own in containers this script starts,
so neither shares an event stream, rate limits, or fleet heartbeats with the other or with a
development stack on the usual ports. Everything else it makes is in `target/dev-federation/`:
the authority and certificates, each deployment's `aspen.toml`, and the servers' logs. The
servers are the debug builds in `target/debug` (`cargo build` first), or, with `--alpha-bin` and
`--beta-bin`, builds of other versions, which is how two versions are checked against each other;
`up` leaves them running and `down` stops them. Databases are kept between runs; `down --drop` drops them too.

Needs Python 3.10, `openssl`, and `docker compose` with the database and SeaweedFS services up.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import signal
import ssl
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
WORK = REPO / "target" / "dev-federation"
BIN = REPO / "target" / "debug"
CA = WORK / "ca.pem"
# The S3 credentials docker-compose.yaml gives SeaweedFS.
S3_ACCESS_KEY = "GK484e56c38fb7e14b182bf47a"
S3_SECRET_KEY = "6b49da9e42f7959cc946d7987a504763f6ec405b88abeeec08aa926b61316027"
PASSWORD = "dev-federation-password"


@dataclass(frozen=True)
class Deployment:
    name: str
    port: int
    nats_port: int
    valkey_port: int
    metrics_port: int
    federation: str

    @property
    def host(self) -> str:
        return f"{self.name}.localhost"

    @property
    def domain(self) -> str:
        return f"{self.host}:{self.port}"

    @property
    def dir(self) -> Path:
        return WORK / self.name

    @property
    def database(self) -> str:
        return f"aspen_dev_{self.name}"

    @property
    def containers(self) -> list[str]:
        return [f"aspen-dev-{self.name}-nats", f"aspen-dev-{self.name}-valkey"]


# Alpha lets its users go only where it allows and takes anyone's; beta shares one block list
# between both directions. Their bots stay home, but alpha takes others' bots unless blocked.
ALPHA = Deployment("alpha", 8443, 4231, 6391, 19471, """
[federation.users]
emigration = "allowList"
immigration = "open"
[federation.bots]
emigration = "closed"
immigration = "blockList"
""")
BETA = Deployment("beta", 8444, 4232, 6392, 19472, """
[federation.users]
emigration = "blockList"
immigration = "blockList"
shared_list = true
""")
DEPLOYMENTS = [ALPHA, BETA]


class Failed(Exception):
    pass


def say(message: str) -> None:
    print(f"dev-federation: {message}", flush=True)


def run(*args: str, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(list(args), capture_output=True, text=True, **kwargs)


def psql(sql: str, database: str = "postgres") -> str:
    done = run("docker", "compose", "exec", "-T", "database", "psql", "-U", "postgres", "-d", database, "-tAc", sql,
               cwd=REPO, check=True)
    return done.stdout.strip()


def clean_env() -> dict[str, str]:
    """This environment without ASPEN_ variables, which would override the configs written."""
    return {k: v for k, v in os.environ.items() if not k.startswith("ASPEN_")}


def make_certificates() -> None:
    WORK.mkdir(parents=True, exist_ok=True)
    if not CA.exists():
        say("making the development certificate authority")
        run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
            "-keyout", str(WORK / "ca.key"), "-out", str(CA), "-days", "3650",
            "-subj", "/CN=Aspen development certificate authority",
            "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign", check=True)
    for deployment in DEPLOYMENTS:
        cert = WORK / f"{deployment.host}.pem"
        if cert.exists():
            continue
        say(f"issuing a certificate for {deployment.host}")
        key = WORK / f"{deployment.host}.key"
        request = WORK / f"{deployment.host}.csr"
        extensions = WORK / f"{deployment.host}.ext"
        extensions.write_text(
            f"subjectAltName=DNS:{deployment.host}\n"
            "extendedKeyUsage=serverAuth\n"
            "basicConstraints=critical,CA:FALSE\n"
            "keyUsage=critical,digitalSignature\n"
        )
        run("openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
            "-keyout", str(key), "-out", str(request), "-subj", f"/CN={deployment.host}", check=True)
        run("openssl", "x509", "-req", "-in", str(request), "-CA", str(CA), "-CAkey", str(WORK / "ca.key"),
            "-CAcreateserial", "-out", str(cert), "-days", "825", "-extfile", str(extensions), check=True)
        request.unlink()
        extensions.unlink()


def start_services(deployment: Deployment) -> None:
    nats, valkey = deployment.containers
    for name, image, ports, command in [
        (nats, "nats:2.11-alpine", f"127.0.0.1:{deployment.nats_port}:4222", ["--jetstream", "--auth", "aspen_test"]),
        (valkey, "valkey/valkey:alpine", f"127.0.0.1:{deployment.valkey_port}:6379", []),
    ]:
        state = run("docker", "inspect", "-f", "{{.State.Running}}", name)
        if state.returncode == 0 and state.stdout.strip() == "true":
            continue
        if state.returncode == 0:
            run("docker", "start", name, check=True)
        else:
            run("docker", "run", "-d", "--name", name, "-p", ports, image, *command, check=True)


def write_config(deployment: Deployment) -> None:
    deployment.dir.mkdir(parents=True, exist_ok=True)
    (deployment.dir / "aspen.toml").write_text(
        f'database_url = "postgres://postgres:aspen_test@127.0.0.1:5432/{deployment.database}"\n'
        f'nats_url = "nats://127.0.0.1:{deployment.nats_port}"\n'
        'nats_auth_token = "aspen_test"\n'
        f'valkey_url = "redis://127.0.0.1:{deployment.valkey_port}"\n'
        "[media.s3]\n"
        'endpoint = "http://127.0.0.1:8333"\n'
        'region = "seaweed"\n'
        'bucket = "aspen-media"\n'
        f'access_key = "{S3_ACCESS_KEY}"\n'
        f'secret_key = "{S3_SECRET_KEY}"\n'
        'public_base_url = "http://127.0.0.1:8888/buckets/aspen-media"\n'
        "[metrics]\n"
        f'listen_addr = "127.0.0.1:{deployment.metrics_port}"\n'
        "[cors]\n"
        'allowed_origins = ["*"]\n'
        # Checks run back to back, registering and contacting more often than a person would.
        "[rate_limits]\n"
        "enabled = false\n"
        "[federation]\n"
        f'domain = "{deployment.domain}"\n'
        f"{deployment.federation.strip()}\n"
        "[federation.development]\n"
        f"extra_root_certificates = [{json.dumps(str(CA))}]\n"
        "allow_private_addresses = true\n"
    )


def pid_file(deployment: Deployment) -> Path:
    return deployment.dir / "server.pid"


def running_pid(deployment: Deployment) -> int | None:
    try:
        pid = int(pid_file(deployment).read_text())
        os.kill(pid, 0)
        return pid
    except (FileNotFoundError, ValueError, ProcessLookupError, PermissionError):
        return None


def tls_context() -> ssl.SSLContext:
    return ssl.create_default_context(cafile=str(CA))


def request(method: str, url: str, body: dict | None = None, token: str | None = None) -> tuple[int, str]:
    headers = {"content-type": "application/json"}
    if token:
        headers["authorization"] = f"Bearer {token}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, method=method, data=data, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=20, context=tls_context()) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def wait_for_server(deployment: Deployment, seconds: float = 60) -> None:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if running_pid(deployment) is None:
            raise Failed(f"{deployment.name} exited; see {deployment.dir / 'server.log'}")
        try:
            if request("GET", f"https://{deployment.domain}/api/v1/auth/methods")[0] == 200:
                return
        except (urllib.error.URLError, ConnectionError, TimeoutError):
            pass
        time.sleep(1)
    raise Failed(f"{deployment.name} did not come up within {seconds:.0f} s; see {deployment.dir / 'server.log'}")


def bin_of(deployment: Deployment) -> Path:
    """The build a deployment runs: the one `up` was given for it, or `target/debug`."""
    chosen = deployment.dir / "bin"
    return Path(chosen.read_text().strip()) if chosen.exists() else BIN


# The servers this run started, which it must wait for when it stops them.
STARTED: dict[str, subprocess.Popen] = {}


def start_server(deployment: Deployment, extra_env: dict[str, str] | None = None) -> None:
    say(f"starting {deployment.name} at https://{deployment.domain}")
    log = open(deployment.dir / "server.log", "a")
    process = subprocess.Popen(
        [str(bin_of(deployment) / "aspen-chat-server"), "--port", str(deployment.port),
         "--listen-addr", "::1", "--listen-addr", "127.0.0.1",
         "--key", str(WORK / f"{deployment.host}.key"), "--cert", str(WORK / f"{deployment.host}.pem")],
        cwd=deployment.dir, env={**clean_env(), **(extra_env or {})}, stdout=log, stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    STARTED[deployment.name] = process
    pid_file(deployment).write_text(str(process.pid))


def stop_server(deployment: Deployment) -> None:
    pid = running_pid(deployment)
    if pid is None:
        return
    say(f"stopping {deployment.name}")
    os.kill(pid, signal.SIGINT)
    started = STARTED.pop(deployment.name, None)
    if started is not None:
        try:
            started.wait(timeout=10)
        except subprocess.TimeoutExpired:
            started.kill()
            started.wait()
    else:
        for _ in range(100):
            if running_pid(deployment) is None:
                break
            time.sleep(0.1)
        else:
            os.kill(pid, signal.SIGKILL)
    pid_file(deployment).unlink(missing_ok=True)


def restart(deployment: Deployment, extra_env: dict[str, str] | None = None) -> None:
    """Starts `deployment` again, with `extra_env` laid over its configuration."""
    stop_server(deployment)
    start_server(deployment, extra_env)
    wait_for_server(deployment)


def up(args: argparse.Namespace) -> None:
    for deployment, chosen in [(ALPHA, args.alpha_bin), (BETA, args.beta_bin)]:
        deployment.dir.mkdir(parents=True, exist_ok=True)
        bins = Path(chosen).resolve() if chosen is not None else BIN
        for name in ["aspen-chat-server", "aspen-migrate"]:
            if not (bins / name).exists():
                raise Failed(f"{bins / name} does not exist; run `cargo build` first")
        (deployment.dir / "bin").write_text(str(bins))
    make_certificates()
    for deployment in DEPLOYMENTS:
        start_services(deployment)
        write_config(deployment)
        if psql(f"SELECT 1 FROM pg_database WHERE datname = '{deployment.database}'") != "1":
            say(f"making the database {deployment.database}")
            psql(f"CREATE DATABASE {deployment.database}")
        migrated = run(str(bin_of(deployment) / "aspen-migrate"), "up", cwd=deployment.dir, env=clean_env())
        if migrated.returncode != 0:
            raise Failed(f"migrating {deployment.name} failed: {migrated.stderr[-1000:]}")
    for deployment in DEPLOYMENTS:
        if running_pid(deployment) is not None:
            say(f"{deployment.name} is already running")
            continue
        start_server(deployment)
    for deployment in DEPLOYMENTS:
        wait_for_server(deployment)
    say("both deployments are up:")
    for deployment in DEPLOYMENTS:
        print(f"  https://{deployment.domain}  log {deployment.dir / 'server.log'}")
    print(f"  trust {CA} to reach them from elsewhere")


def down(args: argparse.Namespace) -> None:
    for deployment in DEPLOYMENTS:
        stop_server(deployment)
        for container in deployment.containers:
            run("docker", "rm", "-f", container)
        if args.drop:
            say(f"dropping the database {deployment.database}")
            psql(f"DROP DATABASE IF EXISTS {deployment.database}")


def terminal(deployment: Deployment, *args: str) -> str:
    """Runs an operator command on `deployment` and returns what it printed."""
    done = run(str(bin_of(deployment) / "aspen-chat-server"), *args, cwd=deployment.dir, env=clean_env())
    if done.returncode != 0:
        raise Failed(f"`{' '.join(args)}` on {deployment.name} failed: {done.stderr[-1000:]}")
    return done.stdout


def api(deployment: Deployment, method: str, path: str, body: dict | None = None, token: str | None = None,
        expect: tuple = (200, 201, 204)) -> dict:
    status, text = request(method, f"https://{deployment.domain}/api/v1{path}", body, token)
    if status not in expect:
        raise Failed(f"{deployment.name}: {method} {path}: {status} {text[:400]}")
    return json.loads(text) if text else {}


def sign_in(deployment: Deployment, name: str) -> str:
    status, _ = request("POST", f"https://{deployment.domain}/api/v1/users", {"name": name, "password": PASSWORD})
    if status not in (201, 409):
        raise Failed(f"registering {name} on {deployment.name} answered {status}")
    return api(deployment, "POST", "/auth/login", {"username": name, "password": PASSWORD})["sessionToken"]


def expect(condition: bool, what: str) -> None:
    if not condition:
        raise Failed(what)
    say(f"ok: {what}")


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
    rogue = sign_in(ALPHA, f"alpharogue{stamp}")
    status, roguish = sign_in_abroad(assertion_for(rogue))
    api(BETA, "PUT", f"/admin/users/{roguish['userId']}/ban", token=moderator, expect=(201,))
    expect(not abroad_alive(roguish["sessionToken"]), "a ban ends the user's sessions at beta")
    expect(problem(*sign_in_abroad(assertion_for(rogue))) == "403 federationRefused",
           "a banned user cannot sign in at beta again")
    api(BETA, "PUT", f"/admin/users/{deleter_at_beta}/ban", token=moderator, expect=(404,))
    log = api(BETA, "GET", "/admin/moderation-log", token=moderator)
    entries = log if isinstance(log, list) else log.get("entries", log.get("data", []))
    expect(any(e.get("action") == "banForeignUser" for e in entries), "the ban is in beta's moderation log")
    api(BETA, "DELETE", f"/admin/users/{roguish['userId']}/ban", token=moderator, expect=(204,))
    status, _ = sign_in_abroad(assertion_for(rogue))
    expect(status == 200, "once the ban is lifted they may sign in again")
    restart(BETA)


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
    upload = api(ALPHA, "POST", "/icons", {"mimeType": "image/png"}, token=traveller)
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
    terminal(BETA, "federation", "list-add", ALPHA.domain, "usersSharedBlock")
    expect(problem(*sign_in_abroad(assertion_for(traveller))) == "403 federationRefused",
           "on beta's block list, alpha's users are turned away")
    terminal(BETA, "federation", "list-remove", ALPHA.domain, "usersSharedBlock")

    planned = terminal(ALPHA, "federation", "rotate-key", "--planned").split()[-1]
    status, _ = sign_in_abroad(assertion_for(traveller))
    expect(status == 200 and planned in terminal(BETA, "federation", "list"),
           "after alpha hands over to a new key, beta follows the handover on its own")
    compromised = terminal(ALPHA, "federation", "rotate-key", "--compromised").split()[-1]
    expect(problem(*sign_in_abroad(assertion_for(traveller))) == "401 assertionInvalid"
           and f"offers a new key {compromised}" in terminal(BETA, "federation", "list"),
           "after alpha replaces a compromised key, beta refuses it and holds it as offered")
    terminal(BETA, "federation", "accept-key", ALPHA.domain, "--fingerprint", compromised)
    status, _ = sign_in_abroad(assertion_for(traveller))
    expect(status == 200, "once beta's operator accepts the new key, alpha's users sign in again")

    check_dms_abroad(traveller)
    check_standing()

    restart(BETA, {"ASPEN_AUTH__REQUIRE_TWO_FACTOR": "true",
                   "ASPEN_FEDERATION__USERS__IMMIGRATION_INVITE_REQUIRED": "true"})
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
        restart(BETA)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    start = commands.add_parser("up", help="start both deployments")
    start.add_argument("--alpha-bin", help="directory of the builds alpha runs (target/debug by default)")
    start.add_argument("--beta-bin", help="directory of the builds beta runs, such as an older release's")
    start.set_defaults(run=up)
    commands.add_parser("check", help="exercise federation between them").set_defaults(run=check)
    stop = commands.add_parser("down", help="stop both deployments")
    stop.add_argument("--drop", action="store_true", help="drop their databases too")
    stop.set_defaults(run=down)
    args = parser.parse_args()
    try:
        args.run(args)
    except Failed as failure:
        say(f"FAILED: {failure}")
        return 1
    except subprocess.CalledProcessError as error:
        say(f"FAILED: {' '.join(error.cmd)}: {error.stderr}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
