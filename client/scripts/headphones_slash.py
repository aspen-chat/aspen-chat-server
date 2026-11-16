#!/usr/bin/env python3
"""Draws HeadphonesSlashIcon, Phosphor's Headphones crossed by Phosphor's slash, in all six weights.

Phosphor has no slashed headphones, so the app draws its own from Phosphor's geometry and writes it
as a Phosphor icon (`packages/app/src/icons/HeadphonesSlashIcon.tsx`, on Phosphor's `IconBase`), so
it takes `size`, `weight`, `color`, `mirrored`, and `IconContext` like any other.

Phosphor's Headphones are strokes around fixed centrelines, the weight being the stroke's width: a
band of radius 96 about the middle of the 256-unit grid, and two ear cups 48 by 72 with corners of
16, square where the band meets them. The fill weight fills the cups, and duotone lays them under
the regular weight at 20% opacity. Before writing anything, this script draws those centrelines
unslashed in every weight and compares them, rendered, with Phosphor's own, and refuses to write
when they differ.

Phosphor's slash is the same in every slashed icon: a round-capped line from (48,40) to (208,216),
as wide as the weight's stroke. Where it crosses the glyph, what lies on its lower left runs into
it, and what lies on its upper right stops short of it, ending in a round cap. How far short is
MicrophoneSlash's (`GAP`), which sits beside this icon in the call bar. Duotone's tint is left
whole under the slash, as in Phosphor's own.

Run it again after changing the drawing; never edit its output by hand. It needs `skia-pathops`
(`headphones_slash.requirements.txt`), `rsvg-convert`, ImageMagick's `magick`, and the client's
dependencies installed (Phosphor's icons are read from them).
"""

from __future__ import annotations

import math
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import pathops

CLIENT = Path(__file__).resolve().parent.parent
APP = CLIENT / "packages" / "app"
OUTPUT = APP / "src" / "icons" / "HeadphonesSlashIcon.tsx"

WEIGHTS = ("bold", "duotone", "fill", "light", "regular", "thin")
HALF_WIDTH = {"thin": 4, "light": 6, "regular": 8, "bold": 12, "fill": 8, "duotone": 8}
# Where an interrupted stroke's centreline ends, measured from the slash's centreline. It is where
# MicrophoneSlash's capsule ends (the centre of its round cap): 26.6 in thin, light, and regular,
# and 34.6 in bold, which leaves 10.6 between the cap and the slash in every weight. BellSlash and
# EyeSlash leave more (32 in regular); the microphone is the one shown beside this icon.
GAP = {"thin": 26.6, "light": 26.6, "regular": 26.6, "bold": 34.6, "fill": 26.6, "duotone": 26.6}
DUOTONE_OPACITY = "0.2"
# Pixels of a 256-pixel rendering that may differ from Phosphor's by more than antialiasing does.
MATCH_TOLERANCE = 4

SLASH_START = (48.0, 40.0)
SLASH_END = (208.0, 216.0)
_length = math.dist(SLASH_START, SLASH_END)
SLASH_DIRECTION = (
    (SLASH_END[0] - SLASH_START[0]) / _length,
    (SLASH_END[1] - SLASH_START[1]) / _length,
)
# Points to the slash's upper right (y grows downward).
SLASH_NORMAL = (SLASH_DIRECTION[1], -SLASH_DIRECTION[0])

BAND_CENTRE = (128.0, 128.0)
BAND_RADIUS = 96.0
CUP_CORNER = 16.0
# x0, y0, x1, y1, and the corner radii: top left, top right, bottom right, bottom left.
LEFT_CUP = (32.0, 128.0, 80.0, 200.0, (0.0, CUP_CORNER, CUP_CORNER, CUP_CORNER))
RIGHT_CUP = (176.0, 128.0, 224.0, 200.0, (CUP_CORNER, 0.0, CUP_CORNER, CUP_CORNER))
# Far enough past the 256-unit grid to stand for infinity in a half plane or a cut.
FAR = 1000.0


