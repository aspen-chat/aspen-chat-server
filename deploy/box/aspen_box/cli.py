"""Runs one whole Aspen deployment on one Linux machine, with Docker, or a voice server of one on
another machine.

    aspen-box init --tag TAG       ask what the deployment is, and write its configuration
    aspen-box up                   start it, or bring it in line with its configuration
    aspen-box upgrade --tag TAG    move to new images, the API never stopping
    aspen-box doctor               check the machine, the network, and the deployment
    aspen-box run -- COMMAND…      an operator command: aspen-chat-server admin grant <username>
    aspen-box backup | restore FILE  the database, wherever it is kept
    aspen-box export DIR           stop, and write out everything, for a deployment that replaces it
    aspen-box voice add NAME | remove NAME | list   voice servers on other machines
    aspen-box init-voice --join FILE --tag TAG      a box for one of them, on its machine
    aspen-box capacity             measure how many people the voice server carries
    aspen-box refresh-address      follow a public address that changed
    aspen-box status | logs [SERVICE] | down

Everything it keeps is in one directory (`--dir`, ~/aspen by default): `box.json`, what `init`
was told and the passwords it made; `config/`, the files written from it; `data/`, the
database, the certificates, and the bundled storage; `backups/`. README.md beside this tool is
the guide. It needs Python 3.9 and Docker with Compose 2.20, and nothing else.
"""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import secrets
import sys
import uuid

from . import data, doctor, export, lifecycle, voice
from .common import HERE_VOICE, Box, Failed, addresses, ask
from .database import DATABASE_URL_VARIABLE, MINIMUM_POSTGRES, check_database_url
from .render import render


def init(store: Box, opt: argparse.Namespace) -> None:
    if store.file.exists() and not opt.force:
        raise Failed(
            f"{store.dir} already holds a deployment. Edit its box.json and run `aspen-box up`, or "
            "start over with --force, which makes new passwords and so needs an empty data/"
        )
    images = lifecycle.published(opt.tag) if opt.tag else {"api": opt.api_image, "voice": opt.voice_image}
    images.update({key: value for key, value in (("api", opt.api_image), ("voice", opt.voice_image)) if value})
    if not images.get("api") or not images.get("voice"):
        raise Failed("give the release to run: --tag, or --api-image and --voice-image")
    interactive = opt.domain is None
    if interactive:
        print("A deployment's address is for good: its accounts' passkeys and other deployments both")
        print("know it by that name (docs/operators/installing/3-configuring.md).\n")
    domain = (opt.domain or ask("The deployment's address, without https:// (chat.example.org)")).lower()
    if not re.fullmatch(r"([a-z0-9-]+\.)+[a-z0-9-]+", domain):
        raise Failed(f"{domain!r} is not a host name")
    acme_email = opt.acme_email or (ask("Email for Let's Encrypt to write to about certificates (- for none)", "-") if interactive else "-")
    storage = storage_answers(opt, interactive)
    database = database_answers(opt, interactive)
    upload = opt.upload_mbps
    if upload is None and interactive:
        given = ask("This machine's internet upload, in Mbit/s, as a speed test run on it measures (- if you do not know)", "-")
        upload = None if given == "-" else float(given)
    box = {
        "role": "main",
        "domain": domain,
        "acme_email": None if acme_email == "-" else acme_email,
        "address4": opt.address4,
        "address6": opt.address6,
        "storage": storage,
        "database": database,
        "email": None,
        "images": images,
        "rtc": {"min_port": 40000, "max_port": 40199, "upload_mbps": upload},
        # Letters and digits alone, so none needs escaping where it is written.
        "voice_servers": [
            {"name": HERE_VOICE, "id": str(uuid.uuid4()), "password": secrets.token_hex(20), "capacity": opt.voice_capacity, "here": True},
        ],
        "secrets": {
            "database": secrets.token_hex(20),
            "nats_server": secrets.token_hex(20),
            "valkey": secrets.token_hex(20),
            "garage_rpc": secrets.token_hex(32),
            # The forms Garage's own key pairs have.
            "s3_access_key": "GK" + secrets.token_hex(12),
            "s3_secret_key": secrets.token_hex(32),
        },
        "active_slot": "api-a",
    }
    store.save(box)
    render(store, box)
    print(f"\nWrote {store.dir}.\n\nPoint these names at this machine, in your domain's DNS:\n")
    for name in doctor.names(box):
        for address in addresses(box):
            print(f"    {name}.  {'AAAA' if ':' in address else 'A'}  {address}")
    print(
        f"\nIf the machine is behind a router, forward to it: {doctor.ports(box)}.\n\n"
        "Then: aspen-box up, and aspen-box doctor."
    )


def storage_answers(opt: argparse.Namespace, interactive: bool) -> dict:
    kind = opt.storage or (ask("Keep people's files on this machine, or in an S3 bucket elsewhere? (machine/s3)", "machine") if interactive else "machine")
    if kind in ("machine", "bundled"):
        return {"kind": "bundled"}
    field = lambda name, question: getattr(opt, f"s3_{name}") or ask(question)  # noqa: E731
    storage = {
        "kind": "external",
        "endpoint": field("endpoint", "The bucket's S3 API (https://<account>.r2.cloudflarestorage.com)"),
        "bucket": field("bucket", "The bucket's name"),
        "region": field("region", "Its region (auto, for R2)"),
        "public_base_url": field("public_base_url", "Where its read path is served (https://media.chat.example.org)"),
    }
    if opt.s3_public_endpoint:
        storage["public_endpoint"] = opt.s3_public_endpoint
    if not opt.s3_role:
        storage["access_key"] = field("access_key", "Access key (on AWS, give none and use --s3-role)")
        storage["secret_key"] = field("secret_key", "Secret key")
    return storage


