from __future__ import annotations

import asyncio
from typing import Any, Callable, Coroutine, TypeVar

import httpx

from aspen_client.config import ClientConfig
from aspen_client.generated import openapi_models as gen
from aspen_client.types import (
    Channel,
    Community,
    Icon,
    LoginSession,
    Message,
    UserProfile,
)


class AspenApiError(Exception):
    pass


T = TypeVar("T")


def _unwrap_create_response(
    parsed: Any,
    ok_field: str,
    parser: Callable[[dict[str, Any]], T],
    default_msg: str,
) -> T:
    """Collapse the repeated ``createOk`` / ``error`` discriminated-union shape.

    The server's CRUD command responses are unions of: an ``ok``-style variant
    carrying the created record, a ``notAllowed`` variant, and an ``error``
    variant carrying a localised cause string. The handful of fields differs
    per entity, but the unwrap dance does not — every caller wants ``parser``
    applied to the JSON-mode ``model_dump`` of the ``ok`` payload if present,
    the error cause raised if not, and ``default_msg`` raised as a last
    resort. JSON mode is required because the generated ``ok`` payloads
    carry UUID-typed fields and the client-side records expect plain
    strings.
    """
    if hasattr(parsed, ok_field):
        return parser(getattr(parsed, ok_field).model_dump(mode="json"))
    if hasattr(parsed, "error"):
        cause = parsed.error.cause
        if cause:
            raise AspenApiError(str(cause))
    raise AspenApiError(default_msg)


