from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMessageBox,
    QPushButton,
    QSplitter,
    QStackedWidget,
    QTextEdit,
    QVBoxLayout,
    QWidget,
)

from aspen_client.api_client import AspenApiClient, AspenApiError
from aspen_client.event_client import EventStreamClient
from aspen_client.generated.event_models import ServerEvent as GeneratedServerEvent
from aspen_client.state import ClientState
from aspen_client.types import Message


class ChatWindow(QMainWindow):
    def __init__(self, api: AspenApiClient, events: EventStreamClient) -> None:
        super().__init__()
        self._api = api
        self._events = events

        self._state = ClientState()
        self._current_channel_id: str | None = None
        self._user_names_by_id: dict[str, str] = {}

        self.setWindowTitle("Aspen Chat Client")
        self.resize(1100, 700)

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

        self._password_input = QLineEdit(root)
        self._password_input.setPlaceholderText("Password")
        self._password_input.setEchoMode(QLineEdit.EchoMode.Password)

        self._login_button = QPushButton("Login", root)
        self._login_button.clicked.connect(self._login_clicked)
        self._create_user_button = QPushButton("Create User", root)
        self._create_user_button.clicked.connect(self._create_user_clicked)

        self._login_status_label = QLabel("", root)
        self._login_status_label.setAlignment(Qt.AlignmentFlag.AlignCenter)

        box = QWidget(root)
        box_layout = QVBoxLayout(box)
        box_layout.addWidget(title)
        box_layout.addWidget(self._username_input)
        box_layout.addWidget(self._password_input)
        login_actions = QHBoxLayout()
        login_actions.addWidget(self._login_button)
        login_actions.addWidget(self._create_user_button)
        box_layout.addLayout(login_actions)
        box_layout.addWidget(self._login_status_label)
        box.setMaximumWidth(400)

        layout.addWidget(box, alignment=Qt.AlignmentFlag.AlignCenter)
        return root

    def _build_chat_page(self) -> QWidget:
        root = QWidget(self)
        layout = QVBoxLayout(root)

        header_row = QHBoxLayout()
        self._status_label = QLabel("Not connected", root)
        self._create_community_button = QPushButton("Create Community", root)
        self._create_community_button.clicked.connect(self._create_community_clicked)
        self._create_channel_button = QPushButton("Create Channel", root)
        self._create_channel_button.clicked.connect(self._create_channel_clicked)
        self._refresh_button = QPushButton("Refresh", root)
        self._refresh_button.clicked.connect(self._refresh_clicked)
        header_row.addWidget(self._status_label, 1)
        header_row.addWidget(self._refresh_button)
        header_row.addWidget(self._create_channel_button)
        header_row.addWidget(self._create_community_button)
        layout.addLayout(header_row)

        splitter = QSplitter(root)
        layout.addWidget(splitter)

        self._community_list = QListWidget(splitter)
        self._community_list.itemSelectionChanged.connect(self._community_changed)

        self._channel_list = QListWidget(splitter)
        self._channel_list.itemSelectionChanged.connect(self._channel_changed)

        right_panel = QWidget(splitter)
        right_layout = QVBoxLayout(right_panel)
        self._messages_list = QListWidget(right_panel)
        right_layout.addWidget(self._messages_list)

        composer_row = QHBoxLayout()
        self._composer = QTextEdit(right_panel)
        self._composer.setPlaceholderText("Write a message...")
        self._composer.setFixedHeight(90)
        send_button = QPushButton("Send", right_panel)
        send_button.clicked.connect(self._send_clicked)
        composer_row.addWidget(self._composer, 1)
        composer_row.addWidget(send_button)
        right_layout.addLayout(composer_row)

        splitter.setSizes([220, 260, 620])
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
            return
        community_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(community_id, str):
            return
        channels = self._api.read_community_channels(community_id)
        self._state.set_channels(channels)
        self._rebuild_channel_list(community_id=community_id, preferred_channel_id=self._current_channel_id)

    def _channel_changed(self) -> None:
        selected = self._channel_list.selectedItems()
        if not selected:
            return
        channel_id = selected[0].data(Qt.ItemDataRole.UserRole)
        if not isinstance(channel_id, str):
            return
        self._current_channel_id = channel_id
        if not self._state.get_messages_for_channel(channel_id):
            messages = self._api.read_channel_messages(channel_id)
            self._state.set_channel_messages(channel_id, messages)
        self._refresh_messages()

    def _refresh_messages(self) -> None:
        self._messages_list.clear()
        if self._current_channel_id is None:
            return
        messages = self._state.get_messages_for_channel(self._current_channel_id)
        self._resolve_author_names(messages)
        for message in messages:
            self._messages_list.addItem(self._format_message(message))
        self._messages_list.scrollToBottom()

    def _format_message(self, message: Message) -> str:
        ts = message.timestamp.astimezone().strftime("%H:%M:%S")
        author = self._user_names_by_id.get(message.author, message.author[:8])
        return f"[{ts}] {author}: {message.content}"

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
        self._community_list.clear()
        for community in self._state.get_communities_sorted():
            item = QListWidgetItem(community.name)
            item.setData(Qt.ItemDataRole.UserRole, community.id)
            self._community_list.addItem(item)
        if self._community_list.count() > 0:
            row = self._row_for_user_role(self._community_list, preferred_community_id)
            if row is None:
                row = 0
            self._community_list.setCurrentRow(row)
        self._community_list.blockSignals(False)
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
        self._channel_changed()

    def _resolve_author_names(self, messages: list[Message]) -> None:
        missing_authors = {message.author for message in messages if message.author not in self._user_names_by_id}
        for author_id in missing_authors:
            try:
                self._user_names_by_id[author_id] = self._api.read_user_name(author_id)
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

    def closeEvent(self, event) -> None:  # type: ignore[override]
        self._events.stop()
        self._api.close()
        super().closeEvent(event)
