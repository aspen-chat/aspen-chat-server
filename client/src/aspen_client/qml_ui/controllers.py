"""``QObject`` controllers that bridge Qt Quick QML to the Aspen layers.

The controllers are the only objects on the Quick path that hold a
reference to :class:`AspenApiClient` or :class:`TaskSpawner`. Every
``@Slot`` is the entry point QML uses to ask for some work; each one
dispatches through ``self._tasks.run(...)`` exactly the way
:class:`ChatWindow` does on the Widgets path \u2014 GUI slots stay
synchronous, and resumption after ``await`` lands on the GUI thread
(qasync drives the asyncio loop on top of Qt's loop) so result
callbacks may freely touch the models.

Three controllers in total:

* :class:`LoginController` owns the login form's busy / status state and
  the ``login`` / ``createUser`` API calls.
* :class:`ChatController` owns the four caches (``IconCache``,
  ``UserDirectory``, ``LinkPreviewImageCache``, plus the trio of list
  models), routes ``EventStreamClient`` signals into ``ClientState``
  and the models, and holds the active community/channel selection.
* :class:`MessagePaneController` owns the per-channel paged-read
  dedupe, the ``_image_subscribers`` map for preview thumbnails,
  ``sendMessage``, the jump-to-latest reset, and ``openLink`` (which
  enforces the ``_SAFE_LINK_SCHEMES`` allowlist).

The bookkeeping is intentionally a near-line-for-line port from
:mod:`aspen_client.ui` and :mod:`aspen_client.ui_messages`; the only
change is that "patch this widget" calls are replaced by "tell the
model to mark this row dirty".
"""

from __future__ import annotations

from typing import Any, TYPE_CHECKING

from PySide6.QtCore import (
    Property,
    QObject,
    QUrl,
    Signal,
    Slot,
)
from PySide6.QtGui import QDesktopServices, QGuiApplication

from aspen_client.api_client import AspenApiClient, TaskSpawner
from aspen_client.event_client import EventStreamClient
from aspen_client.generated.event_models import ServerEvent as GeneratedServerEvent
from aspen_client.icons import IconCache
from aspen_client.link_preview import LinkPreviewImageCache
from aspen_client.state import ClientState
from aspen_client.types import Channel, Community, LinkPreview, Message, UserProfile
from aspen_client.ui_messages import (
    INITIAL_MESSAGE_LOAD,
    MESSAGE_PAGE_SIZE,
    _SAFE_LINK_SCHEMES,
)
from aspen_client.user_directory import UserDirectory

from aspen_client.qml_ui.models import (
    ChannelListModel,
    CommunityListModel,
    MessageListModel,
    UserListModel,
)

if TYPE_CHECKING:
    from aspen_client.qml_ui.image_provider import AspenImageProvider


# ---------- Login ----------

