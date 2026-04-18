from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Callable, TypeVar

import httpx
from PySide6.QtCore import QObject, QRunnable, QThreadPool, Signal

from aspen_client.config import ClientConfig
from aspen_client.generated import openapi_models as gen
from aspen_client.types import Channel, Community, LoginSession, Message, UserProfile

T = TypeVar("T")


class AspenApiError(Exception):
    pass


class AspenApiClient:
    def __init__(self, config: ClientConfig) -> None:
        self._config = config
        self._client = httpx.Client(
            base_url=config.api_base_url,
            verify=config.verify_tls,
            timeout=15.0,
        )
        self._session: LoginSession | None = None

    @property
    def session(self) -> LoginSession | None:
        return self._session

    def close(self) -> None:
        self._client.close()

    def login(self, username: str, password: str) -> LoginSession:
        command = gen.Login(
            username=username,
            password=password,
        )
        response = self._client.post(
            "/login",
            json=_model_to_json(command),
        )
        data = self._expect_json(response)
        login_response = gen.LoginResponse.model_validate(data).root
        status = getattr(login_response, "status", None)
        if status != "ok":
            if status == "invalidCredentials":
                raise AspenApiError("Invalid credentials")
            raise AspenApiError("Login failed")
        session = LoginSession(
            user_id=str(login_response.user_id.root),
            refresh_token=login_response.refresh_token,
            session_token=login_response.session_token,
            session_token_expires=login_response.session_token_expires,
        )
        self._session = session
        return session

    def create_user(self, username: str, password: str) -> str:
        command = gen.UserCreateCommand(
            name=username,
            password=password,
            icon=None,
        )
        response = self._client.post(
            "/user",
            json=_model_to_json(command),
        )
        data = self._expect_json(response)
        parsed = gen.UserCreateCommandResponse.model_validate(data).root
        if hasattr(parsed, "createOk"):
            return str(parsed.createOk.id.root)
        if hasattr(parsed, "error"):
            cause = parsed.error.cause
            if cause:
                raise AspenApiError(str(cause))
        raise AspenApiError("Failed to create user")

    def logout(self) -> None:
        if self._session is None:
            return
        response = self._authorized_request(
            "POST",
            "/logout",
            gen.Logout(refreshToken=self._session.refresh_token),
        )
        data = self._expect_json(response)
        if data.get("status") not in {"ok", "invalidToken"}:
            raise AspenApiError("Logout failed")
        self._session = None

    def read_user_communities(self) -> list[Community]:
        response = self._authorized_request("GET", "/user/communities", None)
        data = self._expect_json(response)
        parsed = gen.UserCommunitiesReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "communities"):
            raise AspenApiError("Failed to load communities")
        return [self._parse_community(item.model_dump()) for item in parsed.communities.data]

    def create_community(self, name: str) -> Community:
        response = self._authorized_request(
            "POST",
            "/community",
            gen.CommunityCreateCommand(name=name, icon=None),
        )
        data = self._expect_json(response)
        parsed = gen.CommunityCreateCommandResponse.model_validate(data).root
        if hasattr(parsed, "createOk"):
            return self._parse_community(parsed.createOk.model_dump())
        if hasattr(parsed, "error"):
            cause = parsed.error.cause
            if cause:
                raise AspenApiError(str(cause))
        raise AspenApiError("Failed to create community")

    def read_community_users(self, community_id: str) -> list[UserProfile]:
        response = self._authorized_request(
            "GET",
            "/community/users",
            gen.CommunityUsersReadCommand(community=community_id),
        )
        data = self._expect_json(response)
        parsed = gen.CommunityUsersReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "users"):
            raise AspenApiError("Failed to load community users")
        return [
            UserProfile(
                id=str(user.id.root),
                name=str(user.name),
                icon=(str(user.icon.root) if user.icon is not None else None),
            )
            for user in parsed.users.data
        ]

    def create_invite(self, community_id: str) -> str:
        response = self._authorized_request(
            "POST",
            "/invite",
            gen.InviteCreateCommand(
                community=community_id,
                customCode=None,
                expiresAt=None,
            ),
        )
        data = self._expect_json(response)
        parsed = gen.InviteCreateCommandResponse.model_validate(data).root
        if hasattr(parsed, "invite"):
            return str(parsed.invite.code)
        if parsed == "codeAlreadyTaken":
            raise AspenApiError("Invite code collision, try again")
        if hasattr(parsed, "error"):
            cause = parsed.error.cause
            if cause:
                raise AspenApiError(str(cause))
        raise AspenApiError("Failed to create invite")

    def read_community_channels(self, community_id: str) -> list[Channel]:
        response = self._authorized_request(
            "GET",
            "/community/channels",
            gen.CommunityChannelsReadCommand(community=community_id),
        )
        data = self._expect_json(response)
        parsed = gen.CommunityChannelsReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "channels"):
            raise AspenApiError("Failed to load channels")
        channels = [self._parse_channel(item.model_dump()) for item in parsed.channels.data]
        return sorted(channels, key=lambda c: c.sort_index)

    def read_user_name(self, user_id: str) -> str:
        return self.read_user_profile(user_id).name

    def read_user_profile(self, user_id: str) -> UserProfile:
        response = self._authorized_request(
            "GET",
            "/user",
            gen.UserReadCommand(id=user_id),
        )
        data = self._expect_json(response)
        parsed = gen.UserReadCommandResponse.model_validate(data).root
        if hasattr(parsed, "user"):
            return UserProfile(
                id=str(parsed.user.id.root),
                name=str(parsed.user.name),
                icon=(str(parsed.user.icon.root) if parsed.user.icon is not None else None),
            )
        raise AspenApiError("Failed to read user")

    def create_channel(self, community_id: str, name: str, sort_index: int = 0) -> Channel:
        response = self._authorized_request(
            "POST",
            "/channel",
            gen.ChannelCreateCommand(
                name=name,
                sortIndex=sort_index,
                ty=gen.ChannelType.Text,
                community=community_id,
                parentCategory=None,
            ),
        )
        data = self._expect_json(response)
        parsed = gen.ChannelCreateCommandResponse.model_validate(data).root
        if hasattr(parsed, "createOk"):
            return self._parse_channel(parsed.createOk.model_dump())
        if hasattr(parsed, "error"):
            cause = parsed.error.cause
            if cause:
                raise AspenApiError(str(cause))
        raise AspenApiError("Failed to create channel")

    # Max UUID anchor used when the caller wants the most recent page with no
    # prior context; the server's Before query treats this as "return the
    # newest N messages" since every real UUID v7 sorts below it.
    _MAX_UUID_ANCHOR = "ffffffff-ffff-ffff-ffff-ffffffffffff"

    def read_channel_messages(
        self,
        channel_id: str,
        *,
        before: str | None = None,
        after: str | None = None,
        around: str | None = None,
        count: int = 50,
    ) -> list[Message]:
        """Read a bounded page of messages in a channel.

        Exactly zero or one of ``before`` / ``after`` / ``around`` may be set;
        when all are ``None`` the request defaults to the most recent page
        (``before = max UUID``). ``count`` is clamped to the server's [1, 200]
        range to match ``MAX_MESSAGES_QUERIED`` in the Rust handler.
        """
        anchors_set = sum(1 for anchor in (before, after, around) if anchor is not None)
        if anchors_set > 1:
            raise AspenApiError("read_channel_messages accepts at most one of before/after/around")

        clamped_count = max(1, min(count, 200))

        if after is not None:
            view = gen.ChannelViewDescription(
                root=gen.ChannelViewDescription2(
                    adjective="after",
                    message=after,
                    count=clamped_count,
                )
            )
        elif around is not None:
            view = gen.ChannelViewDescription(
                root=gen.ChannelViewDescription3(
                    adjective="around",
                    message=around,
                    radius=clamped_count,
                )
            )
        else:
            anchor = before if before is not None else self._MAX_UUID_ANCHOR
            view = gen.ChannelViewDescription(
                root=gen.ChannelViewDescription1(
                    adjective="before",
                    message=anchor,
                    count=clamped_count,
                )
            )

        response = self._authorized_request(
            "GET",
            "/channel/messages",
            gen.ChannelMessagesReadCommand(
                channel=channel_id,
                viewDescription=view,
            ),
        )
        data = self._expect_json(response)
        parsed = gen.ChannelMessagesReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "messages"):
            raise AspenApiError("Failed to load messages")
        messages = [self._parse_message(item.model_dump()) for item in parsed.messages.data]
        return sorted(messages, key=lambda m: m.timestamp)

    def read_icon_bytes(self, icon_id: str) -> bytes:
        response = self._authorized_request(
            "GET",
            "/icon",
            gen.IconReadCommand(id=icon_id),
        )
        data = self._expect_json(response)
        parsed = gen.IconReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "icon"):
            raise AspenApiError("Failed to load icon")
        return bytes(int(value.root) for value in parsed.icon.data)

    def send_message(self, channel_id: str, content: str) -> Message:
        response = self._authorized_request(
            "POST",
            "/message",
            gen.MessageCreateCommand(
                channelId=channel_id,
                content=content,
                attachments=[],
            ),
        )
        data = self._expect_json(response)
        parsed = gen.MessageCreateCommandResponse.model_validate(data).root
        if not hasattr(parsed, "createOk"):
            raise AspenApiError("Failed to send message")
        return self._parse_message(parsed.createOk.model_dump())

    def _authorized_request(
        self,
        method: str,
        path: str,
        body: Any | None,
    ) -> httpx.Response:
        if self._session is None:
            raise AspenApiError("Not logged in")
        headers = {"Authorization": f"Token {self._session.session_token}"}
        kwargs: dict[str, Any] = {"headers": headers}
        if body is not None:
            kwargs["json"] = _model_to_json(body)
        response = self._client.request(method, path, **kwargs)
        return response

    @staticmethod
    def _expect_json(response: httpx.Response) -> dict[str, Any]:
        if response.status_code >= 400:
            raise AspenApiError(f"HTTP {response.status_code}: {response.text}")
        data = response.json()
        if not isinstance(data, dict):
            raise AspenApiError("Unexpected response payload")
        return data

    @staticmethod
    def _parse_community(data: dict[str, Any]) -> Community:
        return Community(
            id=str(data["id"]),
            name=str(data["name"]),
            icon=(str(data["icon"]) if data.get("icon") is not None else None),
        )

    @staticmethod
    def _parse_channel(data: dict[str, Any]) -> Channel:
        community = data.get("community")
        parent_category = data.get("parentCategory")
        return Channel(
            id=str(data["id"]),
            name=str(data["name"]),
            ty=str(data["ty"]),
            community=(str(community) if community is not None else None),
            parent_category=(str(parent_category) if parent_category is not None else None),
            sort_index=int(data["sortIndex"]),
        )

    @staticmethod
    def _parse_message(data: dict[str, Any]) -> Message:
        return Message(
            id=str(data["id"]),
            author=str(data["author"]),
            channel_id=str(data["channelId"]),
            timestamp=_parse_dt(str(data["timestamp"])),
            content=str(data["content"]),
            attachments=[str(value) for value in data.get("attachments", [])],
        )


