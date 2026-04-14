from __future__ import annotations

import sys

from PySide6.QtWidgets import QApplication

from aspen_client.api_client import AspenApiClient
from aspen_client.config import ClientConfig
from aspen_client.event_client import EventStreamClient
from aspen_client.ui import ChatWindow


def main() -> int:
    config = ClientConfig.from_env()
    app = QApplication(sys.argv)
    api_client = AspenApiClient(config)
    event_client = EventStreamClient(config)
    window = ChatWindow(api_client, event_client)
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
