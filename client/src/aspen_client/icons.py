from __future__ import annotations

import hashlib
import logging
from collections.abc import Callable
from typing import TYPE_CHECKING

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPainterPath, QPen, QPixmap

from aspen_client.qml_ui.theme import COLOR_BG_MAIN, COLOR_TEXT_MAIN
from aspen_client.types import Community, UserProfile

logger = logging.getLogger(__name__)

if TYPE_CHECKING:
    from aspen_client.api_client import AspenApiClient, TaskSpawner

class IconCache:
    """Caches per-user / per-community avatar pixmaps and icons.

    The render hot-path (per-row on insert) calls ``user_avatar_pixmap`` /
    ``community_avatar_icon`` and must not block, so every method returns
    either a cached pixmap or a fallback drawn from the display name. When
    the entity has a real icon assigned, a background fetch is dispatched
    via the supplied ``TaskSpawner`` and the registered ``on_*_icon_ready``
    callback is invoked once the bytes land — the caller (the chat window)
    is then responsible for patching live widgets.

    Owning this state outside the ``ChatWindow`` keeps the avatar / icon
    rendering logic isolated from layout concerns and lets ``_reset_client_state``
    discard the entire cache via ``clear()`` rather than touching four
    private maps.
    """

    def __init__(
        self,
        api: "AspenApiClient",
        tasks: "TaskSpawner",
        on_user_icon_ready: Callable[[str], None],
        on_community_icon_ready: Callable[[str, str], None],
    ) -> None:
        self._api = api
        self._tasks = tasks
        self._on_user_icon_ready = on_user_icon_ready
        self._on_community_icon_ready = on_community_icon_ready
        self._user_avatar_cache: dict[str, QPixmap] = {}
        self._community_icon_cache: dict[str, QIcon] = {}
        self._pending_user_icon_fetches: set[str] = set()
        self._pending_community_icon_fetches: set[str] = set()

    def clear(self) -> None:
        """Drop every cached pixmap / icon and forget pending fetches.

        Used during a state resync: the four maps must be wiped together so
        a stale icon never shadows a fresh one across a reconnect.
        """
        self._user_avatar_cache.clear()
        self._community_icon_cache.clear()
        self._pending_user_icon_fetches.clear()
        self._pending_community_icon_fetches.clear()

    def invalidate_user(self, user_id: str) -> None:
        """Drop every cached avatar for ``user_id`` (across all sizes).

        Called after a profile fetch lands so subsequent reads fall through
        to a fresh fallback render (or kick off an icon fetch for the
        profile's icon id, if any).
        """
        stale_keys = [key for key in self._user_avatar_cache if key.startswith(f"{user_id}:")]
        for key in stale_keys:
            self._user_avatar_cache.pop(key, None)

    def user_avatar_pixmap(
        self,
        user_id: str,
        size: int,
        profile: UserProfile | None = None,
    ) -> QPixmap:
        """Return an avatar pixmap synchronously.

        Cached pixmaps short-circuit; otherwise a fallback is rendered and
        the real icon (if any) is fetched in the background.
        """
        cache_key = f"{user_id}:{size}"
        if cache_key in self._user_avatar_cache:
            return self._user_avatar_cache[cache_key]

        if profile is not None and profile.icon is not None:
            self._request_user_icon_async(user_id, profile.icon)

        fallback_name = profile.name if profile is not None else user_id
        avatar = self._fallback_user_avatar_pixmap(user_id, fallback_name, size)
        self._user_avatar_cache[cache_key] = avatar
        return avatar

    def community_avatar_icon(self, community: Community) -> QIcon:
        """Return a community icon synchronously.

        Mirrors ``user_avatar_pixmap``: never blocks the render path. Kicks
        off a background fetch for the actual icon bytes when a community
        has one assigned and notifies via ``on_community_icon_ready`` once
        the bytes land.
        """
        if community.icon is not None and community.icon in self._community_icon_cache:
            return self._community_icon_cache[community.icon]
        if community.icon is not None:
            self._request_community_icon_async(community.id, community.icon)
        return self._fallback_community_avatar_icon(community.id, community.name)

    def _request_user_icon_async(self, user_id: str, icon_id: str) -> None:
        pending_key = f"{user_id}:{icon_id}"
        if pending_key in self._pending_user_icon_fetches:
            return
        self._pending_user_icon_fetches.add(pending_key)
        self._tasks.spawn(self._do_load_user_icon(user_id, icon_id, pending_key))

    async def _do_load_user_icon(
        self, user_id: str, icon_id: str, pending_key: str
    ) -> None:
        # Two-step: resolve the id to a metadata DTO carrying the
        # public ``download_url``, then fetch the bytes from the media
        # store. The dedupe slot covers both hops, so a second renderer
        # asking for the same id while either hop is in flight is a
        # no-op.
        try:
            icon = await self._api.read_icon(icon_id)
            icon_bytes, _content_type = await self._api.download_media_bytes(
                icon.download_url
            )
        except Exception as err:
            self._pending_user_icon_fetches.discard(pending_key)
            logger.warning(f"Failed to download user icon {err}")
            return
        self._pending_user_icon_fetches.discard(pending_key)
        # We render user avatars at two sizes (28 in message rows, 20 in
        # the users-preview list). Pre-populate both so subsequent cache
        # reads are hits regardless of which view asks first.
        for size in (28, 20):
            self._user_avatar_cache[f"{user_id}:{size}"] = self._circular_pixmap_from_bytes(
                icon_bytes, size
            )
        self._on_user_icon_ready(user_id)

    def _request_community_icon_async(self, community_id: str, icon_id: str) -> None:
        if icon_id in self._pending_community_icon_fetches:
            return
        self._pending_community_icon_fetches.add(icon_id)
        self._tasks.spawn(self._do_load_community_icon(community_id, icon_id))

    async def _do_load_community_icon(self, community_id: str, icon_id: str) -> None:
        # Same two-step as ``_do_load_user_icon``: metadata round-trip
        # to get the public ``download_url``, then bytes from the media
        # store. The dedupe slot covers both hops.
        try:
            icon = await self._api.read_icon(icon_id)
            icon_bytes, _content_type = await self._api.download_media_bytes(
                icon.download_url
            )
        except Exception as err:
            self._pending_community_icon_fetches.discard(icon_id)
            logger.warning(f"Failed to download community icon {err}")
            return
        self._pending_community_icon_fetches.discard(icon_id)
        pixmap = QPixmap()
        if not pixmap.loadFromData(icon_bytes):
            return
        icon = QIcon(
            pixmap.scaled(
                QSize(28, 28),
                Qt.AspectRatioMode.KeepAspectRatioByExpanding,
                Qt.TransformationMode.SmoothTransformation,
            )
        )
        self._community_icon_cache[icon_id] = icon
        self._on_community_icon_ready(community_id, icon_id)

    @staticmethod
    def _circular_pixmap_from_bytes(icon_bytes: bytes, size: int) -> QPixmap:
        source = QPixmap()
        if not source.loadFromData(icon_bytes):
            fallback = QPixmap(size, size)
            fallback.fill(Qt.GlobalColor.transparent)
            return fallback
        scaled = source.scaled(
            QSize(size, size),
            Qt.AspectRatioMode.KeepAspectRatioByExpanding,
            Qt.TransformationMode.SmoothTransformation,
        )
        result = QPixmap(size, size)
        result.fill(Qt.GlobalColor.transparent)
        painter = QPainter(result)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        clip_path = QPainterPath()
        clip_path.addEllipse(0, 0, size, size)
        painter.setClipPath(clip_path)
        painter.drawPixmap(0, 0, scaled)
        painter.setClipping(False)
        ring_pen = QPen(QColor(COLOR_BG_MAIN))
        ring_pen.setWidth(1)
        painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.setPen(ring_pen)
        painter.drawEllipse(0, 0, size - 1, size - 1)
        painter.end()
        return result

    @staticmethod
    def _fallback_user_avatar_pixmap(user_id: str, name: str, size: int) -> QPixmap:
        pixmap = QPixmap(size, size)
        pixmap.fill(Qt.GlobalColor.transparent)
        painter = QPainter(pixmap)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        hue_seed = int(hashlib.sha1(user_id.encode("utf-8")).hexdigest()[:2], 16)
        bg_color = QColor.fromHsv(int((hue_seed / 255) * 359), 90, 120)
        painter.setBrush(bg_color)
        painter.setPen(Qt.PenStyle.NoPen)
        painter.drawEllipse(0, 0, size, size)
        ring_pen = QPen(QColor(COLOR_BG_MAIN))
        ring_pen.setWidth(1)
        painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.setPen(ring_pen)
        painter.drawEllipse(0, 0, size - 1, size - 1)
        initials = IconCache._community_initials(name)
        painter.setPen(QColor(COLOR_TEXT_MAIN))
        font = painter.font()
        font.setBold(True)
        font.setPointSize(max(size // 3, 8))
        painter.setFont(font)
        painter.drawText(pixmap.rect(), Qt.AlignmentFlag.AlignCenter, initials)
        painter.end()
        return pixmap

    @staticmethod
    def _fallback_community_avatar_icon(community_id: str, community_name: str) -> QIcon:
        size = 28
        pixmap = QPixmap(size, size)
        pixmap.fill(Qt.GlobalColor.transparent)
        painter = QPainter(pixmap)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        hue_seed = int(hashlib.sha1(community_id.encode("utf-8")).hexdigest()[:2], 16)
        # Dark-theme friendly avatar tones: keep hue variety while reducing glare.
        bg_color = QColor.fromHsv(int((hue_seed / 255) * 359), 90, 120)
        painter.setBrush(bg_color)
        painter.setPen(Qt.PenStyle.NoPen)
        painter.drawEllipse(0, 0, size, size)
        ring_pen = QPen(QColor(COLOR_BG_MAIN))
        ring_pen.setWidth(1)
        painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.setPen(ring_pen)
        painter.drawEllipse(0, 0, size - 1, size - 1)

        initials = IconCache._community_initials(community_name)
        painter.setPen(QColor(COLOR_TEXT_MAIN))
        font = painter.font()
        font.setBold(True)
        font.setPointSize(9)
        painter.setFont(font)
        painter.drawText(pixmap.rect(), Qt.AlignmentFlag.AlignCenter, initials)
        painter.end()
        return QIcon(pixmap)

    @staticmethod
    def _community_initials(name: str) -> str:
        words = [part for part in name.strip().split() if part]
        if not words:
            return "?"
        if len(words) == 1:
            return words[0][:2].upper()
        return (words[0][0] + words[1][0]).upper()
