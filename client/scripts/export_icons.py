#!/usr/bin/env python3
"""Draws Aspen's canopy mark and exports every icon the apps ship.

The mark is drawn once, here, in two levels of detail: `full` (bark marks on the trunk, fine gaps
between the leaves) for anything drawn 64 pixels or larger, and `small` (no bark marks, a thicker
trunk, wider gaps) for the sizes where fine detail turns to mud. The gaps are cut out of the
leaves beneath rather than painted, so the mark sits on any background. Everything else is laid
out from those two: the brand art in `client/brand/`, the web client's favicons, the desktop
app's icons for each platform, the Android launcher, notification, and splash images, and the
favicon inlined in the server's passkey page.

Run it from anywhere after changing the mark: `client/scripts/export_icons.py`. It needs
`rsvg-convert` (librsvg) and `magick` (ImageMagick 7) on the path.
"""

from __future__ import annotations

import re
import subprocess
import tempfile
import urllib.parse
from dataclasses import dataclass
from pathlib import Path

CLIENT = Path(__file__).resolve().parent.parent
REPO = CLIENT.parent
BRAND = CLIENT / "brand"
APP_PUBLIC = CLIENT / "packages/app/public"
DESKTOP_BUILD = CLIENT / "packages/desktop/build"
ANDROID_RES = CLIENT / "packages/mobile/android/app/src/main/res"
PASSKEY_PAGE = REPO / "server/src/api/passkey_page/page.html"

EMERALD = "#047857"
MINT = "#34d399"
GOLD = "#F2B43A"
BARK = "#E7E1D3"
INK = "#23262B"
PAPER = "#F5F3EE"
CREAM = "#FBF8F1"
"""The tile behind the mark wherever an icon needs a background of its own."""


@dataclass(frozen=True)
class Detail:
    # (cx, cy, r) of the left, right, and top leaves, the top one drawn over the others
    left: tuple[float, float, float]
    right: tuple[float, float, float]
    top: tuple[float, float, float]
    trunk: tuple[float, float, float, float, float]  # x, y, width, height, corner radius
    gap: float
    bark_marks: tuple[float, ...]  # heights of the dark "eyes" on the trunk
    bark: str

    @property
    def top_edge(self) -> float:
        return self.top[1] - self.top[2]

    @property
    def bottom_edge(self) -> float:
        return self.trunk[1] + self.trunk[3]


FULL = Detail((90, 112, 44), (166, 112, 44), (128, 76, 48), (116, 120, 24, 106, 8), 3.5, (168, 204), BARK)
# A pale trunk a few pixels wide vanishes on a light tab bar, so the small mark's bark is a shade
# darker, which still reads on a dark one.
SMALL = Detail((88, 118, 48), (168, 118, 48), (128, 76, 52), (110, 130, 36, 96, 10), 12, (), "#C7BCA5")


def mark(detail: Detail, cx: float, cy: float, height: float, key: str, mono: str | None = None) -> str:
    """The mark `height` tall, centred on (cx, cy); `mono` draws it in that one colour."""
    scale = height / (detail.bottom_edge - detail.top_edge)
    top = cy - height / 2
    transform = f"translate({cx - 128 * scale:.3f} {top - detail.top_edge * scale:.3f}) scale({scale:.5f})"

    def cutter(*leaves: tuple[float, float, float]) -> str:
        return "".join(f'<circle cx="{x}" cy="{y}" r="{r + detail.gap}" fill="black"/>' for x, y, r in leaves)

    def mask(name: str, *leaves: tuple[float, float, float]) -> str:
        return (
            f'<mask id="{key}-{name}" maskUnits="userSpaceOnUse" x="-8" y="-8" width="272" height="272">'
            f'<rect x="-8" y="-8" width="272" height="272" fill="white"/>{cutter(*leaves)}</mask>'
        )

    x, y, w, h, rx = detail.trunk
    colour = (lambda c: mono) if mono else (lambda c: c)
    eyes = (
        ""
        if mono
        else "".join(
            f'<path d="M{128 - 7.2} {e} Q128 {e - 4} {128 + 7.2} {e} Q128 {e + 4} {128 - 7.2} {e}Z" fill="{INK}"/>'
            for e in detail.bark_marks
        )
    )
    (lx, ly, lr), (rx_, ry, rr), (tx, ty, tr) = detail.left, detail.right, detail.top
    return (
        f'<defs>{mask("trunk", detail.left, detail.right, detail.top)}'
        f'{mask("left", detail.right, detail.top)}{mask("right", detail.top)}</defs>'
        f'<g transform="{transform}">'
        f'<g mask="url(#{key}-trunk)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{colour(detail.bark)}"/>{eyes}</g>'
        f'<circle cx="{lx}" cy="{ly}" r="{lr}" fill="{colour(MINT)}" mask="url(#{key}-left)"/>'
        f'<circle cx="{rx_}" cy="{ry}" r="{rr}" fill="{colour(GOLD)}" mask="url(#{key}-right)"/>'
        f'<circle cx="{tx}" cy="{ty}" r="{tr}" fill="{colour(EMERALD)}"/>'
        f"</g>"
    )