def side(point: tuple[float, float]) -> float:
    """Distance from the slash's centreline, positive on its upper right."""
    return (point[0] - SLASH_START[0]) * SLASH_NORMAL[0] + (
        point[1] - SLASH_START[1]
    ) * SLASH_NORMAL[1]


def arc(path: pathops.Path, centre, radius, start, end, move=False) -> None:
    """Appends a circular arc (angles in radians, y downward) as cubics of at most 90 degrees.

    skia-pathops's boolean operations take no conics, so arcs are cubics until `svg_path_data`
    writes them back out as arcs."""
    pieces = max(1, math.ceil(abs(end - start) / (math.pi / 2) - 1e-9))
    step = (end - start) / pieces
    k = 4 / 3 * math.tan(step / 4) * radius
    for i in range(pieces):
        a0 = start + i * step
        a1 = a0 + step
        p0 = (centre[0] + radius * math.cos(a0), centre[1] + radius * math.sin(a0))
        p3 = (centre[0] + radius * math.cos(a1), centre[1] + radius * math.sin(a1))
        if i == 0:
            (path.moveTo if move else path.lineTo)(*p0)
        path.cubicTo(
            p0[0] - k * math.sin(a0),
            p0[1] + k * math.cos(a0),
            p3[0] + k * math.sin(a1),
            p3[1] - k * math.cos(a1),
            *p3,
        )


def disk(centre, radius) -> pathops.Path:
    path = pathops.Path()
    arc(path, centre, radius, 0, 2 * math.pi, move=True)
    path.close()
    return path


def polygon(points) -> pathops.Path:
    path = pathops.Path()
    path.moveTo(*points[0])
    for point in points[1:]:
        path.lineTo(*point)
    path.close()
    return path


def union(*paths: pathops.Path) -> pathops.Path:
    result = paths[0]
    for path in paths[1:]:
        result = pathops.op(result, path, pathops.PathOp.UNION)
    return result


def intersection(a: pathops.Path, b: pathops.Path) -> pathops.Path:
    return pathops.op(a, b, pathops.PathOp.INTERSECTION)


def difference(a: pathops.Path, b: pathops.Path) -> pathops.Path:
    return pathops.op(a, b, pathops.PathOp.DIFFERENCE)


def rounded_rect(x0, y0, x1, y1, radii) -> pathops.Path:
    top_left, top_right, bottom_right, bottom_left = radii
    path = pathops.Path()
    path.moveTo(x0 + top_left, y0)
    path.lineTo(x1 - top_right, y0)
    if top_right:
        arc(path, (x1 - top_right, y0 + top_right), top_right, -math.pi / 2, 0)
    path.lineTo(x1, y1 - bottom_right)
    if bottom_right:
        arc(path, (x1 - bottom_right, y1 - bottom_right), bottom_right, 0, math.pi / 2)
    path.lineTo(x0 + bottom_left, y1)
    if bottom_left:
        arc(path, (x0 + bottom_left, y1 - bottom_left), bottom_left, math.pi / 2, math.pi)
    path.lineTo(x0, y0 + top_left)
    if top_left:
        arc(path, (x0 + top_left, y0 + top_left), top_left, math.pi, 1.5 * math.pi)
    path.close()
    return path


def band(half_width, start=math.pi, end=2 * math.pi) -> pathops.Path:
    """The band's stroke between two angles of its centreline, cut square to it at both ends."""
    path = pathops.Path()
    arc(path, BAND_CENTRE, BAND_RADIUS + half_width, start, end, move=True)
    arc(path, BAND_CENTRE, BAND_RADIUS - half_width, end, start)
    path.close()
    return path


def cup_outline(cup, half_width) -> pathops.Path:
    """The outside of a cup's stroke, which is the whole cup when it is filled."""
    x0, y0, x1, y1, radii = cup
    return rounded_rect(
        x0 - half_width,
        y0 - half_width,
        x1 + half_width,
        y1 + half_width,
        [r + half_width if r else 0.0 for r in radii],
    )


