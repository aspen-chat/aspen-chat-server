"""QML-facing wrapper around the shared :mod:`aspen_client.theme` palette.

The Widgets path imports the palette constants directly; the QML path
needs the same colour values plus the small ``presenceColor`` helper
that maps ``"online"`` / ``"away"`` / ``"offline"`` strings to the dot
colours used elsewhere. Exposing them as a context property keeps QML
files free of hard-coded hex codes that could drift away from
``IconCache``'s painters.
"""

from __future__ import annotations

from PySide6.QtCore import Property, QObject, Slot

from aspen_client.theme import (
    COLOR_ACCENT,
    COLOR_AWAY,
    COLOR_BG_MAIN,
    COLOR_BG_PANE,
    COLOR_HIGHLIGHT,
    COLOR_TEXT_MAIN,
    COLOR_TEXT_MUTED,
)


class ThemeBridge(QObject):
    """Context-property singleton exposing the palette to QML.

    Read-only; the constants are shared with the painter routines in
    :class:`IconCache`, so introducing a setter here would create two
    sources of truth. Adjusting the palette is a code change in
    :mod:`aspen_client.theme`.
    """

    @Property(str, constant=True)
    def bgMain(self) -> str:  # noqa: N802 - QML naming convention
        return COLOR_BG_MAIN

    @Property(str, constant=True)
    def bgPane(self) -> str:  # noqa: N802
        return COLOR_BG_PANE

    @Property(str, constant=True)
    def accent(self) -> str:  # noqa: N802
        return COLOR_ACCENT

    @Property(str, constant=True)
    def away(self) -> str:  # noqa: N802
        return COLOR_AWAY

    @Property(str, constant=True)
    def highlight(self) -> str:  # noqa: N802
        return COLOR_HIGHLIGHT

    @Property(str, constant=True)
    def textMain(self) -> str:  # noqa: N802
        return COLOR_TEXT_MAIN

    @Property(str, constant=True)
    def textMuted(self) -> str:  # noqa: N802
        return COLOR_TEXT_MUTED

    @Slot(str, result=str)
    def presenceColor(self, status: str) -> str:  # noqa: N802
        """Return the dot colour matching ``IconCache.presence_dot_pixmap``.

        ``"online"`` is filled with the accent; ``"away"`` is filled
        with the away colour; everything else is treated as offline and
        rendered as an empty ring \u2014 callers paint the ring colour
        themselves and use this helper to decide whether to fill the
        dot, with empty string meaning "no fill".
        """
        normalised = status.strip().lower() if status else ""
        if normalised == "online":
            return COLOR_ACCENT
        if normalised == "away":
            return COLOR_AWAY
        return ""

    @Slot(str, result=str)
    def presenceRing(self, status: str) -> str:  # noqa: N802
        """Return the outline colour for the presence dot.

        Mirrors the pen colour in
        :meth:`IconCache.presence_dot_pixmap`: the ring matches the
        fill for online/away, and uses the accent for the empty
        offline circle so the dot is still visible against the
        sidebar.
        """
        normalised = status.strip().lower() if status else ""
        if normalised == "away":
            return COLOR_AWAY
        return COLOR_ACCENT
