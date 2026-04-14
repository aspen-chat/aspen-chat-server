from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

from aspen_client.types import Channel, Community, Message


@dataclass(slots=True)
class ClientState:
    communities: dict[str, Community] = field(default_factory=dict)
    channels: dict[str, Channel] = field(default_factory=dict)
    messages: dict[str, Message] = field(default_factory=dict)
    channel_message_ids: dict[str, list[str]] = field(default_factory=dict)
    deleted_communities: set[str] = field(default_factory=set)
    deleted_channels: set[str] = field(default_factory=set)
    deleted_messages: set[str] = field(default_factory=set)

    def set_communities(self, communities: list[Community]) -> None:
        for community in communities:
            self.upsert_community(community)

    def set_channels(self, channels: list[Channel]) -> None:
        for channel in channels:
            self.upsert_channel(channel)

    def set_channel_messages(self, channel_id: str, messages: list[Message]) -> None:
        for message in messages:
            self.upsert_message(message)
        ordered_ids = sorted(
            {message.id for message in messages},
            key=lambda msg_id: self.messages[msg_id].timestamp,
        )
        self.channel_message_ids[channel_id] = ordered_ids

    def upsert_community(self, community: Community) -> None:
        if community.id in self.deleted_communities:
            return
        self.communities[community.id] = community

    def upsert_channel(self, channel: Channel) -> None:
        if channel.id in self.deleted_channels:
            return
        self.channels[channel.id] = channel

    def upsert_message(self, message: Message) -> None:
        if message.id in self.deleted_messages:
            return
        self.messages[message.id] = message
        message_ids = self.channel_message_ids.setdefault(message.channel_id, [])
        if message.id not in message_ids:
            message_ids.append(message.id)
            message_ids.sort(key=lambda msg_id: self.messages[msg_id].timestamp)

    def remove_community(self, community_id: str) -> None:
        self.deleted_communities.add(community_id)
        self.communities.pop(community_id, None)

    def remove_channel(self, channel_id: str) -> None:
        self.deleted_channels.add(channel_id)
        self.channels.pop(channel_id, None)
        self.channel_message_ids.pop(channel_id, None)

    def remove_message(self, message_id: str) -> None:
        self.deleted_messages.add(message_id)
        message = self.messages.pop(message_id, None)
        if message is None:
            return
        ids = self.channel_message_ids.get(message.channel_id)
        if ids is None:
            return
        if message_id in ids:
            ids.remove(message_id)

    def get_communities_sorted(self) -> list[Community]:
        return sorted(self.communities.values(), key=lambda community: community.name.lower())

    def get_channels_for_community(self, community_id: str) -> list[Channel]:
        channels = [
            channel
            for channel in self.channels.values()
            if channel.community == community_id and channel.id not in self.deleted_channels
        ]
        return sorted(channels, key=lambda channel: channel.sort_index)

    def get_messages_for_channel(self, channel_id: str) -> list[Message]:
        ids = self.channel_message_ids.get(channel_id, [])
        return [self.messages[msg_id] for msg_id in ids if msg_id in self.messages]

    def apply_server_event(self, payload: dict[str, Any]) -> bool:
        server_event = str(payload.get("serverEvent", ""))
        event_type = str(payload.get("type", ""))
        changed = False

        if server_event == "community":
            changed = self._apply_community_event(payload, event_type)
        elif server_event == "channel":
            changed = self._apply_channel_event(payload, event_type)
        elif server_event == "message":
            changed = self._apply_message_event(payload, event_type)

        return changed

    def _apply_community_event(self, payload: dict[str, Any], event_type: str) -> bool:
        community_id = payload.get("id")
        if community_id is None:
            return False
        community_id = str(community_id)
        if event_type == "delete":
            existed = community_id in self.communities
            self.remove_community(community_id)
            return existed
        if event_type in {"create", "update"}:
            current = self.communities.get(community_id)
            name = payload.get("name")
            icon = payload.get("icon")
            merged = Community(
                id=community_id,
                name=str(name if name is not None else (current.name if current else community_id)),
                icon=str(icon) if icon is not None else (current.icon if current else None),
            )
            self.upsert_community(merged)
            return True
        return False

    def _apply_channel_event(self, payload: dict[str, Any], event_type: str) -> bool:
        channel_id = payload.get("id")
        if channel_id is None:
            return False
        channel_id = str(channel_id)
        if event_type == "delete":
            existed = channel_id in self.channels
            self.remove_channel(channel_id)
            return existed
        if event_type in {"create", "update"}:
            current = self.channels.get(channel_id)
            community = payload.get("community")
            parent_category = payload.get("parentCategory")
            sort_index = payload.get("sortIndex")
            channel = Channel(
                id=channel_id,
                name=str(
                    payload.get("name")
                    if payload.get("name") is not None
                    else (current.name if current else channel_id)
                ),
                ty=str(
                    payload.get("ty")
                    if payload.get("ty") is not None
                    else (current.ty if current else "Text")
                ),
                community=(
                    str(community)
                    if community is not None
                    else (current.community if current else None)
                ),
                parent_category=(
                    str(parent_category)
                    if parent_category is not None
                    else (current.parent_category if current else None)
                ),
                sort_index=int(
                    sort_index if sort_index is not None else (current.sort_index if current else 0)
                ),
            )
            self.upsert_channel(channel)
            return True
        return False

    def _apply_message_event(self, payload: dict[str, Any], event_type: str) -> bool:
        message_id = payload.get("id")
        if message_id is None:
            return False
        message_id = str(message_id)
        if event_type == "delete":
            existed = message_id in self.messages
            self.remove_message(message_id)
            return existed
        if event_type == "update":
            current = self.messages.get(message_id)
            if current is None:
                # Unknown record updates are ignored until a create/read arrives.
                return False
            content = payload.get("content")
            attachments = payload.get("attachments")
            self.upsert_message(
                Message(
                    id=current.id,
                    author=current.author,
                    channel_id=current.channel_id,
                    timestamp=current.timestamp,
                    content=str(content) if content is not None else current.content,
                    attachments=(
                        [str(value) for value in attachments]
                        if attachments is not None
                        else current.attachments
                    ),
                )
            )
            return True
        if event_type == "create":
            channel_id = payload.get("channelId")
            author = payload.get("author")
            timestamp = payload.get("timestamp")
            content = payload.get("content")
            attachments = payload.get("attachments", [])
            if channel_id is None or author is None or timestamp is None or content is None:
                return False
            self.upsert_message(
                Message(
                    id=message_id,
                    author=str(author),
                    channel_id=str(channel_id),
                    timestamp=_to_datetime(str(timestamp)),
                    content=str(content),
                    attachments=[str(value) for value in attachments],
                )
            )
            return True
        return False


def _to_datetime(value: str) -> datetime:
    if value.endswith("Z"):
        value = f"{value[:-1]}+00:00"
    return datetime.fromisoformat(value)
