"""QML image provider that serves cached avatars and preview thumbnails.

QML ``Image`` elements bind their ``source`` to URLs of the form
``image://aspen/<kind>/<key>[/<size>]``; the provider parses the URL,
asks the relevant cache for a synchronous hit, and either returns the
cached :class:`QImage` immediately or returns a placeholder while
kicking off the same async fetch the Widgets path uses. The cache's
existing "ready" callback (``IconCache.on_user_icon_ready`` /
``IconCache.on_community_icon_ready`` /
``LinkPreviewImageCache.on_image_ready``) fires on the GUI thread once
bytes land; the controller layer then bumps an integer ``epoch``
property that QML ``Image.source`` bindings read in their query string,
which forces QML to re-issue the request \u2014 this time hitting the cache.

The provider never makes network calls itself. Every actual fetch is
dispatched through the existing :class:`TaskSpawner` discipline owned by
:class:`IconCache` and :class:`LinkPreviewImageCache`; the AGENTS.md
"every API call goes through ``TaskSpawner``" rule continues to hold
even though QML pulls the bytes through a synchronous-looking
:class:`QQuickImageProvider` interface.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QImage, QPixmap
from PySide6.QtQuick import QQuickImageProvider

from aspen_client.types import Community

if TYPE_CHECKING:
    from aspen_client.icons import IconCache
    from aspen_client.link_preview import LinkPreviewImageCache
    from aspen_client.state import ClientState
    from aspen_client.user_directory import UserDirectory


# Default sizes for the three image kinds. The QML side is free to pass
# an explicit size segment in the URL (``image://aspen/user/<id>/40``);
# when omitted we fall back to these defaults so a delegate can request
# ``"image://aspen/community/" + model.id`` without thinking about it.
_DEFAULT_USER_SIZE = 28
_DEFAULT_COMMUNITY_SIZE = 28


class AspenImageProvider(QQuickImageProvider):
    """Resolves ``image://aspen/...`` URLs against the existing caches.

    URL schemes accepted:

    * ``image://aspen/user/<userId>`` \u2014 user avatar at the default size.
    * ``image://aspen/user/<userId>/<size>`` \u2014 user avatar at ``size``
      pixels square.
    * ``image://aspen/community/<communityId>`` \u2014 community icon.
    * ``image://aspen/preview/<imageId>`` \u2014 link-preview thumbnail.

    The optional ``?v=<epoch>`` query string the QML side appends to its
    ``source`` is part of the URL Qt passes here only as far as
    invalidating the QML-side cache; this provider ignores it because
    the answer it returns is always the freshest bytes it has on hand
    (the controller bumps the epoch precisely so QML reissues the
    request and we serve the just-landed bytes).
    """

    def __init__(
        self,
        icons: "IconCache",
        users: "UserDirectory",
        link_previews: "LinkPreviewImageCache",
        state: "ClientState",
    ) -> None:
        # ``Image`` requests yield a ``QImage``; we convert from the
        # cached ``QPixmap`` once per request. Qt copies the result
        # before handing it to the QML render thread, so it's safe to
        # touch shared state on the GUI thread here.
        super().__init__(QQuickImageProvider.ImageType.Image)
        self._icons = icons
        self._users = users
        self._link_previews = link_previews
        # ``state`` is held by reference so a ``_reset_client_state``
        # swap (which rebinds ``ChatController._state`` to a fresh
        # instance) requires the controller to re-point this attribute
        # too. ``set_state`` exists for exactly that case.
        self._state = state

    def set_state(self, state: "ClientState") -> None:
        """Re-target the provider after a ``_reset_client_state`` swap."""
        self._state = state

    def requestImage(  # type: ignore[override]
        self, id: str, size: QSize, requested_size: QSize
    ) -> QImage:
        kind, key, requested_pixels = self._parse_id(id)
        pixmap = self._resolve(kind, key, requested_pixels)
        if pixmap is None or pixmap.isNull():
            pixmap = self._placeholder(requested_pixels)
        if size is not None:
            size.setWidth(pixmap.width())
            size.setHeight(pixmap.height())
        return pixmap.toImage()

    @staticmethod
    def _parse_id(raw_id: str) -> tuple[str, str, int | None]:
        """Split ``user/<id>[/<size>]`` (etc.) into ``(kind, key, size)``.

        The ``?v=<epoch>`` query suffix QML appends for cache busting is
        stripped here; the controller bumps the epoch explicitly when
        new bytes are available, which is what triggers QML to reissue
        the request \u2014 the value itself carries no semantic.
        """
        cleaned = raw_id.split("?", 1)[0]
        parts = [segment for segment in cleaned.split("/") if segment]
        if not parts:
            return "", "", None
        kind = parts[0]
        key = parts[1] if len(parts) >= 2 else ""
        if len(parts) >= 3:
            try:
                return kind, key, int(parts[2])
            except ValueError:
                return kind, key, None
        return kind, key, None

    def _resolve(
        self, kind: str, key: str, requested_pixels: int | None
    ) -> QPixmap | None:
        if not key:
            return None
        if kind == "user":
            size = requested_pixels or _DEFAULT_USER_SIZE
            profile = self._users.get_profile(key)
            return self._icons.user_avatar_pixmap(key, size, profile)
        if kind == "community":
            community = self._state.communities.get(key) or Community(id=key, name=key)
            icon = self._icons.community_avatar_icon(community)
            return icon.pixmap(QSize(_DEFAULT_COMMUNITY_SIZE, _DEFAULT_COMMUNITY_SIZE))
        if kind == "preview":
            pixmap = self._link_previews.get_pixmap(key)
            if pixmap is None and not self._link_previews.has_settled(key):
                self._link_previews.request_image(key)
            return pixmap
        return None

    @staticmethod
    def _placeholder(requested_pixels: int | None) -> QPixmap:
        size = requested_pixels or _DEFAULT_USER_SIZE
        pixmap = QPixmap(size, size)
        pixmap.fill(Qt.GlobalColor.transparent)
        return pixmap


def _user_avatar_url(user_id: str, epoch: int, size: int = _DEFAULT_USER_SIZE) -> str:
    """Format a stable ``image://`` URL for a user avatar.

    Exposed as a helper so QML callers don't reimplement the URL shape
    inline. ``epoch`` is appended as a query string so QML sees the
    binding change when the controller bumps it; the provider strips
    the query before resolving.
    """
    return f"image://aspen/user/{user_id}/{size}?v={epoch}"


def _community_icon_url(community_id: str, epoch: int) -> str:
    return f"image://aspen/community/{community_id}?v={epoch}"


def _preview_image_url(image_id: str, epoch: int) -> str:
    return f"image://aspen/preview/{image_id}?v={epoch}"


__all__ = [
    "AspenImageProvider",
    "_user_avatar_url",
    "_community_icon_url",
    "_preview_image_url",
]
