from __future__ import annotations

from PySide6.QtCore import QSize
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import QPushButton

import qtawesome as qta


def material_icon(name: str, color: str = "#9A9080") -> QIcon:
    """Build a Material Design icon from a short mdi6 name."""
    return qta.icon(f"mdi6.{name}", color=color)


def apply_button_icon(
    button: QPushButton,
    icon_name: str,
    *,
    color: str = "#8EBA54",
    size: int = 18,
) -> None:
    """Apply a Material icon to a push button with consistent sizing."""
    button.setIcon(material_icon(icon_name, color))
    button.setIconSize(QSize(size, size))