def svg(width: float, height: float, body: str, title: str = "Aspen") -> str:
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:g} {height:g}" '
        f'width="{width:g}" height="{height:g}" role="img" aria-label="{title}">'
        f"<title>{title}</title>{body}</svg>\n"
    )


def detail_for(pixels: int) -> Detail:
    return FULL if pixels >= 64 else SMALL


def tile(size: int, shape: str = "rounded") -> str:
    """The mark on the cream tile: `rounded` (a rounded square), `circle`, or `square` (full bleed)."""
    if shape == "circle":
        ground = f'<circle cx="{size / 2}" cy="{size / 2}" r="{size / 2}" fill="{CREAM}"/>'
        height = size * 0.6
    elif shape == "square":
        ground = f'<rect width="{size}" height="{size}" fill="{CREAM}"/>'
        height = size * 0.66
    else:
        ground = f'<rect width="{size}" height="{size}" rx="{size * 0.22}" fill="{CREAM}"/>'
        # A small icon spends little on its tile: the mark is what has to be recognised.
        height = size * (0.66 if size >= 64 else 0.84)
    return svg(size, size, ground + mark(detail_for(size), size / 2, size / 2, height, "m"))


def mac_icon() -> str:
    """macOS draws icons unmasked, so they follow its grid: an 824 tile in a 1024 canvas, shadowed."""
    return svg(
        1024,
        1024,
        '<defs><filter id="shadow" x="-10%" y="-10%" width="120%" height="130%">'
        '<feGaussianBlur in="SourceAlpha" stdDeviation="14"/><feOffset dy="12"/>'
        '<feComponentTransfer><feFuncA type="linear" slope="0.3"/></feComponentTransfer>'
        '<feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>'
        f'<rect x="100" y="100" width="824" height="824" rx="185" fill="{CREAM}" filter="url(#shadow)"/>'
        + mark(FULL, 512, 512, 824 * 0.64, "m"),
    )


LETTERS_WIDTH = 345
"""How wide the hand-drawn "aspen" is, from the a's bowl to the n's last stem."""


def letters(x: float, y: float, ink: str) -> str:
    """ "aspen" in round monoline strokes, its left edge at x and its baseline at y + 150."""
    a, s, p, e, n = x + 30, x + 81, x + 144, x + 241, x + 292
    return (
        f'<g transform="translate(0 {y})" fill="none" stroke="{ink}" stroke-width="14" '
        'stroke-linecap="round" stroke-linejoin="round">'
        f'<circle cx="{a}" cy="120" r="23"/><path d="M{a + 23} 97 V150"/>'
        f'<path d="M{s + 32} 104 C{s + 26} 96 {s + 14} 95 {s + 8} 97 C{s} 100 {s} 112 {s + 8} 116 '
        f'L{s + 26} 124 C{s + 35} 128 {s + 35} 140 {s + 26} 143 C{s + 18} 146 {s + 6} 145 {s} 137"/>'
        f'<path d="M{p} 97 V185"/><circle cx="{p + 23}" cy="120" r="23"/>'
        f'<path d="M{e - 23} 120 H{e + 23} A23 23 0 1 0 {e + 16.3} 136.3"/>'
        f'<path d="M{n} 150 V97 M{n} 120 A23 23 0 0 1 {n + 46} 120 V150"/></g>'
    )


def wordmark(ink: str) -> str:
    height = 119
    body = mark(FULL, 20 + height * 164 / 198 / 2, 157 - height / 2, height, "m") + letters(150, 0, ink)
    return svg(150 + LETTERS_WIDTH + 20, 200, body)


def stacked(ink: str) -> str:
    width, height = LETTERS_WIDTH + 40, 158
    body = mark(FULL, width / 2, 20 + height / 2, height, "m") + letters(20, 112, ink)
    return svg(width, 324, body)


def favicon() -> str:
    """The mark alone, filling its square, for browser tabs."""
    return svg(64, 64, mark(SMALL, 32, 32, 62, "f"))