def _parse_dt(value: str) -> datetime:
    if value.endswith("Z"):
        value = f"{value[:-1]}+00:00"
    dt = datetime.fromisoformat(value)
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt


def _model_to_json(value: Any) -> Any:
    if hasattr(value, "model_dump"):
        return value.model_dump(mode="json", by_alias=True, exclude_none=False)
    return value


class AsyncApiCaller(QObject):
    """The one and only way to invoke ``AspenApiClient`` from the GUI layer.

    Every blocking HTTP call in the client must be submitted through an
    instance of this class. The operation runs on a ``QThreadPool`` worker
    thread, and the result (or exception) is delivered to a callback on the
    GUI thread via a queued Qt signal. This means:

    * The GUI event loop never blocks on network I/O, so keystrokes,
      scrolling, repaints, and window management stay responsive regardless
      of how slow or unresponsive the server is.
    * Callbacks always fire on the thread that owns this ``QObject`` (the
      thread it was constructed on, normally the GUI thread), so callers
      can safely mutate widgets and ``ClientState`` from inside them.

    ``AspenApiClient`` uses ``httpx.Client`` under the hood, which is
    documented as safe to share across threads, so a single ``AspenApiClient``
    instance backs all concurrent ``submit`` calls.
    """

    # Carries a zero-arg callable to invoke on the GUI thread. Using a
    # single ``object`` signal lets us submit arbitrary operations without
    # having to declare a new typed signal per call site, which kept the
    # old ``MessagePageFetcher`` pattern from scaling to the rest of the
    # API surface.
    _deliver = Signal(object)

    # Upper bound on how long ``shutdown()`` will wait for already-running
    # HTTP tasks to finish. Kept small so the GUI exit path stays snappy;
    # the underlying httpx client will be closed immediately afterwards,
    # which causes any request still in flight to unwind with an error
    # rather than running out its full 15s request timeout.
    _SHUTDOWN_WAIT_MS = 200

    def __init__(self, api: AspenApiClient, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._api = api
        # Dedicated pool (rather than ``QThreadPool.globalInstance()``) so
        # shutdown can clear the queue and bound the wait without
        # disturbing unrelated Qt subsystems that might also be using the
        # global pool.
        self._pool = QThreadPool(self)
        self._stopped = False
        # Queued delivery: emit is invoked from worker threads, but the
        # connected slot runs on the thread that owns ``self``.
        self._deliver.connect(self._invoke)

    def submit(
        self,
        operation: Callable[[AspenApiClient], T],
        on_success: Callable[[T], None],
        on_error: Callable[[Exception], None] | None = None,
    ) -> None:
        """Run ``operation(api)`` on a worker thread.

        On completion, ``on_success(result)`` is called on the GUI thread.
        On any exception from ``operation``, ``on_error(exc)`` is called on
        the GUI thread (or the exception is re-raised into the GUI event
        loop if ``on_error`` is ``None`` — tests will catch that, users
        should not).

        After ``shutdown()`` has been called this is a no-op, so late
        callers during teardown don't resurrect work on a closed HTTP
        client.
        """
        if self._stopped:
            return
        self._pool.start(_AsyncApiTask(self, operation, on_success, on_error))

    def shutdown(self) -> None:
        """Stop accepting new work and bound the wait on in-flight tasks.

        Queued-but-not-yet-running runnables are dropped outright; tasks
        that were already executing are given a brief window to finish so
        their callbacks aren't orphaned mid-flight. Whether or not they
        finish inside that window we return; the GUI thread must not be
        held hostage to network latency on the way out.
        """
        if self._stopped:
            return
        self._stopped = True
        self._pool.clear()
        self._pool.waitForDone(self._SHUTDOWN_WAIT_MS)

    def _enqueue(self, callback: Callable[[], None]) -> None:
        # Suppress late GUI-thread callbacks once we've begun tearing down,
        # so slots can't touch widgets that are already being destroyed.
        if self._stopped:
            return
        self._deliver.emit(callback)

    @staticmethod
    def _invoke(callback: object) -> None:
        if callable(callback):
            callback()


class _AsyncApiTask(QRunnable):
    """QRunnable body for one submitted ``AsyncApiCaller`` operation."""

    def __init__(
        self,
        caller: AsyncApiCaller,
        operation: Callable[[AspenApiClient], Any],
        on_success: Callable[[Any], None],
        on_error: Callable[[Exception], None] | None,
    ) -> None:
        super().__init__()
        self._caller = caller
        self._operation = operation
        self._on_success = on_success
        self._on_error = on_error

    def run(self) -> None:  # type: ignore[override]
        try:
            result = self._operation(self._caller._api)
        except Exception as exc:  # noqa: BLE001 - surfaced to GUI callback
            on_error = self._on_error
            if on_error is None:
                # Nothing to route to; re-raise on the GUI thread so the
                # error is at least visible in logs rather than silently
                # swallowed in a worker.
                self._caller._enqueue(lambda exc=exc: _reraise(exc))
                return
            self._caller._enqueue(lambda exc=exc: on_error(exc))
            return
        on_success = self._on_success
        self._caller._enqueue(lambda result=result: on_success(result))


def _reraise(exc: BaseException) -> None:
    raise exc
