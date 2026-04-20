from __future__ import annotations

import asyncio
import html
from typing import Any, Callable

from PySide6.QtCore import QEvent, QObject, QTimer, QSize, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPen, QPixmap
from PySide6.QtWidgets import (
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QListView,
    QMainWindow,
    QMessageBox,
    QStyle,
    QStyledItemDelegate,
    QSplitter,
    QSplitterHandle,
    QStackedWidget,
    QTextEdit,
    QToolButton,
    QVBoxLayout,
    QWidget,
    QFrame,
)

from aspen_client.api_client import AspenApiClient, TaskSpawner
from aspen_client.event_client import EventStreamClient
from aspen_client.generated.event_models import ServerEvent as GeneratedServerEvent
from aspen_client.icons import IconCache, apply_button_icon, material_icon
from aspen_client.keyed_list import KeyedListWidget
from aspen_client.link_preview import LinkPreviewImageCache
from aspen_client.state import ClientState
from aspen_client.types import Channel, Community, LinkPreview, Message, UserProfile
from aspen_client.ui_login import LoginPage
from aspen_client.ui_messages import MessagePane
from aspen_client.user_directory import UserDirectory

from aspen_client.theme import (
    COLOR_ACCENT,
    COLOR_AWAY,
    COLOR_BG_MAIN,
    COLOR_BG_PANE,
    COLOR_HIGHLIGHT,
    COLOR_TEXT_MAIN,
    COLOR_TEXT_MUTED,
)


class CollapsibleSidebarHandle(QSplitterHandle):
    def __init__(self, orientation: Qt.Orientation, parent: QSplitter) -> None:
        super().__init__(orientation, parent)
        self._toggle_button = QToolButton(self)
        self._toggle_button.setAutoRaise(True)
        self._toggle_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._toggle_button.setFixedSize(QSize(20, 20))
        self._toggle_button.clicked.connect(parent.toggle_community_sidebar)  # type: ignore[attr-defined]

    def resizeEvent(self, event) -> None:  # type: ignore[override]
        super().resizeEvent(event)
        self._toggle_button.move(
            (self.width() - self._toggle_button.width()) // 2,
            (self.height() - self._toggle_button.height()) // 2,
        )

    def set_collapsed(self, collapsed: bool) -> None:
        icon_name = (
            "arrow-right-drop-circle-outline"
            if collapsed
            else "arrow-left-drop-circle-outline"
        )
        self._toggle_button.setIcon(material_icon(icon_name))
        self._toggle_button.setIconSize(QSize(18, 18))
        self._toggle_button.setToolTip(
            "Show communities" if collapsed else "Hide communities"
        )

    def mousePressEvent(self, event) -> None:  # type: ignore[override]
        # Keep the centered button clickable, but disable splitter dragging.
        event.ignore()

    def mouseMoveEvent(self, event) -> None:  # type: ignore[override]
        event.ignore()

    def mouseReleaseEvent(self, event) -> None:  # type: ignore[override]
        event.ignore()


class CollapsibleSidebarSplitter(QSplitter):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(Qt.Orientation.Horizontal, parent)
        self._community_collapsed = False
        self._expanded_community_width = 72
        self._collapsed_community_width = 32
        self._collapse_state_changed_callback: Callable[[bool], None] | None = None
        self.setHandleWidth(16)
        self.setStyleSheet("QSplitter::handle { background: transparent; border: none; }")
        self.splitterMoved.connect(self._on_splitter_moved)

    def createHandle(self) -> QSplitterHandle:
        return CollapsibleSidebarHandle(self.orientation(), self)

    def toggle_community_sidebar(self) -> None:
        sizes = self.sizes()
        if len(sizes) < 2:
            return

        if self._community_collapsed:
            target = max(self._expanded_community_width, self._collapsed_community_width + 24)
            remainder = max(sizes[0] + sizes[1] - target, 1)
            new_sizes = [target, remainder]
            if len(sizes) > 2:
                new_sizes.append(sizes[2])
            self.setSizes(new_sizes)
            self._community_collapsed = False
        else:
            if sizes[0] > 0:
                self._expanded_community_width = sizes[0]
            new_sizes = [
                self._collapsed_community_width,
                max(sizes[0] + sizes[1] - self._collapsed_community_width, 1),
            ]
            if len(sizes) > 2:
                new_sizes.append(sizes[2])
            self.setSizes(new_sizes)
            self._community_collapsed = True

        self._refresh_toggle_icon()

    def _on_splitter_moved(self, _pos: int, _index: int) -> None:
        sizes = self.sizes()
        self._community_collapsed = bool(sizes and sizes[0] <= self._collapsed_community_width)
        if sizes and sizes[0] > 0:
            self._expanded_community_width = sizes[0]
        self._refresh_toggle_icon()

    def _refresh_toggle_icon(self) -> None:
        if self.count() < 2:
            return
        handle = self.handle(1)
        if isinstance(handle, CollapsibleSidebarHandle):
            handle.set_collapsed(self._community_collapsed)
        if self._collapse_state_changed_callback is not None:
            self._collapse_state_changed_callback(self._community_collapsed)

    def initialize_toggle_icon(self) -> None:
        self._refresh_toggle_icon()

    def set_collapse_state_changed_callback(
        self, callback: Callable[[bool], None]
    ) -> None:
        self._collapse_state_changed_callback = callback
        callback(self._community_collapsed)

    def set_expanded_community_width(self, target_width: int) -> None:
        self._expanded_community_width = max(
            target_width,
            self._collapsed_community_width + 24,
        )
        if self._community_collapsed:
            return
        sizes = self.sizes()
        if len(sizes) < 2:
            return
        target = self._expanded_community_width
        remainder = max(sizes[0] + sizes[1] - target, 1)
        new_sizes = [target, remainder]
        if len(sizes) > 2:
            new_sizes.append(sizes[2])
        self.setSizes(new_sizes)


