#!/usr/bin/env python3
"""Checks that every list of the storage's public prefixes names the same ones.

    scripts/check_media_prefixes.py

Clients read objects under a few prefixes of the bucket without credentials
(docs/operators/installing/storage-read-path.md). The server lists them once, as `PublicPrefix`
(`server/app/src/media_store.rs`), under which it builds every public object's key, and each read
path repeats them: the box's Caddy (`deploy/box/caddy/media.tmpl`), the bucket policy on AWS
(`deploy/aws/modules/media/main.tf`), and the development Caddy (`scripts/caddy/Caddyfile`). A
prefix the server writes and a read path lacks is a kind of file nobody can download there, and one
a read path has beyond the server's is served without the server meaning it. The development
Caddy also names the bucket, in the path SeaweedFS's filer serves it at, which is
`docker-compose.yaml`'s.

It reads files alone, so it needs no deployment, and fails when a list differs from the server's or
is not written as it expects to read it.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SERVER_LIST = REPO / "server" / "app" / "src" / "media_store.rs"
BOX_CADDY = REPO / "deploy" / "box" / "caddy" / "media.tmpl"
DEV_CADDY = REPO / "scripts" / "caddy" / "Caddyfile"
COMPOSE = REPO / "docker-compose.yaml"
TOFU = REPO / "deploy" / "aws" / "modules" / "media" / "main.tf"

# The server's one list of the public prefixes: the arms of `PublicPrefix::as_str`.
PREFIX_LIST = re.compile(r"impl PublicPrefix \{\s*pub const fn as_str\(self\) -> &'static str \{\s*match self \{(.*?)\n\s*\}", re.S)
PREFIX_ARM = re.compile(r'PublicPrefix::(\w+) => "([a-z0-9-]+)",')
# The one path a read path's `@public` matcher passes: single objects under the prefixes.
PUBLIC_PATH = re.compile(r"\^(?P<under>(?:/[^/()|]+)*)/\((?P<prefixes>[a-z0-9|-]+)\)/\[\^/\]\+\$")


class Unreadable(Exception):
    pass


def named(path: Path) -> str:
    return str(path.relative_to(REPO))


def server_prefixes() -> dict[str, str]:
    """Each prefix the server stores public objects under, and the variant that names it."""
    listed = PREFIX_LIST.search(SERVER_LIST.read_text())
    if listed is None:
        raise Unreadable(f"{named(SERVER_LIST)} has no `PublicPrefix::as_str` written as a match of names")
    arms = [arm.strip() for arm in listed.group(1).strip().splitlines()]
    found = {}
    for arm in arms:
        shape = PREFIX_ARM.fullmatch(arm)
        if shape is None:
            raise Unreadable(f"{named(SERVER_LIST)}'s `PublicPrefix::as_str` has the arm `{arm}`, which is not "
                             'a variant and a plain name')
        found[shape[2]] = f"{named(SERVER_LIST)} (PublicPrefix::{shape[1]})"
    if len(found) != len(arms) or not found:
        raise Unreadable(f"{named(SERVER_LIST)}'s `PublicPrefix::as_str` names {len(found)} prefixes in {len(arms)} arms")
    return found


def caddy_prefixes(path: Path, under: str) -> set[str]:
    """The prefixes the `@public` matcher of a Caddy configuration passes, which it must pass
    under the path `under` and nowhere else."""
    matcher = re.search(r"^\t@public \{\n(.*?)^\t\}", path.read_text(), re.M | re.S)
    if matcher is None:
        raise Unreadable(f"{named(path)} has no `@public` matcher")
    paths = re.findall(r"^\s*path_regexp (\S+)$", matcher.group(1), re.M)
    if len(paths) != 1:
        raise Unreadable(f"{named(path)}'s `@public` matcher has {len(paths)} `path_regexp` lines, not one")
    shape = PUBLIC_PATH.fullmatch(paths[0])
    if shape is None:
        raise Unreadable(f"{named(path)}'s `@public` matcher passes {paths[0]}, which is not `/(<prefixes>)/<one name>`")
    if shape["under"] != under:
        raise Unreadable(f"{named(path)}'s `@public` matcher passes paths under `{shape['under']}/`, not `{under}/`")
    return set(shape["prefixes"].split("|"))


def development_bucket() -> str:
    buckets = re.findall(r"^\s*- S3_BUCKET=(\S+)$", COMPOSE.read_text(), re.M)
    if len(buckets) != 1:
        raise Unreadable(f"{named(COMPOSE)} sets `S3_BUCKET` {len(buckets)} times, not once")
    return buckets[0]


def tofu_prefixes(path: Path) -> set[str]:
    listed = re.findall(r"^\s*public_prefixes\s*=\s*\[([^\]]*)\]$", path.read_text(), re.M)
    if len(listed) != 1:
        raise Unreadable(f"{named(path)} sets `public_prefixes` {len(listed)} times, not once")
    items = [item.strip() for item in listed[0].split(",")]
    if not all(re.fullmatch(r'"[a-z0-9-]+"', item) for item in items):
        raise Unreadable(f"{named(path)}'s `public_prefixes` is not a list of plain names: [{listed[0]}]")
    return {item.strip('"') for item in items}


def main() -> int:
    try:
        server = server_prefixes()
        copies = {
            named(BOX_CADDY): caddy_prefixes(BOX_CADDY, ""),
            named(DEV_CADDY): caddy_prefixes(DEV_CADDY, f"/buckets/{development_bucket()}"),
        }
        copies[named(TOFU)] = tofu_prefixes(TOFU)
    except Unreadable as error:
        print(f"media prefixes: {error}", file=sys.stderr)
        return 1
    failed = False
    for where, prefixes in copies.items():
        for prefix in sorted(set(server) - prefixes):
            failed = True
            print(f"media prefixes: {where} lacks `{prefix}`, which {server[prefix]} stores under", file=sys.stderr)
        for prefix in sorted(prefixes - set(server)):
            failed = True
            print(f"media prefixes: {where} passes `{prefix}`, which the server stores nothing public under",
                  file=sys.stderr)
    if failed:
        return 1
    print(f"media prefixes: {', '.join(sorted(server))} in the server and in {len(copies)} read paths")
    return 0


if __name__ == "__main__":
    sys.exit(main())
