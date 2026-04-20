"""Client-side cache of server-rendered link-preview thumbnails.

The server is authoritative for everything about a link preview: it
scans the message body for URLs, fetches the remote page, extracts the
Open Graph / Twitter Card / HTML metadata, and — when the page publishes
an ``og:image`` — downloads the referenced image into our own media
store and mints a stable ``imageId`` for it. The client never touches a
third-party URL: it trades the id for bytes through the authenticated
``/link-preview-image`` endpoint and caches the decoded ``QPixmap``.

This cache mirrors the shape of :class:`aspen_client.icons.IconCache`:

- A synchronous getter (:meth:`get_pixmap`) that the render hot-path
  can call during row construction without blocking.
- A request method (:meth:`request_image`) that dedupes an in-flight
  fetch per ``imageId`` and kicks off the background coroutine via
  ``TaskSpawner``.
- A ready callback that the owner hands in at construction time, which
  fires on the GUI thread once the bytes land so the owner can patch
  live preview cards.

Failed fetches are stored as ``None`` so a second row that references
the same ``imageId`` doesn't re-issue the request every time it is
re-rendered (or a later message that re-uses the same thumbnail finds
it already settled).
"""

from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QPainter, QPainterPath, QPixmap

if TYPE_CHECKING:
    from aspen_client.api_client import AspenApiClient, TaskSpawner


# Preview thumbnails render inside a bounded card slot; rendering anything
# larger than this and then scaling down is pure waste, so the cache
# pre-scales to fit on load. The constant is also what
# ``_LinkPreviewCard`` sizes its thumbnail QLabel to, so the pixmap
# lands pixel-for-pixel without a second scaling pass at paint time.
PREVIEW_IMAGE_SIZE = 72


class LinkPreviewImageCache:
    """Owns decoded preview-image pixmaps and the async fetch plumbing.

    Lives next to :class:`IconCache` / :class:`UserDirectory` on
    ``ChatWindow``. ``MessagePane`` receives it through its constructor
    and consults it from :meth:`_LinkPreviewCard.populate`: a hit paints
    the thumbnail synchronously, a miss kicks off
    :meth:`request_image` and leaves the thumbnail slot empty; the
    ready-callback (``on_image_ready``) then fires on the GUI thread and
    ``MessagePane`` repaints the affected row.

    Every byte comes back through the authenticated Aspen API, never
    from a third-party host: there is no separate HTTP client to own,
    no TLS trust store to configure, and nothing to close on shutdown.
    Clients only know the opaque ``imageId`` the server minted when it
    ingested the page's ``og:image``; the original third-party URL never
    reaches this process.
    """

    def __init__(
        self,
        api: "AspenApiClient",
        tasks: "TaskSpawner",
        on_image_ready: Callable[[str], None],
    ) -> None:
        self._api = api
        self._tasks = tasks
        self._on_image_ready = on_image_ready
        # ``None`` in the cache means "fetch completed but the bytes
        # weren't decodable as an image" — distinct from "not yet
        # fetched" (key absent) so the preview card can settle on a
        # final state and stop asking.
        self._pixmaps: dict[str, QPixmap | None] = {}
        self._pending: set[str] = set()

    def clear(self) -> None:
        """Drop every cached thumbnail and forget pending fetches.

        Called from ``ChatWindow._reset_client_state`` on a
        post-grace reconnect, alongside the other render-state caches.
        """
        self._pixmaps.clear()
        self._pending.clear()

    def get_pixmap(self, image_id: str) -> QPixmap | None:
        """Return the decoded thumbnail, or ``None`` if we don't have one yet.

        ``None`` collapses two render-time states together — "fetch not
        yet issued" and "fetch failed" — because both answer the same
        question the caller has ("do I have a pixmap to paint?"). Rows
        that want to distinguish the two use :meth:`has_settled`.
        """
        return self._pixmaps.get(image_id)

    def has_settled(self, image_id: str) -> bool:
        """Return True if we already tried and decided (successfully or not).

        Used by the preview card to decide whether an empty thumbnail
        slot should stay hidden (``True``: fetch failed, no point
        holding space for a picture that never arrives) or keep the
        request pending.
        """
        return image_id in self._pixmaps

    def request_image(self, image_id: str) -> None:
        """Kick off the fetch coroutine if one isn't already settled or running.

        The dedupe set + cache check between them ensure we issue at
        most one request per ``imageId`` per session, regardless of how
        many preview cards reference the same thumbnail.
        """
        if image_id in self._pixmaps or image_id in self._pending:
            return
        self._pending.add(image_id)
        self._tasks.run(
            self._api.read_link_preview_image(image_id),
            on_success=lambda result, i=image_id: self._on_loaded(i, result),
            on_failure=lambda _exc, i=image_id: self._on_loaded(i, None),
        )

    def _on_loaded(
        self,
        image_id: str,
        result: tuple[bytes, str] | None,
    ) -> None:
        self._pending.discard(image_id)
        pixmap: QPixmap | None = None
        if result is not None:
            image_bytes, _mime_type = result
            decoded = QPixmap()
            if decoded.loadFromData(image_bytes):
                pixmap = self._rounded_thumbnail(decoded, PREVIEW_IMAGE_SIZE)
        self._pixmaps[image_id] = pixmap
        # Always fire the callback — the UI wants to know about
        # failures too, so it can collapse the empty thumbnail slot.
        self._on_image_ready(image_id)

    @staticmethod
    def _rounded_thumbnail(source: QPixmap, size: int) -> QPixmap:
        """Scale ``source`` to a square, rounded-corner thumbnail.

        Matches the corner radius of the card's right-hand panel so the
        thumbnail tucks flush against the card chrome instead of poking
        out of it. ``KeepAspectRatioByExpanding`` plus a clipping path
        centred on the destination rect gives a cover-crop, which looks
        right for both wide and tall source images.
        """
        scaled = source.scaled(
            QSize(size, size),
            Qt.AspectRatioMode.KeepAspectRatioByExpanding,
            Qt.TransformationMode.SmoothTransformation,
        )
        result = QPixmap(size, size)
        result.fill(Qt.GlobalColor.transparent)
        painter = QPainter(result)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        path = QPainterPath()
        path.addRoundedRect(0, 0, size, size, 4, 4)
        painter.setClipPath(path)
        # Centre the cover-cropped source over the destination.
        x = (size - scaled.width()) // 2
        y = (size - scaled.height()) // 2
        painter.drawPixmap(x, y, scaled)
        painter.end()
        return result
