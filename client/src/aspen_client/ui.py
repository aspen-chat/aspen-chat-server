from __future__ import annotations

import html
import hashlib
from typing import Callable

from PySide6.QtCore import QTimer, QSize, Qt
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

from aspen_client.api_client import AspenApiClient, AspenApiError
from aspen_client.event_client import EventStreamClient
from aspen_client.generated.event_models import ServerEvent as GeneratedServerEvent
from aspen_client.icons import apply_button_icon, material_icon
from aspen_client.state import ClientState
from aspen_client.types import Community, Message, UserProfile

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

        self._state = ClientState()
        self._current_community_id: str | None = None
        self._current_channel_id: str | None = None
        self._user_profiles_by_id: dict[str, UserProfile] = {}
        self._user_online_status_by_id: dict[str, str] = {}
        self._community_icon_cache: dict[str, QIcon] = {}
        self._user_avatar_cache: dict[str, QPixmap] = {}

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
        self._messages_list = QListWidget(chat_content_panel)
        self._messages_list.setStyleSheet(
            f"QListWidget {{ background-color: {COLOR_BG_PANE}; border: none; color: {COLOR_TEXT_MAIN}; }}"
        )
        chat_content_layout.addWidget(self._messages_list, 1)

        composer_container = QWidget(chat_content_panel)
        composer_container.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; border: none; border-radius: 0px;"
        )
        composer_row = QHBoxLayout(composer_container)
        composer_row.setContentsMargins(6, 6, 6, 6)
        composer_row.setSpacing(6)
        self._composer = QTextEdit(composer_container)
        self._composer.setPlaceholderText("Write a message...")
        self._composer.setFixedHeight(90)
        self._composer.setStyleSheet("background: transparent; border: none;")
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
        try:
            self._api.login(username, password)
            self._events.start()
            self._load_initial_state()
        except AspenApiError as exc:
            self._login_status_label.setText(str(exc))
            self._set_login_busy(False)
            return
        self._stack.setCurrentWidget(self._chat_page)
        self._schedule_sidebar_layout_apply()
        QTimer.singleShot(0, self._refresh_messages)
        self._status_label.setText("Connected")
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
        try:
            self._api.create_user(username, password)
        except AspenApiError as exc:
            self._login_status_label.setText(f"Create user failed: {exc}")
            self._set_login_busy(False)
            return

        self._set_login_busy(False)
        self._login_status_label.setText("User created. You can now log in.")

    def _load_initial_state(self) -> None:
        communities = self._api.read_user_communities()
        self._state.set_communities(communities)
        self._rebuild_community_list(preferred_community_id=None)
        if not communities:
            self._status_label.setText(
                "Connected. No communities yet — click Create Community to start chatting."
            )

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
        channels = self._api.read_community_channels(community_id)
        self._state.set_channels(channels)
        self._rebuild_channel_list(community_id=community_id, preferred_channel_id=self._current_channel_id)
        self._refresh_users_preview(community_id)

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
            self._refresh_messages()
            return
        channel_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(channel_id, str):
            return
        self._current_channel_id = channel_id
        self._update_active_channel_header()
        if not self._state.get_messages_for_channel(channel_id):
            messages = self._api.read_channel_messages(channel_id)
            self._state.set_channel_messages(channel_id, messages)
        self._refresh_messages()

    def _refresh_messages(self) -> None:
        self._messages_list.clear()
        if self._current_channel_id is None:
            return
        messages = self._state.get_messages_for_channel(self._current_channel_id)
        self._resolve_author_profiles(messages)
        for message in messages:
            self._add_message_item(message)
        self._resize_message_item_widgets()
        QTimer.singleShot(0, self._resize_message_item_widgets)
        self._messages_list.scrollToBottom()

    def _add_message_item(self, message: Message) -> None:
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

        label = QLabel(self._format_message_html(message), container)
        label.setObjectName("messageBodyLabel")
        label.setTextFormat(Qt.TextFormat.RichText)
        label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        label.setWordWrap(True)
        label.setContentsMargins(0, 0, 0, 0)
        container_layout.addWidget(label, 1)

        self._messages_list.addItem(item)
        self._messages_list.setItemWidget(item, container)

    def _resize_message_item_widgets(self) -> None:
        available_width = max(self._messages_list.viewport().width() - 16, 180)
        for index in range(self._messages_list.count()):
            item = self._messages_list.item(index)
            if item is None:
                continue
            widget = self._messages_list.itemWidget(item)
            if not isinstance(widget, QWidget):
                continue
            body_label = widget.findChild(QLabel, "messageBodyLabel")
            if body_label is None:
                continue
            body_width = max(available_width - 44, 120)
            body_label.setFixedWidth(body_width)
            widget.adjustSize()
            item.setSizeHint(widget.sizeHint())

    def _format_message_html(self, message: Message) -> str:
        ts = message.timestamp.astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")
        profile = self._user_profiles_by_id.get(message.author)
        author = profile.name if profile is not None else message.author[:8]
        escaped_author = html.escape(author)
        escaped_ts = html.escape(ts)
        escaped_content = html.escape(message.content).replace("\n", "<br/>")
        return (
            f"<span style='color:{COLOR_TEXT_MAIN}'><b>{escaped_author}</b></span> - "
            f"<i><span style='color:{COLOR_TEXT_MUTED}'>{escaped_ts}</span></i>"
            f"<br/><span style='color:{COLOR_TEXT_MAIN}'>{escaped_content}</span>"
        )

    def _send_clicked(self) -> None:
        if self._current_channel_id is None:
            self._status_label.setText(
                "Pick a channel first (create a community if you do not have one yet)."
            )
            return
        text = self._composer.toPlainText().strip()
        if not text:
            return
        try:
            message = self._api.send_message(self._current_channel_id, text)
        except AspenApiError as exc:
            self._show_error(str(exc))
            return
        self._composer.clear()
        self._state.upsert_message(message)
        self._refresh_messages()
        self._status_label.setText("Message sent")

    def _handle_event(self, payload: dict) -> None:
        if payload.get("serverEvent") == "userStatus":
            self._apply_user_status_event(payload)
            if self._current_community_id is not None:
                self._refresh_users_preview(self._current_community_id)
            return

        try:
            parsed = GeneratedServerEvent.model_validate(payload).root
        except Exception:
            return
        event_payload = parsed.model_dump(mode="json")
        changed = self._state.apply_server_event(event_payload)
        if not changed:
            return

        selected_community_id = self._selected_item_user_role(self._community_list)
        selected_channel_id = self._selected_item_user_role(self._channel_list)
        self._rebuild_community_list(preferred_community_id=selected_community_id)
        selected_community_id = self._selected_item_user_role(self._community_list)
        if selected_community_id is not None:
            self._rebuild_channel_list(
                community_id=selected_community_id,
                preferred_channel_id=selected_channel_id,
            )
            self._refresh_users_preview(selected_community_id)
        self._refresh_messages()

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
        try:
            existing = self._state.get_channels_for_community(community_id)
            created = self._api.create_channel(
                community_id=community_id,
                name=channel_name,
                sort_index=len(existing),
            )
            self._state.upsert_channel(created)
            self._rebuild_channel_list(
                community_id=community_id,
                preferred_channel_id=created.id,
            )
            self._status_label.setText(f"Created channel #{created.name}.")
        except AspenApiError as exc:
            self._show_error(str(exc))

    def _refresh_clicked(self) -> None:
        selected_community_id = self._selected_item_user_role(self._community_list)
        selected_channel_id = self._selected_item_user_role(self._channel_list)
        try:
            communities = self._api.read_user_communities()
            self._state.set_communities(communities)
            self._rebuild_community_list(preferred_community_id=selected_community_id)
            selected_community_id = self._selected_item_user_role(self._community_list)
            if selected_community_id is not None:
                channels = self._api.read_community_channels(selected_community_id)
                self._state.set_channels(channels)
                self._rebuild_channel_list(
                    community_id=selected_community_id,
                    preferred_channel_id=selected_channel_id,
                )
            self._status_label.setText("Refreshed communities/channels.")
        except AspenApiError as exc:
            self._show_error(str(exc))

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
        try:
            created = self._api.create_community(community_name)
            self._state.upsert_community(created)
            self._rebuild_community_list(preferred_community_id=created.id)
            self._status_label.setText(f"Created community '{created.name}'.")
        except AspenApiError as exc:
            self._show_error(str(exc))

    def _rebuild_community_list(self, preferred_community_id: str | None) -> None:
        self._community_list.blockSignals(True)
        self._community_avatar_list.blockSignals(True)
        self._community_list.clear()
        self._community_avatar_list.clear()
        for community in self._state.get_communities_sorted():
            item = QListWidgetItem(community.name)
            item.setIcon(self._community_avatar_icon(community))
            item.setData(Qt.ItemDataRole.UserRole, community.id)
            self._community_list.addItem(item)
            avatar_item = QListWidgetItem(self._community_avatar_icon(community), "")
            avatar_item.setData(Qt.ItemDataRole.UserRole, community.id)
            avatar_item.setToolTip(community.name)
            avatar_item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
            avatar_item.setSizeHint(QSize(28, 36))
            self._community_avatar_list.addItem(avatar_item)
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
        for channel in self._state.get_channels_for_community(community_id):
            item = QListWidgetItem(f"#{channel.name}")
            item.setData(Qt.ItemDataRole.UserRole, channel.id)
            self._channel_list.addItem(item)
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
        missing_authors = {
            message.author for message in messages if message.author not in self._user_profiles_by_id
        }
        for author_id in missing_authors:
            try:
                self._user_profiles_by_id[author_id] = self._api.read_user_profile(author_id)
                stale_keys = [key for key in self._user_avatar_cache if key.startswith(f"{author_id}:")]
                for key in stale_keys:
                    self._user_avatar_cache.pop(key, None)
            except AspenApiError:
                # Keep fallback short-id rendering when user lookup fails.
                continue

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
        if self._current_community_id is None:
            self._status_label.setText("Select a community first.")
            return
        try:
            code = self._api.create_invite(self._current_community_id)
        except AspenApiError as exc:
            self._show_error(str(exc))
            return
        self._status_label.setText(f"Invite created: {code}")
        QMessageBox.information(self, "Invite Created", f"Invite code: {code}")

    def _refresh_users_preview(self, community_id: str) -> None:
        self._users_preview_list.clear()
        try:
            users = self._api.read_community_users(community_id)
        except AspenApiError:
            return
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
        cache_key = f"{user_id}:{size}"
        if cache_key in self._user_avatar_cache:
            return self._user_avatar_cache[cache_key]

        profile = self._user_profiles_by_id.get(user_id)
        if profile is not None and profile.icon is not None:
            try:
                icon_bytes = self._api.read_icon_bytes(profile.icon)
                avatar = self._circular_pixmap_from_bytes(icon_bytes, size)
                self._user_avatar_cache[cache_key] = avatar
                return avatar
            except AspenApiError:
                pass

        fallback_name = profile.name if profile is not None else user_id
        avatar = self._fallback_user_avatar_pixmap(user_id, fallback_name, size)
        self._user_avatar_cache[cache_key] = avatar
        return avatar

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
        if community.icon is not None and community.icon in self._community_icon_cache:
            return self._community_icon_cache[community.icon]
        if community.icon is not None:
            try:
                icon_bytes = self._api.read_icon_bytes(community.icon)
                pixmap = QPixmap()
                if pixmap.loadFromData(icon_bytes):
                    icon = QIcon(
                        pixmap.scaled(
                            QSize(28, 28),
                            Qt.AspectRatioMode.KeepAspectRatioByExpanding,
                            Qt.TransformationMode.SmoothTransformation,
                        )
                    )
                    self._community_icon_cache[community.icon] = icon
                    return icon
            except AspenApiError:
                pass
        return self._fallback_community_avatar_icon(community.id, community.name)

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

    def closeEvent(self, event) -> None:  # type: ignore[override]
        self._events.stop()
        self._api.close()
        super().closeEvent(event)

    def resizeEvent(self, event) -> None:  # type: ignore[override]
        super().resizeEvent(event)
        self._apply_collapsed_sidebar_ratio()
        self._apply_expanded_sidebar_layout()
        self._resize_message_item_widgets()