def cup(cup, weight) -> pathops.Path:
    half_width = HALF_WIDTH[weight]
    outline = cup_outline(cup, half_width)
    if weight == "fill":
        return outline
    x0, y0, x1, y1, radii = cup
    hole = rounded_rect(
        x0 + half_width,
        y0 + half_width,
        x1 - half_width,
        y1 - half_width,
        [max(r - half_width, 0.0) for r in radii],
    )
    return difference(outline, hole)


def duotone_tint() -> pathops.Path:
    """The cups' insides, out to their centrelines, which duotone tints."""
    return union(*(rounded_rect(*c[:4], c[4]) for c in (LEFT_CUP, RIGHT_CUP)))


def headphones(weight) -> pathops.Path:
    return union(band(HALF_WIDTH[weight]), cup(LEFT_CUP, weight), cup(RIGHT_CUP, weight))


def half_plane(offset, upper) -> pathops.Path:
    """The points whose `side` is at least `offset` (upper) or at most `offset`."""
    origin = (
        SLASH_START[0] + SLASH_NORMAL[0] * offset,
        SLASH_START[1] + SLASH_NORMAL[1] * offset,
    )
    a = (origin[0] - SLASH_DIRECTION[0] * FAR, origin[1] - SLASH_DIRECTION[1] * FAR)
    b = (origin[0] + SLASH_DIRECTION[0] * FAR, origin[1] + SLASH_DIRECTION[1] * FAR)
    sign = 1 if upper else -1
    out = (SLASH_NORMAL[0] * FAR * sign, SLASH_NORMAL[1] * FAR * sign)
    return polygon([a, b, (b[0] + out[0], b[1] + out[1]), (a[0] + out[0], a[1] + out[1])])


def band_cut(gap) -> float:
    """The angle of the band's centreline, past its crossing with the slash, where `side` is gap."""
    normal_angle = math.atan2(SLASH_NORMAL[1], SLASH_NORMAL[0])
    angle = normal_angle - math.acos((gap - side(BAND_CENTRE)) / BAND_RADIUS)
    angle %= 2 * math.pi
    if not math.pi <= angle <= 2 * math.pi:
        raise ValueError("the slash's gap does not cross the band over its top")
    return angle


def right_cup_cuts(gap):
    """Where the right cup's centreline crosses `side` == gap, walking it clockwise: the cut that
    starts the part kept and the cut that ends it, each as (point, inner), where inner is a point
    in the cup's hole on the centreline's normal through the cut, so that a line from inner
    through the point cuts the stroke square to its centreline."""
    x0, y0, x1, y1, _ = RIGHT_CUP
    r = CUP_CORNER
    # Clockwise from the top edge, each a line (start, end) or a corner arc (centre, start, end).
    segments = [
        ("line", (x0 + r, y0), (x1, y0)),
        ("line", (x1, y0), (x1, y1 - r)),
        ("arc", (x1 - r, y1 - r), 0.0, math.pi / 2),
        ("line", (x1 - r, y1), (x0 + r, y1)),
        ("arc", (x0 + r, y1 - r), math.pi / 2, math.pi),
        ("line", (x0, y1 - r), (x0, y0 + r)),
        ("arc", (x0 + r, y0 + r), math.pi, 1.5 * math.pi),
    ]

    def point(segment, t):
        if segment[0] == "line":
            (ax, ay), (bx, by) = segment[1], segment[2]
            return (ax + (bx - ax) * t, ay + (by - ay) * t)
        centre, start, end = segment[1], segment[2], segment[3]
        angle = start + (end - start) * t
        return (centre[0] + r * math.cos(angle), centre[1] + r * math.sin(angle))

    def inner(segment, p):
        if segment[0] == "arc":
            return segment[1]
        (ax, ay), (bx, by) = segment[1], segment[2]
        length = math.dist((ax, ay), (bx, by))
        # Walking clockwise with y downward, the hole is on the right of the direction of travel.
        return (p[0] - (by - ay) / length * r, p[1] + (bx - ax) / length * r)

    start = end = None
    for segment in segments:
        before = side(point(segment, 0)) >= gap
        if before == (side(point(segment, 1)) >= gap):
            continue
        lo, hi = 0.0, 1.0
        for _ in range(60):
            middle = (lo + hi) / 2
            if (side(point(segment, middle)) >= gap) == before:
                lo = middle
            else:
                hi = middle
        p = point(segment, (lo + hi) / 2)
        if before:
            end = (p, inner(segment, p))
        else:
            start = (p, inner(segment, p))
    if start is None or end is None:
        raise ValueError("the slash's gap does not cross the right cup twice")
    return start, end


