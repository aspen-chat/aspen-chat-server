from __future__ import annotations

import asyncio
import signal
import sys

import qasync
from PySide6.QtWidgets import QApplication

from aspen_client.api_client import AspenApiClient
from aspen_client.config import ClientConfig
from aspen_client.event_client import EventStreamClient
from aspen_client.ui import ChatWindow


def main() -> int:
    config = ClientConfig.from_env()
    app = QApplication(sys.argv)

    # ``qasync.QEventLoop`` is a full ``asyncio.AbstractEventLoop`` that
    # schedules callbacks on top of Qt's event loop. Unlike
    # ``PySide6.QtAsyncio``, it implements the network primitives
    # (``create_connection`` / ``getaddrinfo`` / readers / writers / ...)
    # that ``httpx.AsyncClient`` (via ``anyio``) and ``websockets`` both
    # require, so async HTTP requests and the event-stream WebSocket
    # issued from Qt slots actually work.
    loop = qasync.QEventLoop(app)
    asyncio.set_event_loop(loop)

    api_client = AspenApiClient(config)
    event_client = EventStreamClient(config)
    # ``app_close_event`` is the handshake between the window's async
    # shutdown path and ``main()``'s ``run_until_complete`` below.
    # ``ChatWindow._async_shutdown`` sets it at the end of cleanup,
    # which unblocks ``event.wait()`` while the event loop is still
    # running normally. Using ``aboutToQuit`` instead causes the
    # wake-up to be scheduled on an already-stopping loop and crashes
    # on exit; see the comment in ``_async_shutdown`` for the full
    # failure mode.
    app_close_event = asyncio.Event()
    window = ChatWindow(api_client, event_client, app_close_event)
    window.show()

    # Signal handling is routed through asyncio: ``add_signal_handler``
    # registers a Python-level callback that fires on the next loop
    # tick after the OS delivers the signal, which lands in the same
    # qasync-driven loop every HTTP/WebSocket coroutine runs on.
    # ``add_signal_handler`` is Unix-only; on Windows asyncio raises
    # ``NotImplementedError`` for it, so we fall back to a no-op
    # (Windows users close the window via the title-bar X, which
    # routes through ``ChatWindow.closeEvent`` either way).
    def _request_shutdown() -> None:
        # ``window.close`` triggers the async-aware shutdown path in
        # ``ChatWindow.closeEvent`` which awaits the http client and
        # spawned tasks before letting the window actually close.
        window.close()

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, _request_shutdown)
        except NotImplementedError:
            break

    # Block on the close event until ``ChatWindow._async_shutdown``
    # sets it. Setting from inside the async shutdown (rather than
    # wiring it to ``app.aboutToQuit``) guarantees the wake-up is
    # scheduled while the event loop is still fully running; by the
    # time Qt actually begins tearing itself down we've already
    # returned from ``run_until_complete``.
    with loop:
        loop.run_until_complete(app_close_event.wait())

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
