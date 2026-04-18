from __future__ import annotations

import html
import hashlib
from typing import Callable

from PySide6.QtCore import QEvent, QObject, QTimer, QSize, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPainterPath, QPen, QPixmap
from PySide6.QtWidgets import (
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QListView,
    QMainWindow,
    QMessageBox,
    QPushButton,
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

from aspen_client.api_client import AspenApiClient, AsyncApiCaller
from aspen_client.event_client import EventStreamClient
from aspen_client.generated.event_models import ServerEvent as GeneratedServerEvent
from aspen_client.icons import apply_button_icon, material_icon
from aspen_client.state import MESSAGE_WINDOW_CAP, ClientState
from aspen_client.types import Channel, Community, Message, UserProfile

# Tunable limits for the bidirectional message window. INITIAL_MESSAGE_LOAD
# fills the viewport with one round-trip on channel switch; MESSAGE_PAGE_SIZE
# is used for subsequent older/newer loads. ``MESSAGE_WINDOW_CAP`` is imported
# from the state module so both paged reads and live-event appends evict to
# the same ceiling. ``SCROLL_EDGE_PIXELS`` is how close to the top/bottom of
# the viewport the user must scroll before we trigger the next page.
INITIAL_MESSAGE_LOAD = 50
MESSAGE_PAGE_SIZE = 50
SCROLL_EDGE_PIXELS = 200

COLOR_BG_MAIN = "#1E2A18"
COLOR_BG_PANE = "#141C10"
COLOR_ACCENT = "#8EBA54"
COLOR_AWAY = "#D7B25C"
COLOR_HIGHLIGHT = "#FDF4E3"
COLOR_TEXT_MAIN = "#EDE4D0"
COLOR_TEXT_MUTED = "#9A9080"


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
    def __init__(self, api: AspenApiClient, events: EventStreamClient) -> None:
        super().__init__()
        self._api = api
        self._events = events
        # Guard so ``shutdown()`` is safe to call from both
        # ``closeEvent`` and ``QApplication.aboutToQuit`` without
        # double-tearing-down the worker pool or the HTTP client.
        self._shutting_down = False

        # The single worker every API call in this window flows through.
        # Constructed on the GUI thread, so its queued signals deliver
        # callbacks back here. Direct calls to ``self._api`` from GUI
        # handlers are forbidden (see client/AGENTS.md).
        self._async_api = AsyncApiCaller(self._api, self)

        self._state = ClientState()
        self._current_community_id: str | None = None
        self._current_channel_id: str | None = None
        self._user_profiles_by_id: dict[str, UserProfile] = {}
        self._user_online_status_by_id: dict[str, str] = {}
        self._community_icon_cache: dict[str, QIcon] = {}
        self._user_avatar_cache: dict[str, QPixmap] = {}
        # Row-index maps keep O(1) lookup for incremental event updates so we
        # don't have to rebuild an entire list to touch a single record.
        self._message_items_by_id: dict[str, QListWidgetItem] = {}
        self._community_items_by_id: dict[str, QListWidgetItem] = {}
        self._community_avatar_items_by_id: dict[str, QListWidgetItem] = {}
        self._channel_items_by_id: dict[str, QListWidgetItem] = {}
        # The avatar label is tracked alongside the status dot so async
        # icon fetches can refresh the right row when they complete.
        self._user_row_entries_by_id: dict[str, tuple[QListWidgetItem, QLabel, QLabel]] = {}

        # Loading sentinel rows rendered at the top/bottom of the message list
        # while a page fetch is in flight in that direction. Kept as separate
        # handles so incremental inserts can skip them when computing index.
        self._loading_header_item: QListWidgetItem | None = None
        self._loading_footer_item: QListWidgetItem | None = None
        # Set of in-flight fetch directions keyed by channel id. Used to
        # avoid piling up concurrent identical fetches when the scroll bar
        # dwells near an edge.
        self._pending_fetches: dict[str, set[str]] = {}
        # Dedupe sets for the "just-in-time" lookups triggered during
        # render (author profiles and icon blobs). The first paint for a
        # given id kicks off exactly one background fetch; subsequent
        # paints see the id here and wait for the result.
        self._pending_user_profile_fetches: set[str] = set()
        self._pending_user_icon_fetches: set[str] = set()
        self._pending_community_icon_fetches: set[str] = set()

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
        self._events.connection_error.connect(self._on_event_error)

    def _build_login_page(self) -> QWidget:
        root = QWidget(self)
        layout = QVBoxLayout(root)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)

        title = QLabel("Aspen Login", root)
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        title.setStyleSheet("font-size: 24px; font-weight: 600;")

        self._username_input = QLineEdit(root)
        self._username_input.setPlaceholderText("Username")
        self._username_input.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; color: {COLOR_TEXT_MAIN}; border: 1px solid {COLOR_TEXT_MUTED};"
        )

        self._password_input = QLineEdit(root)
        self._password_input.setPlaceholderText("Password")
        self._password_input.setEchoMode(QLineEdit.EchoMode.Password)
        self._password_input.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; color: {COLOR_TEXT_MAIN}; border: 1px solid {COLOR_TEXT_MUTED};"
        )

        self._login_button = QPushButton("Login", root)
        self._login_button.clicked.connect(self._login_clicked)
        self._create_user_button = QPushButton("Create User", root)
        self._create_user_button.clicked.connect(self._create_user_clicked)
        apply_button_icon(self._login_button, "login-variant")
        apply_button_icon(self._create_user_button, "account-plus-outline")

        self._login_status_label = QLabel("", root)
        self._login_status_label.setAlignment(Qt.AlignmentFlag.AlignCenter)

        box = QWidget(root)
        box_layout = QVBoxLayout(box)
        box_layout.addWidget(title)
        box_layout.addWidget(self._username_input)
        box_layout.addWidget(self._password_input)
        login_actions = QVBoxLayout()
        login_actions.setAlignment(Qt.AlignmentFlag.AlignHCenter)
        login_actions.addWidget(self._login_button, alignment=Qt.AlignmentFlag.AlignHCenter)
        login_actions.addWidget(self._create_user_button, alignment=Qt.AlignmentFlag.AlignHCenter)
        box_layout.addLayout(login_actions)
        box_layout.addWidget(self._login_status_label)
        box.setMaximumWidth(400)

        layout.addWidget(box, alignment=Qt.AlignmentFlag.AlignCenter)
        return root

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
        self._messages_container = QWidget(chat_content_panel)
        self._messages_container.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        messages_container_layout = QVBoxLayout(self._messages_container)
        messages_container_layout.setContentsMargins(0, 0, 0, 0)
        messages_container_layout.setSpacing(0)

        self._messages_list = QListWidget(self._messages_container)
        self._messages_list.setStyleSheet(
            f"QListWidget {{ background-color: {COLOR_BG_PANE}; border: none; color: {COLOR_TEXT_MAIN}; }}"
        )
        self._messages_list.setVerticalScrollMode(QListWidget.ScrollMode.ScrollPerPixel)
        self._messages_list.verticalScrollBar().valueChanged.connect(self._on_messages_scrolled)
        messages_container_layout.addWidget(self._messages_list, 1)

        # Jump-to-latest chip overlays the bottom of the message pane whenever
        # the window is not sitting on the server tip. Parented to the
        # messages container (not added to a layout) so we can position it
        # as a floating widget in _position_jump_to_latest_button.
        self._jump_to_latest_button = QToolButton(self._messages_container)
        self._jump_to_latest_button.setText("Jump to latest")
        self._jump_to_latest_button.setIcon(material_icon("arrow-down-circle", color=COLOR_BG_PANE))
        self._jump_to_latest_button.setIconSize(QSize(16, 16))
        self._jump_to_latest_button.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        self._jump_to_latest_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._jump_to_latest_button.setStyleSheet(
            f"QToolButton {{ background-color: {COLOR_ACCENT}; color: {COLOR_BG_PANE}; "
            "border: none; border-radius: 12px; padding: 4px 12px; font-weight: 600; }}"
        )
        self._jump_to_latest_button.clicked.connect(self._jump_to_latest_clicked)
        self._jump_to_latest_button.hide()
        self._messages_container.installEventFilter(self)

        chat_content_layout.addWidget(self._messages_container, 1)

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

    def _login_clicked(self) -> None:
        username = self._username_input.text().strip()
        password = self._password_input.text()
        if not username or not password:
            self._login_status_label.setText("Username and password are required.")
            return
        self._set_login_busy(True)
        self._async_api.submit(
            lambda api: api.login(username, password),
            on_success=lambda _session: self._on_login_succeeded(),
            on_error=self._on_login_failed,
        )

    def _on_login_succeeded(self) -> None:
        self._events.start()
        self._stack.setCurrentWidget(self._chat_page)
        self._schedule_sidebar_layout_apply()
        self._status_label.setText("Connected")
        # Defer initial communities load one more hop so the chat page has
        # already been shown before the network round-trip starts. The
        # request itself runs on a worker; only the follow-up UI update
        # will execute here.
        self._async_api.submit(
            lambda api: api.read_user_communities(),
            on_success=self._on_initial_communities_loaded,
            on_error=self._on_initial_communities_failed,
        )

    def _on_login_failed(self, exc: Exception) -> None:
        self._login_status_label.setText(str(exc))
        self._set_login_busy(False)

    def _on_initial_communities_loaded(self, communities: list[Community]) -> None:
        self._state.set_communities(communities)
        self._rebuild_community_list(preferred_community_id=None)
        if not communities:
            self._status_label.setText(
                "Connected. No communities yet — click Create Community to start chatting."
            )
        QTimer.singleShot(0, self._refresh_messages)
        self._set_login_busy(False)

    def _on_initial_communities_failed(self, exc: Exception) -> None:
        self._status_label.setText(f"Failed to load communities: {exc}")
        self._set_login_busy(False)

    def _set_login_busy(self, busy: bool) -> None:
        self._login_button.setEnabled(not busy)
        self._create_user_button.setEnabled(not busy)
        self._username_input.setEnabled(not busy)
        self._password_input.setEnabled(not busy)
        if busy:
            self._login_status_label.setText("Working...")

    def _create_user_clicked(self) -> None:
        username = self._username_input.text().strip()
        password = self._password_input.text()
        if not username or not password:
            self._login_status_label.setText("Enter username and password, then click Create User.")
            return

        self._set_login_busy(True)
        self._async_api.submit(
            lambda api: api.create_user(username, password),
            on_success=lambda _user_id: self._on_user_created(),
            on_error=self._on_create_user_failed,
        )

    def _on_user_created(self) -> None:
        self._set_login_busy(False)
        self._login_status_label.setText("User created. You can now log in.")

    def _on_create_user_failed(self, exc: Exception) -> None:
        self._login_status_label.setText(f"Create user failed: {exc}")
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
        self._async_api.submit(
            lambda api: api.read_community_channels(community_id),
            on_success=lambda channels: self._on_community_channels_loaded(
                community_id, channels, preferred_channel_id
            ),
            on_error=lambda exc: self._status_label.setText(
                f"Failed to load channels: {exc}"
            ),
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
            self._clear_message_view()
            self._update_jump_to_latest_button()
            return
        channel_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(channel_id, str):
            return
        self._current_channel_id = channel_id
        self._update_active_channel_header()
        self._clear_message_view()
        window = self._state.channel_windows.get(channel_id)
        if window is not None and window.ordered_ids:
            # We've been here before; rebuild from the cached window without
            # a round-trip. If the user wants fresher state they can scroll
            # or hit "jump to latest".
            self._render_cached_window()
        else:
            self._dispatch_message_fetch(channel_id, "initial", None, INITIAL_MESSAGE_LOAD)
        self._update_jump_to_latest_button()

    def _clear_message_view(self) -> None:
        """Drop all row widgets and sentinel rows for the message list."""
        self._messages_list.clear()
        self._message_items_by_id.clear()
        self._loading_header_item = None
        self._loading_footer_item = None

    def _render_cached_window(self) -> None:
        """Rebuild the message view from the currently-cached window."""
        if self._current_channel_id is None:
            return
        messages = self._state.get_messages_for_channel(self._current_channel_id)
        self._resolve_author_profiles(messages)
        for message in messages:
            self._add_message_item(message)
        self._resize_message_item_widgets()
        window = self._state.channel_windows.get(self._current_channel_id)
        if window is None or not window.has_newer:
            self._messages_list.scrollToBottom()

    # Retained for external call sites that still want a full resync (e.g.
    # the explicit refresh button). Now routes through the cached-window
    # rebuild path.
    def _refresh_messages(self) -> None:
        self._clear_message_view()
        if self._current_channel_id is None:
            return
        self._render_cached_window()

    def _add_message_item(self, message: Message) -> None:
        """Append a message row at the end of the list.

        Used by full re-renders (``_render_cached_window``). For incremental
        inserts that need to land in the middle, use
        ``_insert_message_item_at``.
        """
        row = self._messages_list.count()
        if self._loading_footer_item is not None:
            row -= 1
        self._insert_message_item_at(max(row, 0), message)

    def _resize_message_item_widgets(self) -> None:
        for index in range(self._messages_list.count()):
            item = self._messages_list.item(index)
            if item is None:
                continue
            self._size_message_item(item)

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
        profile = self._user_profiles_by_id.get(message.author)
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
        self._async_api.submit(
            lambda api: api.send_message(channel_id, text),
            on_success=self._on_message_sent,
            on_error=lambda exc: self._on_message_send_failed(text, exc),
        )

    def _on_message_sent(self, message: Message) -> None:
        window = self._state.channel_windows.get(message.channel_id)
        if window is not None and window.has_newer:
            # The user was reading older history; their own message only
            # makes sense as the new tip. Reset the window and reload from
            # the server so we don't end up with a non-contiguous slice.
            self._state.clear_channel_window(message.channel_id)
            if message.channel_id == self._current_channel_id:
                self._clear_message_view()
                self._dispatch_message_fetch(
                    message.channel_id,
                    "initial",
                    None,
                    INITIAL_MESSAGE_LOAD,
                )
                self._update_jump_to_latest_button()
        else:
            self._state.upsert_message(message)
            # Show the outbound message immediately via the incremental path;
            # when the matching server event echoes back, _handle_message_event
            # will find the row already present and fall through to a cheap
            # update.
            if message.channel_id == self._current_channel_id:
                if message.id in self._message_items_by_id:
                    self._update_message_row(message)
                else:
                    self._append_message_row(message)
                    self._evict_ui_rows_not_in_window()
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
        event_payload = parsed.model_dump(mode="json")
        changed = self._state.apply_server_event(event_payload)
        if not changed:
            return

        event_type = str(event_payload.get("type", ""))
        if server_event == "community":
            self._handle_community_event(event_payload, event_type)
        elif server_event == "channel":
            self._handle_channel_event(event_payload, event_type)
        elif server_event == "message":
            self._handle_message_event(event_payload, event_type)

    def _handle_community_event(self, payload: dict, event_type: str) -> None:
        community_id = payload.get("id")
        if not isinstance(community_id, str) or not community_id:
            return
        if event_type == "delete":
            self._remove_community_row(community_id)
            return
        community = self._state.communities.get(community_id)
        if community is None:
            return
        if community_id in self._community_items_by_id:
            self._update_community_row(community)
        else:
            self._insert_community_row(community)
        self._fit_community_sidebar_width_to_content()

    def _handle_channel_event(self, payload: dict, event_type: str) -> None:
        channel_id = payload.get("id")
        if not isinstance(channel_id, str) or not channel_id:
            return
        if event_type == "delete":
            if channel_id in self._channel_items_by_id:
                self._remove_channel_row(channel_id)
            return
        channel = self._state.channels.get(channel_id)
        if channel is None:
            return
        # Channels outside the currently-displayed community have no row;
        # state was already updated so there's nothing to do in the UI.
        if channel.community != self._current_community_id:
            if channel_id in self._channel_items_by_id:
                self._remove_channel_row(channel_id)
            return
        if channel_id in self._channel_items_by_id:
            self._update_channel_row(channel)
        else:
            self._insert_channel_row(channel)

    def _handle_message_event(self, payload: dict, event_type: str) -> None:
        message_id = payload.get("id")
        if not isinstance(message_id, str) or not message_id:
            return
        if event_type == "delete":
            self._remove_message_row(message_id)
            return
        message = self._state.messages.get(message_id)
        if message is None:
            return
        # Only the channel we are currently viewing has materialized rows; all
        # other channels stay as in-memory state until the user navigates to
        # them and _refresh_messages populates the widget.
        if message.channel_id != self._current_channel_id:
            return
        # The state layer's has_newer gate has already filtered out creates
        # the user shouldn't see (they're reading older history); by the time
        # we're here, the message has been appended to the current window.
        if event_type == "create":
            if message_id in self._message_items_by_id:
                self._update_message_row(message)
            else:
                self._append_message_row(message)
                # State may have evicted older rows when the append pushed the
                # window past the cap; drop matching UI rows to stay in sync.
                self._evict_ui_rows_not_in_window()
        elif event_type == "update":
            self._update_message_row(message)

    def _dispatch_message_fetch(
        self,
        channel_id: str,
        direction: str,
        anchor_id: str | None,
        count: int,
    ) -> None:
        """Kick off a page fetch if one isn't already in flight in this direction."""
        in_flight = self._pending_fetches.setdefault(channel_id, set())
        if direction in in_flight:
            return
        in_flight.add(direction)
        if direction == "older":
            self._show_loading_sentinel("header")
        elif direction == "newer":
            self._show_loading_sentinel("footer")

        def _operation(api: AspenApiClient) -> list[Message]:
            if direction == "older":
                return api.read_channel_messages(
                    channel_id, before=anchor_id, count=count
                )
            if direction == "newer":
                return api.read_channel_messages(
                    channel_id, after=anchor_id, count=count
                )
            # "initial" — no anchor means "most recent page".
            return api.read_channel_messages(channel_id, count=count)

        self._async_api.submit(
            _operation,
            on_success=lambda messages: self._on_message_page_loaded(
                channel_id, direction, messages, count
            ),
            on_error=lambda exc: self._on_message_page_failed(
                channel_id, direction, str(exc)
            ),
        )

    def _on_message_page_loaded(
        self,
        channel_id: str,
        direction: str,
        messages: list[Message],
        requested_count: int,
    ) -> None:
        """Apply a paginated page onto the window and the message list."""
        in_flight = self._pending_fetches.get(channel_id)
        if in_flight is not None:
            in_flight.discard(direction)

        # Use the raw (pre-dedupe) count to decide whether the server has
        # more history in this direction. A full page means "probably more";
        # a short page means "we hit the end".
        raw_count = len(messages)
        hit_end = raw_count < requested_count

        # The anchor id itself is echoed back inclusively by the server's
        # le/ge filters; strip it so we don't double-render.
        window = self._state.channel_windows.get(channel_id)
        if window is not None:
            known = set(window.ordered_ids)
            messages = [m for m in messages if m.id not in known]

        if direction == "initial":
            # Replace the window entirely. An initial fetch at the tip
            # implies we are sitting on the newest message, so ``has_newer``
            # is False. ``has_older`` depends on whether the page was full.
            self._state.set_channel_window(
                channel_id,
                messages,
                has_older=not hit_end,
                has_newer=False,
            )
        elif direction == "older":
            self._state.merge_channel_page(channel_id, messages)
            window = self._state.channel_windows.get(channel_id)
            if window is not None and hit_end:
                window.has_older = False
        elif direction == "newer":
            self._state.merge_channel_page(channel_id, messages)
            window = self._state.channel_windows.get(channel_id)
            if window is not None and hit_end:
                window.has_newer = False

        # If the user switched channels while the fetch was in flight, state
        # was still updated above so a return trip to this channel is warm.
        # The message list is now showing a different channel, and its
        # sentinels were cleared by _channel_changed, so there's nothing
        # left to do on the UI side.
        if channel_id != self._current_channel_id:
            return

        self._hide_loading_sentinel("header")
        self._hide_loading_sentinel("footer")

        if direction == "initial":
            # Full re-render from the fresh window.
            self._clear_message_view()
            self._render_cached_window()
            self._update_jump_to_latest_button()
            return

        top_anchor = self._capture_top_anchor() if direction == "older" else None

        self._resolve_author_profiles(messages)
        for message in messages:
            if message.id in self._message_items_by_id:
                continue
            row = self._compute_insert_row_for(message.id)
            self._insert_message_item_at(row, message)

        self._resize_message_item_widgets()

        # Enforce the window cap from the end opposite to the one we just
        # loaded. State stays authoritative about which ids are retained;
        # we mirror by removing UI rows for ids that fell out.
        if direction == "older":
            self._state.evict_newer_to_cap(channel_id, MESSAGE_WINDOW_CAP)
        else:
            self._state.evict_older_to_cap(channel_id, MESSAGE_WINDOW_CAP)
        self._evict_ui_rows_not_in_window()

        if top_anchor is not None:
            self._restore_top_anchor(top_anchor)

        self._update_jump_to_latest_button()

    def _on_message_page_failed(self, channel_id: str, direction: str, error: str) -> None:
        in_flight = self._pending_fetches.get(channel_id)
        if in_flight is not None:
            in_flight.discard(direction)
        if direction == "older":
            self._hide_loading_sentinel("header")
        elif direction == "newer":
            self._hide_loading_sentinel("footer")
        if channel_id == self._current_channel_id:
            self._status_label.setText(f"Message fetch failed: {error}")

    def _on_messages_scrolled(self, _value: int) -> None:
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        window = self._state.channel_windows.get(channel_id)
        if window is None:
            return
        bar = self._messages_list.verticalScrollBar()
        if window.has_older and bar.value() <= SCROLL_EDGE_PIXELS:
            oldest = window.ordered_ids[0] if window.ordered_ids else None
            if oldest is not None:
                self._dispatch_message_fetch(channel_id, "older", oldest, MESSAGE_PAGE_SIZE)
        if window.has_newer and (bar.maximum() - bar.value()) <= SCROLL_EDGE_PIXELS:
            newest = window.ordered_ids[-1] if window.ordered_ids else None
            if newest is not None:
                self._dispatch_message_fetch(channel_id, "newer", newest, MESSAGE_PAGE_SIZE)

    def _compute_insert_row_for(self, message_id: str) -> int:
        """Find the widget-row index for ``message_id`` based on the current window."""
        if self._current_channel_id is None:
            return self._messages_list.count()
        window = self._state.channel_windows.get(self._current_channel_id)
        if window is None:
            return self._messages_list.count()
        try:
            window_index = window.ordered_ids.index(message_id)
        except ValueError:
            return self._messages_list.count()
        # Walk the window in order and find the Nth already-materialized row.
        widget_row = 0
        if self._loading_header_item is not None:
            widget_row += 1
        for window_pos, mid in enumerate(window.ordered_ids):
            if window_pos == window_index:
                return widget_row
            if mid in self._message_items_by_id:
                widget_row += 1
        return widget_row

    def _insert_message_item_at(self, row: int, message: Message) -> None:
        item = QListWidgetItem()
        item.setFlags(Qt.ItemFlag.NoItemFlags)
        container = QWidget(self._messages_list)
        container.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        container_layout = QHBoxLayout(container)
        container_layout.setContentsMargins(6, 3, 6, 3)
        container_layout.setSpacing(8)

        avatar_label = QLabel(container)
        avatar_label.setObjectName("messageAvatarLabel")
        avatar_label.setFixedSize(28, 28)
        avatar_label.setPixmap(self._user_avatar_pixmap(message.author, size=28))
        avatar_label.setAlignment(Qt.AlignmentFlag.AlignTop)
        container_layout.addWidget(avatar_label, 0, alignment=Qt.AlignmentFlag.AlignTop)

        # Right column: rich-text header line + plain-text body. Splitting
        # body out into its own plain-text QLabel is the key perf win for
        # very long messages — rich-text wrapping goes through QTextDocument
        # and scales badly, plain-text wrapping is linear.
        text_column = QWidget(container)
        text_column.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        text_column_layout = QVBoxLayout(text_column)
        text_column_layout.setContentsMargins(0, 0, 0, 0)
        text_column_layout.setSpacing(2)

        header_label = QLabel(self._format_message_header_html(message), text_column)
        header_label.setObjectName("messageHeaderLabel")
        header_label.setTextFormat(Qt.TextFormat.RichText)
        header_label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        header_label.setWordWrap(False)
        header_label.setContentsMargins(0, 0, 0, 0)
        text_column_layout.addWidget(header_label)

        body_label = QLabel(message.content, text_column)
        body_label.setObjectName("messageBodyLabel")
        body_label.setTextFormat(Qt.TextFormat.RichText)
        body_label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        body_label.setWordWrap(True)
        body_label.setContentsMargins(0, 0, 0, 0)
        body_label.setStyleSheet(f"color: {COLOR_TEXT_MAIN};")
        text_column_layout.addWidget(body_label)

        container_layout.addWidget(text_column, 1)

        self._messages_list.insertItem(row, item)
        self._messages_list.setItemWidget(item, container)
        self._message_items_by_id[message.id] = item

    def _evict_ui_rows_not_in_window(self) -> None:
        if self._current_channel_id is None:
            return
        window = self._state.channel_windows.get(self._current_channel_id)
        if window is None:
            return
        valid = set(window.ordered_ids)
        stale_ids = [mid for mid in self._message_items_by_id if mid not in valid]
        for mid in stale_ids:
            item = self._message_items_by_id.pop(mid, None)
            if item is None:
                continue
            row = self._messages_list.row(item)
            if row >= 0:
                self._messages_list.takeItem(row)

    def _capture_top_anchor(self) -> tuple[str, int] | None:
        """Record the id + pixel offset of the topmost visible message row.

        Used to preserve the user's visual position across a prepend: after
        inserting older rows, the content shifts downward and we compensate
        by advancing the scrollbar by exactly that delta.
        """
        viewport_top = 0
        for row in range(self._messages_list.count()):
            item = self._messages_list.item(row)
            if item is None:
                continue
            rect = self._messages_list.visualItemRect(item)
            if rect.bottom() < viewport_top:
                continue
            # Sentinel rows don't correspond to a real message; skip them.
            msg_id = next(
                (mid for mid, it in self._message_items_by_id.items() if it is item),
                None,
            )
            if msg_id is None:
                continue
            return msg_id, rect.top()
        return None

    def _restore_top_anchor(self, anchor: tuple[str, int]) -> None:
        msg_id, saved_top = anchor
        item = self._message_items_by_id.get(msg_id)
        if item is None:
            return
        new_top = self._messages_list.visualItemRect(item).top()
        bar = self._messages_list.verticalScrollBar()
        bar.setValue(bar.value() + (new_top - saved_top))

    def _show_loading_sentinel(self, position: str) -> None:
        """Insert a non-interactive "loading..." row at top or bottom."""
        if position == "header" and self._loading_header_item is not None:
            return
        if position == "footer" and self._loading_footer_item is not None:
            return

        item = QListWidgetItem()
        item.setFlags(Qt.ItemFlag.NoItemFlags)
        label = QLabel(
            "Loading older messages..." if position == "header" else "Loading newer messages...",
            self._messages_list,
        )
        label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        label.setContentsMargins(6, 6, 6, 6)
        label.setStyleSheet(f"color: {COLOR_TEXT_MUTED}; font-style: italic;")

        if position == "header":
            self._messages_list.insertItem(0, item)
            self._loading_header_item = item
        else:
            self._messages_list.addItem(item)
            self._loading_footer_item = item
        self._messages_list.setItemWidget(item, label)
        item.setSizeHint(label.sizeHint())

    def _hide_loading_sentinel(self, position: str) -> None:
        if position == "header":
            item = self._loading_header_item
            self._loading_header_item = None
        else:
            item = self._loading_footer_item
            self._loading_footer_item = None
        if item is None:
            return
        row = self._messages_list.row(item)
        if row >= 0:
            self._messages_list.takeItem(row)

    def _update_jump_to_latest_button(self) -> None:
        visible = False
        if self._current_channel_id is not None:
            window = self._state.channel_windows.get(self._current_channel_id)
            if window is not None and window.has_newer:
                visible = True
        self._jump_to_latest_button.setVisible(visible)
        if visible:
            self._position_jump_to_latest_button()

    def _position_jump_to_latest_button(self) -> None:
        container = self._messages_container
        if container is None:
            return
        button = self._jump_to_latest_button
        button.adjustSize()
        size = button.sizeHint()
        margin = 16
        x = (container.width() - size.width()) // 2
        y = container.height() - size.height() - margin
        button.move(max(x, margin), max(y, margin))
        button.raise_()

    def _jump_to_latest_clicked(self) -> None:
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        # Nuke the current window (plus its cached message records) and start
        # from the server tip. This matches the behavior the user expects from
        # "catch me up to now" chips in other chat clients.
        self._state.clear_channel_window(channel_id)
        self._clear_message_view()
        self._dispatch_message_fetch(channel_id, "initial", None, INITIAL_MESSAGE_LOAD)
        self._update_jump_to_latest_button()

    def eventFilter(self, watched: QObject, event: QEvent) -> bool:  # type: ignore[override]
        if watched is self._messages_container and event.type() == QEvent.Type.Resize:
            if self._jump_to_latest_button.isVisible():
                self._position_jump_to_latest_button()
        elif watched is self._composer and event.type() == QEvent.Type.KeyPress:
            key = event.key()
            if key in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
                # Shift+Enter inserts a newline as usual; bare Enter sends.
                # Ignore keypad modifier (Key_Enter comes from the numpad and
                # carries KeypadModifier on some platforms).
                if not (event.modifiers() & Qt.KeyboardModifier.ShiftModifier):
                    self._send_clicked()
                    return True
        return super().eventFilter(watched, event)

    def _insert_community_row(self, community: Community) -> None:
        # Preserve the alphabetical order used by _rebuild_community_list so
        # incremental inserts match the order the user sees on a cold start.
        name_key = community.name.lower()
        insert_index = self._community_list.count()
        for index in range(self._community_list.count()):
            other_id = self._community_list.item(index).data(Qt.ItemDataRole.UserRole)
            other = self._state.communities.get(other_id) if isinstance(other_id, str) else None
            if other is None:
                continue
            if name_key < other.name.lower():
                insert_index = index
                break

        item = QListWidgetItem(community.name)
        item.setIcon(self._community_avatar_icon(community))
        item.setData(Qt.ItemDataRole.UserRole, community.id)
        self._community_list.insertItem(insert_index, item)
        self._community_items_by_id[community.id] = item

        avatar_item = QListWidgetItem(self._community_avatar_icon(community), "")
        avatar_item.setData(Qt.ItemDataRole.UserRole, community.id)
        avatar_item.setToolTip(community.name)
        avatar_item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
        avatar_item.setSizeHint(QSize(28, 36))
        self._community_avatar_list.insertItem(insert_index, avatar_item)
        self._community_avatar_items_by_id[community.id] = avatar_item

        # Auto-select the first community so _community_changed loads its
        # channels and users without requiring a manual click.
        if self._community_list.count() == 1:
            self._community_list.setCurrentRow(0)

    def _update_community_row(self, community: Community) -> None:
        existing_item = self._community_items_by_id.get(community.id)
        if existing_item is None:
            self._insert_community_row(community)
            return
        ordered = self._state.get_communities_sorted()
        try:
            expected_row = next(
                index for index, record in enumerate(ordered) if record.id == community.id
            )
        except StopIteration:
            return
        current_row = self._community_list.row(existing_item)
        if current_row != expected_row:
            was_selected = self._community_list.currentItem() is existing_item
            prev_block_main = self._community_list.signalsBlocked()
            prev_block_avatar = self._community_avatar_list.signalsBlocked()
            self._community_list.blockSignals(True)
            self._community_avatar_list.blockSignals(True)
            try:
                self._remove_community_row(community.id)
                self._insert_community_row(community)
                if was_selected:
                    new_item = self._community_items_by_id.get(community.id)
                    if new_item is not None:
                        self._community_list.setCurrentItem(new_item)
                        avatar_item = self._community_avatar_items_by_id.get(community.id)
                        if avatar_item is not None:
                            self._community_avatar_list.setCurrentItem(avatar_item)
            finally:
                self._community_list.blockSignals(prev_block_main)
                self._community_avatar_list.blockSignals(prev_block_avatar)
            return

        existing_item.setText(community.name)
        existing_item.setIcon(self._community_avatar_icon(community))
        avatar_item = self._community_avatar_items_by_id.get(community.id)
        if avatar_item is not None:
            avatar_item.setIcon(self._community_avatar_icon(community))
            avatar_item.setToolTip(community.name)

    def _remove_community_row(self, community_id: str) -> None:
        item = self._community_items_by_id.pop(community_id, None)
        if item is not None:
            row = self._community_list.row(item)
            if row >= 0:
                self._community_list.takeItem(row)
        avatar_item = self._community_avatar_items_by_id.pop(community_id, None)
        if avatar_item is not None:
            row = self._community_avatar_list.row(avatar_item)
            if row >= 0:
                self._community_avatar_list.takeItem(row)

    def _insert_channel_row(self, channel: Channel) -> None:
        insert_index = self._channel_list.count()
        for index in range(self._channel_list.count()):
            other_id = self._channel_list.item(index).data(Qt.ItemDataRole.UserRole)
            other = self._state.channels.get(other_id) if isinstance(other_id, str) else None
            if other is None:
                continue
            if channel.sort_index < other.sort_index:
                insert_index = index
                break

        item = QListWidgetItem(f"#{channel.name}")
        item.setData(Qt.ItemDataRole.UserRole, channel.id)
        self._channel_list.insertItem(insert_index, item)
        self._channel_items_by_id[channel.id] = item

        if self._channel_list.count() == 1:
            self._channel_list.setCurrentRow(0)

    def _update_channel_row(self, channel: Channel) -> None:
        existing_item = self._channel_items_by_id.get(channel.id)
        if existing_item is None:
            self._insert_channel_row(channel)
            return
        # sort_index changes should move the row to its new position.
        ordered = self._state.get_channels_for_community(self._current_community_id or "")
        try:
            expected_row = next(
                index for index, record in enumerate(ordered) if record.id == channel.id
            )
        except StopIteration:
            return
        current_row = self._channel_list.row(existing_item)
        if current_row != expected_row:
            was_selected = self._channel_list.currentItem() is existing_item
            prev_block = self._channel_list.signalsBlocked()
            self._channel_list.blockSignals(True)
            try:
                self._remove_channel_row(channel.id)
                self._insert_channel_row(channel)
                if was_selected:
                    new_item = self._channel_items_by_id.get(channel.id)
                    if new_item is not None:
                        self._channel_list.setCurrentItem(new_item)
            finally:
                self._channel_list.blockSignals(prev_block)
            return

        existing_item.setText(f"#{channel.name}")

    def _remove_channel_row(self, channel_id: str) -> None:
        item = self._channel_items_by_id.pop(channel_id, None)
        if item is None:
            return
        row = self._channel_list.row(item)
        if row >= 0:
            self._channel_list.takeItem(row)

    def _append_message_row(self, message: Message) -> None:
        # Fill in the author profile (and its avatar-cache entry) before we
        # paint the row so the first render shows the real display name.
        if message.author not in self._user_profiles_by_id:
            self._resolve_author_profiles([message])
        self._add_message_item(message)
        item = self._message_items_by_id.get(message.id)
        if item is not None:
            self._size_message_item(item)
        self._messages_list.scrollToBottom()

    def _update_message_row(self, message: Message) -> None:
        item = self._message_items_by_id.get(message.id)
        if item is None:
            return
        widget = self._messages_list.itemWidget(item)
        if widget is None:
            return
        header_label = widget.findChild(QLabel, "messageHeaderLabel")
        if isinstance(header_label, QLabel):
            header_label.setText(self._format_message_header_html(message))
        body_label = widget.findChild(QLabel, "messageBodyLabel")
        if isinstance(body_label, QLabel):
            body_label.setText(message.content)
        # Content changed — invalidate the cached-at-width marker so the
        # next sizing pass actually recomputes heightForWidth.
        item.setData(self._SIZED_AT_WIDTH_ROLE, None)
        self._size_message_item(item)

    def _remove_message_row(self, message_id: str) -> None:
        item = self._message_items_by_id.pop(message_id, None)
        if item is None:
            return
        row = self._messages_list.row(item)
        if row >= 0:
            self._messages_list.takeItem(row)

    # Qt.ItemDataRole custom roles used to cache the viewport width the row
    # was last sized at. When bulk resize runs after an insert, rows whose
    # cached width matches the current viewport can be skipped entirely,
    # which makes repeated paging across a full 500-row window O(pages)
    # instead of O(pages x window_cap).
    _SIZED_AT_WIDTH_ROLE = Qt.ItemDataRole.UserRole + 100

    def _size_message_item(self, item: QListWidgetItem) -> None:
        """Apply width + size-hint to one message row.

        Shared by the incremental-append path (``_append_message_row`` /
        ``_update_message_row``) and the bulk relayout path
        (``_resize_message_item_widgets``). Keeping a single implementation
        guarantees the outbound user message lands with the same margins,
        the same x-placement of its avatar/body, and the same wrapping as
        messages arriving via any other path.

        Critically, we pin both the container widget and the row sizeHint
        to the viewport's full width. If we let ``adjustSize`` shrink the
        widget to its children's natural width, QListView can end up
        placing a narrower widget at a different x offset than its
        siblings (this is what made a freshly-sent message appear shifted
        right of all the previously-rendered rows). Forcing every row to
        the same width eliminates that class of drift entirely.

        The per-row viewport-width cache is the main lever for very long
        messages: computing ``heightForWidth`` on a word-wrapped
        QLabel with, say, 14 kB of content costs real CPU time, and doing
        it redundantly every time a page is inserted adds up. We only
        re-measure when the viewport width actually changed from the last
        time we sized this row; content edits invalidate the cache via
        ``_update_message_row``.
        """
        widget = self._messages_list.itemWidget(item)
        if not isinstance(widget, QWidget):
            return
        body_label = widget.findChild(QLabel, "messageBodyLabel")
        if not isinstance(body_label, QLabel):
            return
        viewport_width = max(self._messages_list.viewport().width(), 200)
        cached_width = item.data(self._SIZED_AT_WIDTH_ROLE)
        if cached_width == viewport_width:
            return
        # 16px subtracts a conservative scrollbar/inner-padding allowance;
        # 44px subtracts the avatar column (28) + layout spacing (8) + left
        # container margin (6) + right container margin (~2 of slop).
        body_width = max(viewport_width - 16 - 44, 120)
        body_label.setFixedWidth(body_width)
        widget.setFixedWidth(viewport_width)
        widget.ensurePolished()
        layout = widget.layout()
        if layout is not None:
            layout.activate()
        item.setSizeHint(QSize(viewport_width, widget.sizeHint().height()))
        item.setData(self._SIZED_AT_WIDTH_ROLE, viewport_width)

    def _update_user_status_dot(self, user_id: str) -> None:
        entry = self._user_row_entries_by_id.get(user_id)
        if entry is None:
            return
        _item, dot_label, _avatar = entry
        status = self._user_online_status_by_id.get(user_id, "offline")
        dot_label.setPixmap(self._presence_dot_pixmap(status, size=8))

    def _on_event_error(self, err: str) -> None:
        self._status_label.setText(f"Event stream issue: {err}")

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
        self._async_api.submit(
            lambda api: api.create_channel(
                community_id=community_id,
                name=channel_name,
                sort_index=sort_index,
            ),
            on_success=lambda created: self._on_channel_created(community_id, created),
            on_error=lambda exc: self._show_error(str(exc)),
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
        self._async_api.submit(
            lambda api: api.read_user_communities(),
            on_success=lambda communities: self._on_refresh_communities_loaded(
                communities, selected_community_id, selected_channel_id
            ),
            on_error=lambda exc: self._show_error(str(exc)),
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
        self._async_api.submit(
            lambda api: api.read_community_channels(final_community_id),
            on_success=lambda channels: self._on_refresh_channels_loaded(
                final_community_id, channels, preferred_channel_id
            ),
            on_error=lambda exc: self._show_error(str(exc)),
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
        self._async_api.submit(
            lambda api: api.create_community(community_name),
            on_success=self._on_community_created,
            on_error=lambda exc: self._show_error(str(exc)),
        )

    def _on_community_created(self, community: Community) -> None:
        self._state.upsert_community(community)
        self._rebuild_community_list(preferred_community_id=community.id)
        self._status_label.setText(f"Created community '{community.name}'.")

    def _rebuild_community_list(self, preferred_community_id: str | None) -> None:
        self._community_list.blockSignals(True)
        self._community_avatar_list.blockSignals(True)
        self._community_list.clear()
        self._community_avatar_list.clear()
        self._community_items_by_id.clear()
        self._community_avatar_items_by_id.clear()
        for community in self._state.get_communities_sorted():
            item = QListWidgetItem(community.name)
            item.setIcon(self._community_avatar_icon(community))
            item.setData(Qt.ItemDataRole.UserRole, community.id)
            self._community_list.addItem(item)
            self._community_items_by_id[community.id] = item
            avatar_item = QListWidgetItem(self._community_avatar_icon(community), "")
            avatar_item.setData(Qt.ItemDataRole.UserRole, community.id)
            avatar_item.setToolTip(community.name)
            avatar_item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
            avatar_item.setSizeHint(QSize(28, 36))
            self._community_avatar_list.addItem(avatar_item)
            self._community_avatar_items_by_id[community.id] = avatar_item
        if self._community_list.count() > 0:
            row = self._row_for_user_role(self._community_list, preferred_community_id)
            if row is None:
                row = 0
            self._community_list.setCurrentRow(row)
            self._community_avatar_list.setCurrentRow(row)
        self._community_list.blockSignals(False)
        self._community_avatar_list.blockSignals(False)
        self._fit_community_sidebar_width_to_content()
        self._community_changed()

    def _rebuild_channel_list(self, community_id: str, preferred_channel_id: str | None) -> None:
        self._channel_list.blockSignals(True)
        self._channel_list.clear()
        self._channel_items_by_id.clear()
        for channel in self._state.get_channels_for_community(community_id):
            item = QListWidgetItem(f"#{channel.name}")
            item.setData(Qt.ItemDataRole.UserRole, channel.id)
            self._channel_list.addItem(item)
            self._channel_items_by_id[channel.id] = item
        if self._channel_list.count() > 0:
            row = self._row_for_user_role(self._channel_list, preferred_channel_id)
            if row is None:
                row = 0
            self._channel_list.setCurrentRow(row)
            self._current_channel_id = self._selected_item_user_role(self._channel_list)
        else:
            self._current_channel_id = None
        self._channel_list.blockSignals(False)
        self._apply_collapsed_sidebar_ratio()
        self._apply_expanded_sidebar_layout()
        self._channel_changed()
        self._schedule_sidebar_layout_apply()

    def _resolve_author_profiles(self, messages: list[Message]) -> None:
        """Kick off async profile loads for authors we don't know yet.

        Returns immediately. Rows are rendered with a short-id fallback in
        the header and a fallback avatar; once each profile load completes
        on the GUI thread (see ``_on_user_profile_loaded``) the affected
        rows are patched in place.
        """
        missing_authors = {
            message.author
            for message in messages
            if message.author not in self._user_profiles_by_id
        }
        for author_id in missing_authors:
            if author_id in self._pending_user_profile_fetches:
                continue
            self._pending_user_profile_fetches.add(author_id)
            self._async_api.submit(
                lambda api, uid=author_id: api.read_user_profile(uid),
                on_success=lambda profile, uid=author_id: self._on_user_profile_loaded(
                    uid, profile
                ),
                on_error=lambda _exc, uid=author_id: self._pending_user_profile_fetches.discard(
                    uid
                ),
            )

    def _on_user_profile_loaded(self, user_id: str, profile: UserProfile) -> None:
        self._pending_user_profile_fetches.discard(user_id)
        self._user_profiles_by_id[user_id] = profile
        # The avatar cache was keyed on the fallback pixmap; throw it out
        # so _user_avatar_pixmap re-renders with the real name / kicks off
        # an icon fetch for the profile's icon id.
        stale_keys = [
            key for key in self._user_avatar_cache if key.startswith(f"{user_id}:")
        ]
        for key in stale_keys:
            self._user_avatar_cache.pop(key, None)
        # Re-paint any visible message rows authored by this user so the
        # header shows the real name and the avatar gets a fresh render.
        for msg_id, item in self._message_items_by_id.items():
            message = self._state.messages.get(msg_id)
            if message is None or message.author != user_id:
                continue
            widget = self._messages_list.itemWidget(item)
            if widget is None:
                continue
            header_label = widget.findChild(QLabel, "messageHeaderLabel")
            if isinstance(header_label, QLabel):
                header_label.setText(self._format_message_header_html(message))
            avatar_label = widget.findChild(QLabel, "messageAvatarLabel")
            if isinstance(avatar_label, QLabel):
                avatar_label.setPixmap(self._user_avatar_pixmap(user_id, size=28))

    @staticmethod
    def _selected_item_user_role(list_widget: QListWidget) -> str | None:
        selected = list_widget.selectedItems()
        if not selected:
            return None
        value = selected[0].data(Qt.ItemDataRole.UserRole)
        return value if isinstance(value, str) else None

    @staticmethod
    def _row_for_user_role(list_widget: QListWidget, value: str | None) -> int | None:
        if value is None:
            return None
        for row in range(list_widget.count()):
            item = list_widget.item(row)
            if item is not None and item.data(Qt.ItemDataRole.UserRole) == value:
                return row
        return None

    def _fit_community_sidebar_width_to_content(self) -> None:
        item_texts = [
            self._community_list.item(index).text()
            for index in range(self._community_list.count())
            if self._community_list.item(index) is not None
        ]
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
        self._user_online_status_by_id[str(user_id)] = status

    def _create_invite_clicked(self) -> None:
        community_id = self._current_community_id
        if community_id is None:
            self._status_label.setText("Select a community first.")
            return
        self._async_api.submit(
            lambda api: api.create_invite(community_id),
            on_success=self._on_invite_created,
            on_error=lambda exc: self._show_error(str(exc)),
        )

    def _on_invite_created(self, code: str) -> None:
        self._status_label.setText(f"Invite created: {code}")
        QMessageBox.information(self, "Invite Created", f"Invite code: {code}")

    def _refresh_users_preview(self, community_id: str) -> None:
        self._users_preview_list.clear()
        self._user_row_entries_by_id.clear()
        self._async_api.submit(
            lambda api: api.read_community_users(community_id),
            on_success=lambda users: self._on_community_users_loaded(community_id, users),
            on_error=lambda exc: self._status_label.setText(
                f"Failed to load users: {exc}"
            ),
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
            self._user_profiles_by_id[user.id] = user
            item = QListWidgetItem()
            row = QWidget(self._users_preview_list)
            row_layout = QHBoxLayout(row)
            row_layout.setContentsMargins(6, 2, 6, 2)
            row_layout.setSpacing(6)

            status_dot = QLabel(row)
            status_dot.setFixedSize(8, 8)
            status = self._user_online_status_by_id.get(user.id, "offline")
            status_dot.setPixmap(self._presence_dot_pixmap(status, size=8))
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

    @staticmethod
    def _presence_dot_pixmap(status: str, size: int) -> QPixmap:
        pixmap = QPixmap(size, size)
        pixmap.fill(Qt.GlobalColor.transparent)
        painter = QPainter(pixmap)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        status_color = COLOR_ACCENT if status == "online" else COLOR_AWAY
        ring_pen = QPen(QColor(status_color))
        ring_pen.setWidth(1)
        painter.setPen(ring_pen)
        if status in {"online", "away"}:
            painter.setBrush(QColor(status_color))
        else:
            painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.drawEllipse(0, 0, size - 1, size - 1)
        painter.end()
        return pixmap

    def _sync_community_avatar_selection(self, community_id: str) -> None:
        row = self._row_for_user_role(self._community_avatar_list, community_id)
        if row is None:
            return
        if self._community_avatar_list.currentRow() == row:
            return
        self._community_avatar_list.blockSignals(True)
        self._community_avatar_list.setCurrentRow(row)
        self._community_avatar_list.blockSignals(False)

    def _select_community_by_id(self, community_id: str) -> None:
        row = self._row_for_user_role(self._community_list, community_id)
        if row is None:
            return
        if self._community_list.currentRow() == row:
            return
        self._community_list.setCurrentRow(row)

    def _user_avatar_pixmap(self, user_id: str, size: int) -> QPixmap:
        """Return an avatar pixmap synchronously, loading the real icon async.

        This is called from the render hot-path (per-row on insert). We
        must not block here, so we always return either a cached pixmap
        or a fallback drawn from the display name. If the user has a
        real icon assigned, we kick off a background fetch for the bytes
        and patch the cached pixmap (and any live widgets) once it lands.
        """
        cache_key = f"{user_id}:{size}"
        if cache_key in self._user_avatar_cache:
            return self._user_avatar_cache[cache_key]

        profile = self._user_profiles_by_id.get(user_id)
        if profile is not None and profile.icon is not None:
            self._request_user_icon_async(user_id, profile.icon)

        fallback_name = profile.name if profile is not None else user_id
        avatar = self._fallback_user_avatar_pixmap(user_id, fallback_name, size)
        self._user_avatar_cache[cache_key] = avatar
        return avatar

    def _request_user_icon_async(self, user_id: str, icon_id: str) -> None:
        pending_key = f"{user_id}:{icon_id}"
        if pending_key in self._pending_user_icon_fetches:
            return
        self._pending_user_icon_fetches.add(pending_key)
        self._async_api.submit(
            lambda api, iid=icon_id: api.read_icon_bytes(iid),
            on_success=lambda icon_bytes, uid=user_id, iid=icon_id: self._on_user_icon_loaded(
                uid, iid, icon_bytes
            ),
            on_error=lambda _exc, pk=pending_key: self._pending_user_icon_fetches.discard(pk),
        )

    def _on_user_icon_loaded(
        self, user_id: str, icon_id: str, icon_bytes: bytes
    ) -> None:
        self._pending_user_icon_fetches.discard(f"{user_id}:{icon_id}")
        # We render user avatars at two sizes (28 in message rows, 20 in
        # the users-preview list). Pre-populate both so subsequent cache
        # reads are hits regardless of which view asks first.
        sizes_to_cache = (28, 20)
        cached_by_size: dict[int, QPixmap] = {}
        for size in sizes_to_cache:
            pixmap = self._circular_pixmap_from_bytes(icon_bytes, size)
            self._user_avatar_cache[f"{user_id}:{size}"] = pixmap
            cached_by_size[size] = pixmap
        # Patch any visible message rows authored by this user.
        for msg_id, item in self._message_items_by_id.items():
            message = self._state.messages.get(msg_id)
            if message is None or message.author != user_id:
                continue
            widget = self._messages_list.itemWidget(item)
            if widget is None:
                continue
            avatar_label = widget.findChild(QLabel, "messageAvatarLabel")
            if isinstance(avatar_label, QLabel):
                avatar_label.setPixmap(cached_by_size[28])
        # Patch the users preview row if present.
        entry = self._user_row_entries_by_id.get(user_id)
        if entry is not None:
            _item, _dot_label, avatar_label = entry
            avatar_label.setPixmap(cached_by_size[20])

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
        initials = ChatWindow._community_initials(name)
        painter.setPen(QColor(COLOR_TEXT_MAIN))
        font = painter.font()
        font.setBold(True)
        font.setPointSize(max(size // 3, 8))
        painter.setFont(font)
        painter.drawText(pixmap.rect(), Qt.AlignmentFlag.AlignCenter, initials)
        painter.end()
        return pixmap

    def _community_avatar_icon(self, community: Community) -> QIcon:
        """Return a community icon synchronously, loading the real icon async.

        Mirrors ``_user_avatar_pixmap``: never blocks the render path.
        Kicks off a background fetch for the actual icon bytes when a
        community has one assigned and patches list rows when it arrives.
        """
        if community.icon is not None and community.icon in self._community_icon_cache:
            return self._community_icon_cache[community.icon]
        if community.icon is not None:
            self._request_community_icon_async(community.id, community.icon)
        return self._fallback_community_avatar_icon(community.id, community.name)

    def _request_community_icon_async(self, community_id: str, icon_id: str) -> None:
        if icon_id in self._pending_community_icon_fetches:
            return
        self._pending_community_icon_fetches.add(icon_id)
        self._async_api.submit(
            lambda api, iid=icon_id: api.read_icon_bytes(iid),
            on_success=lambda icon_bytes, cid=community_id, iid=icon_id: self._on_community_icon_loaded(
                cid, iid, icon_bytes
            ),
            on_error=lambda _exc, iid=icon_id: self._pending_community_icon_fetches.discard(iid),
        )

    def _on_community_icon_loaded(
        self, community_id: str, icon_id: str, icon_bytes: bytes
    ) -> None:
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
        # Only update rows whose community still points at this icon id —
        # the icon could have been reassigned in between dispatch and
        # completion.
        community = self._state.communities.get(community_id)
        if community is None or community.icon != icon_id:
            return
        item = self._community_items_by_id.get(community_id)
        if item is not None:
            item.setIcon(icon)
        avatar_item = self._community_avatar_items_by_id.get(community_id)
        if avatar_item is not None:
            avatar_item.setIcon(icon)

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

        initials = ChatWindow._community_initials(community_name)
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

    def shutdown(self) -> None:
        """Tear down background work owned by this window.

        Called from both ``closeEvent`` (X button, ``window.close()``
        from a signal handler) and ``QApplication.aboutToQuit`` (any
        ``QApplication.quit()`` path). Idempotent so those two paths
        crossing is fine.

        The order matters: stop the WebSocket reader first so it can't
        emit further GUI-thread events, then bound the HTTP worker pool
        so any in-flight callbacks can't fire on a window we're about to
        tear down, then close the httpx client so any request still
        unwinding inside a worker aborts promptly instead of running out
        its 15s request timeout.
        """
        if self._shutting_down:
            return
        self._shutting_down = True
        self._events.stop()
        self._async_api.shutdown()
        self._api.close()

    def closeEvent(self, event) -> None:  # type: ignore[override]
        self.shutdown()
        super().closeEvent(event)

    def resizeEvent(self, event) -> None:  # type: ignore[override]
        super().resizeEvent(event)
        self._apply_collapsed_sidebar_ratio()
        self._apply_expanded_sidebar_layout()
        self._resize_message_item_widgets()
