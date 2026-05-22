"""Bootstrap for the Qt Quick UI; mirrors :mod:`aspen_client.main` plumbing.

:func:`quick_main` is invoked from :mod:`aspen_client.main` when the
``ASPEN_UI=quick`` environment variable selects the experimental Quick
path. It assumes the caller has already constructed the qasync event
loop, the API client, and the event-stream client \u2014 same construction
order as the Widgets path \u2014 and just hands them to the controllers
plus the QML engine.

The single subtlety is the shutdown handshake. The Widgets ``ChatWindow``
sets the ``app_close_event`` from inside ``closeEvent`` to keep the
``loop.run_until_complete(app_close_event.wait())`` line in ``main``
unblocked at exactly the right moment. Critically, ``app_close_event``
must be set **while the qasync loop is still running** \u2014 routing the
cleanup through ``QGuiApplication.aboutToQuit`` is too late: by the
time it fires Qt has already begun tearing the application down,
which marks the qasync loop as not-running, and any ``Event.set()``
scheduled after that point raises ``RuntimeError: loop ... is not
the running loop`` (the exact same failure mode documented in
:meth:`ChatWindow._async_shutdown`).

The Quick equivalent of ``QMainWindow.closeEvent`` is ``Window.onClosing``
in QML. :func:`quick_main` returns a ``request_shutdown`` callable the
caller installs on signal handlers; QML calls into the same coroutine
through ``chat.requestShutdown()`` from ``Main.qml``'s ``onClosing``
handler.
"""

from __future__ import annotations

import asyncio
import importlib.resources as resources
from pathlib import Path

from PySide6.QtCore import QUrl
from PySide6.QtGui import QGuiApplication
from PySide6.QtQml import QQmlApplicationEngine
from shiboken6 import delete as _shiboken_delete, isValid as _shiboken_is_valid

from aspen_client.api_client import AspenApiClient, TaskSpawner
from aspen_client.event_client import EventStreamClient

from aspen_client.qml_ui.controllers import ChatController, LoginController
from aspen_client.qml_ui.image_provider import AspenImageProvider
from aspen_client.qml_ui.markdown import MarkdownBridge
from aspen_client.qml_ui.theme import ThemeBridge


