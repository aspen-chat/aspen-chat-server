from __future__ import annotations

import json
import socket
import ssl
import threading
from typing import Any

from PySide6.QtCore import QObject, Signal
import websocket

from aspen_client.config import ClientConfig


# Ping cadence for the server stream. The ping itself keeps idle NAT/proxy
# paths warm, and more importantly it guarantees the reader thread wakes up
# periodically so it can notice ``keep_running`` being flipped off by
# ``stop()`` even if no server traffic is arriving.
_PING_INTERVAL_SECONDS = 20.0
_PING_TIMEOUT_SECONDS = 10.0


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
        """Request shutdown of the WebSocket reader and return immediately.

        The reader thread is a daemon and all of its Qt-emitting callbacks
        short-circuit when ``_stop_requested`` is set, so there is no
        correctness reason to block the caller (the GUI thread) while the
        worker winds down. Historically we did a 2-second ``join``; under
        TLS the blocked ``recv()`` does not reliably unblock from a
        cross-thread ``ws.close()``, so that join nearly always hit the
        full timeout on every shutdown.

        Instead, we flip the stop flag, ask the library to close cleanly,
        and then force the underlying socket into half-shutdown so the
        blocked ``recv()`` returns immediately. The daemon thread is then
        either torn down promptly on its own or reaped by the interpreter
        at process exit; either way the GUI thread is no longer held up.
        """
        self._stop_requested = True
        ws = self._ws
        if ws is None:
            return
        try:
            ws.close()
        except Exception:
            # ``close`` can raise if the socket is already gone; we are
            # tearing down regardless, so swallow it.
            pass
        sock = getattr(ws, "sock", None)
        underlying = getattr(sock, "sock", None)
        if underlying is not None:
            try:
                underlying.shutdown(socket.SHUT_RDWR)
            except OSError:
                # Socket may already be closed or half-shut; that is the
                # state we wanted anyway.
                pass

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
        self._ws.run_forever(
            sslopt=sslopt,
            ping_interval=_PING_INTERVAL_SECONDS,
            ping_timeout=_PING_TIMEOUT_SECONDS,
        )

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
