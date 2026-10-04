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

Needs Python 3.10, `openssl`, and `docker compose` with the database and SeaweedFS services up
(`up --start-services` starts them).
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
    # `settings set` flags giving its federation gates, which are deployment settings.
    gates: tuple[str, ...]

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
ALPHA = Deployment("alpha", 8443, 4231, 6391, 19471, (
    "--users-emigration", "allowList", "--users-immigration", "open",
    "--bots-emigration", "closed", "--bots-immigration", "blockList",
))
BETA = Deployment("beta", 8444, 4232, 6392, 19472, (
    "--users-emigration", "blockList", "--users-immigration", "blockList", "--users-shared-list", "true",
))
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
        issue_certificate(deployment.host)


def issue_certificate(host: str) -> tuple[Path, Path]:
    """A certificate for `host` from the development authority, made once: its PEM and key."""
    cert = WORK / f"{host}.pem"
    key = WORK / f"{host}.key"
    if cert.exists():
        return cert, key
    say(f"issuing a certificate for {host}")
    request = WORK / f"{host}.csr"
    extensions = WORK / f"{host}.ext"
    extensions.write_text(
        f"subjectAltName=DNS:{host}\n"
        "extendedKeyUsage=serverAuth\n"
        "basicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature\n"
    )
    run("openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
        "-keyout", str(key), "-out", str(request), "-subj", f"/CN={host}", check=True)
    run("openssl", "x509", "-req", "-in", str(request), "-CA", str(CA), "-CAkey", str(WORK / "ca.key"),
        "-CAcreateserial", "-out", str(cert), "-days", "825", "-extfile", str(extensions), check=True)
    request.unlink()
    extensions.unlink()
    return cert, key


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


def start_shared_services() -> None:
    """Starts the docker-compose PostgreSQL and SeaweedFS both deployments use, and waits for them."""
    say("starting the database and SeaweedFS")
    run("docker", "compose", "up", "-d", "database", "seaweedfs", cwd=REPO, check=True)
    deadline = time.monotonic() + 90
    while run("docker", "compose", "exec", "-T", "database", "pg_isready", "-U", "postgres", cwd=REPO).returncode != 0:
        if time.monotonic() > deadline:
            raise Failed("PostgreSQL did not come up within 90 s")
        time.sleep(1)
    while True:
        try:
            urllib.request.urlopen("http://127.0.0.1:8333/", timeout=5)
            break
        except urllib.error.HTTPError:
            break
        except OSError:
            if time.monotonic() > deadline:
                raise Failed("SeaweedFS's S3 did not come up within 90 s")
            time.sleep(1)


def up(args: argparse.Namespace) -> None:
    if args.start_services:
        start_shared_services()
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
        terminal(deployment, "settings", "set", *deployment.gates)
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


def main() -> int:
    from dev_federation_check import check

    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    start = commands.add_parser("up", help="start both deployments")
    start.add_argument("--alpha-bin", help="directory of the builds alpha runs (target/debug by default)")
    start.add_argument("--beta-bin", help="directory of the builds beta runs, such as an older release's")
    start.add_argument("--start-services", action="store_true",
                       help="docker compose up the database and SeaweedFS first, and wait for them")
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
    # dev_federation_check imports this module by name. Registering the script under that name
    # keeps one copy of its state (the servers it started) and of `Failed`, which `main` catches.
    sys.modules.setdefault("dev_federation", sys.modules[__name__])
    sys.exit(main())
