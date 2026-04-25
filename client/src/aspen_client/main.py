from __future__ import annotations

import asyncio
import os
import signal
import sys

import qasync
from PySide6.QtGui import QGuiApplication

from aspen_client.api_client import AspenApiClient
from aspen_client.config import ClientConfig
from aspen_client.event_client import EventStreamClient


def main() -> int:
    config = ClientConfig.from_env()
    ui_choice = os.getenv("ASPEN_UI", "widgets").strip().lower()
    if ui_choice == "quick":
        return _run_quick(config)
    return _run_widgets(config)


def _run_widgets(config: ClientConfig) -> int:
    """Run the Qt Widgets UI \u2014 the default, fully-featured path."""
    # Imported lazily so ``ASPEN_UI=quick`` runs don't pay the cost of
    # importing the entire Widgets stack (and its transitive ``qtawesome``
    # icon database) just to be discarded.
    from PySide6.QtWidgets import QApplication

    from aspen_client.ui import ChatWindow

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

    _install_signal_handlers(loop, lambda: window.close())

    # Block on the close event until ``ChatWindow._async_shutdown``
    # sets it. Setting from inside the async shutdown (rather than
    # wiring it to ``app.aboutToQuit``) guarantees the wake-up is
    # scheduled while the event loop is still fully running; by the
    # time Qt actually begins tearing itself down we've already
    # returned from ``run_until_complete``.
    with loop:
        loop.run_until_complete(app_close_event.wait())

    return 0


def _run_quick(config: ClientConfig) -> int:
    """Run the experimental Qt Quick UI; opt in via ``ASPEN_UI=quick``.

    Mirrors :func:`_run_widgets` line-for-line for the asyncio + qasync
    + signal-handler plumbing; the only divergence is that
    :class:`QGuiApplication` replaces :class:`QApplication` (the QML
    path doesn't pull in ``QtWidgets`` at all) and the post-construct
    handoff goes through :func:`aspen_client.qml_ui.app.quick_main`,
    which builds the QML engine and wires the controllers in.
    """
    # Same lazy import shape as the Widgets path so a Widgets-only run
    # never pays the cost of importing the QML controllers.
    from aspen_client.qml_ui.app import quick_main

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

    # SIGINT/SIGTERM go through the same async-cleanup path the QML
    # window's onClosing handler triggers; invoking ``app.quit``
    # directly fires ``aboutToQuit`` while qasync's loop is already
    # tearing down, which crashes ``app_close_event.set()`` (see the
    # comment in ``aspen_client.qml_ui.app``).
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
