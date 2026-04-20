from __future__ import annotations

# Centralised palette so non-UI modules (icons.py renders avatars and presence
# dots) can reference the same colors without importing ui.py and pulling in
# the entire QtWidgets stack — and without re-introducing a circular dependency
# between ui.py and the icon cache.

COLOR_BG_MAIN = "#1E2A18"
COLOR_BG_PANE = "#141C10"
COLOR_ACCENT = "#8EBA54"
COLOR_AWAY = "#D7B25C"
COLOR_HIGHLIGHT = "#FDF4E3"
COLOR_TEXT_MAIN = "#EDE4D0"
COLOR_TEXT_MUTED = "#9A9080"