class CommunityAvatarDelegate(QStyledItemDelegate):
    def __init__(self, icon_size: int = 28, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._icon_size = icon_size

    def paint(self, painter: QPainter, option, index) -> None:  # type: ignore[override]
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        icon_value = index.data(Qt.ItemDataRole.DecorationRole)
        if not isinstance(icon_value, QIcon):
            painter.restore()
            return

        icon_rect = option.rect.adjusted(0, 0, 0, 0)
        icon_rect.setSize(QSize(self._icon_size, self._icon_size))
        icon_rect.moveCenter(option.rect.center())

        if bool(option.state & QStyle.StateFlag.State_Selected):
            ring_rect = icon_rect.adjusted(-1, -1, 1, 1)
            ring_pen = QPen(QColor(COLOR_HIGHLIGHT))
            ring_pen.setWidth(1)
            painter.setBrush(Qt.BrushStyle.NoBrush)
            painter.setPen(ring_pen)
            painter.drawEllipse(ring_rect)

        pixmap = icon_value.pixmap(QSize(self._icon_size, self._icon_size))
        painter.drawPixmap(icon_rect.topLeft(), pixmap)
        painter.restore()

    def sizeHint(self, option, index):  # type: ignore[override]
        return QSize(max(option.rect.width(), self._icon_size + 4), self._icon_size + 8)


class ChatWindow(QMainWindow):
    def __init__(
        self,
        api: AspenApiClient,
        events: EventStreamClient,
        app_close_event: asyncio.Event,
    ) -> None:
        super().__init__()
        self._api = api
        self._events = events
        # ``main()`` is blocked on ``app_close_event.wait()`` inside
        # ``loop.run_until_complete`` -- we set this event at the end of
        # the async-shutdown coroutine to unblock it cleanly, instead of
        # relying on Qt's ``aboutToQuit`` which fires too late in Qt's
        # teardown dance (by the time it fires, qasync has already
        # started stopping the event loop, and the ``Event.set()``
        # wake-up callback then runs on a not-running loop and raises
        # ``RuntimeError: loop ... is not the running loop`` -- see the
        # comment in ``_async_shutdown``).
        self._app_close_event = app_close_event
        # Two-phase shutdown flags. ``_shutdown_started`` flips as soon
        # as ``closeEvent`` first fires so we don't spawn the cleanup
        # coroutine more than once if the user mashes the close button.
        # ``_shutdown_complete`` only flips once the async cleanup
        # (``TaskSpawner.shutdown`` + ``AspenApiClient.aclose``) has
        # finished; until then ``closeEvent`` ignores the close request
        # so the window stays alive while in-flight HTTP work unwinds.
        self._shutdown_started = False
        self._shutdown_complete = False

        # The single spawner every API call in this window flows
        # through. ``spawn`` schedules an ``async def`` helper on the
        # asyncio loop that ``qasync`` drives on top of the Qt event
        # loop, so resumption after ``await`` lands on the GUI thread
        # and is safe to mutate widgets from. Direct calls to
        # ``self._api`` from GUI handlers are forbidden (see
        # client/AGENTS.md); the only sanctioned bypass is
        # ``self._api.aclose()`` during the final shutdown phase.
        self._tasks = TaskSpawner()

        self._state = ClientState()
        self._current_community_id: str | None = None
        self._current_channel_id: str | None = None
        # Keyed-list wrappers for the three sortable id-keyed panels --
        # the community list, its parallel avatar strip, and the
        # channel list. Each holds its own O(1) id->item map plus the
        # insert-sorted / re-insert-on-reorder logic shared across all
        # three. The values are bound after the widgets are built; see
        # ``_build_chat_page``. The user-row map below stays as a dict
        # because those rows are custom QWidgets (status dot + avatar +
        # name) rather than plain text/icon items, which is outside
        # KeyedListWidget's render contract.
        self._community_kw: KeyedListWidget[Community] | None = None
        self._community_avatar_kw: KeyedListWidget[Community] | None = None
        self._channel_kw: KeyedListWidget[Channel] | None = None
        # The avatar label is tracked alongside the status dot so async
        # icon fetches can refresh the right row when they complete.
        self._user_row_entries_by_id: dict[str, tuple[QListWidgetItem, QLabel, QLabel]] = {}

        # Avatar / community-icon cache lives on a dedicated object so the
        # render hot-path can ask for a pixmap without any of the painters
        # or async fetch plumbing being mixed into ChatWindow. Callbacks
        # come back here purely to patch live widgets once bytes land.
        self._icons = IconCache(
            api,
            self._tasks,
            on_user_icon_ready=self._on_user_icon_ready,
            on_community_icon_ready=self._on_community_icon_ready,
        )

        # User profile + presence cache, plus dedupe for "just-in-time"
        # profile lookups triggered during message-row render. Mirrors
        # ``IconCache`` for the user side; ``ChatWindow`` only patches
        # the live widgets in ``_on_user_profile_loaded`` once a fetch
        # lands.
        self._users = UserDirectory(
            api,
            self._tasks,
            on_profile_loaded=self._on_user_profile_loaded,
        )

        # Link-preview thumbnails are stored by the server and streamed
        # back through the authenticated API; this cache keeps the
        # decoded ``QPixmap`` per ``imageId`` so a row's preview card
        # can paint its thumbnail synchronously on first render. The
        # text half of the preview (title/description/site name /
        # theme colour) lives on the ``Message`` record itself, so
        # there is no second cache to own for that side. Same shape as
        # ``IconCache`` / ``UserDirectory`` — synchronous getter,
        # dedupe-guarded background fetch, ready callback on the GUI
        # thread — and it's wiped alongside the rest of the
        # render-adjacent state in ``_reset_client_state``.
        self._link_preview_images = LinkPreviewImageCache(
            self._api,
            self._tasks,
            on_image_ready=self._on_link_preview_image_ready,
        )

        self.setWindowTitle("Aspen Chat Client")
        self.resize(1100, 700)
        self.setStyleSheet(
            f"QMainWindow, QWidget {{ background-color: {COLOR_BG_MAIN}; color: {COLOR_TEXT_MAIN}; }}"
        )

        self._stack = QStackedWidget(self)
        self.setCentralWidget(self._stack)

        self._login_page = self._build_login_page()
        self._chat_page = self._build_chat_page()
        self._stack.addWidget(self._login_page)
        self._stack.addWidget(self._chat_page)
        self._stack.setCurrentWidget(self._login_page)

        self._events.event_received.connect(self._handle_event)
        self._events.connection_lost.connect(self._on_event_stream_lost)
        self._events.connected.connect(self._on_event_stream_connected)
        # Important: ``state_resync_required`` is emitted *before*
        # ``connected`` for grace-window-exceeding reconnects, but Qt
        # delivers queued cross-thread signals in emission order, so the
        # resync slot will run first and stage the rebootstrap before
        # the "Connected" status update from ``_on_event_stream_connected``
        # overwrites the transient "refreshing" status.
        self._events.state_resync_required.connect(self._on_state_resync_required)

    def _build_login_page(self) -> LoginPage:
        page = LoginPage(self)
        page.login_requested.connect(self._login_clicked)
        page.create_user_requested.connect(self._create_user_clicked)
        return page

    def _build_chat_page(self) -> QWidget:
        root = QWidget(self)
        layout = QVBoxLayout(root)

        self._chat_splitter = CollapsibleSidebarSplitter(root)
        layout.addWidget(self._chat_splitter)

        self._community_panel = QWidget(self._chat_splitter)
        self._community_panel.setMinimumWidth(0)
        self._community_panel.setStyleSheet("background: transparent;")
        community_layout = QVBoxLayout(self._community_panel)
        community_layout.setContentsMargins(0, 0, 0, 0)
        community_layout.setSpacing(6)

        self._community_title_row = QHBoxLayout()
        self._community_title_row.setContentsMargins(8, 8, 8, 0)
        self._community_title_icon = QLabel(self._community_panel)
        self._community_title_icon.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._community_title_icon.setPixmap(
            material_icon("account-group-outline").pixmap(QSize(18, 18))
        )
        self._community_title_text = QLabel("Communities", self._community_panel)
        self._community_title_text.setStyleSheet(f"font-weight: 600; color: {COLOR_TEXT_MUTED};")
        self._community_create_button = QToolButton(self._community_panel)
        self._community_create_button.setAutoRaise(True)
        self._community_create_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._community_create_button.setIcon(material_icon("plus", color=COLOR_ACCENT))
        self._community_create_button.setIconSize(QSize(16, 16))
        self._community_create_button.setToolTip("Create community")
        self._community_create_button.clicked.connect(self._create_community_clicked)
        self._community_title_row.addWidget(self._community_title_icon)
        self._community_title_row.addWidget(self._community_title_text)
        self._community_title_row.addStretch(1)
        self._community_title_row.addWidget(self._community_create_button)
        self._community_title_row.setAlignment(Qt.AlignmentFlag.AlignLeft)
        community_layout.addLayout(self._community_title_row)

        self._community_list = QListWidget(self._community_panel)
        self._community_list.setIconSize(QSize(20, 20))
        self._community_list.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAlwaysOff
        )
        self._community_list.itemSelectionChanged.connect(self._community_changed)
        community_layout.addWidget(self._community_list, 1)
        self._community_kw = KeyedListWidget[Community](
            self._community_list,
            sort_key=lambda c: c.name.lower(),
            render=self._render_community_row,
        )

        self._community_avatar_list = QListWidget(self._community_panel)
        self._community_avatar_list.setViewMode(QListView.ViewMode.ListMode)
        self._community_avatar_list.setIconSize(QSize(28, 28))
        self._community_avatar_list.setSpacing(2)
        self._community_avatar_list.setFrameShape(QFrame.Shape.NoFrame)
        self._community_avatar_list.setItemDelegate(CommunityAvatarDelegate(28, self._community_avatar_list))
        self._community_avatar_list.itemSelectionChanged.connect(self._community_avatar_changed)
        self._community_avatar_list.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAlwaysOff
        )
        self._community_avatar_list.setVerticalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAsNeeded
        )
        self._community_avatar_list.setVerticalScrollMode(QListWidget.ScrollMode.ScrollPerPixel)
        self._community_avatar_list.setSelectionRectVisible(False)
        self._community_avatar_list.setContentsMargins(0, 0, 0, 0)
        self._community_avatar_list.setViewportMargins(0, 0, 0, 0)
        self._community_avatar_list.setStyleSheet(
            "QListWidget { background: transparent; border: none; padding: 0px; } "
            "QListWidget::item { margin: 0px; padding: 0px; } "
            "QListWidget::item:selected { background: transparent; border: none; }"
        )
        community_layout.addWidget(self._community_avatar_list, 1)
        self._community_avatar_kw = KeyedListWidget[Community](
            self._community_avatar_list,
            sort_key=lambda c: c.name.lower(),
            render=self._render_community_avatar_row,
        )

        self._channel_panel = QWidget(self._chat_splitter)
        self._channel_panel.setStyleSheet("background: transparent;")
        channel_layout = QVBoxLayout(self._channel_panel)
        channel_layout.setContentsMargins(0, 0, 0, 0)
        channel_layout.setSpacing(6)

        self._channel_title_row = QHBoxLayout()
        self._channel_title_row.setContentsMargins(8, 8, 8, 0)
        self._channel_title_icon = QLabel(self._channel_panel)
        self._channel_title_icon.setPixmap(
            material_icon("pound-box-outline").pixmap(QSize(18, 18))
        )
        self._channel_title_text = QLabel("Channels", self._channel_panel)
        self._channel_title_text.setStyleSheet(f"font-weight: 600; color: {COLOR_TEXT_MUTED};")
        self._channel_create_button = QToolButton(self._channel_panel)
        self._channel_create_button.setAutoRaise(True)
        self._channel_create_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._channel_create_button.setIcon(material_icon("plus", color=COLOR_ACCENT))
        self._channel_create_button.setIconSize(QSize(16, 16))
        self._channel_create_button.setToolTip("Create channel")
        self._channel_create_button.clicked.connect(self._create_channel_clicked)
        self._channel_title_row.addWidget(self._channel_title_icon)
        self._channel_title_row.addWidget(self._channel_title_text)
        self._channel_title_row.addStretch(1)
        self._channel_title_row.addWidget(self._channel_create_button)
        channel_layout.addLayout(self._channel_title_row)

        self._channel_list = QListWidget(self._channel_panel)
        self._channel_list.setFrameShape(QFrame.Shape.NoFrame)
        self._channel_list.itemSelectionChanged.connect(self._channel_changed)
        channel_layout.addWidget(self._channel_list, 1)
        self._channel_kw = KeyedListWidget[Channel](
            self._channel_list,
            sort_key=lambda c: c.sort_index,
            render=self._render_channel_row,
        )

        right_panel = QWidget(self._chat_splitter)
        right_layout = QVBoxLayout(right_panel)
        right_layout.setContentsMargins(0, 0, 0, 0)
        right_layout.setSpacing(6)

        self._chat_content_splitter = QSplitter(Qt.Orientation.Horizontal, right_panel)
        right_layout.addWidget(self._chat_content_splitter, 1)

        chat_content_panel = QWidget(self._chat_content_splitter)
        chat_content_layout = QVBoxLayout(chat_content_panel)
        chat_content_layout.setContentsMargins(0, 0, 0, 0)
        chat_content_layout.setSpacing(6)
        self._chat_title_row = QHBoxLayout()
        self._chat_title_row.setContentsMargins(8, 8, 8, 0)
        self._chat_title_icon = QLabel(chat_content_panel)
        self._chat_title_icon.setPixmap(material_icon("pound").pixmap(QSize(18, 18)))
        self._chat_title_text = QLabel("No channel selected", chat_content_panel)
        self._chat_title_text.setStyleSheet(f"font-weight: 600; color: {COLOR_TEXT_MUTED};")
        self._chat_title_row.addWidget(self._chat_title_icon)
        self._chat_title_row.addWidget(self._chat_title_text)
        self._chat_title_row.addStretch(1)
        chat_content_layout.addLayout(self._chat_title_row)
        self._message_pane = MessagePane(
            self._state,
            self._api,
            self._tasks,
            self._icons,
            self._link_preview_images,
            profile_resolver=self._resolve_author_profiles,
            header_formatter=self._format_message_header_html,
            avatar_pixmap=self._user_avatar_pixmap,
            status_setter=lambda text: self._status_label.setText(text),
            parent=chat_content_panel,
        )
        chat_content_layout.addWidget(self._message_pane, 1)

        composer_container = QWidget(chat_content_panel)
        composer_container.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; border: none; border-radius: 0px;"
        )
        composer_row = QHBoxLayout(composer_container)
        composer_row.setContentsMargins(6, 6, 6, 6)
        composer_row.setSpacing(6)
        self._composer = QTextEdit(composer_container)
        self._composer.setPlaceholderText("Write a message (Enter to send, Shift+Enter for newline)")
        self._composer.setFixedHeight(90)
        self._composer.setStyleSheet("background: transparent; border: none;")
        # Aspen messages are markdown plain text, not rich text. Keeping
        # the composer in plain-text mode means a pasted HTML fragment
        # (or anything from a rich-text source) lands as the literal
        # characters the user can see — so what they type and paste is
        # exactly what gets sent, with no hidden styling and no HTML
        # smuggled past the render-side ``html.escape`` in the message
        # pane. Combined with that escape, ``<b>`` typed or pasted into
        # the composer renders as the four characters ``<b>`` everywhere
        # downstream.
        self._composer.setAcceptRichText(False)
        # Intercept Enter (without Shift) so it sends the message instead of
        # inserting a newline. Shift+Enter keeps the default newline behavior.
        self._composer.installEventFilter(self)
        send_button = QToolButton(composer_container)
        send_button.setAutoRaise(True)
        send_button.setCursor(Qt.CursorShape.PointingHandCursor)
        send_button.setIcon(material_icon("send-circle", color=COLOR_ACCENT))
        send_button.setIconSize(QSize(30, 30))
        send_button.setFixedSize(QSize(42, 42))
        send_button.setToolTip("Send message")
        send_button.clicked.connect(self._send_clicked)
        composer_row.addWidget(self._composer, 1)
        composer_row.addWidget(send_button)
        chat_content_layout.addWidget(composer_container)

        self._users_preview_panel = QWidget(self._chat_content_splitter)
        self._users_preview_panel.setStyleSheet("background: transparent;")
        users_layout = QVBoxLayout(self._users_preview_panel)
        users_layout.setContentsMargins(0, 0, 0, 0)
        users_layout.setSpacing(6)
        self._users_title_row = QHBoxLayout()
        self._users_title_row.setContentsMargins(8, 8, 8, 0)
        self._users_title_icon = QLabel(self._users_preview_panel)
        self._users_title_icon.setPixmap(
            material_icon("account-multiple-outline").pixmap(QSize(18, 18))
        )
        self._users_title_text = QLabel("Users", self._users_preview_panel)
        self._users_title_text.setStyleSheet(f"font-weight: 600; color: {COLOR_TEXT_MUTED};")
        self._users_invite_button = QToolButton(self._users_preview_panel)
        self._users_invite_button.setAutoRaise(True)
        self._users_invite_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._users_invite_button.setIcon(material_icon("account-plus", color=COLOR_ACCENT))
        self._users_invite_button.setIconSize(QSize(16, 16))
        self._users_invite_button.setToolTip("Create invite")
        self._users_invite_button.clicked.connect(self._create_invite_clicked)
        self._users_title_row.addWidget(self._users_title_icon)
        self._users_title_row.addWidget(self._users_title_text)
        self._users_title_row.addStretch(1)
        self._users_title_row.addWidget(self._users_invite_button)
        users_layout.addLayout(self._users_title_row)
        self._users_preview_list = QListWidget(self._users_preview_panel)
        self._users_preview_list.setFrameShape(QFrame.Shape.NoFrame)
        self._users_preview_list.setIconSize(QSize(20, 20))
        self._users_preview_list.setStyleSheet(
            f"QListWidget {{ background-color: {COLOR_BG_MAIN}; border: none; color: {COLOR_TEXT_MAIN}; }} "
            "QListWidget::item { padding: 2px 6px; border-radius: 8px; }"
        )
        users_layout.addWidget(self._users_preview_list, 1)
        self._chat_content_splitter.setSizes([760, 190])

        self._chat_splitter.set_collapse_state_changed_callback(
            self._set_community_sidebar_collapsed
        )
        self._community_list.setStyleSheet(
            f"QListWidget {{ background-color: {COLOR_BG_MAIN}; border: none; color: {COLOR_TEXT_MAIN}; }} "
            "QListWidget::item { padding: 2px 6px; border-radius: 8px; } "
            f"QListWidget::item:selected {{ background-color: {COLOR_BG_MAIN}; border: 1px solid {COLOR_HIGHLIGHT}; color: {COLOR_TEXT_MAIN}; }}"
        )
        self._channel_list.setStyleSheet(
            f"QListWidget {{ background-color: {COLOR_BG_MAIN}; border: none; color: {COLOR_TEXT_MAIN}; }} "
            "QListWidget::item { padding: 2px 6px; border-radius: 8px; } "
            f"QListWidget::item:selected {{ background-color: {COLOR_HIGHLIGHT}; color: {COLOR_BG_PANE}; }}"
        )
        footer_row = QHBoxLayout()
        footer_row.setContentsMargins(8, 0, 8, 4)
        footer_row.setSpacing(6)
        self._status_label = QLabel("Not connected", root)
        self._status_label.setStyleSheet(f"font-size: 10px; color: {COLOR_TEXT_MUTED};")
        self._refresh_button = QToolButton(root)
        self._refresh_button.setAutoRaise(True)
        self._refresh_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._refresh_button.setIcon(material_icon("refresh", color=COLOR_ACCENT))
        self._refresh_button.setIconSize(QSize(14, 14))
        self._refresh_button.setFixedSize(QSize(20, 20))
        self._refresh_button.setToolTip("Refresh communities/channels")
        self._refresh_button.clicked.connect(self._refresh_clicked)
        footer_row.addWidget(self._status_label)
        footer_row.addWidget(self._refresh_button)
        footer_row.addStretch(1)
        layout.addLayout(footer_row)
        self._chat_splitter.setSizes([72, 320, 692])
        self._chat_splitter.initialize_toggle_icon()
        self._chat_splitter.toggle_community_sidebar()
        return root

    def _login_clicked(self, username: str, password: str) -> None:
        self._set_login_busy(True)
        self._tasks.run(
            self._api.login(username, password),
            on_success=lambda _session: self._on_login_succeeded(),
            on_failure=self._on_login_failed,
        )

    def _on_login_succeeded(self) -> None:
        self._events.start(self._tasks)
        self._stack.setCurrentWidget(self._chat_page)
        self._schedule_sidebar_layout_apply()
        self._status_label.setText("Connected")
        # Defer initial communities load one more hop so the chat page has
        # already been shown before the network round-trip starts. The
        # request itself yields control back to the event loop at every
        # await; only the follow-up UI update will run synchronously here.
        self._dispatch_initial_communities_load(preferred_community_id=None)

    def _dispatch_initial_communities_load(
        self, preferred_community_id: str | None
    ) -> None:
        self._tasks.run(
            self._api.read_user_communities(),
            on_success=lambda communities: self._on_initial_communities_loaded(
                communities, preferred_community_id
            ),
            on_failure=self._on_initial_communities_failed,
        )

    def _on_login_failed(self, exc: Exception) -> None:
        self._login_page.set_status(str(exc))
        self._set_login_busy(False)

    def _on_initial_communities_loaded(
        self,
        communities: list[Community],
        preferred_community_id: str | None = None,
    ) -> None:
        self._state.set_communities(communities)
        self._rebuild_community_list(preferred_community_id=preferred_community_id)
        if not communities:
            self._status_label.setText(
                "Connected. No communities yet — click Create Community to start chatting."
            )
        self._set_login_busy(False)

    def _on_initial_communities_failed(self, exc: Exception) -> None:
        self._status_label.setText(f"Failed to load communities: {exc}")
        self._set_login_busy(False)

    def _set_login_busy(self, busy: bool) -> None:
        self._login_page.set_busy(busy)

    def _create_user_clicked(self, username: str, password: str) -> None:
        self._set_login_busy(True)
        self._tasks.run(
            self._api.create_user(username, password),
            on_success=lambda _user_id: self._on_user_created(),
            on_failure=self._on_create_user_failed,
        )

    def _on_user_created(self) -> None:
        self._set_login_busy(False)
        self._login_page.set_status("User created. You can now log in.")

    def _on_create_user_failed(self, exc: Exception) -> None:
        self._login_page.set_status(f"Create user failed: {exc}")
        self._set_login_busy(False)

    def _community_changed(self) -> None:
        selected = self._community_list.selectedItems()
        if not selected:
            self._current_community_id = None
            self._users_preview_list.clear()
            return
        community_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(community_id, str):
            return
        self._current_community_id = community_id
        self._sync_community_avatar_selection(community_id)
        preferred_channel_id = self._current_channel_id
        self._tasks.run(
            self._api.read_community_channels(community_id),
            on_success=lambda channels: self._on_community_channels_loaded(
                community_id, channels, preferred_channel_id
            ),
            on_failure=lambda exc: self._status_label.setText(f"Failed to load channels: {exc}"),
        )
        self._refresh_users_preview(community_id)

    def _on_community_channels_loaded(
        self,
        community_id: str,
        channels: list[Channel],
        preferred_channel_id: str | None,
    ) -> None:
        # Drop stale responses when the user switched community while the
        # fetch was in flight. Without this the slow response would clobber
        # the newer community's channel list.
        if community_id != self._current_community_id:
            return
        self._state.set_channels(channels)
        self._rebuild_channel_list(
            community_id=community_id,
            preferred_channel_id=preferred_channel_id,
        )

    def _community_avatar_changed(self) -> None:
        selected = self._community_avatar_list.selectedItems()
        if not selected:
            return
        community_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(community_id, str):
            return
        self._select_community_by_id(community_id)

    def _channel_changed(self) -> None:
        selected = self._channel_list.selectedItems()
        if not selected:
            self._current_channel_id = None
            self._update_active_channel_header()
            self._message_pane.set_active_channel(None)
            return
        channel_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(channel_id, str):
            return
        self._current_channel_id = channel_id
        self._update_active_channel_header()
        self._message_pane.set_active_channel(channel_id)

    def _format_message_header_html(self, message: Message) -> str:
        """Small rich-text snippet shown above the message body.

        The header is a single short line (author + timestamp) so rich-text
        rendering cost is negligible. The body is kept in a separate
        ``QLabel`` with plain-text word wrap, which is dramatically cheaper
        than rich-text wrap for long messages (Qt's plain-text layout is
        linear in characters, while rich-text routes through QTextDocument
        and scales much worse).
        """
        ts = message.timestamp.astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")
        profile = self._users.get_profile(message.author)
        author = profile.name if profile is not None else message.author[:8]
        escaped_author = html.escape(author)
        escaped_ts = html.escape(ts)
        return (
            f"<span style='color:{COLOR_TEXT_MAIN}'><b>{escaped_author}</b></span> - "
            f"<i><span style='color:{COLOR_TEXT_MUTED}'>{escaped_ts}</span></i>"
        )

    def _send_clicked(self) -> None:
        channel_id = self._current_channel_id
        if channel_id is None:
            self._status_label.setText(
                "Pick a channel first (create a community if you do not have one yet)."
            )
            return
        # Snapshot the composer text and clear it immediately. Any further
        # keystrokes the user produces while the send is in flight will
        # land in the now-empty composer and form the next message — this
        # is the fix for the "characters skipped" feel that happens when
        # a blocking send freezes the GUI event loop.
        text = self._composer.toPlainText().strip()
        if not text:
            return
        self._composer.clear()
        self._tasks.run(
            self._api.send_message(channel_id, text),
            on_success=self._on_message_sent,
            on_failure=lambda exc: self._on_message_send_failed(text, exc),
        )

    def _on_message_sent(self, message: Message) -> None:
        self._message_pane.handle_message_sent(message)
        self._status_label.setText("Message sent")

    def _on_message_send_failed(self, original_text: str, exc: Exception) -> None:
        # If the composer is still empty (user hasn't started typing the
        # next message yet) we restore their draft so nothing is lost. If
        # they've already started a new draft we don't clobber it; the
        # error dialog plus the failed status line still surface the
        # failure, and the user can re-send from their history manually.
        if not self._composer.toPlainText().strip():
            self._composer.setPlainText(original_text)
        self._show_error(str(exc))

    def _handle_event(self, payload: dict) -> None:
        server_event = payload.get("serverEvent")

        # Presence updates never pass through ClientState and only affect a
        # single presence dot, so we short-circuit before the pydantic step.
        if server_event == "userStatus":
            self._apply_user_status_event(payload)
            user_id = payload.get("id")
            if isinstance(user_id, str):
                self._update_user_status_dot(user_id)
            return

        try:
            parsed = GeneratedServerEvent.model_validate(payload).root
        except Exception:
            return

        # ``messageLinkPreviewsReady`` is a standalone event (no
        # ``type`` discriminator) emitted by the server after a
        # message's asynchronous preview fetch lands. It carries the
        # complete preview list for the message, so we replace the
        # cached ``link_previews`` wholesale and hand off to the pane
        # to rebuild the row's preview cards.
        if server_event == "messageLinkPreviewsReady":
            self._handle_message_link_previews_ready(parsed)
            return

        changed = self._state.apply_server_event(parsed)
        if not changed:
            return

        event_type = str(getattr(parsed, "type", ""))
        record_id = str(getattr(parsed, "id", ""))
        if not record_id:
            return
        if server_event == "community":
            self._handle_community_event(record_id, event_type)
        elif server_event == "channel":
            self._handle_channel_event(record_id, event_type)
        elif server_event == "message":
            self._handle_message_event(record_id, event_type)

    def _handle_community_event(self, community_id: str, event_type: str) -> None:
        assert self._community_kw is not None
        assert self._community_avatar_kw is not None
        if event_type == "delete":
            self._community_kw.remove(community_id)
            self._community_avatar_kw.remove(community_id)
            return
        community = self._state.communities.get(community_id)
        if community is None:
            return
        was_empty = len(self._community_kw) == 0
        self._community_kw.upsert(community_id, community)
        self._community_avatar_kw.upsert(community_id, community)
        # Auto-select the first community so ``_community_changed`` loads
        # its channels and users without requiring a manual click. This
        # must fire ``itemSelectionChanged``, so we use the raw widget
        # call rather than the signal-blocked ``KeyedListWidget.select``.
        if was_empty and len(self._community_kw) == 1:
            item = self._community_kw.item_for(community_id)
            if item is not None:
                self._community_list.setCurrentItem(item)
        self._fit_community_sidebar_width_to_content()

    def _handle_channel_event(self, channel_id: str, event_type: str) -> None:
        assert self._channel_kw is not None
        if event_type == "delete":
            self._channel_kw.remove(channel_id)
            return
        channel = self._state.channels.get(channel_id)
        if channel is None:
            return
        # Channels outside the currently-displayed community have no row;
        # state was already updated so there's nothing to do in the UI.
        if channel.community != self._current_community_id:
            self._channel_kw.remove(channel_id)
            return
        was_empty = len(self._channel_kw) == 0
        self._channel_kw.upsert(channel_id, channel)
        if was_empty and len(self._channel_kw) == 1:
            item = self._channel_kw.item_for(channel_id)
            if item is not None:
                self._channel_list.setCurrentItem(item)

    def _handle_message_link_previews_ready(self, parsed: Any) -> None:
        """Apply a landed ``messageLinkPreviewsReady`` event to state + UI.

        The event is the server's way of telling us "the async preview
        fetch I kicked off for this message has finished; here's the
        authoritative list". We overwrite the cached message's
        ``link_previews`` (the server sends the full list even if the
        final count is zero, so re-edits that clear the previews land
        correctly) and, if the affected row is currently materialised,
        tell the pane to rebuild its preview cards.
        """
        message_id = str(getattr(parsed, "messageId", ""))
        channel_id = str(getattr(parsed, "channelId", ""))
        if not message_id or not channel_id:
            return
        raw_previews = getattr(parsed, "previews", None)
        if raw_previews is None:
            return
        previews = [
            LinkPreview.model_validate(p.model_dump(mode="json"))
            for p in raw_previews
        ]
        if not self._state.apply_link_previews_ready(message_id, previews):
            return
        if channel_id != self._current_channel_id:
            return
        self._message_pane.handle_link_previews_ready(message_id)

    def _handle_message_event(self, message_id: str, event_type: str) -> None:
        if event_type == "delete":
            self._message_pane.remove_message_row(message_id)
            return
        message = self._state.messages.get(message_id)
        if message is None:
            return
        # Only the channel we are currently viewing has materialized rows; all
        # other channels stay as in-memory state until the user navigates to
        # them and the message pane's _render_cached_window populates the
        # widget.
        if message.channel_id != self._current_channel_id:
            return
        self._message_pane.handle_message_event(message, event_type)

    def eventFilter(self, watched: QObject, event: QEvent) -> bool:  # type: ignore[override]
        if watched is self._composer and event.type() == QEvent.Type.KeyPress:
            key = event.key()
            if key in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
                # Shift+Enter inserts a newline as usual; bare Enter sends.
                # Ignore keypad modifier (Key_Enter comes from the numpad and
                # carries KeypadModifier on some platforms).
                if not (event.modifiers() & Qt.KeyboardModifier.ShiftModifier):
                    self._send_clicked()
                    return True
        return super().eventFilter(watched, event)

    def _render_community_row(
        self, item: QListWidgetItem, community: Community
    ) -> None:
        """``KeyedListWidget`` render hook for the text-and-icon community list."""
        item.setText(community.name)
        item.setIcon(self._community_avatar_icon(community))

    def _render_community_avatar_row(
        self, item: QListWidgetItem, community: Community
    ) -> None:
        """``KeyedListWidget`` render hook for the parallel avatar strip.

        ``setSizeHint`` and ``setTextAlignment`` are idempotent and cheap;
        re-applying them on every render keeps this single source of
        truth for the strip's row geometry.
        """
        item.setIcon(self._community_avatar_icon(community))
        item.setText("")
        item.setToolTip(community.name)
        item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
        item.setSizeHint(QSize(28, 36))

    def _render_channel_row(self, item: QListWidgetItem, channel: Channel) -> None:
        """``KeyedListWidget`` render hook for the channel list."""
        item.setText(f"#{channel.name}")

    def _update_user_status_dot(self, user_id: str) -> None:
        entry = self._user_row_entries_by_id.get(user_id)
        if entry is None:
            return
        _item, dot_label, _avatar = entry
        dot_label.setPixmap(
            IconCache.presence_dot_pixmap(self._users.get_status(user_id), size=8)
        )

    def _on_event_stream_lost(self, reason: str) -> None:
        # Single status update at the start of an outage; the reconnect
        # loop in EventStreamClient will keep retrying behind the scenes
        # and emit ``connected`` (and possibly ``state_resync_required``)
        # when it succeeds. We surface ``reason`` as a tooltip rather
        # than a status string so the visible label stays compact.
        self._status_label.setText("Disconnected — reconnecting…")
        self._status_label.setToolTip(reason)

    def _on_event_stream_connected(self) -> None:
        # Clear any reconnect tooltip from a prior outage. We only set
        # the visible text when we were actually disconnected so this
        # doesn't stomp on user-action statuses (e.g. "Message sent")
        # during the steady-state lifetime of the connection: the
        # "Disconnected" text is the only one we know we own here.
        self._status_label.setToolTip("")
        if self._status_label.text().startswith("Disconnected"):
            self._status_label.setText("Connected")

    def _on_state_resync_required(self) -> None:
        # The 45s grace window was exceeded, so the server's NATS replay
        # buffer can't be trusted to have covered the entire gap. Wipe
        # the in-memory caches and re-run the post-login bootstrap; the
        # rebuild will reselect whichever community/channel was active
        # before the outage. The previous WS connect has just succeeded,
        # so any events that arrive during the rebootstrap will simply
        # miss in the freshly-emptied lookup maps and bail out until the
        # rebuild repopulates them.
        self._status_label.setText("Reconnected — refreshing state…")
        # Capture the active selection before the wipe so the rebootstrap
        # can land us back in the same community/channel; the channel id
        # is consumed inside ``_community_changed`` (which fires off the
        # channel list load) via ``self._current_channel_id``, which we
        # leave untouched in ``_reset_client_state`` for exactly this
        # reason.
        preferred_community_id = self._current_community_id
        self._reset_client_state()
        self._dispatch_initial_communities_load(
            preferred_community_id=preferred_community_id
        )

    def _reset_client_state(self) -> None:
        """Drop every in-memory cache derived from the server's state.

        Mirrors the field initialisation in ``__init__`` for everything
        that's purely a cache of server state. We deliberately do *not*
        touch the visible widgets here -- the user picked the "minimal"
        UX, so the existing UI stays on screen until the rebootstrap
        replaces it pane-by-pane via the existing ``_rebuild_*`` paths.

        ``_current_community_id`` and ``_current_channel_id`` are
        preserved so ``_rebuild_community_list`` (and the channel-load
        chain it triggers via ``_community_changed``) can reselect the
        same room the user was last in.
        """
        self._state = ClientState()
        self._users.clear()
        self._icons.clear()
        self._link_preview_images.clear()
        if self._community_kw is not None:
            self._community_kw.clear()
        if self._community_avatar_kw is not None:
            self._community_avatar_kw.clear()
        if self._channel_kw is not None:
            self._channel_kw.clear()
        self._user_row_entries_by_id.clear()
        self._message_pane.clear_for_resync()

    def _show_error(self, text: str) -> None:
        QMessageBox.critical(self, "Error", text)

    def _create_channel_clicked(self) -> None:
        community_id = self._selected_item_user_role(self._community_list)
        if community_id is None:
            self._status_label.setText("Select a community first.")
            return
        name, ok = QInputDialog.getText(
            self,
            "Create Channel",
            "Channel name:",
        )
        if not ok:
            return
        channel_name = name.strip()
        if not channel_name:
            self._status_label.setText("Channel name cannot be empty.")
            return
        sort_index = len(self._state.get_channels_for_community(community_id))
        self._tasks.run(
            self._api.create_channel(
                community_id=community_id,
                name=channel_name,
                sort_index=sort_index,
            ),
            on_success=lambda channel: self._on_channel_created(community_id, channel),
            on_failure=lambda exc: self._show_error(str(exc)),
        )

    def _on_channel_created(self, community_id: str, channel: Channel) -> None:
        self._state.upsert_channel(channel)
        self._rebuild_channel_list(
            community_id=community_id,
            preferred_channel_id=channel.id,
        )
        self._status_label.setText(f"Created channel #{channel.name}.")

    def _refresh_clicked(self) -> None:
        selected_community_id = self._selected_item_user_role(self._community_list)
        selected_channel_id = self._selected_item_user_role(self._channel_list)
        self._tasks.run(
            self._api.read_user_communities(),
            on_success=lambda communities: self._on_refresh_communities_loaded(
                communities, selected_community_id, selected_channel_id
            ),
            on_failure=lambda exc: self._show_error(str(exc)),
        )

    def _on_refresh_communities_loaded(
        self,
        communities: list[Community],
        preferred_community_id: str | None,
        preferred_channel_id: str | None,
    ) -> None:
        self._state.set_communities(communities)
        self._rebuild_community_list(preferred_community_id=preferred_community_id)
        final_community_id = self._selected_item_user_role(self._community_list)
        if final_community_id is None:
            self._status_label.setText("Refreshed communities.")
            return
        self._tasks.run(
            self._api.read_community_channels(final_community_id),
            on_success=lambda channels: self._on_refresh_channels_loaded(
                final_community_id, channels, preferred_channel_id
            ),
            on_failure=lambda exc: self._show_error(str(exc)),
        )

    def _on_refresh_channels_loaded(
        self,
        community_id: str,
        channels: list[Channel],
        preferred_channel_id: str | None,
    ) -> None:
        self._state.set_channels(channels)
        self._rebuild_channel_list(
            community_id=community_id,
            preferred_channel_id=preferred_channel_id,
        )
        self._status_label.setText("Refreshed communities/channels.")

    def _create_community_clicked(self) -> None:
        name, ok = QInputDialog.getText(
            self,
            "Create Community",
            "Community name:",
        )
        if not ok:
            return
        community_name = name.strip()
        if not community_name:
            self._status_label.setText("Community name cannot be empty.")
            return
        self._tasks.run(
            self._api.create_community(community_name),
            on_success=self._on_community_created,
            on_failure=lambda exc: self._show_error(str(exc)),
        )

    def _on_community_created(self, community: Community) -> None:
        self._state.upsert_community(community)
        self._rebuild_community_list(preferred_community_id=community.id)
        self._status_label.setText(f"Created community '{community.name}'.")

    def _rebuild_community_list(self, preferred_community_id: str | None) -> None:
        assert self._community_kw is not None
        assert self._community_avatar_kw is not None
        communities = self._state.get_communities_sorted()
        self._community_kw.replace_all(
            communities,
            key_fn=lambda c: c.id,
            preferred=preferred_community_id,
        )
        self._community_avatar_kw.replace_all(
            communities,
            key_fn=lambda c: c.id,
            preferred=preferred_community_id,
        )
        self._fit_community_sidebar_width_to_content()
        # Slot fired explicitly because ``replace_all`` blocks signals
        # during the rebuild (so a programmatic selection during the
        # tear-down doesn't pop a spurious ``itemSelectionChanged``).
        self._community_changed()

    def _rebuild_channel_list(self, community_id: str, preferred_channel_id: str | None) -> None:
        assert self._channel_kw is not None
        selected = self._channel_kw.replace_all(
            self._state.get_channels_for_community(community_id),
            key_fn=lambda c: c.id,
            preferred=preferred_channel_id,
        )
        self._current_channel_id = selected
        self._apply_collapsed_sidebar_ratio()
        self._apply_expanded_sidebar_layout()
        # Same explicit-fire reason as ``_rebuild_community_list``.
        self._channel_changed()
        self._schedule_sidebar_layout_apply()

    def _resolve_author_profiles(self, messages: list[Message]) -> None:
        """Kick off async profile loads for authors we don't know yet.

        Returns immediately. Rows are rendered with a short-id fallback in
        the header and a fallback avatar; once each profile load completes
        on the GUI thread (see ``_on_user_profile_loaded``) the affected
        rows are patched in place.
        """
        self._users.request_profiles(message.author for message in messages)

    def _on_user_profile_loaded(self, user_id: str) -> None:
        # The avatar cache was keyed on the fallback pixmap; throw it out
        # so _user_avatar_pixmap re-renders with the real name / kicks off
        # an icon fetch for the profile's icon id.
        self._icons.invalidate_user(user_id)
        self._message_pane.refresh_author_row(user_id)

    def _on_link_preview_image_ready(self, image_id: str) -> None:
        """Fan a landed preview-thumbnail fetch out to the message pane.

        :class:`LinkPreviewImageCache` calls this on the GUI thread —
        its fetch completion is scheduled via ``TaskSpawner.run`` so
        callbacks run where widgets live. We just forward to
        ``MessagePane``, which holds the subscriber map keyed by
        ``imageId`` and knows how to repaint the affected rows.
        """
        self._message_pane.handle_preview_image_ready(image_id)

    @staticmethod
    def _selected_item_user_role(list_widget: QListWidget) -> str | None:
        selected = list_widget.selectedItems()
        if not selected:
            return None
        value = selected[0].data(Qt.ItemDataRole.UserRole)
        return value if isinstance(value, str) else None

    def _fit_community_sidebar_width_to_content(self) -> None:
        assert self._community_kw is not None
        item_texts = self._community_kw.texts()
        metrics = self._community_list.fontMetrics()
        longest_name_width = max(
            (metrics.horizontalAdvance(name) for name in item_texts),
            default=metrics.horizontalAdvance("Communities"),
        )
        # Icon + text + list paddings for the widest visible row.
        row_width = self._community_list.iconSize().width() + longest_name_width + 44
        # Header includes icon, "Communities" text, plus-button, and row spacing/margins.
        header_width = (
            self._community_title_icon.pixmap().width()  # type: ignore[union-attr]
            + metrics.horizontalAdvance("Communities")
            + self._community_create_button.iconSize().width()
            + 40
        )
        target_width = min(max(row_width, header_width, 72), 220)
        self._chat_splitter.set_expanded_community_width(target_width)
        self._apply_collapsed_sidebar_ratio()
        self._apply_expanded_sidebar_layout()

    def _fit_channel_sidebar_width_to_content(self) -> int:
        channel_texts = [
            self._channel_list.item(index).text()
            for index in range(self._channel_list.count())
            if self._channel_list.item(index) is not None
        ]
        metrics = self._channel_list.fontMetrics()
        longest_channel_width = max(
            (metrics.horizontalAdvance(name) for name in channel_texts),
            default=metrics.horizontalAdvance("#general"),
        )
        row_width = longest_channel_width + 24
        header_width = (
            self._channel_title_icon.pixmap().width()  # type: ignore[union-attr]
            + metrics.horizontalAdvance("Channels")
            + self._channel_create_button.iconSize().width()
            + 40
        )
        return min(max(row_width, header_width, 120), 240)

    def _apply_expanded_sidebar_layout(self) -> None:
        sizes = self._chat_splitter.sizes()
        if len(sizes) < 3:
            return
        if self._chat_splitter._community_collapsed:
            return

        total_width = sum(sizes)
        if total_width <= 0:
            return

        community_target = min(self._chat_splitter._expanded_community_width, 220)
        channel_target = self._fit_channel_sidebar_width_to_content()
        combined_target = community_target + channel_target

        # Keep message panel as the visual priority area in expanded mode.
        max_combined = max(int(total_width * 0.45), 260)
        combined_target = min(combined_target, max_combined)
        if combined_target < community_target + 100:
            channel_target = max(combined_target - community_target, 100)

        right_width = max(total_width - community_target - channel_target, 180)
        self._chat_splitter.setSizes([community_target, channel_target, right_width])

    def _set_community_sidebar_collapsed(self, collapsed: bool) -> None:
        self._community_title_text.setVisible(not collapsed)
        self._community_create_button.setVisible(not collapsed)
        self._community_list.setVisible(not collapsed)
        self._community_avatar_list.setVisible(collapsed)
        self._community_title_row.setAlignment(
            Qt.AlignmentFlag.AlignCenter if collapsed else Qt.AlignmentFlag.AlignLeft
        )
        if collapsed:
            self._community_title_row.setContentsMargins(0, 8, 0, 0)
            self._community_title_row.setSpacing(0)
            collapsed_width = self._chat_splitter._collapsed_community_width
            self._community_panel.setMinimumWidth(collapsed_width)
            self._community_panel.setMaximumWidth(collapsed_width)
            self._community_title_icon.setMinimumWidth(collapsed_width)
            self._community_title_icon.setMaximumWidth(collapsed_width)
            self._community_title_icon.setAlignment(Qt.AlignmentFlag.AlignCenter)
            self._apply_collapsed_sidebar_ratio()
        else:
            self._community_title_row.setContentsMargins(8, 8, 8, 0)
            self._community_title_row.setSpacing(6)
            self._community_panel.setMinimumWidth(0)
            self._community_panel.setMaximumWidth(16777215)
            self._community_title_icon.setMinimumWidth(0)
            self._community_title_icon.setMaximumWidth(16777215)
            self._community_title_icon.setAlignment(Qt.AlignmentFlag.AlignCenter)
            self._apply_expanded_sidebar_layout()

    def _apply_collapsed_sidebar_ratio(self) -> None:
        sizes = self._chat_splitter.sizes()
        if len(sizes) < 3:
            return
        if not self._chat_splitter._community_collapsed:
            return
        collapsed_width = self._chat_splitter._collapsed_community_width

        total_width = sum(sizes)
        if total_width <= 0:
            return

        # Keep collapsed-left area compact on both small and large displays.
        combined_target = int(total_width * 0.22)
        combined_target = max(combined_target, collapsed_width + 90)
        combined_target = min(combined_target, 260, total_width - 140)
        channels_width = max(combined_target - collapsed_width, 72)
        right_width = max(total_width - collapsed_width - channels_width, 120)
        self._chat_splitter.setSizes([collapsed_width, channels_width, right_width])

    def _update_active_channel_header(self) -> None:
        if self._current_channel_id is None:
            self._chat_title_text.setText("No channel selected")
            return
        channel = self._state.channels.get(self._current_channel_id)
        if channel is None:
            self._chat_title_text.setText("Channel")
            return
        self._chat_title_text.setText(f"#{channel.name}")

    def _schedule_sidebar_layout_apply(self) -> None:
        QTimer.singleShot(0, self._apply_collapsed_sidebar_ratio)
        QTimer.singleShot(0, self._apply_expanded_sidebar_layout)

    def _apply_user_status_event(self, event_payload: dict) -> None:
        user_id = event_payload.get("id")
        if user_id is None:
            return
        status = str(event_payload.get("status", "")).strip().lower()
        if status not in {"online", "offline", "away"}:
            return
        self._users.set_status(str(user_id), status)

    def _create_invite_clicked(self) -> None:
        community_id = self._current_community_id
        if community_id is None:
            self._status_label.setText("Select a community first.")
            return
        self._tasks.run(
            self._api.create_invite(community_id),
            on_success=self._on_invite_created,
            on_failure=lambda exc: self._show_error(str(exc)),
        )

    def _on_invite_created(self, code: str) -> None:
        self._status_label.setText(f"Invite created: {code}")
        QMessageBox.information(self, "Invite Created", f"Invite code: {code}")

    def _refresh_users_preview(self, community_id: str) -> None:
        self._users_preview_list.clear()
        self._user_row_entries_by_id.clear()
        self._tasks.run(
            self._api.read_community_users(community_id),
            on_success=lambda users: self._on_community_users_loaded(community_id, users),
            on_failure=lambda exc: self._status_label.setText(f"Failed to load users: {exc}"),
        )

    def _on_community_users_loaded(
        self, community_id: str, users: list[UserProfile]
    ) -> None:
        # Drop stale responses when the user switched community before the
        # fetch returned.
        if community_id != self._current_community_id:
            return
        self._users_preview_list.clear()
        self._user_row_entries_by_id.clear()
        for user in users:
            self._users.upsert_profile(user)
            item = QListWidgetItem()
            row = QWidget(self._users_preview_list)
            row_layout = QHBoxLayout(row)
            row_layout.setContentsMargins(6, 2, 6, 2)
            row_layout.setSpacing(6)

            status_dot = QLabel(row)
            status_dot.setFixedSize(8, 8)
            status_dot.setPixmap(
                IconCache.presence_dot_pixmap(self._users.get_status(user.id), size=8)
            )
            row_layout.addWidget(status_dot, 0, alignment=Qt.AlignmentFlag.AlignVCenter)

            avatar = QLabel(row)
            avatar.setFixedSize(20, 20)
            avatar.setPixmap(self._user_avatar_pixmap(user.id, size=20))
            row_layout.addWidget(avatar, 0, alignment=Qt.AlignmentFlag.AlignVCenter)

            name_label = QLabel(user.name, row)
            row_layout.addWidget(name_label, 1, alignment=Qt.AlignmentFlag.AlignVCenter)

            item.setSizeHint(row.sizeHint())
            self._users_preview_list.addItem(item)
            self._users_preview_list.setItemWidget(item, row)
            self._user_row_entries_by_id[user.id] = (item, status_dot, avatar)

    def _sync_community_avatar_selection(self, community_id: str) -> None:
        if self._community_avatar_kw is None:
            return
        # Signal-blocked: this fires whenever the text-list selection
        # changes, so we just mirror the row over without re-triggering
        # the avatar list's own ``itemSelectionChanged`` slot (which
        # would push the change right back).
        self._community_avatar_kw.select(community_id)

    def _select_community_by_id(self, community_id: str) -> None:
        # Signal-firing: this is the user clicking the avatar strip,
        # so the text list's selection-changed slot must run to load
        # the chosen community's channels.
        if self._community_kw is None:
            return
        item = self._community_kw.item_for(community_id)
        if item is None or self._community_list.currentItem() is item:
            return
        self._community_list.setCurrentItem(item)

    def _user_avatar_pixmap(self, user_id: str, size: int) -> QPixmap:
        """Render-hot-path shim that supplies the cached profile to ``IconCache``."""
        return self._icons.user_avatar_pixmap(
            user_id,
            size,
            self._users.get_profile(user_id),
        )

    def _community_avatar_icon(self, community: Community) -> QIcon:
        return self._icons.community_avatar_icon(community)

    def _on_user_icon_ready(self, user_id: str) -> None:
        """Patch live widgets that show ``user_id``'s avatar after a fetch lands."""
        # The message pane owns its own row map and re-renders message
        # avatars/headers in one shot; ChatWindow only patches the
        # users-preview row that it owns directly.
        self._message_pane.refresh_author_row(user_id)
        entry = self._user_row_entries_by_id.get(user_id)
        if entry is not None:
            _item, _dot_label, avatar_label = entry
            avatar_label.setPixmap(
                self._icons.user_avatar_pixmap(
                    user_id, 20, self._users.get_profile(user_id)
                )
            )

    def _on_community_icon_ready(self, community_id: str, icon_id: str) -> None:
        """Patch list rows once the community's real icon bytes have landed."""
        # Only update rows whose community still points at this icon id —
        # the icon could have been reassigned in between dispatch and
        # completion.
        community = self._state.communities.get(community_id)
        if community is None or community.icon != icon_id:
            return
        icon = self._icons.community_avatar_icon(community)
        if self._community_kw is not None:
            item = self._community_kw.item_for(community_id)
            if item is not None:
                item.setIcon(icon)
        if self._community_avatar_kw is not None:
            avatar_item = self._community_avatar_kw.item_for(community_id)
            if avatar_item is not None:
                avatar_item.setIcon(icon)

    async def _async_shutdown(self) -> None:
        """Cancel in-flight tasks and close the HTTP client, then close the window.

        Runs on the asyncio loop ``qasync`` integrates with Qt.
        Order matters and mirrors the original synchronous shutdown:
        stop the WebSocket reader first (sync, returns immediately) so
        it can't emit further GUI-thread events, then await
        ``TaskSpawner.shutdown`` so any in-flight HTTP coroutines are
        cancelled and drained before we close the underlying client,
        then await ``aclose`` so transport sockets aren't leaked.
        Finally, flip ``_shutdown_complete``, signal ``main()`` that it
        may return from ``run_until_complete``, and re-trigger
        ``close``; the second pass through ``closeEvent`` then accepts
        the close.

        The order of the last three steps matters. We must set
        ``_app_close_event`` *before* ``self.close()`` -- not after --
        because ``self.close()`` triggers the second ``closeEvent``
        which accepts the close, which causes Qt to begin tearing down
        the QApplication (since ``quitOnLastWindowClosed`` is True by
        default), which causes qasync to mark its loop as no longer
        running. Any ``Event.set()`` scheduled after that point is
        invoked on a stopped loop and raises ``RuntimeError: loop ...
        is not the running loop`` from the internal ``Task.task_wakeup``
        callback, which in turn leaves the awaited future incomplete
        and crashes ``run_until_complete`` with ``"Event loop stopped
        before Future completed."``. Setting the event first guarantees
        the wake-up is scheduled while the loop is unambiguously alive
        and the future completes cleanly; by the time Qt actually
        tears down, ``run_until_complete`` has already returned.
        """
        try:
            await self._tasks.shutdown()
            await self._api.aclose()
        finally:
            self._shutdown_complete = True
            self._app_close_event.set()
            self.close()

    def closeEvent(self, event) -> None:  # type: ignore[override]
        # The async cleanup has already finished. ``_async_shutdown``
        # has already set ``_app_close_event`` (so ``main()``'s
        # ``run_until_complete`` is ready to return as soon as the loop
        # next yields), so we just let Qt actually close the window; the
        # loop will observe the completed future on the next tick and
        # exit ``run_until_complete`` cleanly.
        if self._shutdown_complete:
            super().closeEvent(event)
            return
        # First close attempt: defer the actual teardown until the
        # asyncio cleanup finishes. Ignoring the event keeps the window
        # alive while the cleanup coroutine runs; ``_async_shutdown``
        # calls ``self.close()`` again once it's done, which lands in
        # the branch above. Subsequent close attempts (e.g. user
        # mashing the X button) hit ``_shutdown_started`` and skip the
        # spawn rather than queueing a duplicate.
        event.ignore()
        if self._shutdown_started:
            return
        self._shutdown_started = True
        # Stop the WebSocket reader synchronously. ``stop`` only flips a
        # flag and shuts the underlying socket down for read, so it
        # returns immediately and is safe to call from this slot.
        self._events.stop()
        # Schedule the cleanup directly (not via ``self._tasks``);
        # routing it through the spawner would cause
        # ``TaskSpawner.shutdown`` to cancel the very coroutine driving
        # the cleanup.
        asyncio.ensure_future(self._async_shutdown())

    def resizeEvent(self, event) -> None:  # type: ignore[override]
        super().resizeEvent(event)
        self._apply_collapsed_sidebar_ratio()
        self._apply_expanded_sidebar_layout()
        # MessagePane runs its own resizeEvent in response to the splitter
        # / sidebar collapse repositioning it, so we don't need to forward
        # the relayout from here as well.
