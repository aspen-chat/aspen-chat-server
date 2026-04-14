from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

import httpx

from aspen_client.config import ClientConfig
from aspen_client.generated import openapi_models as gen
from aspen_client.types import Channel, Community, LoginSession, Message


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
        response = self._authorized_request(
            "GET",
            "/user",
            gen.UserReadCommand(id=user_id),
        )
        data = self._expect_json(response)
        parsed = gen.UserReadCommandResponse.model_validate(data).root
        if hasattr(parsed, "user"):
            return str(parsed.user.name)
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

    def read_channel_messages(self, channel_id: str, limit: int = 100) -> list[Message]:
        # The API currently requires an anchor message id for reads. Using max UUID
        # requests the most recent messages by id ordering.
        response = self._authorized_request(
            "GET",
            "/channel/messages",
            gen.ChannelMessagesReadCommand(
                channel=channel_id,
                viewDescription=gen.ChannelViewDescription(
                    root=gen.ChannelViewDescription1(
                        adjective="before",
                        message="ffffffff-ffff-ffff-ffff-ffffffffffff",
                        count=max(1, min(limit, 200)),
                    )
                ),
            ),
        )
        data = self._expect_json(response)
        parsed = gen.ChannelMessagesReadCommandResponse.model_validate(data).root
        if not hasattr(parsed, "messages"):
            raise AspenApiError("Failed to load messages")
        messages = [self._parse_message(item.model_dump()) for item in parsed.messages.data]
        return sorted(messages, key=lambda m: m.timestamp)

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
