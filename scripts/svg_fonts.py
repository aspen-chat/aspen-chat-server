#!/usr/bin/env python3
"""Builds the fonts `aspen-svg-render` draws SVG previews' text with, from the web client's own.

SVG previews draw text in the faces the app shows messages in: Inclusive Sans for text, Intel One
Mono for `monospace`, then Noto Sans, Noto Color Emoji, the Noto face of every script the app
falls back to, and the five regional CJK faces, the families `client/packages/app/fallbackFonts.json`
names, the same list the app's stylesheets are written from. They come from the Fontsource
packages the client installs (`pnpm install` in `client/` first) and the emoji font
`client/scripts/noto_emoji.py` builds, as WOFF2, which the renderer cannot read, so each is
written out here as a TrueType or OpenType file.

The renderer reads every font in the directory in the order of the files' names, which is the
order it looks through for a character a face lacks; the names begin with their place in that
order. Each family's licence is written beside them under `licenses/`.

Needs `fonttools` and `brotli` (`client/scripts/noto_emoji.requirements.txt`).
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / "client/packages/app"
FAMILIES = APP / "fallbackFonts.json"
EMOJI = APP / "fonts/noto-color-emoji"
DEFAULT_OUT = ROOT / "target/svg-fonts"


@dataclass(frozen=True)
class Family:
    """A Fontsource package and which of its files are wanted."""

    package: str
    id: str
    # `wght` for a variable family, `400` for one Fontsource has in that weight alone.
    axis: str
    styles: tuple[str, ...]
    # Slices left out: those the faces before it already draw.
    skipped: frozenset[str] = frozenset()


def families() -> list[Family]:
    listed = json.loads(FAMILIES.read_text())
    shared = frozenset(listed["sharedSubsets"])
    both = ("normal", "italic")

    def variable(prefix: str, id: str) -> Family:
        return Family(
            f"@fontsource-variable/{prefix}-{id}", f"{prefix}-{id}", "wght", ("normal",), shared
        )

    return [
        Family("@fontsource-variable/inclusive-sans", "inclusive-sans", "wght", both),
        Family("@fontsource-variable/intel-one-mono", "intel-one-mono", "wght", both),
        Family("@fontsource-variable/noto-sans", "noto-sans", "wght", both),
        *(variable("noto-sans", id) for id in listed["variable"]),
        *(
            Family(f"@fontsource/noto-sans-{id}", f"noto-sans-{id}", "400", ("normal",), shared)
            for id in listed["static"]
        ),
        *(variable("noto-serif", id) for id in listed["serifVariable"]),
        *(variable("noto-sans", id) for id in listed["han"]),
    ]


def package_dir(package: str) -> Path:
    path = APP / "node_modules" / package
    if not (path / "unicode.json").is_file():
        sys.exit(f"{package} is not installed under {APP}: run `pnpm install` in client/ first")
    return path


def slices(family: Family) -> list[Path]:
    """The family's WOFF2 files wanted, in its own order of slices: each named slice, or each
    numbered one where Fontsource cuts a large family by Unicode range."""
    directory = package_dir(family.package)
    names = json.loads((directory / "unicode.json").read_text())
    files = []
    for name in names:
        slice_ = name.strip("[]")
        if slice_ in family.skipped:
            continue
        for style in family.styles:
            path = directory / "files" / f"{family.id}-{slice_}-{family.axis}-{style}.woff2"
            if not path.is_file():
                sys.exit(f"{family.package} has no {path.name}")
            files.append(path)
    if not files:
        sys.exit(f"{family.package} has no slices of its own")
    return files


def build(out: Path) -> None:
    sources: list[tuple[Path, str]] = []
    licenses: list[tuple[Path, str]] = []
    for family in families():
        files = slices(family)
        # Inclusive Sans comes first, then Intel One Mono, then the Noto faces.
        sources.extend((path, path.stem) for path in files)
        licenses.append((package_dir(family.package) / "LICENSE", family.id))
        if family.id == "noto-sans":
            manifest = json.loads((EMOJI / "manifest.json").read_text())
            sources.append((EMOJI / manifest["colrv1"], "noto-color-emoji-colrv1"))
            licenses.append((EMOJI / "LICENSE", "noto-color-emoji"))

    staging = out.with_name(out.name + ".building")
    shutil.rmtree(staging, ignore_errors=True)
    (staging / "licenses").mkdir(parents=True)
    width = len(str(len(sources)))
    for place, (path, stem) in enumerate(sources):
        font = TTFont(path)
        font.flavor = None
        extension = "otf" if "CFF " in font or "CFF2" in font else "ttf"
        font.save(staging / f"{place:0{width}d}-{stem}.{extension}")
    for path, id in licenses:
        shutil.copyfile(path, staging / "licenses" / f"{id}.txt")
    shutil.rmtree(out, ignore_errors=True)
    staging.rename(out)
    print(f"{len(sources)} fonts in {out}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--out", type=Path, default=DEFAULT_OUT, help=f"where to write them ({DEFAULT_OUT})"
    )
    args = parser.parse_args()
    build(args.out.resolve())


if __name__ == "__main__":
    main()