def toward(origin, through):
    distance = math.dist(origin, through)
    return (
        origin[0] + (through[0] - origin[0]) / distance * FAR,
        origin[1] + (through[1] - origin[1]) / distance * FAR,
    )


def headphones_slash(weight) -> pathops.Path:
    half_width, gap = HALF_WIDTH[weight], GAP[weight]
    offset = (SLASH_NORMAL[0] * half_width, SLASH_NORMAL[1] * half_width)
    slash = union(
        polygon(
            [
                (SLASH_START[0] + offset[0], SLASH_START[1] + offset[1]),
                (SLASH_END[0] + offset[0], SLASH_END[1] + offset[1]),
                (SLASH_END[0] - offset[0], SLASH_END[1] - offset[1]),
                (SLASH_START[0] - offset[0], SLASH_START[1] - offset[1]),
            ]
        ),
        disk(SLASH_START, half_width),
        disk(SLASH_END, half_width),
    )

    # The band and the left cup run into the slash on its lower left. Of the right cup only a
    # sliver of its bottom-left corner lies there, which would stand out past the slash's far edge
    # as a bump, so the right cup keeps only what lies beyond the gap.
    parts = [
        slash,
        intersection(union(band(half_width), cup(LEFT_CUP, weight)), half_plane(0, upper=False)),
    ]

    cut = band_cut(gap)
    cap = (
        BAND_CENTRE[0] + BAND_RADIUS * math.cos(cut),
        BAND_CENTRE[1] + BAND_RADIUS * math.sin(cut),
    )
    parts += [band(half_width, cut), disk(cap, half_width)]

    (top, top_inner), (bottom, bottom_inner) = right_cup_cuts(gap)
    if weight == "fill":
        # A filled cup has no centreline to end on; it is cut flat where its stroke's caps reach.
        kept = intersection(cup(RIGHT_CUP, weight), half_plane(gap - half_width, upper=True))
    else:
        x1, y0 = RIGHT_CUP[2], RIGHT_CUP[1]
        keep = polygon(
            [
                top_inner,
                toward(top_inner, top),
                (x1 + FAR, y0 - FAR),
                toward(bottom_inner, bottom),
                bottom_inner,
            ]
        )
        kept = intersection(cup(RIGHT_CUP, weight), keep)
    parts += [kept, disk(top, half_width), disk(bottom, half_width)]
    return union(*parts)


def number(value: float) -> str:
    text = f"{value:.2f}".rstrip("0").rstrip(".")
    return "0" if text in ("", "-0") else text


def _cubic_at(p0, p1, p2, p3, t):
    u = 1 - t
    return tuple(
        u**3 * p0[i] + 3 * u * u * t * p1[i] + 3 * u * t * t * p2[i] + t**3 * p3[i]
        for i in range(2)
    )


def _circle_through(a, b, c):
    d = 2 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]))
    if abs(d) < 1e-9:
        return None
    sa, sb, sc = (a[0] ** 2 + a[1] ** 2), (b[0] ** 2 + b[1] ** 2), (c[0] ** 2 + c[1] ** 2)
    centre = (
        (sa * (b[1] - c[1]) + sb * (c[1] - a[1]) + sc * (a[1] - b[1])) / d,
        (sa * (c[0] - b[0]) + sb * (a[0] - c[0]) + sc * (b[0] - a[0])) / d,
    )
    return centre, math.dist(centre, a)


# A cubic strays from the circle it stands for by up to 0.027% of its radius, 0.03 units at the
# largest radius here; a cubic within this of a circle is that circle's arc.
ARC_TOLERANCE = 0.05


