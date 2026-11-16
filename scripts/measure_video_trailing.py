#!/usr/bin/env python3
"""Measure how far a phone's native video trails the page from a recording made by
`scripts/check_mobile_call.py --record-trailing`: the page there hides itself, draws a box with a
two-pixel red border on a black ground, and moves it across the screen and back, reporting the box's
inside as the hole for a picture every animation frame, as the app does. Each frame of the recording
is read along the row through the box's middle: where the red border's left edge is, and where the
picture's left edge is (the first pixel that is neither black nor red). The picture should begin
right inside the border; how far it is from there, in CSS pixels, is the trailing, negative when the
picture lags the box in the direction it moves, and in time the trailing divided by the box's speed
between that frame and the one before (the recordings keep a frame only when the screen changed,
so the frames' own timestamps give the speed). The output is one line per frame while the box moves
(the frame's time, the box's position, its speed, the picture's offset from the border's inside, the
trailing in pixels and in milliseconds) and a summary.

    scripts/measure_video_trailing.py target/mobile-call/trailing-ios.mp4

Needs `ffmpeg` and `ffprobe`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

BOX_TOP_CSS = 300
BOX_HEIGHT_CSS = 120
BOX_WIDTH_CSS = 160
BORDER_CSS = 2


def probe(path: Path) -> tuple[int, int, list[float]]:
    """The frames' size and each one's time in seconds."""
    out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                          "stream=width,height:frame=pts_time", "-of", "json", str(path)], capture_output=True, text=True, check=True).stdout
    parsed = json.loads(out)
    stream = parsed["streams"][0]
    times = [float(frame["pts_time"]) for frame in parsed["frames"]]
    return int(stream["width"]), int(stream["height"]), times


def rows(path: Path, width: int, scale: float) -> list[bytes]:
    """The row through the box's middle of every frame, as RGB bytes (two rows are cut, the
    least a subsampled frame allows, and the first kept)."""
    y = round((BOX_TOP_CSS + BOX_HEIGHT_CSS / 2) * scale) & ~1
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-vf", f"crop={width}:2:0:{y}", "-f", "rawvideo",
                          "-pix_fmt", "rgb24", "-"], capture_output=True, check=True).stdout
    stride = width * 3 * 2
    return [out[at:at + width * 3] for at in range(0, len(out) - stride + 1, stride)]


def red(pixel: bytes) -> bool:
    # The border is pure red on the page; the recording's compression softens it.
    return pixel[0] > 140 and pixel[0] - max(pixel[1], pixel[2]) > 70


def picture(pixel: bytes) -> bool:
    return not red(pixel) and max(pixel) > 40


def edges(row: bytes) -> tuple[int | None, int | None]:
    """The first red pixel's x and the first picture pixel's x, or None for each not found."""
    red_at = None
    picture_at = None
    for x in range(0, len(row) // 3):
        pixel = row[x * 3:x * 3 + 3]
        if red_at is None and red(pixel):
            red_at = x
        if picture_at is None and picture(pixel):
            picture_at = x
        if red_at is not None and picture_at is not None:
            break
    return red_at, picture_at


def main() -> None:
    parser = argparse.ArgumentParser(description="Measure the native picture's trailing in a recording.")
    parser.add_argument("recording", type=Path)
    parser.add_argument("--scale", type=float, help="device pixels per CSS pixel; measured from the red box when not given")
    args = parser.parse_args()
    width, height, times = probe(args.recording)
    scale = args.scale
    sampled = rows(args.recording, width, scale or 3.0)
    if scale is None:
        # The box's width in pixels, from a frame where it stands whole on the row.
        for row in sampled:
            xs = [x for x in range(width) if red(row[x * 3:x * 3 + 3])]
            if len(xs) >= 2 and xs[-1] - xs[0] > 100:
                scale = (xs[-1] - xs[0] + 1) / BOX_WIDTH_CSS
                break
        if scale is None:
            sys.exit("no red box found; is this a trailing recording?")
        sampled = rows(args.recording, width, scale)
    print(f"{width}x{height}, {scale:.3f} px per CSS px, {len(sampled)} frames over {times[-1] - times[0]:.1f} s")
    previous: tuple[float, float] | None = None
    trailings: list[tuple[float, float]] = []
    for index, row in enumerate(sampled):
        red_at, picture_at = edges(row)
        if red_at is None or picture_at is None:
            previous = None
            continue
        box_css = red_at / scale
        time = times[index] if index < len(times) else float(index)
        moved = None if previous is None else box_css - previous[0]
        elapsed = None if previous is None else time - previous[1]
        previous = (box_css, time)
        # The picture should start right inside the border.
        offset = (picture_at - (red_at + BORDER_CSS * scale)) / scale
        if moved is not None and elapsed is not None and abs(moved) > 0.25 and elapsed > 0:
            # Trailing is measured against the direction of motion: negative lags behind.
            trailing = offset if moved > 0 else -offset
            speed = abs(moved) / elapsed
            lag_ms = -trailing / speed * 1000
            trailings.append((trailing, lag_ms))
            print(f"t={time:7.3f}: box at {box_css:6.1f}, {speed:6.0f} px/s, picture {offset:+5.1f} inside the border, trailing {trailing:+5.1f} px = {lag_ms:+5.1f} ms behind")
    if trailings:
        px = sorted(t for t, _ in trailings)
        ms = sorted(m for _, m in trailings)
        print(f"moving frames {len(trailings)}: trailing median {px[len(px) // 2]:+.1f} px ({ms[len(ms) // 2]:+.1f} ms), worst {px[0]:+.1f} px ({ms[-1]:+.1f} ms), best {px[-1]:+.1f} px ({ms[0]:+.1f} ms)")
    else:
        print("no frame showed the box moving with a picture in it")


if __name__ == "__main__":
    main()
