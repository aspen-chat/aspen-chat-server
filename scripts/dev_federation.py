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
servers are the debug builds in `target/debug` (`cargo build` first); `up` leaves them running
and `down` stops them. Databases are kept between runs; `down --drop` drops them too.

Needs Python 3.10, `openssl`, and `docker compose` with the database and SeaweedFS services up.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import ssl
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


def up(_args: argparse.Namespace) -> None:
    for name in ["aspen-chat-server", "aspen-migrate"]:
        if not (BIN / name).exists():
            raise Failed(f"{BIN / name} does not exist; run `cargo build` first")
    make_certificates()
    for deployment in DEPLOYMENTS:
        start_services(deployment)
        write_config(deployment)
        if psql(f"SELECT 1 FROM pg_database WHERE datname = '{deployment.database}'") != "1":
            say(f"making the database {deployment.database}")
            psql(f"CREATE DATABASE {deployment.database}")
        migrated = run(str(BIN / "aspen-migrate"), "up", cwd=deployment.dir, env=clean_env())
        if migrated.returncode != 0:
            raise Failed(f"migrating {deployment.name} failed: {migrated.stderr[-1000:]}")
    for deployment in DEPLOYMENTS:
        if running_pid(deployment) is not None:
            say(f"{deployment.name} is already running")
            continue
        say(f"starting {deployment.name} at https://{deployment.domain}")
        log = open(deployment.dir / "server.log", "a")
        process = subprocess.Popen(
            [str(BIN / "aspen-chat-server"), "--port", str(deployment.port),
             "--listen-addr", "::1", "--listen-addr", "127.0.0.1",
             "--key", str(WORK / f"{deployment.host}.key"), "--cert", str(WORK / f"{deployment.host}.pem")],
            cwd=deployment.dir, env=clean_env(), stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        pid_file(deployment).write_text(str(process.pid))
    for deployment in DEPLOYMENTS:
        wait_for_server(deployment)
    say("both deployments are up:")
    for deployment in DEPLOYMENTS:
        print(f"  https://{deployment.domain}  log {deployment.dir / 'server.log'}")
    print(f"  trust {CA} to reach them from elsewhere")


def down(args: argparse.Namespace) -> None:
    for deployment in DEPLOYMENTS:
        pid = running_pid(deployment)
        if pid is not None:
            say(f"stopping {deployment.name}")
            os.kill(pid, signal.SIGINT)
            for _ in range(50):
                if running_pid(deployment) is None:
                    break
                time.sleep(0.1)
            else:
                os.kill(pid, signal.SIGKILL)
        pid_file(deployment).unlink(missing_ok=True)
        for container in deployment.containers:
            run("docker", "rm", "-f", container)
        if args.drop:
            say(f"dropping the database {deployment.database}")
            psql(f"DROP DATABASE IF EXISTS {deployment.database}")


def terminal(deployment: Deployment, *args: str) -> str:
    """Runs an operator command on `deployment` and returns what it printed."""
    done = run(str(BIN / "aspen-chat-server"), *args, cwd=deployment.dir, env=clean_env())
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
    expect(status == 200 and document["domain"] == BETA.domain and len(document["keys"]) == 1,
           "beta publishes its document with one key")
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

    new_key = terminal(BETA, "federation", "rotate-key", "--yes").split()[-1]
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
    say("every check passed")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("up", help="start both deployments").set_defaults(run=up)
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