def _fits(points, centre, radius) -> bool:
    return all(abs(math.dist(p, centre) - radius) < ARC_TOLERANCE for p in points)


def _centre_for(a, b, radius, toward_point):
    """The centre of the circle of `radius` through a and b on the side away from toward_point,
    a point of the arc between them, or None when the chord is longer than the diameter."""
    chord = math.dist(a, b)
    if chord > 2 * radius:
        return None
    middle = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
    depth = math.sqrt(radius**2 - (chord / 2) ** 2)
    normal = (-(b[1] - a[1]) / chord, (b[0] - a[0]) / chord)
    # The arc bulges away from its centre, so the centre is on the side opposite its middle.
    if (toward_point[0] - middle[0]) * normal[0] + (toward_point[1] - middle[1]) * normal[1] > 0:
        normal = (-normal[0], -normal[1])
    return (middle[0] + normal[0] * depth, middle[1] + normal[1] * depth)


def svg_path_data(path: pathops.Path) -> str:
    """SVG path data, writing each cubic that is a circular arc as one, as Phosphor's own paths
    are written."""
    out = []
    current = start = None
    for verb, points in path:
        if verb == pathops.PathVerb.MOVE:
            current = start = points[0]
            out.append(f"M{number(current[0])},{number(current[1])}")
        elif verb == pathops.PathVerb.LINE:
            # pathops leaves lines a few hundredths long where shapes meet, which vanish when
            # rounded to the hundredths written.
            end = f"{number(points[0][0])},{number(points[0][1])}"
            if end != f"{number(current[0])},{number(current[1])}":
                out.append(f"L{end}")
            current = points[0]
        elif verb == pathops.PathVerb.CUBIC:
            p1, p2, p3 = points
            middle = _cubic_at(current, p1, p2, p3, 0.5)
            samples = [_cubic_at(current, p1, p2, p3, t) for t in (0.1, 0.25, 0.4, 0.6, 0.75, 0.9)]
            circle = _circle_through(current, middle, p3)
            if circle and _fits(samples, *circle):
                # Every radius drawn here is a whole number; a short piece's fitted one strays
                # from it. Take the whole number when the piece fits it as well.
                whole = float(round(circle[1]))
                centre = _centre_for(current, p3, whole, middle) if whole else None
                if centre and _fits(samples, centre, whole):
                    circle = (centre, whole)
                turn = (middle[0] - current[0]) * (p3[1] - current[1]) - (
                    middle[1] - current[1]
                ) * (p3[0] - current[0])
                # Positive is clockwise on screen, SVG's sweep 1. No cubic here spans 90 degrees
                # or more, so none is the large arc.
                sweep = 1 if turn > 0 else 0
                radius = number(circle[1])
                out.append(f"A{radius},{radius},0,0,{sweep},{number(p3[0])},{number(p3[1])}")
            else:
                out.append("C" + ",".join(number(v) for p in points for v in p))
            current = p3
        elif verb == pathops.PathVerb.QUAD:
            out.append("Q" + ",".join(number(v) for p in points for v in p))
            current = points[1]
        elif verb == pathops.PathVerb.CLOSE:
            out.append("Z")
            current = start
        else:
            raise ValueError(f"unexpected path verb {verb}")
    return "".join(out)


def drawing(weight, glyph: pathops.Path) -> list[tuple[str, str | None]]:
    """The paths of one weight, as (data, opacity)."""
    paths = [(svg_path_data(glyph), None)]
    if weight == "duotone":
        paths.insert(0, (svg_path_data(duotone_tint()), DUOTONE_OPACITY))
    return paths