def quick_main(
    app: QGuiApplication,
    api: AspenApiClient,
    events: EventStreamClient,
    app_close_event: asyncio.Event,
) -> tuple[int, "callable"]:
    """Run the Quick UI; return ``(rc, request_shutdown)``.

    ``rc`` is ``0`` on a successful engine load, ``1`` on failure (no
    QML root objects). ``request_shutdown`` is a zero-argument callable
    the caller installs on signal handlers (SIGINT/SIGTERM) so a
    ``Ctrl+C`` triggers the same async-cleanup-then-set-event sequence
    QML's ``onClosing`` triggers from window-close events.

    ``app`` is the already-constructed :class:`QGuiApplication`; the
    asyncio loop is assumed to be installed by the caller (qasync is
    set up identically to the Widgets path because both paths share
    the same network-driven coroutines).
    """
    tasks = TaskSpawner()

    # Every QObject created here \u2014 the controllers, the bridges, and
    # the QML engine itself \u2014 is parented to ``app`` so Qt's
    # parent/child ownership keeps them alive for the lifetime of the
    # process. Without an explicit parent, the only thing holding a
    # reference is this function's local scope; once :func:`quick_main`
    # returns, Python drops those references, the QObjects are
    # destroyed, and the QML window vanishes from the screen before it
    # ever paints. Parenting to ``app`` is the same pattern the
    # Widgets path gets for free by keeping ``ChatWindow`` in
    # :func:`_run_widgets`'s local scope until ``run_until_complete``
    # returns.
    chat = ChatController(api=api, events=events, tasks=tasks)
    chat.setParent(app)
    login = LoginController(api=api, tasks=tasks, chat_controller=chat)
    login.setParent(app)

    # Provider and bridges are stateless w.r.t. login state but they
    # need to live as long as the engine. ``QQuickImageProvider`` is
    # not a ``QObject``; the engine takes ownership of it via
    # ``addImageProvider`` below, so it doesn't need parenting here.
    image_provider = AspenImageProvider(
        icons=chat._icons,  # noqa: SLF001 - controller intentionally exposes these caches
        users=chat._user_directory,  # noqa: SLF001
        link_previews=chat._link_preview_images,  # noqa: SLF001
        state=chat._state,  # noqa: SLF001
    )
    chat.attach_image_provider(image_provider)
    markdown_bridge = MarkdownBridge()
    markdown_bridge.setParent(app)
    theme = ThemeBridge()
    theme.setParent(app)

    engine = QQmlApplicationEngine()
    engine.setParent(app)
    # Image providers must be registered *before* the QML files that
    # reference ``image://aspen/...`` URLs are loaded, otherwise the
    # first paint pass returns the QML default placeholder and the
    # ``Image.source`` binding never re-runs to pick up the now-attached
    # provider.
    engine.addImageProvider("aspen", image_provider)

    root_context = engine.rootContext()
    root_context.setContextProperty("login", login)
    root_context.setContextProperty("chat", chat)
    root_context.setContextProperty("messagePane", chat.messagePane)
    root_context.setContextProperty("markdown", markdown_bridge)
    root_context.setContextProperty("theme", theme)

    # Shutdown handshake. Same shape as ``ChatWindow._async_shutdown``:
    # stop the event-stream reader synchronously, drain the spawned
    # tasks, close the HTTP client. The ``app_close_event.set()``
    # **must** happen while the qasync loop is still running (see the
    # module docstring for the failure mode if it doesn't).
    shutdown_state = {"started": False}

    async def _async_shutdown() -> None:
        try:
            events.stop()
            await tasks.shutdown()
            await api.aclose()
        finally:
            # Synchronously destroy the QML engine while the
            # context-property QObjects (``theme`` / ``login`` /
            # ``chat`` / ``markdown_bridge``) are still alive.
            #
            # If we leave the engine for Qt to clean up as part of
            # ``~QGuiApplication``, the destruction order of ``app``'s
            # children is unspecified (per Qt's QObject docs). When
            # ``theme`` or ``login`` happens to go first, every QML
            # binding that reads ``theme.bgMain`` / ``login.busy`` /
            # ``login.status`` re-evaluates against a now-null receiver
            # and prints ``TypeError: Cannot read property X of null``
            # to stderr \u2014 a dozen lines of teardown noise per exit.
            #
            # ``shiboken6.delete`` is the synchronous equivalent of
            # ``deleteLater`` + ``processEvents(DeferredDeletion)``;
            # it runs ``~QQmlApplicationEngine`` right here, which
            # tears down the QML scene and its bindings before any
            # context-property QObject is freed. ``isValid`` guards
            # against a double-call (the SIGINT handler and the
            # window-close handler can both reach this branch on a
            # racy exit; the second call would otherwise touch a
            # freed C++ object).
            if _shiboken_is_valid(engine):
                _shiboken_delete(engine)
            # Set the close event *after* the engine teardown but
            # *before* anything else that might pull the application
            # down. The Widgets path runs the equivalent
            # ``self._app_close_event.set()`` before ``self.close()``
            # for the same "wake main while the loop is still running"
            # reason.
            app_close_event.set()

    def request_shutdown() -> None:
        """Trigger the async cleanup; idempotent and safe from any thread."""
        if shutdown_state["started"]:
            return
        shutdown_state["started"] = True
        # ``ensure_future`` on the qasync loop puts the coroutine on
        # the same loop everything else runs on. Routing it through
        # ``TaskSpawner.spawn`` would cause ``TaskSpawner.shutdown``
        # to cancel the very coroutine driving the cleanup.
        asyncio.ensure_future(_async_shutdown())

    chat.attach_shutdown_request(request_shutdown)

    qml_root = _resolve_qml_root()
    engine.addImportPath(str(qml_root))
    engine.load(QUrl.fromLocalFile(str(qml_root / "Main.qml")))
    if not engine.rootObjects():
        # Mirror the Widgets path's behaviour on a fatal startup error
        # (``ChatWindow`` would have raised before ``app.exec``).
        return 1, request_shutdown

    return 0, request_shutdown


def _resolve_qml_root() -> Path:
    """Locate the ``qml/`` directory that ships with this package.

    Using ``importlib.resources.files`` keeps the lookup working both
    when the client is run from a source checkout (where ``qml/`` is a
    sibling directory of this module) and when it's installed from a
    wheel (where ``[tool.setuptools.package-data]`` ships the QML
    files alongside the package).
    """
    package_root = resources.files("aspen_client.qml_ui")
    qml_dir = package_root / "qml"
    # ``importlib.resources`` returns a Traversable; for a real
    # directory on disk this is a ``PosixPath`` we can pass to QML's
    # file URL helpers without copying.
    return Path(str(qml_dir))