class AspenApiClient:
    def __init__(self, config: ClientConfig) -> None:
        self._config = config
        self._client = httpx.AsyncClient(
            base_url=config.api_base_url,
            verify=config.verify_tls,
            timeout=15.0,
        )
        self._session: LoginSession | None = None

    @property
    def session(self) -> LoginSession | None:
        return self._session

    async def aclose(self) -> None:
        await self._client.aclose()

    async def login(self, username: str, password: str) -> LoginSession:
        command = gen.Login(
            username=username,
            password=password,
        )
        response = await self._client.post(
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

    async def create_user(self, username: str, password: str) -> str:
        command = gen.UserCreateCommand(
            name=username,
            password=password,
            icon=None,
        )
        response = await self._client.post(
            "/user",
            json=_model_to_json(command),
        )
        data = self._expect_json(response)
        parsed = gen.UserCreateCommandResponse.model_validate(data).root
        return _unwrap_create_response(
            parsed,
            "createOk",
            lambda payload: str(payload["id"]),
            "Failed to create user",
        )

    async def logout(self) -> None:
        if self._session is None:
            return
        response = await self._authorized_request(
            "POST",
            "/logout",
            gen.Logout(refreshToken=self._session.refresh_token),
        )
        data = self._expect_json(response)
        if data.get("status") not in {"ok", "invalidToken"}:
            raise AspenApiError("Logout failed")
        self._session = None

    async def read_user_communities(self) -> list[Community]:
        response = await self._authorized_request("GET", "/user/communities", None)
        data = self._expect_json(response)
        parsed = gen.UserCommunitiesReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "communities"):
            raise AspenApiError("Failed to load communities")
        return [
            Community.model_validate(item.model_dump(mode="json"))
            for item in parsed.communities.data
        ]

    async def create_community(self, name: str) -> Community:
        response = await self._authorized_request(
            "POST",
            "/community",
            gen.CommunityCreateCommand(name=name, icon=None),
        )
        data = self._expect_json(response)
        parsed = gen.CommunityCreateCommandResponse.model_validate(data).root
        return _unwrap_create_response(
            parsed,
            "createOk",
            Community.model_validate,
            "Failed to create community",
        )

    async def read_community_users(self, community_id: str) -> list[UserProfile]:
        response = await self._authorized_request(
            "GET",
            "/community/users",
            gen.CommunityUsersReadCommand(community=community_id),
        )
        data = self._expect_json(response)
        parsed = gen.CommunityUsersReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "users"):
            raise AspenApiError("Failed to load community users")
        return [
            UserProfile.model_validate(user.model_dump(mode="json"))
            for user in parsed.users.data
        ]

    async def create_invite(self, community_id: str) -> str:
        response = await self._authorized_request(
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

    async def read_community_channels(self, community_id: str) -> list[Channel]:
        response = await self._authorized_request(
            "GET",
            "/community/channels",
            gen.CommunityChannelsReadCommand(community=community_id),
        )
        data = self._expect_json(response)
        parsed = gen.CommunityChannelsReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "channels"):
            raise AspenApiError("Failed to load channels")
        channels = [
            Channel.model_validate(item.model_dump(mode="json"))
            for item in parsed.channels.data
        ]
        return sorted(channels, key=lambda c: c.sort_index)

    async def read_user_profile(self, user_id: str) -> UserProfile:
        response = await self._authorized_request(
            "GET",
            "/user",
            gen.UserReadCommand(id=user_id),
        )
        data = self._expect_json(response)
        parsed = gen.UserReadCommandResponse.model_validate(data).root
        if hasattr(parsed, "user"):
            return UserProfile.model_validate(parsed.user.model_dump(mode="json"))
        raise AspenApiError("Failed to read user")

    async def create_channel(self, community_id: str, name: str, sort_index: int = 0) -> Channel:
        response = await self._authorized_request(
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
        return _unwrap_create_response(
            parsed,
            "createOk",
            Channel.model_validate,
            "Failed to create channel",
        )

    # Max UUID anchor used when the caller wants the most recent page with no
    # prior context; the server's Before query treats this as "return the
    # newest N messages" since every real UUID v7 sorts below it.
    _MAX_UUID_ANCHOR = "ffffffff-ffff-ffff-ffff-ffffffffffff"

    async def read_channel_messages(
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

        response = await self._authorized_request(
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
        messages = [
            Message.model_validate(item.model_dump(mode="json"))
            for item in parsed.messages.data
        ]
        return sorted(messages, key=lambda m: m.timestamp)

    async def read_icon(self, icon_id: str) -> Icon:
        """Resolve an icon id to its metadata + public download URL.

        The server stores the actual bytes in its media store and
        templates a public, anonymous-read URL into ``downloadUrl``;
        callers (today, ``IconCache``) pair this call with
        :meth:`download_media_bytes` to fetch the image itself.
        """
        response = await self._authorized_request(
            "GET",
            "/icon",
            gen.IconReadCommand(id=icon_id),
        )
        data = self._expect_json(response)
        parsed = gen.IconReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "icon"):
            raise AspenApiError("Failed to load icon")
        return Icon.model_validate(parsed.icon.model_dump(mode="json"))

    async def download_media_bytes(self, url: str) -> tuple[bytes, str]:
        """GET an absolute media-store URL and return ``(bytes, content_type)``.

        Used by :class:`IconCache` and :class:`LinkPreviewImageCache` to
        pull the actual image bytes after the server has handed back a
        public ``downloadUrl`` / ``imageUrl``. The request is
        unauthenticated because the server's media-store endpoints are
        anonymous-read by design; ``httpx`` accepts an absolute URL even
        when ``base_url`` is set, so the per-host TLS / connection-pool
        configuration on ``self._client`` carries over for free.

        Raises :class:`AspenApiError` on any transport or HTTP-level
        failure so callers can drop back to a fallback render without
        special-casing shapes.
        """
        try:
            response = await self._client.get(url)
        except httpx.HTTPError as exc:
            raise AspenApiError(f"Failed to download media: {exc}") from exc
        if response.status_code >= 400:
            raise AspenApiError(
                f"Failed to download media: HTTP {response.status_code}"
            )
        content_type = response.headers.get("content-type", "")
        return response.content, content_type

    async def send_message(self, channel_id: str, content: str) -> Message:
        response = await self._authorized_request(
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
        return _unwrap_create_response(
            parsed,
            "createOk",
            Message.model_validate,
            "Failed to send message",
        )

    async def _authorized_request(
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
        response = await self._client.request(method, path, **kwargs)
        return response

    @staticmethod
    def _expect_json(response: httpx.Response) -> dict[str, Any]:
        if response.status_code >= 400:
            raise AspenApiError(f"HTTP {response.status_code}: {response.text}")
        data = response.json()
        if not isinstance(data, dict):
            raise AspenApiError("Unexpected response payload")
        return data

def _model_to_json(value: Any) -> Any:
    if hasattr(value, "model_dump"):
        return value.model_dump(mode="json", by_alias=True, exclude_none=False)
    return value


async def _dispatch(
    coro: Coroutine[Any, Any, T],
    on_success: Callable[[T], None],
    on_failure: Callable[[Exception], None],
) -> None:
    try:
        result = await coro
    except asyncio.CancelledError:
        # Cancellation is part of orderly shutdown -- never route it to
        # ``on_failure`` (which typically pops an error dialog or writes
        # to the status bar). Re-raise so ``TaskSpawner.shutdown`` sees
        # the task as cancelled rather than completed.
        raise
    except Exception as exc:  # noqa: BLE001 - surfaced via on_failure
        on_failure(exc)
        return
    on_success(result)


class TaskSpawner:
    """Schedule coroutines from Qt slots and bound their lifetime to the window.

    GUI slots stay synchronous (Qt requires that), but any work that needs
    to talk to the network or otherwise ``await`` something is wrapped in
    an ``async def`` helper and handed to ``spawn``. The spawner schedules
    the coroutine on the asyncio loop that ``qasync`` drives on top of
    the Qt event loop, so callbacks and ``await``-resumption fire
    on the GUI thread and may safely touch widgets / ``ClientState``.

    Tracked tasks are kept alive for the lifetime of the spawner (asyncio
    only weak-refs running tasks, so without a strong reference the GC can
    collect mid-flight tasks and silently lose their results). On
    ``shutdown`` they are cancelled and awaited to completion so callers
    can guarantee no spawned coroutine outlives the window's teardown.
    """

    def __init__(self) -> None:
        self._tasks: set[asyncio.Task[Any]] = set()
        self._stopped = False

    def spawn(self, coro: Coroutine[Any, Any, Any]) -> asyncio.Task[Any] | None:
        """Schedule ``coro`` on the running asyncio loop.

        Returns the created ``Task`` for callers that want to attach
        further callbacks or cancel selectively, or ``None`` if the
        spawner has already been shut down (in which case the coroutine
        is closed without being scheduled).
        """
        if self._stopped:
            coro.close()
            return None
        task = asyncio.ensure_future(coro)
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)
        return task

    def run(
        self,
        coro: Coroutine[Any, Any, T],
        *,
        on_success: Callable[[T], None],
        on_failure: Callable[[Exception], None],
    ) -> asyncio.Task[Any] | None:
        """Schedule ``coro`` and route its result to GUI handlers.

        This is the canonical way to invoke an ``AspenApiClient`` method
        from a Qt slot (see ``client/AGENTS.md``: "every
        ``AspenApiClient`` call is awaited from a
        ``TaskSpawner``-spawned coroutine"). ``on_success`` is invoked
        with the awaited value; ``on_failure`` with the exception
        (other than ``CancelledError``, which is always re-raised so
        ``shutdown`` can drain in-flight tasks). Both callbacks run on
        the GUI thread (the asyncio loop runs there under qasync) and
        may freely touch widgets and ``ClientState``.
        """
        return self.spawn(_dispatch(coro, on_success, on_failure))

    async def shutdown(self) -> None:
        """Stop accepting new work and await cancellation of in-flight tasks.

        Idempotent. After this returns, ``spawn`` is a no-op forever.
        Cancelling and awaiting (rather than letting tasks run to
        completion) bounds shutdown latency and ensures any teardown that
        follows (e.g. ``AspenApiClient.aclose``) doesn't race with a
        coroutine still trying to issue requests on the client we're
        about to close.
        """
        if self._stopped and not self._tasks:
            return
        self._stopped = True
        for task in list(self._tasks):
            task.cancel()
        if self._tasks:
            await asyncio.gather(*self._tasks, return_exceptions=True)