def phosphor_headphones() -> dict[str, list[tuple[str, str | None]]]:
    """Phosphor's own Headphones in each weight, rendered by the client's installed copy."""
    script = """
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { HeadphonesIcon } from "@phosphor-icons/react/dist/ssr";
const weights = JSON.parse(process.argv[1]);
console.log(JSON.stringify(Object.fromEntries(weights.map((weight) =>
  [weight, renderToStaticMarkup(createElement(HeadphonesIcon, { weight }))]))));
"""
    import json

    result = subprocess.run(
        ["node", "--input-type=module", "-e", script, json.dumps(WEIGHTS)],
        cwd=APP,
        check=True,
        capture_output=True,
        text=True,
    )
    markup = json.loads(result.stdout)
    namespace = "{http://www.w3.org/2000/svg}"
    return {
        weight: [
            (element.attrib["d"], element.attrib.get("opacity"))
            for element in ElementTree.fromstring(svg).iter(f"{namespace}path")
        ]
        for weight, svg in markup.items()
    }


def svg_document(paths) -> str:
    inner = "".join(
        f'<path d="{d}"' + (f' opacity="{opacity}"' if opacity else "") + "/>"
        for d, opacity in paths
    )
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">'
        f'<rect width="256" height="256" fill="white"/>{inner}</svg>'
    )


def differing_pixels(a, b, directory: Path) -> int:
    images = []
    for name, paths in (("a", a), ("b", b)):
        svg = directory / f"{name}.svg"
        svg.write_text(svg_document(paths))
        png = directory / f"{name}.png"
        subprocess.run(["rsvg-convert", str(svg), "-o", str(png)], check=True)
        images.append(str(png))
    # compare exits 1 when the images differ, which is an answer here, not a failure.
    result = subprocess.run(
        ["magick", "compare", "-metric", "AE", "-fuzz", "40%", *images, "null:"],
        capture_output=True,
        text=True,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(f"magick compare failed: {result.stderr}")
    return round(float(result.stderr.split()[0]))


def check_reconstruction() -> None:
    phosphor = phosphor_headphones()
    with tempfile.TemporaryDirectory() as directory:
        for weight in WEIGHTS:
            ours = drawing(weight, headphones(weight))
            pixels = differing_pixels(phosphor[weight], ours, Path(directory))
            if pixels > MATCH_TOLERANCE:
                sys.exit(
                    f"The {weight} headphones drawn here differ from Phosphor's in {pixels} pixels "
                    "of 256 by 256; its geometry has changed, so this script's must follow."
                )


HEADER = (
    "// Written by scripts/headphones_slash.py, which says how it is drawn; run it again rather\n"
    "// than editing this file. Drawn from Phosphor's Headphones and its slash (MIT, Phosphor\n"
    "// Icons).\n"
)


def path_element(d, opacity, indent) -> str:
    """A <path>, laid out as Prettier lays it out, so that formatting leaves the file alone: on
    one line with its one attribute however long, and an attribute to a line when it has two."""
    if not opacity:
        return f'{indent}<path d="{d}" />'
    return f'{indent}<path\n{indent}  d="{d}"\n{indent}  opacity="{opacity}"\n{indent}/>'


def component() -> str:
    entries = []
    for weight in WEIGHTS:
        paths = drawing(weight, headphones_slash(weight))
        if len(paths) == 1:
            element = path_element(*paths[0], "    ")
            entries.append(f'  [\n    "{weight}",\n{element},\n  ],')
        else:
            body = "\n".join(path_element(d, opacity, "      ") for d, opacity in paths)
            entries.append(f'  [\n    "{weight}",\n    <>\n{body}\n    </>,\n  ],')
    return f"""{HEADER}import {{ IconBase, type Icon, type IconWeight }} from "@phosphor-icons/react";
import {{ forwardRef, type ReactElement }} from "react";

const weights = new Map<IconWeight, ReactElement>([
{chr(10).join(entries)}
]);

/** Phosphor's Headphones, crossed out: someone who has deafened themselves in a call. */
export const HeadphonesSlashIcon: Icon = forwardRef((props, ref) => (
  <IconBase ref={{ref}} {{...props}} weights={{weights}} />
));
HeadphonesSlashIcon.displayName = "HeadphonesSlashIcon";
"""


def main() -> None:
    check_reconstruction()
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(component())
    print(f"wrote {OUTPUT.relative_to(CLIENT)}")


if __name__ == "__main__":
    main()
