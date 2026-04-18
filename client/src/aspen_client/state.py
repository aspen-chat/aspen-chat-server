from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

from aspen_client.types import Channel, Community, Message

# Hard ceiling on the number of messages retained per channel window. Shared
# with the UI layer so eviction is consistent regardless of whether a message
# arrives via a paged read or a live WebSocket event.
MESSAGE_WINDOW_CAP = 100


@dataclass(slots=True)
class ChannelMessageWindow:
    """A bounded slice of a channel's message history held on the client.

    ``ordered_ids`` is kept sorted ascending by UUID v7 id, which is also
    ascending by creation time. ``has_older`` / ``has_newer`` track whether
    the server may still hold records outside the slice in either direction,
    so the UI layer can decide when to trigger further paged fetches and when
    to hide the "jump to latest" affordance.
    """

    ordered_ids: list[str] = field(default_factory=list)
    has_older: bool = True
    has_newer: bool = False


@dataclass(slots=True)
class ClientState:
    communities: dict[str, Community] = field(default_factory=dict)
    channels: dict[str, Channel] = field(default_factory=dict)
    messages: dict[str, Message] = field(default_factory=dict)
    channel_windows: dict[str, ChannelMessageWindow] = field(default_factory=dict)
    deleted_communities: set[str] = field(default_factory=set)
    deleted_channels: set[str] = field(default_factory=set)
    deleted_messages: set[str] = field(default_factory=set)

    def set_communities(self, communities: list[Community]) -> None:
        for community in communities:
            self.upsert_community(community)

    def set_channels(self, channels: list[Channel]) -> None:
        for channel in channels:
            self.upsert_channel(channel)

    def set_channel_window(
        self,
        channel_id: str,
        messages: list[Message],
        *,
        has_older: bool,
        has_newer: bool,
    ) -> ChannelMessageWindow:
        """Replace the window for a channel with the given page.

        Used for the initial tip-load and for "jump to latest" resets.
        """
        for message in messages:
            if message.id in self.deleted_messages:
                continue
            self.messages[message.id] = message
        ordered_ids = sorted(
            {m.id for m in messages if m.id not in self.deleted_messages},
            key=lambda mid: mid,
        )
        window = ChannelMessageWindow(
            ordered_ids=ordered_ids,
            has_older=has_older,
            has_newer=has_newer,
        )
        self.channel_windows[channel_id] = window
        return window

    def merge_channel_page(
        self,
        channel_id: str,
        messages: list[Message],
    ) -> list[str]:
        """Merge a page of messages into the window.

        Direction-agnostic: the ordered-id invariant is maintained via a sort,
        so callers can supply either an older or newer slice and the result
        is the same. Returns the list of ids that were newly inserted (in
        ascending order), so the UI layer knows which rows to materialize.
        """
        window = self.channel_windows.setdefault(channel_id, ChannelMessageWindow())
        existing = set(window.ordered_ids)
        added: list[str] = []
        for message in messages:
            if message.id in self.deleted_messages:
                continue
            self.messages[message.id] = message
            if message.id in existing:
                continue
            existing.add(message.id)
            added.append(message.id)
        if added:
            window.ordered_ids = sorted(existing)
        return sorted(added)

    def evict_older_to_cap(self, channel_id: str, cap: int) -> list[str]:
        """Drop the oldest ids past the cap; flips ``has_older`` true."""
        window = self.channel_windows.get(channel_id)
        if window is None:
            return []
        overflow = len(window.ordered_ids) - cap
        if overflow <= 0:
            return []
        evicted = window.ordered_ids[:overflow]
        window.ordered_ids = window.ordered_ids[overflow:]
        window.has_older = True
        self._drop_message_records(evicted)
        return evicted

    def evict_newer_to_cap(self, channel_id: str, cap: int) -> list[str]:
        """Drop the newest ids past the cap; flips ``has_newer`` true."""
        window = self.channel_windows.get(channel_id)
        if window is None:
            return []
        overflow = len(window.ordered_ids) - cap
        if overflow <= 0:
            return []
        evicted = window.ordered_ids[-overflow:]
        window.ordered_ids = window.ordered_ids[:-overflow]
        window.has_newer = True
        self._drop_message_records(evicted)
        return evicted

    def _drop_message_records(self, message_ids: list[str]) -> None:
        for message_id in message_ids:
            self.messages.pop(message_id, None)

    def clear_channel_window(self, channel_id: str) -> list[str]:
        window = self.channel_windows.pop(channel_id, None)
        if window is None:
            return []
        self._drop_message_records(window.ordered_ids)
        return window.ordered_ids

    def upsert_community(self, community: Community) -> None:
        if community.id in self.deleted_communities:
            return
        self.communities[community.id] = community

    def upsert_channel(self, channel: Channel) -> None:
        if channel.id in self.deleted_channels:
            return
        self.channels[channel.id] = channel

    def upsert_message(self, message: Message) -> bool:
        """Insert or update a message in its channel's window.

        Returns ``True`` if the visible window was modified (so the UI layer
        knows whether to reflect it). Live events for channels whose window
        has diverged from the server tip (``has_newer``) are dropped: the
        user is reading older history, and they'll pick up the new tip when
        they scroll down or hit "jump to latest".
        """
        if message.id in self.deleted_messages:
            return False
        window = self.channel_windows.get(message.channel_id)
        if window is None:
            # Channel has no materialized window yet; keep the record alive so
            # the eventual first load can consume it without a refetch penalty.
            self.messages[message.id] = message
            return False
        already_present = message.id in window.ordered_ids
        if already_present:
            self.messages[message.id] = message
            return True
        if window.has_newer:
            # We're not looking at the tip; silently drop so the window stays
            # a contiguous slice of server state.
            return False
        self.messages[message.id] = message
        window.ordered_ids.append(message.id)
        window.ordered_ids.sort()
        # Cap enforcement: live appends to an already-full window trim the
        # oldest end. This applies equally to the channel currently on
        # screen and to channels the user has visited but is not viewing
        # right now, keeping total client memory bounded.
        self.evict_older_to_cap(message.channel_id, MESSAGE_WINDOW_CAP)
        return True

    def remove_community(self, community_id: str) -> None:
        self.deleted_communities.add(community_id)
        self.communities.pop(community_id, None)

    def remove_channel(self, channel_id: str) -> None:
        self.deleted_channels.add(channel_id)
        self.channels.pop(channel_id, None)
        self.clear_channel_window(channel_id)

    def remove_message(self, message_id: str) -> None:
        self.deleted_messages.add(message_id)
        message = self.messages.pop(message_id, None)
        if message is None:
            return
        window = self.channel_windows.get(message.channel_id)
        if window is None:
            return
        try:
            window.ordered_ids.remove(message_id)
        except ValueError:
            pass

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
        window = self.channel_windows.get(channel_id)
        if window is None:
            return []
        return [
            self.messages[msg_id] for msg_id in window.ordered_ids if msg_id in self.messages
        ]

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
            updated = Message(
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
            # upsert_message applies window rules; for an update of a record
            # we already know about we always want the new content persisted.
            self.messages[current.id] = updated
            window = self.channel_windows.get(current.channel_id)
            return window is not None and current.id in window.ordered_ids
        if event_type == "create":
            channel_id = payload.get("channelId")
            author = payload.get("author")
            timestamp = payload.get("timestamp")
            content = payload.get("content")
            attachments = payload.get("attachments", [])
            if channel_id is None or author is None or timestamp is None or content is None:
                return False
            message = Message(
                id=message_id,
                author=str(author),
                channel_id=str(channel_id),
                timestamp=_to_datetime(str(timestamp)),
                content=str(content),
                attachments=[str(value) for value in attachments],
            )
            return self.upsert_message(message)
        return False


def _to_datetime(value: str) -> datetime:
    if value.endswith("Z"):
        value = f"{value[:-1]}+00:00"
    return datetime.fromisoformat(value)
