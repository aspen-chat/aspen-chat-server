"""``QAbstractListModel`` implementations backing every QML ``ListView``.

Four models live here, each owning one observable view of the
client-side state:

* :class:`CommunityListModel` and :class:`ChannelListModel` mirror the
  sorted-upsert / O(1)-by-id semantics that
  :class:`aspen_client.keyed_list.KeyedListWidget` provides for the
  Widgets path. The selection lives on the controller, not the model,
  so two views (the full text list and the collapsed avatar strip) can
  bind to the same :class:`CommunityListModel` instance.
* :class:`UserListModel` is the Users panel; presence updates land
  through :meth:`UserListModel.set_status` and emit
  ``dataChanged([StatusRole])`` so the dot repaints without rebuilding
  the row.
* :class:`MessageListModel` is the sliding-window-aware message view.
  It exposes the four window-mutating verbs the controller needs
  (``set_window``, ``prepend_page``, ``append_page``, ``upsert_message``)
  alongside the per-row ``replace_previews`` and ``bump_icon_epoch``
  patches the link-preview / avatar caches need. The model is the
  only object that calls ``beginInsertRows`` / ``beginRemoveRows`` /
  ``dataChanged`` on the message side; everything else routes through
  these methods so the QML ``ListView`` always sees one consistent
  reorder per logical change.

Roles are exposed as plain integers above ``Qt.UserRole``; QML reads
them by name via ``QAbstractListModel.roleNames`` so the delegates use
``model.id``, ``model.name``, etc. directly without index magic.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Iterable

from PySide6.QtCore import (
    QAbstractListModel,
    QByteArray,
    QModelIndex,
    Qt,
    Signal,
    Slot,
)

from aspen_client.state import MESSAGE_WINDOW_CAP, ChannelMessageWindow, ClientState
from aspen_client.types import Channel, Community, LinkPreview, Message, UserProfile


def _role_names(roles: dict[int, str]) -> dict[int, QByteArray]:
    """Convert a ``{role_int: name_str}`` map to the type Qt expects."""
    return {role: QByteArray(name.encode("utf-8")) for role, name in roles.items()}


class _BaseKeyedListModel(QAbstractListModel):
    """Common scaffolding for the three id-keyed list models.

    Holds an ``_ordered`` list of values and an ``_index_by_id`` dict
    that's kept in sync; ``_sort_key`` decides where new entries land.
    Subclasses define ``_role_names_map``, ``_id_of``, and ``_value_role_data``.
    """

    def __init__(self, parent: QAbstractListModel | None = None) -> None:
        super().__init__(parent)
        self._ordered: list[Any] = []
        self._index_by_id: dict[str, int] = {}

    # ---------- QAbstractListModel API ----------

    def rowCount(  # type: ignore[override]
        self, parent: QModelIndex | None = None
    ) -> int:
        if parent is not None and parent.isValid():
            return 0
        return len(self._ordered)

    def data(  # type: ignore[override]
        self, index: QModelIndex, role: int = Qt.ItemDataRole.DisplayRole
    ) -> Any:
        if not index.isValid():
            return None
        row = index.row()
        if not 0 <= row < len(self._ordered):
            return None
        return self._value_role_data(self._ordered[row], role)

    def roleNames(self) -> dict[int, QByteArray]:  # type: ignore[override]
        return _role_names(self._role_names_map())

    # ---------- mutator surface for the controller ----------

    def get(self, key: str) -> Any | None:
        idx = self._index_by_id.get(key)
        if idx is None:
            return None
        return self._ordered[idx]

    def index_for(self, key: str) -> int | None:
        return self._index_by_id.get(key)

    def upsert(self, value: Any) -> None:
        """Insert ``value`` at its sorted position, or update in place.

        Mirrors :meth:`KeyedListWidget.upsert` \u2014 if the sort key has
        changed the row is moved (via begin/endMoveRows so the
        ``ListView`` slides smoothly rather than blinking the whole
        list).
        """
        key = self._id_of(value)
        existing_index = self._index_by_id.get(key)
        if existing_index is None:
            self._insert_sorted(value)
            return
        existing = self._ordered[existing_index]
        if self._sort_key(value) != self._sort_key(existing):
            self._move_existing(existing_index, value)
            return
        self._ordered[existing_index] = value
        idx = self.createIndex(existing_index, 0)
        self.dataChanged.emit(idx, idx, list(self._role_names_map().keys()))

    def remove(self, key: str) -> None:
        idx = self._index_by_id.pop(key, None)
        if idx is None:
            return
        self.beginRemoveRows(QModelIndex(), idx, idx)
        self._ordered.pop(idx)
        # Reindex everyone after the removed slot. Cheap given the small
        # list sizes (communities/channels/users in a single community).
        for offset_key, offset_idx in list(self._index_by_id.items()):
            if offset_idx > idx:
                self._index_by_id[offset_key] = offset_idx - 1
        self.endRemoveRows()

    def replace_all(self, values: Iterable[Any]) -> None:
        """Wipe and rebuild from ``values``.

        Used for the bootstrap and refresh paths; the controller is
        responsible for replaying the user's selection onto the new
        contents because the model deliberately has no notion of
        selection.
        """
        materialised = sorted(values, key=self._sort_key)
        self.beginResetModel()
        self._ordered = list(materialised)
        self._index_by_id = {self._id_of(v): i for i, v in enumerate(self._ordered)}
        self.endResetModel()

    def clear(self) -> None:
        if not self._ordered:
            return
        self.beginResetModel()
        self._ordered.clear()
        self._index_by_id.clear()
        self.endResetModel()

    def items(self) -> list[Any]:
        """Snapshot of the current values, in display order."""
        return list(self._ordered)

    # ---------- subclass hooks ----------

    def _role_names_map(self) -> dict[int, str]:
        raise NotImplementedError

    def _id_of(self, value: Any) -> str:
        raise NotImplementedError

    def _sort_key(self, value: Any) -> Any:
        raise NotImplementedError

    def _value_role_data(self, value: Any, role: int) -> Any:
        raise NotImplementedError

    # ---------- internals ----------

    def _insert_sorted(self, value: Any) -> None:
        new_sort = self._sort_key(value)
        insert_index = len(self._ordered)
        for index, other in enumerate(self._ordered):
            if new_sort < self._sort_key(other):
                insert_index = index
                break
        self.beginInsertRows(QModelIndex(), insert_index, insert_index)
        self._ordered.insert(insert_index, value)
        self._index_by_id = {
            self._id_of(v): i for i, v in enumerate(self._ordered)
        }
        self.endInsertRows()

    def _move_existing(self, current_index: int, new_value: Any) -> None:
        # Remove + re-insert under one ``layoutAboutToBeChanged`` /
        # ``layoutChanged`` envelope is the simplest correct dance for a
        # general-purpose move that may cross the destination index.
        self.beginRemoveRows(QModelIndex(), current_index, current_index)
        self._ordered.pop(current_index)
        for key, idx in list(self._index_by_id.items()):
            if idx == current_index:
                self._index_by_id.pop(key)
            elif idx > current_index:
                self._index_by_id[key] = idx - 1
        self.endRemoveRows()
        self._insert_sorted(new_value)


# ---------- Communities ----------

class CommunityListModel(_BaseKeyedListModel):
    IdRole = Qt.ItemDataRole.UserRole + 1
    NameRole = Qt.ItemDataRole.UserRole + 2
    IconIdRole = Qt.ItemDataRole.UserRole + 3
    IconEpochRole = Qt.ItemDataRole.UserRole + 4

    def _role_names_map(self) -> dict[int, str]:
        return {
            CommunityListModel.IdRole: "id",
            CommunityListModel.NameRole: "name",
            CommunityListModel.IconIdRole: "iconId",
            CommunityListModel.IconEpochRole: "iconEpoch",
        }

    def _id_of(self, value: Community) -> str:  # type: ignore[override]
        return value.id

    def _sort_key(self, value: Community) -> Any:  # type: ignore[override]
        return value.name.lower()

    def _value_role_data(self, value: Community, role: int) -> Any:  # type: ignore[override]
        if role == CommunityListModel.IdRole:
            return value.id
        if role == CommunityListModel.NameRole:
            return value.name
        if role == CommunityListModel.IconIdRole:
            return value.icon or ""
        if role == CommunityListModel.IconEpochRole:
            return self._icon_epochs.get(value.id, 0)
        if role == Qt.ItemDataRole.DisplayRole:
            return value.name
        return None

    def __init__(self, parent: QAbstractListModel | None = None) -> None:
        super().__init__(parent)
        # Per-row icon epoch. Bumped when an icon fetch completes for a
        # community so the QML ``Image.source`` binding (which embeds the
        # epoch as a query string) re-runs and asks the image provider
        # for fresh bytes.
        self._icon_epochs: dict[str, int] = {}

    def bump_icon_epoch(self, community_id: str) -> None:
        idx = self._index_by_id.get(community_id)
        if idx is None:
            self._icon_epochs[community_id] = self._icon_epochs.get(community_id, 0) + 1
            return
        self._icon_epochs[community_id] = self._icon_epochs.get(community_id, 0) + 1
        model_index = self.createIndex(idx, 0)
        self.dataChanged.emit(
            model_index, model_index, [CommunityListModel.IconEpochRole]
        )

    def clear(self) -> None:  # type: ignore[override]
        super().clear()
        self._icon_epochs.clear()


# ---------- Channels ----------

class ChannelListModel(_BaseKeyedListModel):
    IdRole = Qt.ItemDataRole.UserRole + 1
    NameRole = Qt.ItemDataRole.UserRole + 2
    SortIndexRole = Qt.ItemDataRole.UserRole + 3
    CommunityRole = Qt.ItemDataRole.UserRole + 4

    def _role_names_map(self) -> dict[int, str]:
        return {
            ChannelListModel.IdRole: "id",
            ChannelListModel.NameRole: "name",
            ChannelListModel.SortIndexRole: "sortIndex",
            ChannelListModel.CommunityRole: "community",
        }

    def _id_of(self, value: Channel) -> str:  # type: ignore[override]
        return value.id

    def _sort_key(self, value: Channel) -> Any:  # type: ignore[override]
        return value.sort_index

    def _value_role_data(self, value: Channel, role: int) -> Any:  # type: ignore[override]
        if role == ChannelListModel.IdRole:
            return value.id
        if role == ChannelListModel.NameRole:
            return value.name
        if role == ChannelListModel.SortIndexRole:
            return value.sort_index
        if role == ChannelListModel.CommunityRole:
            return value.community or ""
        if role == Qt.ItemDataRole.DisplayRole:
            return f"#{value.name}"
        return None


# ---------- Users (community-users panel) ----------

class UserListModel(_BaseKeyedListModel):
    IdRole = Qt.ItemDataRole.UserRole + 1
    NameRole = Qt.ItemDataRole.UserRole + 2
    StatusRole = Qt.ItemDataRole.UserRole + 3
    IconEpochRole = Qt.ItemDataRole.UserRole + 4

    def _role_names_map(self) -> dict[int, str]:
        return {
            UserListModel.IdRole: "id",
            UserListModel.NameRole: "name",
            UserListModel.StatusRole: "status",
            UserListModel.IconEpochRole: "iconEpoch",
        }

    def _id_of(self, value: UserProfile) -> str:  # type: ignore[override]
        return value.id

    def _sort_key(self, value: UserProfile) -> Any:  # type: ignore[override]
        return value.name.lower()

    def __init__(self, parent: QAbstractListModel | None = None) -> None:
        super().__init__(parent)
        self._statuses: dict[str, str] = {}
        self._icon_epochs: dict[str, int] = {}

    def _value_role_data(self, value: UserProfile, role: int) -> Any:  # type: ignore[override]
        if role == UserListModel.IdRole:
            return value.id
        if role == UserListModel.NameRole:
            return value.name
        if role == UserListModel.StatusRole:
            return self._statuses.get(value.id, "offline")
        if role == UserListModel.IconEpochRole:
            return self._icon_epochs.get(value.id, 0)
        if role == Qt.ItemDataRole.DisplayRole:
            return value.name
        return None

    def set_status(self, user_id: str, status: str) -> None:
        self._statuses[user_id] = status
        idx = self._index_by_id.get(user_id)
        if idx is None:
            return
        model_index = self.createIndex(idx, 0)
        self.dataChanged.emit(model_index, model_index, [UserListModel.StatusRole])

    def bump_icon_epoch(self, user_id: str) -> None:
        self._icon_epochs[user_id] = self._icon_epochs.get(user_id, 0) + 1
        idx = self._index_by_id.get(user_id)
        if idx is None:
            return
        model_index = self.createIndex(idx, 0)
        self.dataChanged.emit(model_index, model_index, [UserListModel.IconEpochRole])

    def clear(self) -> None:  # type: ignore[override]
        super().clear()
        self._statuses.clear()
        self._icon_epochs.clear()


# ---------- Messages (sliding window) ----------

class MessageListModel(QAbstractListModel):
    """Sliding-window-aware view of a single channel's messages.

    The model holds **no** message data of its own \u2014 every read goes
    through ``ClientState.messages`` / ``ClientState.channel_windows``,
    which is what lets a channel switch be O(1) (rebind to a different
    ``channel_id``, ``beginResetModel`` / ``endResetModel``, done) and
    keeps the eviction rules in one place. The mutator methods
    (:meth:`set_window`, :meth:`prepend_page`, :meth:`append_page`,
    :meth:`upsert_message`, :meth:`remove_message`) mutate
    ``ClientState`` first and then emit the matching Qt
    insert/remove/data-changed signals.

    The ``dataChanged`` channel is used heavily: avatar fetches,
    profile resolution, link-preview ``ready`` events, and the per-row
    ``replace_previews`` all land as targeted updates rather than a
    full reset, which keeps the QML ``ListView``'s scroll position
    stable.
    """

    IdRole = Qt.ItemDataRole.UserRole + 1
    AuthorRole = Qt.ItemDataRole.UserRole + 2
    AuthorNameRole = Qt.ItemDataRole.UserRole + 3
    TimestampRole = Qt.ItemDataRole.UserRole + 4
    ContentRole = Qt.ItemDataRole.UserRole + 5
    LinkPreviewsRole = Qt.ItemDataRole.UserRole + 6
    AvatarEpochRole = Qt.ItemDataRole.UserRole + 7

    # Emitted after the model finishes a logical "prepend a page" mutation.
    # ``inserted`` is the number of rows added at the top, so QML can adjust
    # ``contentY`` by exactly the height of the inserted block to preserve
    # the user's visual scroll position. The signal fires after
    # ``endInsertRows`` so the ``ListView`` has already updated ``originY``
    # by the time the slot runs.
    pagePrepended = Signal(int)

    def __init__(
        self,
        state: ClientState,
        author_name_resolver: Any,
        parent: QAbstractListModel | None = None,
    ) -> None:
        super().__init__(parent)
        self._state = state
        self._channel_id: str | None = None
        # Snapshot of ``ChannelMessageWindow.ordered_ids`` for the
        # currently bound channel. Kept locally so we can compare to the
        # post-mutation order and emit accurate begin/end ranges; the
        # source of truth is still ``ClientState``.
        self._ordered: list[str] = []
        self._avatar_epochs: dict[str, int] = {}
        self._author_name_resolver = author_name_resolver

    def set_state(self, state: ClientState) -> None:
        """Re-target the model after a ``_reset_client_state`` swap.

        The bound channel id is cleared because the new state has no
        windows yet; the controller is expected to call
        :meth:`set_active_channel` again as part of the rebootstrap.
        """
        self.beginResetModel()
        self._state = state
        self._channel_id = None
        self._ordered = []
        self._avatar_epochs.clear()
        self.endResetModel()

    # ---------- channel binding ----------

    def set_active_channel(self, channel_id: str | None) -> None:
        """Rebind to ``channel_id``'s window. ``None`` clears the model."""
        if channel_id == self._channel_id:
            return
        self.beginResetModel()
        self._channel_id = channel_id
        self._ordered = self._snapshot_ordered_ids()
        self.endResetModel()

    @property
    def active_channel_id(self) -> str | None:
        return self._channel_id

    def has_newer(self) -> bool:
        window = self._current_window()
        return bool(window and window.has_newer)

    def has_older(self) -> bool:
        window = self._current_window()
        return bool(window and window.has_older)

    # ---------- QAbstractListModel API ----------

    def rowCount(  # type: ignore[override]
        self, parent: QModelIndex | None = None
    ) -> int:
        if parent is not None and parent.isValid():
            return 0
        return len(self._ordered)

    def roleNames(self) -> dict[int, QByteArray]:  # type: ignore[override]
        return _role_names(
            {
                MessageListModel.IdRole: "id",
                MessageListModel.AuthorRole: "author",
                MessageListModel.AuthorNameRole: "authorName",
                MessageListModel.TimestampRole: "timestamp",
                MessageListModel.ContentRole: "content",
                MessageListModel.LinkPreviewsRole: "linkPreviews",
                MessageListModel.AvatarEpochRole: "avatarEpoch",
            }
        )

    def data(  # type: ignore[override]
        self, index: QModelIndex, role: int = Qt.ItemDataRole.DisplayRole
    ) -> Any:
        if not index.isValid():
            return None
        row = index.row()
        if not 0 <= row < len(self._ordered):
            return None
        message_id = self._ordered[row]
        message = self._state.messages.get(message_id)
        if message is None:
            return None
        if role == MessageListModel.IdRole:
            return message.id
        if role == MessageListModel.AuthorRole:
            return message.author
        if role == MessageListModel.AuthorNameRole:
            return self._author_name_resolver(message.author)
        if role == MessageListModel.TimestampRole:
            return self._format_timestamp(message.timestamp)
        if role == MessageListModel.ContentRole:
            return message.content
        if role == MessageListModel.LinkPreviewsRole:
            return [self._preview_to_dict(p) for p in message.link_previews]
        if role == MessageListModel.AvatarEpochRole:
            return self._avatar_epochs.get(message.author, 0)
        if role == Qt.ItemDataRole.DisplayRole:
            return message.content
        return None

    # ---------- mutator surface for the controller ----------

    def set_window(
        self,
        messages: list[Message],
        *,
        has_older: bool,
        has_newer: bool,
    ) -> None:
        """Replace the bound channel's window wholesale.

        Used for the initial-load and jump-to-latest paths: ``ClientState``
        is updated through :meth:`ClientState.set_channel_window` and the
        model emits a single ``modelReset`` so QML rebuilds the visible
        rows in one pass.
        """
        if self._channel_id is None:
            return
        self._state.set_channel_window(
            self._channel_id, messages, has_older=has_older, has_newer=has_newer
        )
        self.beginResetModel()
        self._ordered = self._snapshot_ordered_ids()
        self.endResetModel()

    def prepend_page(self, messages: list[Message]) -> int:
        """Merge an older-direction page into the head of the window.

        Returns the number of newly-inserted rows so the controller can
        forward the count via ``pagePrepended`` for QML's scroll-anchor
        compensation. Cap enforcement (drop newest beyond
        :data:`MESSAGE_WINDOW_CAP`) is the controller's job and runs
        through :meth:`evict_newer_to_cap` afterwards \u2014 keeping it
        outside the prepend lets the controller decide when to shed.
        """
        if self._channel_id is None or not messages:
            return 0
        previous_ids = set(self._ordered)
        new_ids = self._state.merge_channel_page(self._channel_id, messages)
        added_at_head = [mid for mid in new_ids if mid not in previous_ids]
        if not added_at_head:
            return 0
        # ``merge_channel_page`` resorts ``ordered_ids``; recompute our
        # snapshot and locate the contiguous prefix that was added at
        # the head. The merged ids are always sorted ascending and they
        # always land at the position dictated by their UUID v7 ordering,
        # which for an "older" page is at index 0..N-1 (older ids sort
        # below newer ones). For an "around" anchor variant the inserts
        # could land mid-list; this path is only used for "older" pages
        # so the head assumption is safe.
        new_snapshot = self._snapshot_ordered_ids()
        head_count = self._count_added_at_head(new_snapshot, previous_ids)
        self.beginInsertRows(QModelIndex(), 0, head_count - 1)
        self._ordered = new_snapshot
        self.endInsertRows()
        self.pagePrepended.emit(head_count)
        return head_count

    def append_page(self, messages: list[Message]) -> int:
        """Merge a newer-direction page onto the tail of the window."""
        if self._channel_id is None or not messages:
            return 0
        previous_ids = set(self._ordered)
        new_ids = self._state.merge_channel_page(self._channel_id, messages)
        added_at_tail = [mid for mid in new_ids if mid not in previous_ids]
        if not added_at_tail:
            return 0
        new_snapshot = self._snapshot_ordered_ids()
        tail_count = self._count_added_at_tail(new_snapshot, previous_ids)
        first_row = len(self._ordered)
        self.beginInsertRows(QModelIndex(), first_row, first_row + tail_count - 1)
        self._ordered = new_snapshot
        self.endInsertRows()
        return tail_count

    def upsert_message(self, message: Message) -> str:
        """Apply a live message create/update.

        Returns one of ``"appended"``, ``"updated"``, or ``"dropped"``
        so the controller can decide whether to scroll to the bottom
        (a true append on a tip-pinned window) or do nothing (an update
        in place / a live event for a window that's reading older
        history).
        """
        if self._channel_id is None or message.channel_id != self._channel_id:
            return "dropped"
        if message.id in self._ordered:
            self._state.messages[message.id] = message
            row = self._ordered.index(message.id)
            model_index = self.createIndex(row, 0)
            self.dataChanged.emit(
                model_index,
                model_index,
                [
                    MessageListModel.ContentRole,
                    MessageListModel.LinkPreviewsRole,
                    MessageListModel.TimestampRole,
                ],
            )
            return "updated"
        if not self._state.upsert_message(message):
            # ``ClientState.upsert_message`` already enforced the
            # has_newer-True drop (the user is reading older history,
            # the live event must not pollute the contiguous slice).
            return "dropped"
        new_snapshot = self._snapshot_ordered_ids()
        if message.id not in new_snapshot:
            return "dropped"
        new_row = new_snapshot.index(message.id)
        self.beginInsertRows(QModelIndex(), new_row, new_row)
        self._ordered = new_snapshot
        self.endInsertRows()
        # ``ClientState.upsert_message`` ran its own
        # ``evict_older_to_cap`` after the append; mirror by removing
        # the rows that fell out of the window.
        self._evict_ui_to_match_state()
        return "appended"

    def remove_message(self, message_id: str) -> None:
        if message_id not in self._ordered:
            return
        row = self._ordered.index(message_id)
        self.beginRemoveRows(QModelIndex(), row, row)
        self._ordered.pop(row)
        self.endRemoveRows()

    def replace_previews(self, message_id: str, previews: list[LinkPreview]) -> None:
        """Patch one row's ``linkPreviews`` without touching the rest."""
        if not self._state.apply_link_previews_ready(message_id, previews):
            return
        if message_id not in self._ordered:
            return
        row = self._ordered.index(message_id)
        model_index = self.createIndex(row, 0)
        self.dataChanged.emit(
            model_index, model_index, [MessageListModel.LinkPreviewsRole]
        )

    def bump_avatar_epoch_for_user(self, user_id: str) -> None:
        """Refresh every visible row whose author is ``user_id``.

        Called when an icon-fetch completes and when a profile load
        lands; the avatar epoch is per-user, not per-row, so a single
        increment is enough to rebind every affected ``Image.source``.
        """
        self._avatar_epochs[user_id] = self._avatar_epochs.get(user_id, 0) + 1
        for row, message_id in enumerate(self._ordered):
            message = self._state.messages.get(message_id)
            if message is None or message.author != user_id:
                continue
            model_index = self.createIndex(row, 0)
            self.dataChanged.emit(
                model_index,
                model_index,
                [MessageListModel.AvatarEpochRole, MessageListModel.AuthorNameRole],
            )

    def evict_older_to_cap(self) -> None:
        """Drop the oldest ids past :data:`MESSAGE_WINDOW_CAP`.

        Run by the controller after a newer-direction page lands so the
        head of the window is shed instead of the tail. ``ClientState``
        does the bookkeeping (and updates ``has_older``); we mirror by
        sliding the model's ``_ordered`` list and emitting
        ``rowsRemoved`` for the prefix.
        """
        if self._channel_id is None:
            return
        evicted = self._state.evict_older_to_cap(self._channel_id, MESSAGE_WINDOW_CAP)
        if not evicted:
            return
        count = len(evicted)
        self.beginRemoveRows(QModelIndex(), 0, count - 1)
        self._ordered = self._ordered[count:]
        self.endRemoveRows()

    def evict_newer_to_cap(self) -> None:
        if self._channel_id is None:
            return
        evicted = self._state.evict_newer_to_cap(self._channel_id, MESSAGE_WINDOW_CAP)
        if not evicted:
            return
        count = len(evicted)
        first_row = len(self._ordered) - count
        self.beginRemoveRows(QModelIndex(), first_row, len(self._ordered) - 1)
        self._ordered = self._ordered[:first_row]
        self.endRemoveRows()

    def clear_window(self) -> None:
        """Drop every row for the current channel from the model and state."""
        if self._channel_id is None:
            return
        self._state.clear_channel_window(self._channel_id)
        if not self._ordered:
            return
        self.beginRemoveRows(QModelIndex(), 0, len(self._ordered) - 1)
        self._ordered = []
        self.endRemoveRows()

    # ---------- introspection helpers used by the controller ----------

    @Slot(int, result=str)
    def messageIdAt(self, row: int) -> str:  # noqa: N802 - QML naming
        """Return the message id at ``row`` (or an empty string)."""
        if 0 <= row < len(self._ordered):
            return self._ordered[row]
        return ""

    @Slot(result=str)
    def oldestMessageId(self) -> str:  # noqa: N802
        return self._ordered[0] if self._ordered else ""

    @Slot(result=str)
    def newestMessageId(self) -> str:  # noqa: N802
        return self._ordered[-1] if self._ordered else ""

    @Slot(result=int)
    def count(self) -> int:  # noqa: N802
        return len(self._ordered)

    # ---------- internals ----------

    def _current_window(self) -> ChannelMessageWindow | None:
        if self._channel_id is None:
            return None
        return self._state.channel_windows.get(self._channel_id)

    def _snapshot_ordered_ids(self) -> list[str]:
        window = self._current_window()
        if window is None:
            return []
        return list(window.ordered_ids)

    def _evict_ui_to_match_state(self) -> None:
        """Bring ``_ordered`` back in sync with the state window.

        Used after operations that may have triggered an internal
        eviction inside :class:`ClientState` (currently only
        ``upsert_message``). Walks the diff and emits one
        ``rowsRemoved`` per contiguous evicted run; in practice the
        eviction always trims the head, so this is at most one signal.
        """
        new_snapshot = self._snapshot_ordered_ids()
        if new_snapshot == self._ordered:
            return
        # Drop any prefix that has been evicted; do nothing else \u2014 this
        # helper is only ever invoked from the live-append path, where
        # the only legitimate change is at the head.
        valid = set(new_snapshot)
        head_evicted = 0
        for mid in self._ordered:
            if mid in valid:
                break
            head_evicted += 1
        if head_evicted == 0:
            return
        self.beginRemoveRows(QModelIndex(), 0, head_evicted - 1)
        self._ordered = self._ordered[head_evicted:]
        self.endRemoveRows()

    @staticmethod
    def _format_timestamp(timestamp: datetime) -> str:
        return timestamp.astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")

    @staticmethod
    def _preview_to_dict(preview: LinkPreview) -> dict[str, Any]:
        # QML reads list elements as ``QVariant`` dicts. We deliberately
        # surface plain dict keys (camelCase to match QML idioms) rather
        # than the pydantic record type because passing a Python class
        # across the QML boundary makes the property surface much
        # noisier (every attribute becomes a meta-property) and ties
        # the QML delegate to the pydantic schema, which then can't be
        # versioned independently.
        return {
            "url": preview.url,
            "title": preview.title or "",
            "description": preview.description or "",
            "siteName": preview.site_name or "",
            "imageUrl": preview.image_url or "",
            "themeColor": preview.theme_color or "",
        }

    @staticmethod
    def _count_added_at_head(new_snapshot: list[str], previous: set[str]) -> int:
        count = 0
        for mid in new_snapshot:
            if mid in previous:
                break
            count += 1
        return count

    @staticmethod
    def _count_added_at_tail(new_snapshot: list[str], previous: set[str]) -> int:
        count = 0
        for mid in reversed(new_snapshot):
            if mid in previous:
                break
            count += 1
        return count
