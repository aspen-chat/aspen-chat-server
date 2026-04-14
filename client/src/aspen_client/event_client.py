from __future__ import annotations

import json
import ssl
import threading
from typing import Any

from PySide6.QtCore import QObject, Signal
import websocket

from aspen_client.config import ClientConfig


class EventStreamClient(QObject):
    event_received = Signal(dict)
    connection_error = Signal(str)
    connected = Signal()

    def __init__(self, config: ClientConfig) -> None:
        super().__init__()
        self._config = config
        self._ws: websocket.WebSocketApp | None = None
        self._thread: threading.Thread | None = None
        self._stop_requested = False

    def start(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop_requested = False
        self._thread = threading.Thread(target=self._run, name="event-stream", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop_requested = True
        if self._ws is not None:
            self._ws.close()
        if self._thread is not None:
            self._thread.join(timeout=2.0)

    def _run(self) -> None:
        self._ws = websocket.WebSocketApp(
            self._config.ws_url,
            on_open=lambda ws: self.connected.emit(),
            on_message=self._on_message,
            on_error=lambda ws, err: self._on_error(err),
            on_close=lambda ws, code, msg: self._on_close(code, msg),
        )
        sslopt: dict[str, Any] = {}
        if self._config.ws_url.startswith("wss://") and not self._config.verify_tls:
            sslopt = {
                "cert_reqs": ssl.CERT_NONE,
                "check_hostname": False,
            }
        self._ws.run_forever(sslopt=sslopt)

    def _on_message(self, _ws: websocket.WebSocketApp, message: str) -> None:
        try:
            payload = json.loads(message)
        except json.JSONDecodeError:
            return
        if isinstance(payload, dict):
            self.event_received.emit(payload)

    def _on_error(self, error: Any) -> None:
        if self._stop_requested:
            return
        self.connection_error.emit(str(error))

    def _on_close(self, code: int, msg: str) -> None:
        if self._stop_requested:
            return
        self.connection_error.emit(f"event stream closed ({code}): {msg}")