class LoginController(QObject):
    """Owns the login form's API calls and exposes busy/status to QML."""

    # Emitted once the ``/login`` round-trip completes successfully so
    # ``Main.qml`` can swap the ``StackView`` to ``ChatView``.
    loggedIn = Signal()
    busyChanged = Signal()
    statusChanged = Signal()

    def __init__(
        self,
        api: AspenApiClient,
        tasks: TaskSpawner,
        chat_controller: "ChatController",
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._api = api
        self._tasks = tasks
        self._chat = chat_controller
        self._busy = False
        self._status = ""

    @Property(bool, notify=busyChanged)
    def busy(self) -> bool:
        return self._busy

    @Property(str, notify=statusChanged)
    def status(self) -> str:
        return self._status

    def _set_busy(self, value: bool) -> None:
        if value == self._busy:
            return
        self._busy = value
        self.busyChanged.emit()

    def _set_status(self, value: str) -> None:
        if value == self._status:
            return
        self._status = value
        self.statusChanged.emit()

    @Slot(str, str)
    def login(self, username: str, password: str) -> None:
        cleaned_user = username.strip()
        if not cleaned_user or not password:
            self._set_status("Username and password are required.")
            return
        self._set_busy(True)
        self._set_status("Working\u2026")
        self._tasks.run(
            self._api.login(cleaned_user, password),
            on_success=lambda _session: self._on_login_succeeded(),
            on_failure=self._on_login_failed,
        )

    @Slot(str, str)
    def createUser(self, username: str, password: str) -> None:  # noqa: N802
        cleaned_user = username.strip()
        if not cleaned_user or not password:
            self._set_status(
                "Enter username and password, then click Create User."
            )
            return
        self._set_busy(True)
        self._set_status("Working\u2026")
        self._tasks.run(
            self._api.create_user(cleaned_user, password),
            on_success=lambda _user_id: self._on_user_created(),
            on_failure=self._on_create_user_failed,
        )

    def _on_login_succeeded(self) -> None:
        self._set_busy(False)
        self._set_status("Connected")
        # ``ChatController.start`` mirrors ``ChatWindow._on_login_succeeded``:
        # it kicks off ``EventStreamClient.start`` and the initial
        # communities load. Doing this from the login controller keeps
        # the chain local to the success path.
        self._chat.start()
        self.loggedIn.emit()

    def _on_login_failed(self, exc: Exception) -> None:
        self._set_busy(False)
        self._set_status(str(exc))

    def _on_user_created(self) -> None:
        self._set_busy(False)
        self._set_status("User created. You can now log in.")

    def _on_create_user_failed(self, exc: Exception) -> None:
        self._set_busy(False)
        self._set_status(f"Create user failed: {exc}")


# ---------- Chat shell ----------

class ChatController(QObject):
    """Top-level controller for the chat shell.

    Owns the caches (``IconCache``, ``UserDirectory``,
    ``LinkPreviewImageCache``), the three id-keyed list models, the
    selection ids (``currentCommunityId`` / ``currentChannelId``), and
    the routing of ``EventStreamClient`` signals into ``ClientState``
    and the models. Sub-features (the message pane) read state through
    the ``messagePane`` child controller.
    """

    statusChanged = Signal()
    currentCommunityChanged = Signal()
    currentChannelChanged = Signal()
    sidebarCollapsedChanged = Signal()
    # Surfaced from a successful ``createInvite`` so QML can pop the
    # invite-code dialog. The status bar is a poor place for a code
    # the user is expected to copy verbatim, so we route the payload
    # through a dedicated signal instead of stringifying it into the
    # status line.
    inviteCreated = Signal(str)

    def __init__(
        self,
        api: AspenApiClient,
        events: EventStreamClient,
        tasks: TaskSpawner,
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._api = api
        self._events = events
        self._tasks = tasks

        self._state = ClientState()
        self._current_community_id: str | None = None
        self._current_channel_id: str | None = None
        self._status = "Not connected"
        self._sidebar_collapsed = True

        # Models exposed to QML via context properties.
        self._communities = CommunityListModel(self)
        self._channels = ChannelListModel(self)
        self._users = UserListModel(self)

        self._user_directory = UserDirectory(
            api,
            tasks,
            on_profile_loaded=self._on_user_profile_loaded,
        )
        self._icons = IconCache(
            api,
            tasks,
            on_user_icon_ready=self._on_user_icon_ready,
            on_community_icon_ready=self._on_community_icon_ready,
        )
        self._link_preview_images = LinkPreviewImageCache(
            api,
            tasks,
            on_image_ready=self._on_link_preview_image_ready,
        )

        # Single message model rebound on channel switch (rather than
        # one per channel) because QML ``ListView`` doesn't reuse
        # delegates across model swaps cheaply enough to make a
        # per-channel pool worthwhile, and the cached window in
        # ``ClientState`` already gives us instant rebind without a
        # network round-trip.
        self._message_model = MessageListModel(
            self._state,
            author_name_resolver=self._author_display_name,
            parent=self,
        )

        self._message_pane = MessagePaneController(
            api=api,
            tasks=tasks,
            state=self._state,
            message_model=self._message_model,
            user_directory=self._user_directory,
            link_preview_images=self._link_preview_images,
            chat=self,
            parent=self,
        )

        # Will be wired by ``app.quick_main`` once the image provider is
        # constructed; the provider holds a reference to ``ClientState``
        # for the community fallback render and needs to be re-pointed
        # whenever ``_reset_client_state`` swaps the state instance.
        self._image_provider: "AspenImageProvider | None" = None
        # ``app.quick_main`` hands us a shutdown trigger that runs the
        # async cleanup on the qasync loop and sets ``app_close_event``
        # while the loop is still running. ``requestShutdown`` is
        # ``Main.qml``'s ``onClosing`` hook; it must be a no-op until
        # the trigger is attached, which is guaranteed by construction
        # order in ``quick_main``.
        self._shutdown_request: "callable | None" = None

        # Wire the WebSocket signals exactly as ``ChatWindow`` does. The
        # connection order is preserved: ``state_resync_required`` is
        # connected before ``connected`` so Qt's queued delivery runs
        # the resync slot first when both fire on the same reconnect.
        self._events.event_received.connect(self._handle_event)
        self._events.connection_lost.connect(self._on_event_stream_lost)
        self._events.state_resync_required.connect(self._on_state_resync_required)
        self._events.connected.connect(self._on_event_stream_connected)

    # ---------- properties exposed to QML ----------

    @Property(QObject, constant=True)
    def communities(self) -> CommunityListModel:
        return self._communities

    @Property(QObject, constant=True)
    def channels(self) -> ChannelListModel:
        return self._channels

    @Property(QObject, constant=True)
    def users(self) -> UserListModel:
        return self._users

    @Property(QObject, constant=True)
    def messageModel(self) -> MessageListModel:  # noqa: N802
        return self._message_model

    @Property(QObject, constant=True)
    def messagePane(self) -> "MessagePaneController":  # noqa: N802
        return self._message_pane

    @Property(str, notify=statusChanged)
    def status(self) -> str:
        return self._status

    @Property(str, notify=currentCommunityChanged)
    def currentCommunityId(self) -> str:  # noqa: N802
        return self._current_community_id or ""

    @Property(str, notify=currentChannelChanged)
    def currentChannelId(self) -> str:  # noqa: N802
        return self._current_channel_id or ""

    @Property(bool, notify=sidebarCollapsedChanged)
    def sidebarCollapsed(self) -> bool:  # noqa: N802
        return self._sidebar_collapsed

    # ---------- public lifecycle ----------

    def attach_image_provider(self, provider: "AspenImageProvider") -> None:
        """Hand-off from ``app.quick_main`` after construction."""
        self._image_provider = provider

    def attach_shutdown_request(self, request_shutdown) -> None:
        """Hand-off the QML-callable shutdown trigger from ``app.quick_main``."""
        self._shutdown_request = request_shutdown

    @Slot()
    def requestShutdown(self) -> None:  # noqa: N802 - QML naming
        """Request an orderly shutdown.

        Invoked from ``Main.qml``'s ``onClosing`` handler the first
        time the user closes the window, and from main's signal
        handlers on SIGINT/SIGTERM. The trigger is a Python callable
        installed by ``app.quick_main``; calling before attachment is
        a no-op (the engine isn't fully wired yet).
        """
        if self._shutdown_request is not None:
            self._shutdown_request()

    def start(self) -> None:
        """Kick off the post-login bootstrap. Mirrors ``ChatWindow._on_login_succeeded``."""
        self._events.start(self._tasks)
        self._set_status("Connected")
        self._dispatch_initial_communities_load(preferred_community_id=None)

    # ---------- QML-callable slots ----------

    @Slot(str)
    def selectCommunity(self, community_id: str) -> None:  # noqa: N802
        if not community_id:
            self._current_community_id = None
            self.currentCommunityChanged.emit()
            self._users.clear()
            return
        self._current_community_id = community_id
        self.currentCommunityChanged.emit()
        preferred_channel_id = self._current_channel_id
        self._tasks.run(
            self._api.read_community_channels(community_id),
            on_success=lambda channels: self._on_community_channels_loaded(
                community_id, channels, preferred_channel_id
            ),
            on_failure=lambda exc: self._set_status(
                f"Failed to load channels: {exc}"
            ),
        )
        self._refresh_users_preview(community_id)

    @Slot(str)
    def selectChannel(self, channel_id: str) -> None:  # noqa: N802
        if not channel_id:
            self._current_channel_id = None
            self.currentChannelChanged.emit()
            self._message_pane.set_active_channel(None)
            return
        self._current_channel_id = channel_id
        self.currentChannelChanged.emit()
        self._message_pane.set_active_channel(channel_id)

    @Slot(str)
    def createCommunity(self, name: str) -> None:  # noqa: N802
        community_name = name.strip()
        if not community_name:
            self._set_status("Community name cannot be empty.")
            return
        self._tasks.run(
            self._api.create_community(community_name),
            on_success=self._on_community_created,
            on_failure=lambda exc: self._set_status(f"Create community failed: {exc}"),
        )

    @Slot(str)
    def createChannel(self, name: str) -> None:  # noqa: N802
        community_id = self._current_community_id
        if community_id is None:
            self._set_status("Select a community first.")
            return
        channel_name = name.strip()
        if not channel_name:
            self._set_status("Channel name cannot be empty.")
            return
        sort_index = len(self._state.get_channels_for_community(community_id))
        self._tasks.run(
            self._api.create_channel(
                community_id=community_id,
                name=channel_name,
                sort_index=sort_index,
            ),
            on_success=lambda channel: self._on_channel_created(community_id, channel),
            on_failure=lambda exc: self._set_status(f"Create channel failed: {exc}"),
        )

    @Slot()
    def createInvite(self) -> None:  # noqa: N802
        community_id = self._current_community_id
        if community_id is None:
            self._set_status("Select a community first.")
            return
        self._tasks.run(
            self._api.create_invite(community_id),
            on_success=self._on_invite_created,
            on_failure=lambda exc: self._set_status(f"Create invite failed: {exc}"),
        )

    def _on_invite_created(self, code: str) -> None:
        # Status bar gets a short confirmation; the dialog (driven by
        # the signal below) is what actually surfaces the copyable code.
        self._set_status("Invite code created.")
        self.inviteCreated.emit(str(code))

    @Slot(str)
    def copyToClipboard(self, text: str) -> None:  # noqa: N802
        """Place ``text`` on the system clipboard.

        Routed through Python rather than a pure-QML clipboard binding
        so the same path works on every platform PySide6 supports
        without depending on a particular ``QtCore`` minor version's
        QML clipboard module. ``QGuiApplication.clipboard()`` returns
        the application-wide clipboard which, on platforms with
        multiple selection buffers (X11), targets the standard
        ``Mode.Clipboard`` buffer that ``Ctrl+V`` reads from.
        """
        clipboard = QGuiApplication.clipboard()
        if clipboard is not None:
            clipboard.setText(text)

    @Slot()
    def refresh(self) -> None:
        selected_community_id = self._current_community_id
        selected_channel_id = self._current_channel_id
        self._tasks.run(
            self._api.read_user_communities(),
            on_success=lambda communities: self._on_refresh_communities_loaded(
                communities, selected_community_id, selected_channel_id
            ),
            on_failure=lambda exc: self._set_status(f"Refresh failed: {exc}"),
        )

    @Slot(bool)
    def setSidebarCollapsed(self, collapsed: bool) -> None:  # noqa: N802
        if collapsed == self._sidebar_collapsed:
            return
        self._sidebar_collapsed = collapsed
        self.sidebarCollapsedChanged.emit()

    # ---------- private bootstrap ----------

    def _dispatch_initial_communities_load(
        self, preferred_community_id: str | None
    ) -> None:
        self._tasks.run(
            self._api.read_user_communities(),
            on_success=lambda communities: self._on_initial_communities_loaded(
                communities, preferred_community_id
            ),
            on_failure=lambda exc: self._set_status(
                f"Failed to load communities: {exc}"
            ),
        )

    def _on_initial_communities_loaded(
        self,
        communities: list[Community],
        preferred_community_id: str | None,
    ) -> None:
        self._state.set_communities(communities)
        self._communities.replace_all(self._state.get_communities_sorted())
        if not communities:
            self._set_status(
                "Connected. No communities yet \u2014 click Create Community to start chatting."
            )
            return
        # Mirror ``ChatWindow``'s "auto-select the first community after
        # bootstrap" semantics. ``preferred_community_id`` wins if it
        # still exists in the freshly-loaded set; otherwise we fall
        # back to the first community in the sorted list.
        target = preferred_community_id
        if target is None or self._communities.index_for(target) is None:
            target = self._communities.items()[0].id if self._communities.items() else None
        if target is not None:
            self.selectCommunity(target)

    def _on_refresh_communities_loaded(
        self,
        communities: list[Community],
        preferred_community_id: str | None,
        preferred_channel_id: str | None,
    ) -> None:
        self._state.set_communities(communities)
        self._communities.replace_all(self._state.get_communities_sorted())
        target = preferred_community_id
        if target is None or self._communities.index_for(target) is None:
            target = self._communities.items()[0].id if self._communities.items() else None
        if target is None:
            self._set_status("Refreshed communities.")
            return
        # Carry the preferred channel id through the channel-load via a
        # captured closure, identical to ``ChatWindow._on_refresh_*``.
        self._current_community_id = target
        self.currentCommunityChanged.emit()
        self._refresh_users_preview(target)
        self._tasks.run(
            self._api.read_community_channels(target),
            on_success=lambda channels: self._on_refresh_channels_loaded(
                target, channels, preferred_channel_id
            ),
            on_failure=lambda exc: self._set_status(
                f"Failed to load channels: {exc}"
            ),
        )

    def _on_refresh_channels_loaded(
        self,
        community_id: str,
        channels: list[Channel],
        preferred_channel_id: str | None,
    ) -> None:
        self._state.set_channels(channels)
        self._channels.replace_all(
            self._state.get_channels_for_community(community_id)
        )
        target = preferred_channel_id
        if target is None or self._channels.index_for(target) is None:
            items = self._channels.items()
            target = items[0].id if items else None
        if target is not None:
            self.selectChannel(target)
        self._set_status("Refreshed communities/channels.")

    def _on_community_channels_loaded(
        self,
        community_id: str,
        channels: list[Channel],
        preferred_channel_id: str | None,
    ) -> None:
        # Drop stale responses if the user switched community while the
        # fetch was in flight. Same equality-check guard as
        # ``ChatWindow._on_community_channels_loaded``.
        if community_id != self._current_community_id:
            return
        self._state.set_channels(channels)
        self._channels.replace_all(
            self._state.get_channels_for_community(community_id)
        )
        target = preferred_channel_id
        if target is None or self._channels.index_for(target) is None:
            items = self._channels.items()
            target = items[0].id if items else None
        if target is not None:
            self.selectChannel(target)
        else:
            self.selectChannel("")

    def _on_community_created(self, community: Community) -> None:
        self._state.upsert_community(community)
        self._communities.upsert(community)
        self._set_status(f"Created community '{community.name}'.")
        self.selectCommunity(community.id)

    def _on_channel_created(self, community_id: str, channel: Channel) -> None:
        self._state.upsert_channel(channel)
        if community_id == self._current_community_id:
            self._channels.upsert(channel)
            self.selectChannel(channel.id)
        self._set_status(f"Created channel #{channel.name}.")

    def _refresh_users_preview(self, community_id: str) -> None:
        self._users.clear()
        self._tasks.run(
            self._api.read_community_users(community_id),
            on_success=lambda users: self._on_community_users_loaded(
                community_id, users
            ),
            on_failure=lambda exc: self._set_status(
                f"Failed to load users: {exc}"
            ),
        )

    def _on_community_users_loaded(
        self, community_id: str, users: list[UserProfile]
    ) -> None:
        if community_id != self._current_community_id:
            return
        for user in users:
            self._user_directory.upsert_profile(user)
        self._users.replace_all(users)

    # ---------- WebSocket event routing ----------

    def _handle_event(self, payload: dict) -> None:
        server_event = payload.get("serverEvent")

        if server_event == "userStatus":
            self._apply_user_status_event(payload)
            return

        try:
            parsed = GeneratedServerEvent.model_validate(payload).root
        except Exception:  # noqa: BLE001 - unrecognised events are dropped
            return

        if server_event == "messageLinkPreviewsReady":
            self._handle_message_link_previews_ready(parsed)
            return

        changed = self._state.apply_server_event(parsed)
        if not changed:
            return
        record_id = str(getattr(parsed, "id", ""))
        if not record_id:
            return
        event_type = str(getattr(parsed, "type", ""))
        if server_event == "community":
            self._handle_community_event(record_id, event_type)
        elif server_event == "channel":
            self._handle_channel_event(record_id, event_type)
        elif server_event == "message":
            self._handle_message_event(record_id, event_type)

    def _handle_community_event(self, community_id: str, event_type: str) -> None:
        if event_type == "delete":
            self._communities.remove(community_id)
            return
        community = self._state.communities.get(community_id)
        if community is None:
            return
        self._communities.upsert(community)
        # Auto-select the first community when the list goes from empty
        # to non-empty, matching ``ChatWindow._handle_community_event``.
        if (
            self._current_community_id is None
            and len(self._communities.items()) == 1
        ):
            self.selectCommunity(community_id)

    def _handle_channel_event(self, channel_id: str, event_type: str) -> None:
        if event_type == "delete":
            self._channels.remove(channel_id)
            return
        channel = self._state.channels.get(channel_id)
        if channel is None:
            return
        if channel.community != self._current_community_id:
            self._channels.remove(channel_id)
            return
        was_empty = len(self._channels.items()) == 0
        self._channels.upsert(channel)
        if was_empty and self._current_channel_id is None:
            self.selectChannel(channel_id)

    def _handle_message_event(self, message_id: str, event_type: str) -> None:
        if event_type == "delete":
            self._message_pane.handle_delete(message_id)
            return
        message = self._state.messages.get(message_id)
        if message is None:
            return
        if message.channel_id != self._current_channel_id:
            return
        self._message_pane.handle_create_or_update(message, event_type)

    def _handle_message_link_previews_ready(self, parsed: Any) -> None:
        message_id = str(getattr(parsed, "messageId", ""))
        channel_id = str(getattr(parsed, "channelId", ""))
        if not message_id or not channel_id:
            return
        raw_previews = getattr(parsed, "previews", None)
        if raw_previews is None:
            return
        previews = [
            LinkPreview.model_validate(p.model_dump(mode="json"))
            for p in raw_previews
        ]
        # The model writes through to ``ClientState.apply_link_previews_ready``
        # internally; this is the single line that mirrors
        # ``ChatWindow._handle_message_link_previews_ready``.
        if channel_id == self._current_channel_id:
            self._message_model.replace_previews(message_id, previews)
        else:
            self._state.apply_link_previews_ready(message_id, previews)

    def _apply_user_status_event(self, payload: dict) -> None:
        user_id = payload.get("id")
        if user_id is None:
            return
        status = str(payload.get("status", "")).strip().lower()
        if status not in {"online", "offline", "away"}:
            return
        self._user_directory.set_status(str(user_id), status)
        self._users.set_status(str(user_id), status)

    # ---------- reconnect / resync ----------

    def _on_event_stream_lost(self, reason: str) -> None:
        self._set_status(f"Disconnected \u2014 reconnecting\u2026 ({reason})")

    def _on_event_stream_connected(self) -> None:
        if self._status.startswith("Disconnected") or self._status.startswith(
            "Reconnected"
        ):
            self._set_status("Connected")

    def _on_state_resync_required(self) -> None:
        self._set_status("Reconnected \u2014 refreshing state\u2026")
        preferred_community_id = self._current_community_id
        self._reset_client_state()
        self._dispatch_initial_communities_load(
            preferred_community_id=preferred_community_id
        )

    def _reset_client_state(self) -> None:
        """Drop every cache derived from server state.

        Mirrors ``ChatWindow._reset_client_state`` line-for-line; the
        only differences are that the four ``KeyedListWidget``s are
        replaced by the four list models, and the message-pane state
        wipe goes through ``MessageListModel.set_state`` /
        ``MessagePaneController.clear_for_resync`` instead of touching
        a ``QListWidget``.
        """
        new_state = ClientState()
        self._state = new_state
        self._user_directory.clear()
        self._icons.clear()
        self._link_preview_images.clear()
        self._communities.clear()
        self._channels.clear()
        self._users.clear()
        self._message_model.set_state(new_state)
        self._message_pane.bind_state(new_state)
        if self._image_provider is not None:
            self._image_provider.set_state(new_state)

    # ---------- icon / profile ready callbacks ----------

    def _on_user_icon_ready(self, user_id: str) -> None:
        # ``IconCache`` already stored the bytes; bumping the per-user
        # avatar epoch is what triggers QML to re-resolve every visible
        # ``image://aspen/user/<id>/<size>`` URL through the provider.
        self._users.bump_icon_epoch(user_id)
        self._message_model.bump_avatar_epoch_for_user(user_id)

    def _on_community_icon_ready(self, community_id: str, icon_id: str) -> None:
        community = self._state.communities.get(community_id)
        if community is None or community.icon != icon_id:
            return
        self._communities.bump_icon_epoch(community_id)

    def _on_user_profile_loaded(self, user_id: str) -> None:
        # The avatar cache was keyed on the fallback pixmap; throw it
        # out so the next render re-runs through the real name and
        # kicks off an icon fetch for the profile's icon id (if any).
        self._icons.invalidate_user(user_id)
        self._users.bump_icon_epoch(user_id)
        self._message_model.bump_avatar_epoch_for_user(user_id)

    def _on_link_preview_image_ready(self, image_id: str) -> None:
        # Forward to the message pane controller, which knows the
        # subscriber set for this image id and bumps just the affected
        # rows.
        self._message_pane.handle_preview_image_ready(image_id)

    # ---------- helpers ----------

    def _author_display_name(self, user_id: str) -> str:
        profile = self._user_directory.get_profile(user_id)
        if profile is not None:
            return profile.name
        # Same short-id fallback the Widgets path uses while a profile
        # fetch is in flight.
        return user_id[:8]

    def _set_status(self, value: str) -> None:
        if value == self._status:
            return
        self._status = value
        self.statusChanged.emit()


# ---------- Message pane ----------

class MessagePaneController(QObject):
    """Owns the paged-read dedupe, scroll-anchor, and composer for the message pane."""

    hasOlderChanged = Signal()
    hasNewerChanged = Signal()
    activeChannelChanged = Signal()
    titleChanged = Signal()

    # Forwarded so QML can bind ``MessagePane.qml``'s top-anchor
    # restoration to the model's per-prepend signal.
    pagePrepended = Signal(int)

    # Emitted when the model just appended a live message to a tip-pinned
    # window so QML can scroll to the bottom (matches the
    # ``scrollToBottom`` after an append on the Widgets path).
    liveMessageAppended = Signal()

    def __init__(
        self,
        api: AspenApiClient,
        tasks: TaskSpawner,
        state: ClientState,
        message_model: MessageListModel,
        user_directory: UserDirectory,
        link_preview_images: LinkPreviewImageCache,
        chat: ChatController,
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._api = api
        self._tasks = tasks
        self._state = state
        self._model = message_model
        self._users = user_directory
        self._link_preview_images = link_preview_images
        self._chat = chat
        self._current_channel_id: str | None = None
        # Direction \u2192 ``True`` while a fetch is in flight. Mirrors
        # ``MessagePane._pending_fetches`` per-channel structure but
        # collapsed to the active channel since rebinds wipe it.
        self._pending: dict[str, set[str]] = {}
        # ``imageId`` \u2192 set of ``messageId`` whose currently-rendered
        # row is showing (or expecting) a preview thumbnail with that
        # id. Same shape as ``MessagePane._image_subscribers`` so a
        # landed thumbnail only repaints the rows that asked for it.
        self._image_subscribers: dict[str, set[str]] = {}
        # Forward the model's per-prepend signal so QML doesn't have to
        # bind to two separate objects to do scroll compensation.
        self._model.pagePrepended.connect(self.pagePrepended)

    # ---------- properties ----------

    @Property(bool, notify=hasOlderChanged)
    def hasOlder(self) -> bool:  # noqa: N802
        return self._model.has_older()

    @Property(bool, notify=hasNewerChanged)
    def hasNewer(self) -> bool:  # noqa: N802
        return self._model.has_newer()

    @Property(str, notify=activeChannelChanged)
    def activeChannelId(self) -> str:  # noqa: N802
        return self._current_channel_id or ""

    @Property(str, notify=titleChanged)
    def title(self) -> str:
        if self._current_channel_id is None:
            return "No channel selected"
        channel = self._state.channels.get(self._current_channel_id)
        if channel is None:
            return "Channel"
        return f"#{channel.name}"

    # ---------- public surface used by ChatController ----------

    def bind_state(self, state: ClientState) -> None:
        """Re-target after a ``_reset_client_state`` swap."""
        self._state = state
        self._current_channel_id = None
        self._pending.clear()
        self._image_subscribers.clear()
        # The model has already been pointed at the new state by the
        # caller (``ChatController._reset_client_state``); we just emit
        # the property notifications so QML rebinds.
        self.hasOlderChanged.emit()
        self.hasNewerChanged.emit()
        self.activeChannelChanged.emit()
        self.titleChanged.emit()

    def set_active_channel(self, channel_id: str | None) -> None:
        """Switch the visible channel.

        Cached windows are replayed without a network round-trip;
        cold channels trigger an initial fetch. Mirrors
        ``MessagePane.set_active_channel``.
        """
        self._current_channel_id = channel_id
        self._pending.clear()
        self._image_subscribers.clear()
        self._model.set_active_channel(channel_id)
        self.activeChannelChanged.emit()
        self.titleChanged.emit()
        self.hasOlderChanged.emit()
        self.hasNewerChanged.emit()
        if channel_id is None:
            return
        window = self._state.channel_windows.get(channel_id)
        if window is None or not window.ordered_ids:
            self._dispatch_message_fetch(channel_id, "initial", None, INITIAL_MESSAGE_LOAD)
        else:
            # Cached window: ensure profiles are populated and register
            # any preview subscriptions the rows imply.
            messages = self._state.get_messages_for_channel(channel_id)
            self._users.request_profiles(m.author for m in messages)
            for message in messages:
                self._register_preview_subscribers(message)

    def handle_create_or_update(self, message: Message, event_type: str) -> None:
        """Apply a live ``message`` event to the visible model."""
        # Pre-fetch the author profile so the row paints with the real
        # name on first render, mirroring ``MessagePane._append_message_row``.
        self._users.request_profiles([message.author])
        result = self._model.upsert_message(message)
        if result == "appended":
            self._register_preview_subscribers(message)
            self.liveMessageAppended.emit()
            self.hasOlderChanged.emit()
        elif result == "updated":
            # An update may have changed the link previews wholesale;
            # re-register subscribers so a thumbnail landing later
            # touches the right row. The dedupe inside
            # ``_register_preview_subscribers`` makes this safe to run
            # repeatedly.
            self._unregister_preview_subscribers(message.id)
            self._register_preview_subscribers(message)

    def handle_delete(self, message_id: str) -> None:
        # Drop subscriber bookkeeping before removing the row so a
        # late-arriving thumbnail can't try to paint a row Qt has
        # already collapsed.
        self._unregister_preview_subscribers(message_id)
        self._model.remove_message(message_id)

    def handle_preview_image_ready(self, image_id: str) -> None:
        subscribers = self._image_subscribers.get(image_id)
        if not subscribers:
            return
        # Bump each subscribed row's preview epoch by issuing a
        # targeted ``dataChanged([LinkPreviewsRole])`` on the model;
        # the previews themselves haven't changed, but
        # ``LinkPreviewsRole`` will re-emit fresh dicts and the
        # delegate's nested ``Image`` will re-resolve through the
        # provider, which now has the bytes.
        for message_id in list(subscribers):
            row = self._row_for_message(message_id)
            if row is None:
                subscribers.discard(message_id)
                continue
            model_index = self._model.createIndex(row, 0)
            self._model.dataChanged.emit(
                model_index,
                model_index,
                [MessageListModel.LinkPreviewsRole],
            )

    # ---------- QML-callable slots ----------

    @Slot(str)
    def sendMessage(self, content: str) -> None:  # noqa: N802
        channel_id = self._current_channel_id
        if channel_id is None:
            self._chat._set_status(  # noqa: SLF001 - intentional single status owner
                "Pick a channel first (create a community if you do not have one yet)."
            )
            return
        text = content.strip()
        if not text:
            return
        self._tasks.run(
            self._api.send_message(channel_id, text),
            on_success=self._on_message_sent,
            on_failure=lambda exc: self._on_message_send_failed(text, exc),
        )

    @Slot()
    def jumpToLatest(self) -> None:  # noqa: N802
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        # Same "drop the cached slice and re-fetch the tip" reset as
        # ``MessagePane._jump_to_latest_clicked``.
        self._model.clear_window()
        self._image_subscribers.clear()
        self._pending.clear()
        self._dispatch_message_fetch(channel_id, "initial", None, INITIAL_MESSAGE_LOAD)

    @Slot()
    def requestOlder(self) -> None:  # noqa: N802
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        if not self._model.has_older():
            return
        oldest = self._model.oldestMessageId()
        if not oldest:
            return
        self._dispatch_message_fetch(channel_id, "older", oldest, MESSAGE_PAGE_SIZE)

    @Slot()
    def requestNewer(self) -> None:  # noqa: N802
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        if not self._model.has_newer():
            return
        newest = self._model.newestMessageId()
        if not newest:
            return
        self._dispatch_message_fetch(channel_id, "newer", newest, MESSAGE_PAGE_SIZE)

    @Slot(str)
    def openLink(self, url: str) -> None:  # noqa: N802
        """Hand the URL to the OS default handler iff its scheme is allow-listed.

        Same allow-list (``http`` / ``https`` / ``mailto``) as the
        Widgets path; ``QDesktopServices`` is the same routing call
        ``MessagePane._open_message_link`` makes. Anchors that survive
        ``_disarm_misleading_links`` already have visible text matching
        their href modulo the GFM autolink prefixes, so this list is
        the second half of the safe-link contract.
        """
        parsed = QUrl(url)
        scheme = parsed.scheme().lower()
        if scheme not in _SAFE_LINK_SCHEMES:
            self._chat._set_status(  # noqa: SLF001
                f"Refused to open link with unsupported scheme '{scheme}': {url}"
            )
            return
        if not QDesktopServices.openUrl(parsed):
            self._chat._set_status(  # noqa: SLF001
                f"Failed to open link in default browser: {url}"
            )

    # ---------- internals ----------

    def _on_message_sent(self, message: Message) -> None:
        # Sliding-window invariant 5: if the user was reading older
        # history, their own message only makes sense as the new tip.
        # Drop the cached window and re-fetch so we don't end up with
        # a non-contiguous slice.
        window = self._state.channel_windows.get(message.channel_id)
        if window is not None and window.has_newer:
            if message.channel_id == self._current_channel_id:
                self.jumpToLatest()
            else:
                self._state.clear_channel_window(message.channel_id)
        else:
            self.handle_create_or_update(message, "create")
        self._chat._set_status("Message sent")  # noqa: SLF001

    def _on_message_send_failed(self, original_text: str, exc: Exception) -> None:
        # The composer field is owned by QML; we surface the failure via
        # the status bar. The user can re-paste their text manually if
        # they need to (the composer was cleared on send by the QML
        # ``sendMessage`` caller). ``original_text`` is preserved so a
        # future enhancement can write it back into the composer via a
        # new signal.
        del original_text
        self._chat._set_status(f"Send failed: {exc}")  # noqa: SLF001

    def _dispatch_message_fetch(
        self,
        channel_id: str,
        direction: str,
        anchor_id: str | None,
        count: int,
    ) -> None:
        in_flight = self._pending.setdefault(channel_id, set())
        if direction in in_flight:
            return
        in_flight.add(direction)
        if direction == "older":
            coro = self._api.read_channel_messages(
                channel_id, before=anchor_id, count=count
            )
        elif direction == "newer":
            coro = self._api.read_channel_messages(
                channel_id, after=anchor_id, count=count
            )
        else:
            coro = self._api.read_channel_messages(channel_id, count=count)
        self._tasks.run(
            coro,
            on_success=lambda messages: self._on_message_page_loaded(
                channel_id, direction, messages, count
            ),
            on_failure=lambda exc: self._on_message_page_failed(
                channel_id, direction, str(exc)
            ),
        )

    def _on_message_page_loaded(
        self,
        channel_id: str,
        direction: str,
        messages: list[Message],
        requested_count: int,
    ) -> None:
        in_flight = self._pending.get(channel_id)
        if in_flight is not None:
            in_flight.discard(direction)

        raw_count = len(messages)
        hit_end = raw_count < requested_count

        # Resolve author profiles for every fetched message before any
        # row is materialised so headers paint with real names on
        # first render.
        self._users.request_profiles(m.author for m in messages)

        if direction == "initial":
            if channel_id != self._current_channel_id:
                # Stale: the user switched channels mid-flight. Keep
                # the state side updated so the eventual return trip is
                # warm but don't touch the visible model.
                self._state.set_channel_window(
                    channel_id, messages, has_older=not hit_end, has_newer=False
                )
                return
            self._model.set_window(
                messages, has_older=not hit_end, has_newer=False
            )
            for message in messages:
                self._register_preview_subscribers(message)
        elif direction == "older":
            if channel_id != self._current_channel_id:
                self._state.merge_channel_page(channel_id, messages)
                window = self._state.channel_windows.get(channel_id)
                if window is not None and hit_end:
                    window.has_older = False
                return
            self._model.prepend_page(messages)
            window = self._state.channel_windows.get(channel_id)
            if window is not None and hit_end:
                window.has_older = False
            for message in messages:
                self._register_preview_subscribers(message)
            self._model.evict_newer_to_cap()
        elif direction == "newer":
            if channel_id != self._current_channel_id:
                self._state.merge_channel_page(channel_id, messages)
                window = self._state.channel_windows.get(channel_id)
                if window is not None and hit_end:
                    window.has_newer = False
                return
            self._model.append_page(messages)
            window = self._state.channel_windows.get(channel_id)
            if window is not None and hit_end:
                window.has_newer = False
            for message in messages:
                self._register_preview_subscribers(message)
            self._model.evict_older_to_cap()

        self.hasOlderChanged.emit()
        self.hasNewerChanged.emit()

    def _on_message_page_failed(
        self, channel_id: str, direction: str, error: str
    ) -> None:
        in_flight = self._pending.get(channel_id)
        if in_flight is not None:
            in_flight.discard(direction)
        if channel_id == self._current_channel_id:
            self._chat._set_status(f"Message fetch failed: {error}")  # noqa: SLF001

    # ---------- preview subscribers ----------

    def _register_preview_subscribers(self, message: Message) -> None:
        for preview in message.link_previews:
            if preview.image_id is None:
                continue
            self._image_subscribers.setdefault(preview.image_id, set()).add(message.id)
            # Pull the bytes through the cache if we haven't yet.
            # ``request_image`` is a no-op once the entry is settled
            # (or already in flight), so calling it on every subscribe
            # is safe and keeps the dedupe inside the cache.
            if not self._link_preview_images.has_settled(preview.image_id):
                self._link_preview_images.request_image(preview.image_id)

    def _unregister_preview_subscribers(self, message_id: str) -> None:
        for image_id, subscribers in list(self._image_subscribers.items()):
            subscribers.discard(message_id)
            if not subscribers:
                self._image_subscribers.pop(image_id, None)

    def _row_for_message(self, message_id: str) -> int | None:
        # Linear scan; the model has at most ``MESSAGE_WINDOW_CAP``
        # rows (500) so this stays trivial. Keeping the lookup off the
        # model keeps the controller's preview-subscriber path
        # self-contained.
        count = self._model.count()
        for row in range(count):
            if self._model.messageIdAt(row) == message_id:
                return row
        return None
