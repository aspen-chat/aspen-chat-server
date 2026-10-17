"""The database's backups, operator commands, the voice server's capacity, and following a
public address that changes."""

from __future__ import annotations

import argparse
import sys
import time

from .common import SLOTS, Box, Failed
from .database import database_host, database_name, external_database, pg
from .lifecycle import converge, up, wait_for, voice_answers
from .render import render

BACKUPS_KEPT = 7


def run_command(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if box.get("role") == "voice":
        raise Failed("operator commands run on the deployment's own box")
    if not opt.command:
        raise Failed("give the command after --: aspen-box run -- aspen-chat-server admin grant <username>")
    sys.exit(store.compose("run", "--rm", "tool", *opt.command, check=False).returncode)


def dump(store: Box, box: dict, target) -> None:
    """Writes one consistent moment of the whole database, taken while it runs, to `target`."""
    partial = target.with_suffix(".partial")
    with partial.open("wb") as out:
        done = pg(store, box, "pg_dump", "--format=custom", f"--dbname={database_name(box)}", stdout=out, check=False)
    if done.returncode != 0:
        partial.unlink(missing_ok=True)
        raise Failed("the database could not be dumped; what PostgreSQL said is above")
    partial.chmod(0o600)
    partial.rename(target)


def backup(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if box.get("role") == "voice":
        raise Failed("a voice server keeps nothing to back up")
    target = store.dir / "backups" / f"aspen-{time.strftime('%Y%m%d-%H%M%S', time.gmtime())}.dump"
    dump(store, box, target)
    dumps = sorted((store.dir / "backups").glob("aspen-*.dump"))
    for stale in dumps[: -opt.keep]:
        stale.unlink()
    print(
        f"{target} ({target.stat().st_size >> 10} KiB). It is every account's data: keep a copy off this machine.\n"
        "It does not hold people's files"
        + (f", which are in {store.dir / 'data' / 'garage'}." if box["storage"]["kind"] == "bundled" else ", which are in your bucket.")
    )


def restore(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if not opt.file.exists():
        raise Failed(f"{opt.file} does not exist")
    external = external_database(box)
    host = database_host(box)
    if not opt.yes:
        does = f"fills the database at {host}, which must be empty," if external else "replaces the database"
        if input(f"This {does} of https://{box['domain']} with {opt.file.name}. Type the domain to go on: ").strip() != box["domain"]:
            raise Failed("not confirmed")
    converge(store, box)
    store.compose("stop", *SLOTS, "voice")
    if not external:
        # The box's own database is its to drop. One kept elsewhere is emptied by whoever
        # keeps it.
        pg(store, box, "psql", "--dbname=postgres", "--set=ON_ERROR_STOP=1",
           "--command=DROP DATABASE IF EXISTS aspen WITH (FORCE)", "--command=CREATE DATABASE aspen OWNER aspen")  # fmt: skip
    with opt.file.open("rb") as dumped:
        # All of it or none: a database that already holds tables is left as it was.
        done = pg(store, box, "pg_restore", "--single-transaction", "--exit-on-error", "--no-owner", "--no-acl",
                  f"--dbname={database_name(box)}", stdin=dumped, check=False)  # fmt: skip
    if done.returncode != 0:
        raise Failed(
            f"the dump could not be restored into the database at {host}, which is as it was. It must hold no tables: empty it where it is kept"
            if external
            else "the dump could not be restored, and the database is empty: restore another, or `aspen-box up` starts afresh"
        )
    up(store, opt)


def capacity(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    store.compose("exec", "-T", "voice", "voice_server", "estimate-capacity")
    if not box["rtc"].get("upload_mbps"):
        print(
            "\nThe estimate counts the network card's speed. If the machine's internet connection is slower, as at\n"
            "home, run a speed test on it, set rtc.upload_mbps in box.json to its upload, `aspen-box up`, and measure again."
        )
    where = "this voice server's capacity in the deployment's box.json, under voice_servers," if box.get("role") == "voice" else "the capacity of `voice` under voice_servers in box.json,"
    print(f"\nTo use it: set {where} and `aspen-box up` there.")


def refresh_address(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if "voice_server.toml" in render(store, box) and "voice" in store.running():
        store.compose("up", "-d", "--force-recreate", "voice")
        wait_for("the voice server", lambda: voice_answers(store))
        print("The public address changed. The voice server announces the new one; its calls ended and rejoin.")
