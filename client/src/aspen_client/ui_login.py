from __future__ import annotations

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import QLabel, QLineEdit, QPushButton, QVBoxLayout, QWidget

from aspen_client.icons import apply_button_icon
from aspen_client.theme import COLOR_BG_PANE, COLOR_TEXT_MAIN, COLOR_TEXT_MUTED


class LoginPage(QWidget):
    """Login / create-user form rendered before the chat page is shown.

    Owns its own input widgets, busy state, and status label, but delegates
    the actual API call back to the chat window via signals — that keeps
    the AGENTS.md ``TaskSpawner.spawn`` requirement next to the only object
    that holds the ``AspenApiClient`` reference.
    """

    login_requested = Signal(str, str)
    create_user_requested = Signal(str, str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        layout = QVBoxLayout(self)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)

        title = QLabel("Aspen Login", self)
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        title.setStyleSheet("font-size: 24px; font-weight: 600;")

        self._username_input = QLineEdit(self)
        self._username_input.setPlaceholderText("Username")
        self._username_input.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; color: {COLOR_TEXT_MAIN}; border: 1px solid {COLOR_TEXT_MUTED};"
        )

        self._password_input = QLineEdit(self)
        self._password_input.setPlaceholderText("Password")
        self._password_input.setEchoMode(QLineEdit.EchoMode.Password)
        self._password_input.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; color: {COLOR_TEXT_MAIN}; border: 1px solid {COLOR_TEXT_MUTED};"
        )

        self._login_button = QPushButton("Login", self)
        self._login_button.clicked.connect(self._emit_login)
        self._create_user_button = QPushButton("Create User", self)
        self._create_user_button.clicked.connect(self._emit_create_user)
        apply_button_icon(self._login_button, "login-variant")
        apply_button_icon(self._create_user_button, "account-plus-outline")

        self._status_label = QLabel("", self)
        self._status_label.setAlignment(Qt.AlignmentFlag.AlignCenter)

        box = QWidget(self)
        box_layout = QVBoxLayout(box)
        box_layout.addWidget(title)
        box_layout.addWidget(self._username_input)
        box_layout.addWidget(self._password_input)
        login_actions = QVBoxLayout()
        login_actions.setAlignment(Qt.AlignmentFlag.AlignHCenter)
        login_actions.addWidget(self._login_button, alignment=Qt.AlignmentFlag.AlignHCenter)
        login_actions.addWidget(self._create_user_button, alignment=Qt.AlignmentFlag.AlignHCenter)
        box_layout.addLayout(login_actions)
        box_layout.addWidget(self._status_label)
        box.setMaximumWidth(400)

        layout.addWidget(box, alignment=Qt.AlignmentFlag.AlignCenter)

    def set_busy(self, busy: bool) -> None:
        self._login_button.setEnabled(not busy)
        self._create_user_button.setEnabled(not busy)
        self._username_input.setEnabled(not busy)
        self._password_input.setEnabled(not busy)
        if busy:
            self._status_label.setText("Working...")

    def set_status(self, text: str) -> None:
        self._status_label.setText(text)

    def _emit_login(self) -> None:
        username = self._username_input.text().strip()
        password = self._password_input.text()
        if not username or not password:
            self._status_label.setText("Username and password are required.")
            return
        self.login_requested.emit(username, password)

    def _emit_create_user(self) -> None:
        username = self._username_input.text().strip()
        password = self._password_input.text()
        if not username or not password:
            self._status_label.setText("Enter username and password, then click Create User.")
            return
        self.create_user_requested.emit(username, password)
