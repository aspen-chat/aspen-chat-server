"""The web client a script's servers serve, since a chat server will not start without one.

`stand_in` writes the least a server accepts: an `index.html` with a head for its link preview
tags, and the picture those name. Checks of the server use it, so they need no client build;
`built_or_stand_in` prefers the real build, for deployments someone will open in a browser.
"""

from __future__ import annotations

import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BUILT = REPO / "client" / "packages" / "app" / "dist"
PREVIEW_PICTURE = REPO / "client" / "packages" / "app" / "public" / "open-graph.png"


def stand_in(directory: Path) -> Path:
    """Writes a stand-in web client to `directory` and returns it."""
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "index.html").write_text(
        "<!doctype html><html><head><title>Aspen</title></head><body></body></html>\n")
    shutil.copyfile(PREVIEW_PICTURE, directory / "open-graph.png")
    return directory


def built_or_stand_in(directory: Path) -> tuple[Path, bool]:
    """The built web client when there is one, and otherwise a stand-in written to `directory`;
    and whether it is the built one."""
    if (BUILT / "index.html").is_file() and (BUILT / "open-graph.png").is_file():
        return BUILT, True
    return stand_in(directory), False
