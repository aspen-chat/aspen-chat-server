from __future__ import annotations

import signal
import socket
import sys

from PySide6.QtCore import QSocketNotifier
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

    # Qt's C-level event loop doesn't yield back to the Python interpreter
    # on its own, so Python signal handlers installed via ``signal.signal``
    # never get to fire during ``app.exec()`` unless something pokes the
    # interpreter.
    #
    # The canonical fix is ``signal.set_wakeup_fd``: the interpreter
    # writes one byte to a given file descriptor every time a signal is
    # delivered, and a ``QSocketNotifier`` watching the read end of a
    # socket pair wakes the Qt event loop synchronously to drain that
    # byte and dispatch the shutdown. This is strictly better than the
    # previous no-op QTimer because (a) there is zero steady-state wake
    # cost, and (b) signal-to-shutdown latency is bounded by the event
    # loop's next iteration rather than by a polling interval.
    read_sock, write_sock = socket.socketpair()
    read_sock.setblocking(False)
    write_sock.setblocking(False)
    # ``set_wakeup_fd`` requires a non-blocking fd and returns the
    # previously registered one; we don't use the old value but we must
    # keep the sockets alive for the lifetime of the process, hence the
    # closure captures below.
    signal.set_wakeup_fd(write_sock.fileno())

    notifier = QSocketNotifier(read_sock.fileno(), QSocketNotifier.Type.Read)

    def _drain_and_close(_fd: int) -> None:
        try:
            # Drain whatever the interpreter wrote; the actual signal has
            # already been dispatched to the Python-level handler below.
            while True:
                data = read_sock.recv(4096)
                if not data:
                    break
        except (BlockingIOError, InterruptedError):
            pass
        window.close()

    notifier.activated.connect(_drain_and_close)

    def _signal_handler(_signum: int, _frame: object) -> None:
        # The wakeup fd machinery above is what actually drives the
        # shutdown; this handler exists so the interpreter has something
        # to call (the default action for SIGINT is KeyboardInterrupt,
        # which we don't want to raise across the Qt boundary).
        pass

    signal.signal(signal.SIGINT, _signal_handler)
    signal.signal(signal.SIGTERM, _signal_handler)

    # Ensure the window's cleanup runs exactly once regardless of which
    # exit path trips: X button (closeEvent), signal (wakeup fd above),
    # or a programmatic ``QApplication.quit()``. ``ChatWindow.shutdown``
    # is idempotent.
    app.aboutToQuit.connect(window.shutdown)

    exit_code = app.exec()

    # Explicitly unregister the wakeup fd before the sockets drop out of
    # scope; otherwise the interpreter may attempt to write to a closed
    # fd if a signal arrives during interpreter teardown.
    signal.set_wakeup_fd(-1)
    notifier.setEnabled(False)
    read_sock.close()
    write_sock.close()

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
