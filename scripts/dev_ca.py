#!/usr/bin/env python3
"""The development certificate authority, and certificates from it, for serving Aspen over HTTPS
on a development machine and on the devices that reach it.

    scripts/dev_ca.py issue NAME...    # a certificate for these names and addresses; prints its files
    scripts/dev_ca.py path             # the authority's certificate, which clients trust

The authority is made once, in `target/dev-ca/` (`ca.pem` and its key), and every certificate
issued from it lands beside it, named after the first name given (`*` written as `_wildcard`).
Issuing again for the same names keeps the certificate until it nears expiry or the authority is
made anew. `scripts/dev_federation.py`, `scripts/dev_push.py`, and
`scripts/push_relay_stand_in.py` issue theirs here too, so one authority, trusted once, covers
them all; `docs/development/local-https.md` says how to trust it on each platform.

The authority may sign only for names and addresses that are development ones (`PERMITTED`),
which every client that verifies a certificate enforces, so its key, if it leaked, could not
impersonate any other site to a device that trusts it. A name outside them is refused here.

Needs Python 3.10 and `openssl`.
"""

from __future__ import annotations

import argparse
import ipaddress
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
WORK = REPO / "target" / "dev-ca"
CA = WORK / "ca.pem"
CA_KEY = WORK / "ca.key"
# Names under which the authority may sign: this machine (`localhost`), the names RFC 8375 and
# RFC 2606 set aside for private networks and testing, ICANN's `internal`, and the wildcard DNS
# services that name a LAN address (`192-168-2-220.nip.io`). Each permits itself and every name
# under it (RFC 5280 §4.2.1.10).
PERMITTED_DNS = ["localhost", "home.arpa", "internal", "test", "nip.io", "sslip.io"]
# Loopback, and the private ranges a LAN is numbered from (RFC 1918, RFC 4193).
PERMITTED_IP = ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "::1/128", "fc00::/7"]
# Apple refuses a certificate valid for longer than 825 days.
DAYS = 825
# A certificate expiring within this many seconds is issued anew.
RENEW_WITHIN = 30 * 24 * 3600


def say(message: str) -> None:
    print(f"dev-ca: {message}", file=sys.stderr, flush=True)


def run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(list(args), capture_output=True, text=True, check=True)


def is_address(name: str) -> bool:
    try:
        ipaddress.ip_address(name)
    except ValueError:
        return False
    return True


def is_permitted(name: str) -> bool:
    if is_address(name):
        address = ipaddress.ip_address(name)
        return any(address in ipaddress.ip_network(network) for network in PERMITTED_IP)
    name = name.removeprefix("*.").rstrip(".").lower()
    return any(name == allowed or name.endswith(f".{allowed}") for allowed in PERMITTED_DNS)


def ip_constraint(network: str) -> str:
    """`network` as openssl writes an IP name constraint: an address and its mask."""
    parsed = ipaddress.ip_network(network)
    return f"permitted;IP:{parsed.network_address}/{parsed.netmask}"


def authority() -> Path:
    """The authority's certificate, made the first time it is asked for."""
    if CA.exists():
        return CA
    WORK.mkdir(parents=True, exist_ok=True)
    say(f"making the development certificate authority, {CA}")
    constraints = ",".join(
        [f"permitted;DNS:{name}" for name in PERMITTED_DNS] + [ip_constraint(network) for network in PERMITTED_IP]
    )
    run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
        "-keyout", str(CA_KEY), "-out", str(CA), "-days", "3650",
        "-subj", "/CN=Aspen development certificate authority",
        "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0",
        "-addext", "keyUsage=critical,keyCertSign,cRLSign",
        "-addext", f"nameConstraints=critical,{constraints}")
    CA_KEY.chmod(0o600)
    return CA


def files(names: list[str]) -> tuple[Path, Path, Path]:
    stem = names[0].replace("*", "_wildcard")
    return WORK / f"{stem}.pem", WORK / f"{stem}.key", WORK / f"{stem}.names"


def is_current(cert: Path, names_file: Path, names: list[str]) -> bool:
    if not (cert.exists() and names_file.exists()) or names_file.read_text().split() != names:
        return False
    if cert.stat().st_mtime < CA.stat().st_mtime:
        return False
    expiring = subprocess.run(["openssl", "x509", "-in", str(cert), "-noout", "-checkend", str(RENEW_WITHIN)],
                              capture_output=True)
    return expiring.returncode == 0


def issue(*names: str) -> tuple[Path, Path]:
    """A certificate for `names` (DNS names, `*.` wildcards, or IP addresses), from the
    authority: its PEM and its key, issued again only when it is missing, for other names, or
    near expiry."""
    names_list = list(names)
    if not names_list:
        raise ValueError("a certificate needs at least one name")
    refused = [name for name in names_list if not is_permitted(name)]
    if refused:
        raise ValueError(
            f"the development authority may not sign for {', '.join(refused)}: use a name under "
            f"{', '.join(PERMITTED_DNS)}, or a loopback or private address"
        )
    authority()
    cert, key, names_file = files(names_list)
    if is_current(cert, names_file, names_list):
        return cert, key
    say(f"issuing a certificate for {', '.join(names_list)}")
    request = cert.with_suffix(".csr")
    extensions = cert.with_suffix(".ext")
    alt_names = ",".join(f"{'IP' if is_address(name) else 'DNS'}:{name}" for name in names_list)
    extensions.write_text(
        f"subjectAltName={alt_names}\n"
        "extendedKeyUsage=serverAuth\n"
        "basicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature\n"
    )
    run("openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
        "-keyout", str(key), "-out", str(request), "-subj", f"/CN={names_list[0]}")
    key.chmod(0o600)
    run("openssl", "x509", "-req", "-in", str(request), "-CA", str(CA), "-CAkey", str(CA_KEY),
        "-CAcreateserial", "-out", str(cert), "-days", str(DAYS), "-extfile", str(extensions))
    request.unlink()
    extensions.unlink()
    names_file.write_text(" ".join(names_list) + "\n")
    return cert, key


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    issuing = commands.add_parser("issue", help="issue a certificate for these names and addresses")
    issuing.add_argument("names", nargs="+")
    commands.add_parser("path", help="print the authority's certificate's path, making it if need be")
    arguments = parser.parse_args()
    try:
        if arguments.command == "issue":
            cert, key = issue(*arguments.names)
            print(f"cert {cert}\nkey  {key}\nca   {CA}")
        else:
            print(authority())
    except ValueError as error:
        say(str(error))
        return 2
    except subprocess.CalledProcessError as error:
        say(f"{' '.join(error.cmd)} failed:\n{error.stderr}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
