from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from aspen_client.types import Channel, Community, Message

# Hard ceiling on the number of messages retained per channel window. Shared
# with the UI layer so eviction is consistent regardless of whether a message
# arrives via a paged read or a live WebSocket event.
MESSAGE_WINDOW_CAP = 500


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

    def apply_server_event(self, event: Any) -> bool:
        """Fold a validated ``GeneratedServerEvent.root`` variant into state.

        The caller (the UI layer's ``_handle_event``) hands us the typed
        pydantic object directly — no dict round-trip — so this method
        relies on attribute access for known fields and uses
        ``model_dump(exclude_none=True)`` to derive a sparse delta when
        merging optional update fields. ``None`` is treated as "no
        change", so a JSON ``null`` from the server is not a way to
        clear an optional field.
        """
        server_event = getattr(event, "serverEvent", None)
        event_type = getattr(event, "type", None)
        if server_event == "community":
            return self._apply_community_event(event, event_type)
        if server_event == "channel":
            return self._apply_channel_event(event, event_type)
        if server_event == "message":
            return self._apply_message_event(event, event_type)
        return False

    def _apply_community_event(self, event: Any, event_type: str | None) -> bool:
        community_id = str(event.id)
        if event_type == "delete":
            existed = community_id in self.communities
            self.remove_community(community_id)
            return existed
        merged = self._merge_event_into_record(
            event=event,
            record_id=community_id,
            current=self.communities.get(community_id),
            model=Community,
            event_type=event_type,
            create_defaults={"name": community_id},
        )
        if merged is None:
            return False
        self.upsert_community(merged)
        return True

    def _apply_channel_event(self, event: Any, event_type: str | None) -> bool:
        channel_id = str(event.id)
        if event_type == "delete":
            existed = channel_id in self.channels
            self.remove_channel(channel_id)
            return existed
        merged = self._merge_event_into_record(
            event=event,
            record_id=channel_id,
            current=self.channels.get(channel_id),
            model=Channel,
            event_type=event_type,
            # Fallbacks for a malformed ``create`` event missing
            # required fields. ``sortIndex`` already has a default of
            # ``0`` on the model itself.
            create_defaults={"name": channel_id, "ty": "Text"},
        )
        if merged is None:
            return False
        self.upsert_channel(merged)
        return True

    def _apply_message_event(self, event: Any, event_type: str | None) -> bool:
        message_id = str(event.id)
        if event_type == "delete":
            existed = message_id in self.messages
            self.remove_message(message_id)
            return existed
        merged = self._merge_event_into_record(
            event=event,
            record_id=message_id,
            current=self.messages.get(message_id),
            model=Message,
            event_type=event_type,
        )
        if merged is None:
            return False
        if event_type == "update":
            # ``upsert_message`` applies window rules (drop if the user
            # is reading older history); for an update of a record we
            # already know about we always want the new content
            # persisted, regardless of which slice of history is on
            # screen.
            self.messages[merged.id] = merged
            window = self.channel_windows.get(merged.channel_id)
            return window is not None and merged.id in window.ordered_ids
        return self.upsert_message(merged)

    @staticmethod
    def _merge_event_into_record(
        *,
        event: Any,
        record_id: str,
        current: Any,
        model: type[Any],
        event_type: str | None,
        create_defaults: dict[str, Any] | None = None,
    ) -> Any:
        """Build the post-event record by merging the sparse event delta in.

        The pipeline is:

        * ``model_dump(by_alias=True, exclude_none=True, mode="json")``
          produces a sparse, JSON-friendly delta from the validated
          generated event payload. ``mode="json"`` stringifies
          UUID-typed fields so the client-side ``str`` model accepts
          them. ``exclude_none=True`` gives ``None`` the meaning "no
          change", so a JSON ``null`` from the server is not a way to
          clear an optional field.
        * For an ``update`` event with no matching record, drop the
          event. Synthesising a record from a partial update payload
          that may be missing required fields would poison state, and
          the matching ``create`` event will populate it correctly
          when it arrives.
        * For a ``create`` event, layer ``create_defaults`` under the
          delta so a malformed event missing required fields still
          validates.
        * For an ``update`` event with an existing record, dump the
          existing record (also alias-keyed JSON-mode) and overlay the
          delta on top before re-validating.

        ``populate_by_name=True`` on the record models means the
        merged dict can carry either alias (camelCase) or python
        (snake_case) keys -- both pipelines work.
        """
        if event_type not in {"create", "update"}:
            return None
        delta = event.model_dump(
            exclude_none=True,
            by_alias=True,
            exclude={"serverEvent", "type", "id"},
            mode="json",
        )
        if current is None:
            # An update without a matching record is silently dropped
            # to avoid synthesising a record from a partial payload
            # that may be missing required fields.
            if event_type == "update":
                return None
            merged_data = {**(create_defaults or {}), **delta, "id": record_id}
        else:
            # Both create-with-existing and update-with-existing fold
            # the sparse delta on top of the existing record.
            base = current.model_dump(by_alias=True, mode="json")
            merged_data = {**base, **delta, "id": record_id}
        return model.model_validate(merged_data)
