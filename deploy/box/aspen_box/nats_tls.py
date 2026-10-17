"""TLS for NATS, which a box whose voice servers include one on another machine opens to the
network: the box's own certificate authority, and a certificate it signs for the deployment's
name and for this machine's loopback, where its own servers reach NATS.

The authority's key stays in `data/nats-authority/`, which no container mounts; the server's
certificate and key are in `config/nats-tls/`, which NATS mounts. Every voice server and this
machine's servers trust the authority alone for NATS (`[nats.tls] ca_file`). The certificates
are made with `openssl` in the `api` image, and left this machine's user's.
"""

from __future__ import annotations

import os
import subprocess

from .common import Box

# Ten years: a certificate the box makes for itself has nobody to renew it.
DAYS = 3650


def ensure(store: Box, domain: str) -> str:
    """Makes the authority and the server's certificate if they are not there yet, and answers
    the authority's certificate."""
    authority = store.dir / "data" / "nats-authority"
    served = store.dir / "config" / "nats-tls"
    authority.mkdir(mode=0o700, parents=True, exist_ok=True)
    served.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not (served / "server.pem").exists():
        script = "\n".join([
            "set -e",
            "cd /authority",
            "[ -f ca.pem ] || openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes "
            f"-days {DAYS} -subj '/CN=Aspen NATS authority' -keyout ca.key -out ca.pem 2>/dev/null",
            "openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes "
            f"-subj '/CN={domain}' -keyout /served/server.key -out server.csr 2>/dev/null",
            f"printf 'subjectAltName=DNS:{domain},IP:127.0.0.1\\nextendedKeyUsage=serverAuth\\n' > server.ext",
            "openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial "
            f"-days {DAYS} -extfile server.ext -out /served/server.pem 2>/dev/null",
            "rm server.csr server.ext",
            "chmod 600 ca.key /served/server.key",
            # The container's root is this user under rootless Docker, and root itself
            # otherwise, whose files this user could not read.
            "" if rootless() else f"chown -R {os.getuid()}:{os.getgid()} /authority /served",
        ])  # fmt: skip
        store.compose(
            "run", "--rm", "--no-deps", "-T", "--user", "0",
            "-v", f"{authority}:/authority", "-v", f"{served}:/served", "--entrypoint", "sh",
            "tool", "-c", script,
        )  # fmt: skip
    return (authority / "ca.pem").read_text()


def rootless() -> bool:
    done = subprocess.run(["docker", "info", "--format", "{{json .SecurityOptions}}"], capture_output=True, text=True)
    return "rootless" in done.stdout


NATS_CONF_TLS = """tls {
  cert_file: "/etc/nats/tls/server.pem"
  key_file: "/etc/nats/tls/server.key"
}
"""
