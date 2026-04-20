from __future__ import annotations

from collections.abc import Callable, Iterable
from typing import Any, Generic, TypeVar

from PySide6.QtCore import Qt
from PySide6.QtWidgets import QListWidget, QListWidgetItem

T = TypeVar("T")


class KeyedListWidget(Generic[T]):
    """``QListWidget`` keyed by string id with O(1) lookup and sorted upserts.

    The chat window has several panels (communities, the parallel
    community-avatar strip, channels, ...) that all want the same shape:

    * a ``dict[str, QListWidgetItem]`` for O(1) row lookup so an
      incoming WebSocket event can patch one row without rebuilding the
      list,
    * insert at the position determined by a sort key,
    * update in place; if the sort key changed, re-insert at the new
      position, preserving the selection if the row was selected,
    * full rebuild with optional preferred selection,
    * the canonical ``blockSignals`` dance around any of the above so a
      programmatic mutation does not pop the selection-changed signal.

    This wrapper holds those concerns once. Render is delegated to a
    caller-supplied callback so the per-list cosmetic differences
    (community: name + icon; channel: ``#{name}`` text; avatar strip:
    icon + tooltip + fixed size) stay with the caller. The
    ``Qt.ItemDataRole.UserRole`` is reserved for the entity id and is
    set by the wrapper -- callers must not overwrite it.
    """

    def __init__(
        self,
        widget: QListWidget,
        *,
        sort_key: Callable[[T], Any],
        render: Callable[[QListWidgetItem, T], None],
    ) -> None:
        self._widget = widget
        self._sort_key = sort_key
        self._render = render
        self._items_by_id: dict[str, QListWidgetItem] = {}
        self._values_by_id: dict[str, T] = {}

    @property
    def widget(self) -> QListWidget:
        return self._widget

    def __contains__(self, key: object) -> bool:
        return isinstance(key, str) and key in self._items_by_id

    def __len__(self) -> int:
        return len(self._items_by_id)

    def get(self, key: str) -> T | None:
        return self._values_by_id.get(key)

    def item_for(self, key: str) -> QListWidgetItem | None:
        return self._items_by_id.get(key)

    def selected_id(self) -> str | None:
        items = self._widget.selectedItems()
        if not items:
            return None
        value = items[0].data(Qt.ItemDataRole.UserRole)
        return value if isinstance(value, str) else None

    def select(self, key: str | None) -> bool:
        """Set the current row to the entry for ``key``.

        Returns True if the row was found and selected, False otherwise.
        Signals are blocked across the call so the caller can decide
        whether to re-fire ``itemSelectionChanged`` itself; this matches
        the pre-existing rebuild contract where the caller invokes the
        ``_*_changed`` slot explicitly after a rebuild.
        """
        item = self._items_by_id.get(key) if key is not None else None
        if item is None:
            return False
        prev = self._widget.signalsBlocked()
        self._widget.blockSignals(True)
        try:
            self._widget.setCurrentItem(item)
        finally:
            self._widget.blockSignals(prev)
        return True

    def upsert(self, key: str, value: T) -> None:
        """Insert ``value`` at its sorted position, or update in place.

        If ``key`` is already present and its sort position would
        change, the row is removed and re-inserted, with the selection
        preserved if it was the current row. Signals are blocked across
        the move so a programmatic resort does not pop spurious
        selection-changed events.
        """
        existing = self._items_by_id.get(key)
        if existing is None:
            self._insert_sorted(key, value)
            return

        new_sort = self._sort_key(value)
        old_value = self._values_by_id[key]
        if new_sort != self._sort_key(old_value):
            was_selected = self._widget.currentItem() is existing
            prev_block = self._widget.signalsBlocked()
            self._widget.blockSignals(True)
            try:
                self._take_row(key)
                self._insert_sorted(key, value)
                if was_selected:
                    new_item = self._items_by_id.get(key)
                    if new_item is not None:
                        self._widget.setCurrentItem(new_item)
            finally:
                self._widget.blockSignals(prev_block)
            return

        self._values_by_id[key] = value
        self._render(existing, value)

    def remove(self, key: str) -> None:
        self._take_row(key)

    def replace_all(
        self,
        values: Iterable[T],
        *,
        key_fn: Callable[[T], str],
        preferred: str | None = None,
    ) -> str | None:
        """Wipe the widget and rebuild from ``values``.

        Signals are blocked for the whole rebuild. The preferred
        selection is honoured when the matching row exists; otherwise
        row 0 is selected. Returns the selected id (which the caller
        may want to write back into its ``_current_*_id`` field), or
        ``None`` if the list ended up empty.
        """
        prev_block = self._widget.signalsBlocked()
        self._widget.blockSignals(True)
        try:
            self._widget.clear()
            self._items_by_id.clear()
            self._values_by_id.clear()
            for value in sorted(values, key=self._sort_key):
                self._insert_at_end(key_fn(value), value)
            if self._widget.count() == 0:
                return None
            row = self._row_for_id(preferred) if preferred is not None else None
            self._widget.setCurrentRow(row if row is not None else 0)
        finally:
            self._widget.blockSignals(prev_block)
        return self.selected_id()

    def clear(self) -> None:
        prev_block = self._widget.signalsBlocked()
        self._widget.blockSignals(True)
        try:
            self._widget.clear()
            self._items_by_id.clear()
            self._values_by_id.clear()
        finally:
            self._widget.blockSignals(prev_block)

    def texts(self) -> list[str]:
        """Snapshot the visible text of every row.

        Used by the sidebar-width-fitting code to size the panel to the
        widest visible name without reaching into the widget itself.
        """
        return [
            item.text()
            for index in range(self._widget.count())
            if (item := self._widget.item(index)) is not None
        ]

    def _insert_sorted(self, key: str, value: T) -> None:
        new_sort = self._sort_key(value)
        insert_index = self._widget.count()
        for index in range(self._widget.count()):
            other_item = self._widget.item(index)
            if other_item is None:
                continue
            other_id = other_item.data(Qt.ItemDataRole.UserRole)
            other = self._values_by_id.get(other_id) if isinstance(other_id, str) else None
            if other is None:
                continue
            if new_sort < self._sort_key(other):
                insert_index = index
                break
        item = QListWidgetItem()
        item.setData(Qt.ItemDataRole.UserRole, key)
        self._render(item, value)
        self._widget.insertItem(insert_index, item)
        self._items_by_id[key] = item
        self._values_by_id[key] = value

    def _insert_at_end(self, key: str, value: T) -> None:
        item = QListWidgetItem()
        item.setData(Qt.ItemDataRole.UserRole, key)
        self._render(item, value)
        self._widget.addItem(item)
        self._items_by_id[key] = item
        self._values_by_id[key] = value

    def _take_row(self, key: str) -> None:
        item = self._items_by_id.pop(key, None)
        self._values_by_id.pop(key, None)
        if item is None:
            return
        row = self._widget.row(item)
        if row >= 0:
            self._widget.takeItem(row)

    def _row_for_id(self, key: str | None) -> int | None:
        if key is None:
            return None
        item = self._items_by_id.get(key)
        if item is None:
            return None
        row = self._widget.row(item)
        return row if row >= 0 else None