def render(source: str, out: Path, size: tuple[int, int] | int) -> None:
    width, height = (size, size) if isinstance(size, int) else size
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", suffix=".svg", delete=False) as f:
        f.write(source)
    subprocess.run(["rsvg-convert", "-w", str(width), "-h", str(height), f.name, "-o", str(out)], check=True)
    Path(f.name).unlink()


def ico(out: Path, sizes: list[int], source: callable) -> None:
    """A multi-size .ico, each size drawn at its own detail rather than scaled from one."""
    with tempfile.TemporaryDirectory() as tmp:
        frames = []
        for size in sizes:
            frame = Path(tmp) / f"{size}.png"
            render(source(size), frame, size)
            frames.append(str(frame))
        subprocess.run(["magick", *frames, str(out)], check=True)


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def android() -> None:
    densities = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}
    for name, scale in densities.items():
        launcher, layer, status = round(48 * scale), round(108 * scale), round(24 * scale)
        mipmap = ANDROID_RES / f"mipmap-{name}"
        render(tile(launcher), mipmap / "ic_launcher.png", launcher)
        render(tile(launcher, "circle"), mipmap / "ic_launcher_round.png", launcher)
        # An adaptive icon's layers are 108dp, of which a mask shows the middle 72dp and only a
        # 66dp circle is sure to survive every launcher's shape; the mark stays inside it.
        mark_height = layer * 56 / 108
        render(svg(layer, layer, mark(FULL, layer / 2, layer / 2, mark_height, "a")), mipmap / "ic_launcher_foreground.png", layer)
        render(
            svg(layer, layer, mark(FULL, layer / 2, layer / 2, mark_height, "a", mono="#FFFFFF")),
            mipmap / "ic_launcher_monochrome.png",
            layer,
        )
        # Status bar icons are drawn by their alpha alone, in a 24dp square with 2dp of room.
        render(
            svg(status, status, mark(SMALL, status / 2, status / 2, status * 20 / 24, "s", mono="#FFFFFF")),
            ANDROID_RES / f"drawable-{name}/ic_stat_aspen.png",
            status,
        )
    for splash in sorted(ANDROID_RES.glob("drawable*/splash.png")):
        width, height = (int(v) for v in subprocess.check_output(["magick", "identify", "-format", "%w %h", str(splash)]).split())
        body = f'<rect width="{width}" height="{height}" fill="{CREAM}"/>' + mark(
            FULL, width / 2, height / 2, min(width, height) * 0.3, "p"
        )
        render(svg(width, height, body), splash, (width, height))


def desktop() -> None:
    # electron-builder makes each platform's format from these: Linux takes the sized PNGs as
    # they are, Windows the .ico, and macOS an .icns built from icon-mac.png.
    for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
        render(tile(size), DESKTOP_BUILD / f"icons/{size}x{size}.png", size)
    ico(DESKTOP_BUILD / "icon.ico", [16, 24, 32, 48, 64, 128, 256], tile)
    render(mac_icon(), DESKTOP_BUILD / "icon-mac.png", 1024)


def web() -> None:
    write(APP_PUBLIC / "favicon.svg", favicon())
    ico(APP_PUBLIC / "favicon.ico", [16, 32, 48], lambda size: svg(size, size, mark(SMALL, size / 2, size / 2, size * 0.97, "f")))
    render(tile(180, "square"), APP_PUBLIC / "apple-touch-icon.png", 180)


def passkey_page() -> None:
    """The passkey page is one self-contained document, so its favicon travels inside it."""
    uri = "data:image/svg+xml," + urllib.parse.quote(favicon().strip(), safe="=:/',")
    uri = uri.replace('"', "'")
    page = PASSKEY_PAGE.read_text()
    page, count = re.subn(r'<link rel="icon" href="[^"]*">', f'<link rel="icon" href="{uri}">', page)
    if count != 1:
        raise SystemExit(f"{PASSKEY_PAGE} needs exactly one <link rel=\"icon\">, found {count}")
    PASSKEY_PAGE.write_text(page)


def brand() -> None:
    write(BRAND / "aspen-mark.svg", svg(164, 198, mark(FULL, 82, 99, 198, "m")))
    write(BRAND / "aspen-mark-small.svg", svg(176, 202, mark(SMALL, 88, 101, 202, "m")))
    write(BRAND / "aspen-icon.svg", tile(1024))
    write(BRAND / "aspen-wordmark.svg", wordmark(INK))
    write(BRAND / "aspen-wordmark-dark.svg", wordmark(PAPER))
    write(BRAND / "aspen-stacked.svg", stacked(INK))
    write(BRAND / "aspen-stacked-dark.svg", stacked(PAPER))


if __name__ == "__main__":
    brand()
    web()
    desktop()
    android()
    passkey_page()
