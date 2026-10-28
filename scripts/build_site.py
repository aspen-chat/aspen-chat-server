#!/usr/bin/env python3
"""Assembles the site (`site/`) into a directory of static files ready to host.

The site is the landing page (`site/index.html`) and the download page (`site/downloads/`). Its own
files are in `site/`; Aspen's icon, the favicon, and the Inclusive Sans faces the app bundles are
copied from where the client keeps them, so the site never holds a second copy.
The fonts come from the client's `node_modules`, so `pnpm install` in `client/` comes first.

    scripts/build_site.py [--out target/site]
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SITE = REPO / "site"
CLIENT = REPO / "client"
INCLUSIVE_SANS = CLIENT / "packages" / "app" / "node_modules" / "@fontsource-variable" / "inclusive-sans"
FONT_FILES = ("inclusive-sans-latin-wght-normal.woff2", "inclusive-sans-latin-ext-wght-normal.woff2")


def build(out: Path) -> None:
    if not (INCLUSIVE_SANS / "files").is_dir():
        sys.exit(f"{INCLUSIVE_SANS} is missing; run `pnpm install` in client/ first")
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(SITE, out)
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
    parser.add_argument("--out", type=Path, default=REPO / "target" / "site")
    out = parser.parse_args().out.resolve()
    build(out)
    print(out)


if __name__ == "__main__":
    main()
