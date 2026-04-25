from __future__ import annotations

import asyncio
import json
import ssl
import time
from typing import TYPE_CHECKING

import tenacity
from PySide6.QtCore import QObject, Signal
from websockets.asyncio.client import ClientConnection, connect
from websockets.exceptions import (
    ConnectionClosed,
    InvalidHandshake,
    InvalidURI,
    WebSocketException,
)

from aspen_client.config import ClientConfig

if TYPE_CHECKING:
    from aspen_client.api_client import TaskSpawner


# Ping cadence for the server stream. The ping keeps idle NAT/proxy paths
# warm; the asyncio loop picks up cancellation at every yield, so a stop
# request is observed on the next ``await`` without needing the ping to
# "wake" the reader.
_PING_INTERVAL_SECONDS = 20.0
_PING_TIMEOUT_SECONDS = 10.0
# Upper bound on how long ``ClientConnection.close()`` will wait for the
# server's close-frame acknowledgement before forcibly dropping the TCP
# socket. The default is 10 seconds, which makes a post-login shutdown
# feel like the client has hung. We don't actually rely on the server
# acking the close (the shutdown path skips ``close()`` entirely; see
# ``_consume``'s ``finally`` below) but the bound applies to any other
# close path too \u2014 e.g. a server-initiated drop that's already handed us
# a stale connection \u2014 and one second is a humane ceiling.
_CLOSE_TIMEOUT_SECONDS = 1.0

# Reconnect policy. The server's NATS JetStream consumer is created with
# ``DeliverPolicy::ByStartTime { start_time: now - 60s }`` (see
# ``server/src/api/event_stream.rs``), so any client that reconnects within
# roughly 60 seconds of dropping will receive a replay of the events it
# missed in the gap. We pick 45 seconds for the client-side grace deadline
# to leave a safe margin inside that 60s replay window: if we manage to
# reconnect within 45s the in-memory client state is still consistent with
# the server (the replay catches us up); past that we can no longer assume
# the replay covers the whole gap, so on the eventual reconnect the UI
# must wipe its caches and re-bootstrap from REST.
_RECONNECT_GRACE_SECONDS = 45.0
# Backoff schedule for repeated connect attempts inside a single outage,
# delegated to ``tenacity.wait_exponential``. The first attempt fires with
# zero delay (tenacity does not wait before attempt #1), then attempts #2
# onward wait ``multiplier * 2 ** (attempt - 2)`` capped at ``max``,
# producing the canonical 0.5s, 1s, 2s, 4s, 5s, 5s, ... ladder.
_RECONNECT_INITIAL_BACKOFF_SECONDS = 0.5
_RECONNECT_MAX_BACKOFF_SECONDS = 5.0


class _ConnectFailed(Exception):
    """Raised inside the connect-retry loop when the handshake never completed."""