def database_answers(opt: argparse.Namespace, interactive: bool) -> dict:
    given = opt.database_url or os.environ.get(DATABASE_URL_VARIABLE)
    where = "elsewhere" if given else ("machine" if not interactive else ask("Keep the database on this machine, or use a PostgreSQL elsewhere? (machine/elsewhere)", "machine"))
    if where not in ("machine", "elsewhere"):
        raise Failed("answer machine or elsewhere")
    if where == "machine":
        return {"kind": "bundled"}
    url = given or ask(f"Its address (postgres://user:password@host:5432/database), PostgreSQL {MINIMUM_POSTGRES} or later")
    check_database_url(url)
    ca = opt.database_ca or (ask("A file of the authorities its certificate comes from, if they are its provider's own (- for the usual public ones)", "-") if interactive else "-")
    return {"kind": "external", "url": url, "ca_file": None if ca == "-" else str(pathlib.Path(ca).expanduser().resolve())}


def arguments() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__.split("\n\n")[1]
    )  # fmt: skip
    parser.add_argument("--dir", type=pathlib.Path, default=pathlib.Path(os.environ.get("ASPEN_BOX_DIR", pathlib.Path.home() / "aspen")),
                        help="the box's directory (or ASPEN_BOX_DIR; ~/aspen)")  # fmt: skip
    commands = parser.add_subparsers(dest="step", required=True)

    def where(command: argparse.ArgumentParser) -> None:
        command.add_argument("--address4", default="auto", help="the public IPv4 address clients send media to; `auto` looks it up")
        command.add_argument("--address6", default="auto", help="likewise for IPv6; `none` announces none")
        command.add_argument("--acme-email")
        command.add_argument("--upload-mbps", type=float, help="the internet upload, as a speed test measures it")

    start = commands.add_parser("init", help="every answer may be given as an option instead of asked")
    start.add_argument("--domain")
    where(start)
    start.add_argument("--storage", choices=["machine", "s3"])
    for name in ("endpoint", "public-endpoint", "bucket", "region", "access-key", "secret-key", "public-base-url"):
        start.add_argument(f"--s3-{name}")
    start.add_argument("--s3-role", action="store_true", help="give no key pair: the server takes the role of the machine it runs on")
    start.add_argument("--database-url", help=f"a PostgreSQL elsewhere, in place of one on this machine (or {DATABASE_URL_VARIABLE}, which keeps its password off the command line)")
    start.add_argument("--database-ca", help="its certificate's authorities, when they are its provider's own")
    start.add_argument("--tag", help="the release to run, from the published images")
    start.add_argument("--api-image", help="in place of the published `api` image")
    start.add_argument("--voice-image", help="in place of the published `voice` image")
    start.add_argument("--voice-capacity", type=int, default=25, help="a guess; `aspen-box capacity` measures it")
    start.add_argument("--force", action="store_true")
    commands.add_parser("up")
    move = commands.add_parser("upgrade")
    move.add_argument("--tag", help="the new release of both images")
    move.add_argument("--api-image")
    move.add_argument("--voice-image")
    commands.add_parser("doctor")
    commands.add_parser("run").add_argument("command", nargs="*")
    commands.add_parser("backup").add_argument("--keep", type=int, default=data.BACKUPS_KEPT, help="how many dumps to keep")
    back = commands.add_parser("restore")
    back.add_argument("file", type=pathlib.Path)
    back.add_argument("--yes", action="store_true")
    out = commands.add_parser("export", help="stops the deployment, and writes its database and people's files into DIR")
    out.add_argument("directory", type=pathlib.Path)
    out.add_argument("--database-only", action="store_true", help="leave the files in the bucket, which the next deployment takes over")
    out.add_argument("--yes", action="store_true", help="do not ask before stopping the deployment")
    voices = commands.add_parser("voice").add_subparsers(dest="voice", required=True)
    added = voices.add_parser("add", help="a voice server on another machine")
    added.add_argument("name")
    added.add_argument("--capacity", type=int, default=25, help="a guess; `aspen-box capacity` on its machine measures it")
    voices.add_parser("remove").add_argument("name")
    voices.add_parser("list")
    joined = commands.add_parser("init-voice", help="on another machine: a box that holds one voice server")
    joined.add_argument("--join", required=True, help="the file `aspen-box voice add` wrote")
    where(joined)
    joined.add_argument("--tag", help="the release to run, from the published images")
    joined.add_argument("--voice-image", help="in place of the published `voice` image")
    joined.add_argument("--force", action="store_true")
    commands.add_parser("capacity")
    commands.add_parser("refresh-address")
    commands.add_parser("status")
    commands.add_parser("logs").add_argument("service", nargs="*")
    commands.add_parser("down")
    return parser


def main() -> None:
    opt = arguments().parse_args()
    store = Box(opt.dir.expanduser().resolve())
    steps = {
        "init": init, "up": lifecycle.up, "upgrade": lifecycle.upgrade, "doctor": doctor.doctor,
        "run": data.run_command, "backup": data.backup, "restore": data.restore, "export": export.export,
        "init-voice": voice.init_voice, "capacity": data.capacity, "refresh-address": data.refresh_address,
        "status": lambda store, opt: store.compose("ps", "--all"),
        "logs": lambda store, opt: store.compose("logs", "--tail", "200", *opt.service),
        "down": lambda store, opt: store.compose(
            "--profile", "main", "--profile", "api", "--profile", "bundled-storage", "--profile", "bundled-database", "down"),
    }  # fmt: skip
    if opt.step == "voice":
        {"add": voice.add, "remove": voice.remove, "list": voice.listing}[opt.voice](store, opt)
        return
    steps[opt.step](store, opt)


def run() -> None:
    try:
        main()
    except Failed as failure:
        print(f"aspen-box: {failure}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
