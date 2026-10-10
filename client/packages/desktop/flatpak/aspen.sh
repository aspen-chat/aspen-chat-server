#!/bin/sh
# zypak starts Chromium's processes in Flatpak's sandboxes (see the manifest's `base`).
exec zypak-wrapper /app/aspen/aspen "$@"