class EventStreamClient(QObject):
    """WebSocket event stream with automatic reconnection and a grace deadline.

    Lifecycle (driven by an asyncio coroutine spawned via ``TaskSpawner``):

    * On ``start(spawner)``, schedule the reader coroutine. The first
      connect attempt is always immediate; if it succeeds we're in steady
      state and just dispatch events. If it fails, we enter an outage:
      emit ``connection_lost``, then retry on a tenacity-managed
      exponential backoff (immediate, 0.5s, 1s, 2s, 4s, then 5s
      indefinitely) until the user requests shutdown.
    * If a previously-established connection drops, that's also an outage:
      same backoff schedule, again starting with an immediate retry.
    * As soon as a connect succeeds, emit ``connected``. If the outage
      duration exceeded ``_RECONNECT_GRACE_SECONDS``, *also* emit
      ``state_resync_required`` so the UI knows the server's replay
      window almost certainly didn't cover the whole gap and the cached
      state must be considered stale.

    Signal contract for the UI:

    * ``event_received(dict)`` -- one server event payload.
    * ``connection_lost(str)`` -- emitted exactly once at the start of
      each outage episode (initial-connect failure or post-connect drop).
      Carries a human-readable reason string for the status bar.
    * ``connected()`` -- emitted on every successful connect, including
      reconnects. The UI should treat this as "stream is live".
    * ``state_resync_required()`` -- emitted *just before* the next
      ``connected`` whenever the preceding outage exceeded the grace
      deadline. The UI should clear its in-memory caches and re-run the
      post-login bootstrap (read communities, reload current channel,
      etc.).
    """

    event_received = Signal(dict)
    connected = Signal()
    connection_lost = Signal(str)
    state_resync_required = Signal()

    def __init__(self, config: ClientConfig) -> None:
        super().__init__()
        self._config = config
        # The reader task itself; a single coroutine that owns the
        # connection and the reconnect loop. ``None`` until ``start``,
        # cancelled by ``stop``.
        self._task: asyncio.Task[None] | None = None
        # Cached on the loop thread (which is the GUI thread under
        # qasync) so any in-flight reads can be aborted promptly when
        # ``stop`` cancels the task: closing the websocket is the
        # documented way to make a pending ``recv`` raise.
        self._connection: ClientConnection | None = None
        self._stopping = False

    def start(self, spawner: "TaskSpawner") -> None:
        if self._task is not None and not self._task.done():
            return
        self._stopping = False
        self._task = spawner.spawn(self._run())

    def stop(self) -> None:
        """Request shutdown of the WebSocket reader and return immediately.

        Cancels the reader task on the asyncio loop. ``_run`` cooperates
        with cancellation at every ``await``; since the loop runs on the
        GUI thread (qasync), the cancellation is processed on the next
        event-loop tick and any in-flight ``recv`` unwinds via the
        task's ``CancelledError``.
        """
        self._stopping = True
        task = self._task
        if task is not None and not task.done():
            task.cancel()

    async def _run(self) -> None:
        # Outage bookkeeping. ``outage_start`` is the timestamp of the
        # disconnect (or initial-connect failure) that started the
        # current outage episode; reset to ``None`` after a successful
        # connect so the next disconnect is recognised as a fresh
        # outage.
        outage_start: float | None = None

        try:
            while not self._stopping:
                outage_start, connection = await self._connect_with_backoff(outage_start)
                if connection is None:
                    return

                outage_elapsed = (
                    time.monotonic() - outage_start
                    if outage_start is not None
                    else 0.0
                )
                # Snapshot taken just before the connect succeeded; if
                # the outage we just crawled out of exceeded the grace
                # window, the server's NATS replay buffer (60s by
                # ``MAX_EVENT_AGE``) is no longer a safe guarantee that
                # we'll catch up cleanly, so the UI must resync from
                # REST. Emit ``state_resync_required`` *before*
                # ``connected`` so the UI's resync handler sees an
                # accurate "live" state when it kicks off the
                # rebootstrap.
                if outage_elapsed > _RECONNECT_GRACE_SECONDS:
                    self.state_resync_required.emit()
                self.connected.emit()
                outage_start = None

                await self._consume(connection)

                if self._stopping:
                    return
                # Connection dropped after a successful handshake. Open
                # a fresh outage episode and tell the UI; the next
                # iteration's ``_connect_with_backoff`` immediately
                # tries to reconnect.
                outage_start = time.monotonic()
                self.connection_lost.emit("event stream connection lost")
        except asyncio.CancelledError:
            return

    async def _connect_with_backoff(
        self, outage_start: float | None
    ) -> tuple[float | None, ClientConnection | None]:
        """Retry the connect handshake until success (or shutdown).

        Returns the (possibly updated) ``outage_start`` and the live
        connection. Returns ``(_, None)`` if shutdown was requested
        mid-retry. Tenacity manages the wait schedule; the ``stop_never``
        policy mirrors the AGENTS.md rule that the user expects the
        client to come back if connectivity returns hours later, with no
        failure-count ceiling.
        """
        retrying = tenacity.AsyncRetrying(
            wait=tenacity.wait_exponential(
                multiplier=_RECONNECT_INITIAL_BACKOFF_SECONDS,
                max=_RECONNECT_MAX_BACKOFF_SECONDS,
            ),
            retry=tenacity.retry_if_exception_type(_ConnectFailed),
            stop=tenacity.stop_never,
            reraise=True,
        )
        async for attempt in retrying:
            with attempt:
                if self._stopping:
                    return outage_start, None
                try:
                    connection = await self._open_once()
                except _ConnectFailed:
                    if outage_start is None:
                        outage_start = time.monotonic()
                        # Emitted exactly once per outage episode (per
                        # the AGENTS.md signal contract); subsequent
                        # retries within the same episode just re-extend
                        # the tenacity backoff without re-emitting.
                        self.connection_lost.emit("failed to connect to event stream")
                    raise
                return outage_start, connection
        # Unreachable: ``stop_never`` plus ``reraise=True`` means this
        # loop only exits via ``return`` from inside ``with attempt``.
        return outage_start, None

    async def _open_once(self) -> ClientConnection:
        ssl_context: ssl.SSLContext | None = None
        if self._config.ws_url.startswith("wss://") and not self._config.verify_tls:
            ssl_context = ssl._create_unverified_context()  # noqa: SLF001
        try:
            connection = await connect(
                self._config.ws_url,
                ssl=ssl_context,
                ping_interval=_PING_INTERVAL_SECONDS,
                ping_timeout=_PING_TIMEOUT_SECONDS,
                close_timeout=_CLOSE_TIMEOUT_SECONDS,
            )
        except (
            OSError,
            InvalidURI,
            InvalidHandshake,
            WebSocketException,
            asyncio.TimeoutError,
        ) as exc:
            raise _ConnectFailed from exc
        self._connection = connection
        return connection

    async def _consume(self, connection: ClientConnection) -> None:
        try:
            async for message in connection:
                if self._stopping:
                    break
                if isinstance(message, (bytes, bytearray)):
                    try:
                        message = message.decode("utf-8")
                    except UnicodeDecodeError:
                        continue
                try:
                    payload = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if isinstance(payload, dict):
                    self.event_received.emit(payload)
        except ConnectionClosed:
            # Normal "the other side hung up" — caller treats it as a
            # drop and starts a fresh outage episode.
            return
        finally:
            # On the shutdown path we deliberately skip the graceful
            # close handshake. ``close()`` sends a close frame and then
            # waits for the server's close frame back (bounded by
            # ``close_timeout``), which used to make every post-login
            # shutdown stall for the full default 10 seconds before
            # the process could exit. The server detects the dropped
            # TCP socket within its own ping-timeout and reaps the
            # subscription either way, so politely closing on our way
            # out the door buys us nothing but a stalled UI.
            #
            # In every non-shutdown path (server-initiated drop,
            # transient I/O error) ``close()`` runs as before so we
            # release sockets cleanly between reconnect attempts.
            if not self._stopping:
                try:
                    await connection.close()
                except (OSError, WebSocketException):
                    # Already torn down; the outer loop will reconnect.
                    pass
            self._connection = None
