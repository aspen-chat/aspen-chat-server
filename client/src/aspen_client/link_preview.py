"""Client-side cache of server-rendered link-preview thumbnails.

The server is authoritative for everything about a link preview: it
scans the message body for URLs, fetches the remote page, extracts the
Open Graph / Twitter Card / HTML metadata, and -- when the page
publishes an ``og:image`` -- downloads the referenced image into its
own media store and templates a public, anonymous-read URL into the
``imageUrl`` field of the wire DTO. The client never touches a
third-party URL: it trades the server-supplied ``imageUrl`` for bytes
through :meth:`AspenApiClient.download_media_bytes` and caches the
decoded ``QPixmap``.

This cache mirrors the shape of :class:`aspen_client.icons.IconCache`:

- A synchronous getter (:meth:`get_pixmap`) that the render hot-path
  can call during row construction without blocking.
- A request method (:meth:`request_image`) that dedupes an in-flight
  fetch per ``image_url`` and kicks off the background coroutine via
  ``TaskSpawner``.
- A ready callback that the owner hands in at construction time, which
  fires on the GUI thread once the bytes land so the owner can patch
  live preview cards.

Failed fetches are stored as ``None`` so a second row that references
the same ``image_url`` doesn't re-issue the request every time it is
re-rendered (or a later message that re-uses the same thumbnail finds
it already settled).
"""
from __future__ import annotations

import logging

from collections.abc import Callable
from typing import TYPE_CHECKING

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QPainter, QPainterPath, QPixmap

logger = logging.getLogger(__name__)

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

    Every byte comes back through :meth:`AspenApiClient.download_media_bytes`,
    which targets the Aspen server's own media store via the
    server-templated public URL. The user's IP is therefore exposed
    only to the Aspen media store, never to the third-party origin
    that the preview was generated from.
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
        # weren't decodable as an image" -- distinct from "not yet
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

    def get_pixmap(self, image_url: str) -> QPixmap | None:
        """Return the decoded thumbnail, or ``None`` if we don't have one yet.

        ``None`` collapses two render-time states together -- "fetch not
        yet issued" and "fetch failed" -- because both answer the same
        question the caller has ("do I have a pixmap to paint?"). Rows
        that want to distinguish the two use :meth:`has_settled`.
        """
        return self._pixmaps.get(image_url)

    def has_settled(self, image_url: str) -> bool:
        """Return True if we already tried and decided (successfully or not).

        Used by the preview card to decide whether an empty thumbnail
        slot should stay hidden (``True``: fetch failed, no point
        holding space for a picture that never arrives) or keep the
        request pending.
        """
        return image_url in self._pixmaps

    def request_image(self, image_url: str) -> None:
        """Kick off the fetch coroutine if one isn't already settled or running.

        The dedupe set + cache check between them ensure we issue at
        most one request per ``image_url`` per session, regardless of
        how many preview cards reference the same thumbnail.
        """
        if image_url in self._pixmaps or image_url in self._pending:
            return
        self._pending.add(image_url)
        self._tasks.run(
            self._api.download_media_bytes(image_url),
            on_success=lambda result, u=image_url: self._on_loaded(u, result),
            on_failure=lambda exc, u=image_url: self._on_loaded(u, exc),
        )

    def _on_loaded(
        self,
        image_url: str,
        result: tuple[bytes, str] | Exception,
    ) -> None:
        self._pending.discard(image_url)
        pixmap: QPixmap | None = None
        if result is Exception:
            logger.error(f"failed to load preview image at {image_url} {result}")
        else:
            image_bytes, _content_type = result
            decoded = QPixmap()
            if decoded.loadFromData(image_bytes):
                pixmap = self._rounded_thumbnail(decoded, PREVIEW_IMAGE_SIZE)
        self._pixmaps[image_url] = pixmap
        # Always fire the callback -- the UI wants to know about
        # failures too, so it can collapse the empty thumbnail slot.
        self._on_image_ready(image_url)

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
