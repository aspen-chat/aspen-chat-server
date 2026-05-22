from __future__ import annotations

import asyncio
import signal
import sys
import logging

import qasync
from PySide6.QtGui import QGuiApplication

from aspen_client.api_client import AspenApiClient
from aspen_client.config import ClientConfig
from aspen_client.event_client import EventStreamClient
from aspen_client.qml_ui.app import quick_main

logger = logging.getLogger(__name__)

def main() -> int:
    config = ClientConfig.from_env()
    logging.basicConfig(filename='client.log', datefmt="%a, %d %b %Y %H:%M:%S +0000", level=logging.INFO)
    app = QGuiApplication(sys.argv)
    loop = qasync.QEventLoop(app)
    asyncio.set_event_loop(loop)

    api_client = AspenApiClient(config)
    event_client = EventStreamClient(config)
    app_close_event = asyncio.Event()

    rc, request_shutdown = quick_main(
        app, api_client, event_client, app_close_event
    )
    if rc != 0:
        return rc

    _install_signal_handlers(loop, request_shutdown)

    with loop:
        loop.run_until_complete(app_close_event.wait())

    return 0


def _install_signal_handlers(loop, request_shutdown) -> None:
    """Route SIGINT / SIGTERM through asyncio onto the GUI loop.

    ``add_signal_handler`` registers a Python-level callback that fires
    on the next loop tick after the OS delivers the signal, which
    lands in the same qasync-driven loop every HTTP/WebSocket
    coroutine runs on. ``add_signal_handler`` is Unix-only; on Windows
    asyncio raises ``NotImplementedError`` for it, so we fall back to
    a no-op (Windows users close the window via the title-bar X,
    which routes through the chosen UI's shutdown path either way).
    """
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, request_shutdown)
        except NotImplementedError:
            break


if __name__ == "__main__":
    raise SystemExit(main())
