#!/usr/bin/env python3
"""Assembles Aspen's site into a directory of static files ready to host.

The site's own pages are kept in a repository of their own; give the folder holding them. To them
this adds Aspen's icon, the favicon, and the Inclusive Sans faces the app bundles, copied from where
the client keeps them, so the site never holds a second copy. The fonts come from the client's
`node_modules`, so `pnpm install` in `client/` comes first.

    scripts/build_site.py <the site's folder> [--out target/site]
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CLIENT = REPO / "client"
INCLUSIVE_SANS = CLIENT / "packages" / "app" / "node_modules" / "@fontsource-variable" / "inclusive-sans"
FONT_FILES = ("inclusive-sans-latin-wght-normal.woff2", "inclusive-sans-latin-ext-wght-normal.woff2")


def build(site: Path, out: Path) -> None:
    if not (site / "index.html").is_file():
        sys.exit(f"{site} holds no index.html; give the folder holding the site's pages")
    if not (INCLUSIVE_SANS / "files").is_dir():
        sys.exit(f"{INCLUSIVE_SANS} is missing; run `pnpm install` in client/ first")
    if out == site or out in site.parents:
        sys.exit(f"{out} would hold the site's own folder; choose another --out")
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(site, out, ignore=shutil.ignore_patterns(".git"))
    shutil.copyfile(CLIENT / "brand" / "aspen-icon.svg", out / "aspen-icon.svg")
    shutil.copyfile(CLIENT / "packages" / "app" / "public" / "favicon.svg", out / "favicon.svg")
    fonts = out / "fonts"
    fonts.mkdir()
    for name in FONT_FILES:
        shutil.copyfile(INCLUSIVE_SANS / "files" / name, fonts / name)
    # The SIL Open Font License asks that it travel with the fonts.
    shutil.copyfile(INCLUSIVE_SANS / "LICENSE", fonts / "LICENSE")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("site", type=Path, help="the folder holding the site's pages")
    parser.add_argument("--out", type=Path, default=REPO / "target" / "site")
    arguments = parser.parse_args()
    out = arguments.out.resolve()
    build(arguments.site.resolve(), out)
    print(out)


if __name__ == "__main__":
    main()
