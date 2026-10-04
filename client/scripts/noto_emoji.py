#!/usr/bin/env python3
"""Builds the Noto Color Emoji fonts the app bundles, in the two colour formats browsers draw.

No one colour-font format is drawn by every engine: Chromium and Firefox draw COLRv1, which
WebKit does not (it paints flat blocks in place of gradients), and WebKit on an iPhone draws
`sbix`, Apple's bitmap format, but not the SVG glyphs of Fontsource's build of Noto Color Emoji,
which come out blank there and in Chromium. So the app bundles both: Google's COLRv1 build as it
is, and Google's bitmap (CBDT) build with its pictures moved into an `sbix` table. The one
`@font-face` lists the two with `tech()`, so a browser fetches only the one it draws
(`fallbackFonts.ts`).

Each is one whole file, not slices by `unicode-range`: a sequence (a family, a skin tone, a flag)
is drawn by one ligature, which WebKit forms only when every character in it comes from the same
face, and given slices whose ranges overlap it takes characters from different ones and draws
some sequences apart.

`build` writes the fonts from the upstream commit that
`packages/app/fonts/noto-color-emoji/manifest.json` names, and refuses files whose hashes differ
from the ones it names. `update` finds the newest upstream commit that changed either font and,
when it is not the manifest's, builds from it; `.github/workflows/emoji-font.yml` runs it every
week and proposes what it writes as a pull request. Either needs `fonttools` and `brotli`
(`noto_emoji.requirements.txt`).
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import sys
import urllib.request
from dataclasses import dataclass
from pathlib import Path

from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables import sbixGlyph, sbixStrike
from fontTools.ttLib.tables._g_l_y_f import Glyph

CLIENT = Path(__file__).resolve().parent.parent
OUT = CLIENT / "packages/app/fonts/noto-color-emoji"
MANIFEST = OUT / "manifest.json"

REPOSITORY = "googlefonts/noto-emoji"
COLRV1 = "2D/fonts/Noto-COLRv1.ttf"
BITMAP = "2D/fonts/NotoColorEmoji.ttf"
LICENSE = "LICENSE"

COLRV1_FILE = "noto-color-emoji.colrv1.woff2"
SBIX_FILE = "noto-color-emoji.sbix.woff2"


def fetch(url: str) -> bytes:
    headers = {"User-Agent": "aspen-noto-emoji"}
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if url.startswith("https://api.github.com/"):
        headers["Accept"] = "application/vnd.github+json"
        if token:
            headers["Authorization"] = f"Bearer {token}"
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=120) as r:
        return r.read()


def upstream(commit: str, path: str) -> bytes:
    return fetch(f"https://raw.githubusercontent.com/{REPOSITORY}/{commit}/{path}")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load(data: bytes) -> TTFont:
    # The timestamp is kept as upstream wrote it, so the same commit always builds the same files.
    return TTFont(io.BytesIO(data), recalcTimestamp=False)


def woff2(font: TTFont) -> bytes:
    font.flavor = "woff2"
    out = io.BytesIO()
    font.save(out)
    return out.getvalue()


def bitmap_to_sbix(font: TTFont) -> TTFont:
    """Moves a CBDT font's pictures into an `sbix` table, under empty outlines.

    WebKit refuses a font with neither `glyf` nor `CFF` outlines, and Google's bitmap build has
    neither, so each glyph gets an empty one; the pictures are drawn in their place.
    """
    ppem = font["CBLC"].strikes[0].bitmapSizeTable.ppemY
    pictures = sbixStrike.Strike(ppem=ppem, resolution=72)
    for name, bitmap in font["CBDT"].strikeData[0].items():
        metrics = bitmap.metrics
        # CBDT places a picture by its upper left corner, `sbix` by its lower left, each from the
        # glyph's origin on the baseline. Small metrics name the bearings one way, big ones another.
        bearing_x = getattr(metrics, "BearingX", None)
        if bearing_x is None:
            bearing_x = metrics.horiBearingX
        bearing_y = getattr(metrics, "BearingY", None)
        if bearing_y is None:
            bearing_y = metrics.horiBearingY
        pictures.glyphs[name] = sbixGlyph.Glyph(
            glyphName=name,
            graphicType="png ",
            originOffsetX=bearing_x,
            originOffsetY=bearing_y - metrics.height,
            imageData=bitmap.imageData,
        )
    sbix = newTable("sbix")
    sbix.version = 1
    sbix.flags = 1
    sbix.strikes = {ppem: pictures}
    del font["CBDT"]
    del font["CBLC"]
    font["sbix"] = sbix

    order = font.getGlyphOrder()
    glyf = newTable("glyf")
    glyf.glyphOrder = order
    glyf.glyphs = {name: Glyph() for name in order}
    font["glyf"] = glyf
    font["loca"] = newTable("loca")
    font["head"].indexToLocFormat = 0
    maxp = font["maxp"]
    maxp.tableVersion = 0x00010000
    for field in (
        "maxPoints maxContours maxCompositePoints maxCompositeContours maxTwilightPoints "
        "maxStorage maxFunctionDefs maxInstructionDefs maxStackElements maxSizeOfInstructions "
        "maxComponentElements maxComponentDepth"
    ).split():
        setattr(maxp, field, 0)
    maxp.maxZones = 1
    return font


@dataclass
class Source:
    commit: str
    committed: str


def build(source: Source, expected: dict[str, str] | None) -> None:
    files = {path: upstream(source.commit, path) for path in (COLRV1, BITMAP, LICENSE)}
    hashes = {path: sha256(data) for path, data in files.items()}
    if expected is not None and hashes != expected:
        raise SystemExit(
            f"{REPOSITORY}@{source.commit} no longer matches the hashes in {MANIFEST.name}: "
            f"expected {expected}, found {hashes}"
        )
    OUT.mkdir(parents=True, exist_ok=True)
    for stale in OUT.iterdir():
        stale.unlink()
    for name, font in (
        (COLRV1_FILE, load(files[COLRV1])),
        (SBIX_FILE, bitmap_to_sbix(load(files[BITMAP]))),
    ):
        data = woff2(font)
        (OUT / name).write_bytes(data)
        print(f"{name}: {len(data) / 1_000_000:.1f} MB", file=sys.stderr)
    (OUT / "LICENSE").write_bytes(files[LICENSE])
    manifest = {
        "comment": "Written by client/scripts/noto_emoji.py; do not edit.",
        "source": {
            "repository": REPOSITORY,
            "commit": source.commit,
            "committed": source.committed,
            "files": hashes,
        },
        "colrv1": COLRV1_FILE,
        "sbix": SBIX_FILE,
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")


def newest_commit() -> Source:
    """The newest upstream commit that changed either font."""
    newest: Source | None = None
    for path in (COLRV1, BITMAP):
        url = f"https://api.github.com/repos/{REPOSITORY}/commits?path={path}&per_page=1"
        commits = json.loads(fetch(url))
        if not commits:
            raise SystemExit(
                f"{REPOSITORY} has no commits touching {path}; the fonts have moved, and "
                f"{Path(__file__).name} needs their new paths"
            )
        commit = Source(commits[0]["sha"], commits[0]["commit"]["committer"]["date"])
        if newest is None or commit.committed > newest.committed:
            newest = commit
    assert newest is not None
    return newest


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("command", choices=["build", "update"])
    args = parser.parse_args()
    try:
        manifest = json.loads(MANIFEST.read_text())
    except FileNotFoundError:
        manifest = None
    if args.command == "build":
        if manifest is None:
            raise SystemExit(f"{MANIFEST} is missing; `update` writes it")
        source = manifest["source"]
        build(Source(source["commit"], source["committed"]), source["files"])
        return
    newest = newest_commit()
    if manifest is not None and manifest["source"]["commit"] == newest.commit:
        print(f"Noto Color Emoji is up to date ({newest.commit[:10]}).", file=sys.stderr)
        return
    print(
        f"Building Noto Color Emoji from {newest.commit[:10]} ({newest.committed}).",
        file=sys.stderr,
    )
    build(newest, None)


if __name__ == "__main__":
    main()
