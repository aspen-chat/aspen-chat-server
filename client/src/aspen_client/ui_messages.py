from __future__ import annotations

from collections.abc import Callable

from PySide6.QtCore import QSize, Qt, QUrl, Signal
from PySide6.QtGui import (
    QColor,
    QDesktopServices,
    QMouseEvent,
    QPixmap,
    QTextCharFormat,
    QTextCursor,
    QTextDocument,
)
from PySide6.QtWidgets import (
    QFrame,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from aspen_client.api_client import AspenApiClient, TaskSpawner
from aspen_client.icons import IconCache, material_icon
from aspen_client.link_preview import PREVIEW_IMAGE_SIZE, LinkPreviewImageCache
from aspen_client.state import MESSAGE_WINDOW_CAP, ClientState
from aspen_client.theme import (
    COLOR_ACCENT,
    COLOR_BG_MAIN,
    COLOR_BG_PANE,
    COLOR_TEXT_MAIN,
    COLOR_TEXT_MUTED,
)
from aspen_client.types import LinkPreview, Message

# Tunable limits for the bidirectional message window. INITIAL_MESSAGE_LOAD
# fills the viewport with one round-trip on channel switch; MESSAGE_PAGE_SIZE
# is used for subsequent older/newer loads. ``SCROLL_EDGE_PIXELS`` is how
# close to the top/bottom of the viewport the user must scroll before we
# trigger the next page.
INITIAL_MESSAGE_LOAD = 50
MESSAGE_PAGE_SIZE = 50
SCROLL_EDGE_PIXELS = 200

# Markdown render flags for message bodies.
#
# ``MarkdownDialectGitHub`` is the same default QTextDocument uses (so we
# keep tables, strikethrough, autolinked bare URLs, etc. that GitHub-flavored
# Markdown adds on top of CommonMark). ``MarkdownNoHTML`` is the load-bearing
# bit: it tells the underlying MD4C parser to refuse to pass raw HTML — both
# block-level (``<script>…</script>``) and inline (``<b>…</b>``) — through to
# the rendered output. With it, anything that looks like an HTML tag in the
# source is emitted as escaped text (``&lt;script&gt;``) rather than as a
# live element, so a user typing ``<b>hi</b>`` in the composer sees those
# six characters in the message list everywhere downstream and cannot smuggle
# styling, scripts, or other markup past the renderer. This is the canonical
# Qt-native way to render untrusted markdown safely; it is strictly stronger
# than HTML-escaping the source first (which preserved entities through code
# fences and made literal angle brackets in code blocks render double-escaped
# as ``&lt;``).
MARKDOWN_FEATURES = (
    QTextDocument.MarkdownFeature.MarkdownDialectGitHub
    | QTextDocument.MarkdownFeature.MarkdownNoHTML
)

# URL schemes the message-body click handler is willing to hand to
# ``QDesktopServices.openUrl``. Everything else (``javascript:``, ``data:``,
# ``file:``, ``vbscript:``, custom application schemes registered on the
# user's machine, etc.) is refused outright with a status-bar message.
# Anchors that survive ``_disarm_misleading_links`` already have visible
# text matching their href modulo a small set of GFM autolink prefixes, so
# this list is what the user can actually have read off the screen before
# clicking — keep it narrow.
_SAFE_LINK_SCHEMES = frozenset({"http", "https", "mailto"})


def _render_markdown_to_html(content: str) -> str:
    """Convert an Aspen message body (markdown) to rich-text HTML.

    The body is parsed into a ``QTextDocument``, passed through
    ``_disarm_misleading_links`` so no anchor can hide a deceptive
    destination behind friendly text, and then serialised back to HTML
    ready for ``QLabel.setText`` at ``Qt.TextFormat.RichText``.

    Link-preview extraction is *not* done here. The server parses the
    message body itself, fetches the metadata asynchronously, and
    publishes the authoritative preview list on the
    ``messageLinkPreviewsReady`` event; the client renders preview
    cards from ``Message.link_previews`` rather than re-scanning the
    body, which avoids a second tree walk on the render hot-path and
    keeps the client's preview set identical to the server's.
    """
    doc = QTextDocument()
    doc.setMarkdown(content, MARKDOWN_FEATURES)
    _disarm_misleading_links(doc)
    return doc.toHtml()


def _is_faithful_anchor(text: str, href: str) -> bool:
    """Return True if ``href`` is something the user can read in ``text``.

    Used by ``_disarm_misleading_links`` to decide whether a parsed
    anchor is a hidden link (must be disarmed) or a transparent autolink
    (safe to leave clickable). The exact-match case covers explicit
    autolinks (``<https://x.test>``) and self-form inline links. The
    scheme-prefix cases cover the GFM autolink shortcuts where MD4C
    inserts the scheme on the user's behalf — ``foo@example.com``
    becoming ``mailto:foo@example.com``, and ``www.example.com``
    becoming ``http(s)://www.example.com`` — neither of which conceals
    the destination from the reader.
    """
    if text == href:
        return True
    if href == f"mailto:{text}":
        return True
    if href in (f"http://{text}", f"https://{text}"):
        return True
    return False


def _disarm_misleading_links(doc: QTextDocument) -> None:
    """Rewrite anchors whose visible text doesn't match their href.

    A markdown link like ``[click here](https://scamsite.example)`` is a
    classic phishing primitive: the user sees ``click here`` and clicks
    expecting one destination, but the actual ``href`` is somewhere
    else. Stripping HTML at parse time (via ``MarkdownNoHTML``) closes
    the raw-``<a>``/``<script>`` hole, but markdown's own inline-link
    and reference-link syntaxes can still construct the same mismatch
    purely in markdown source, so we have to handle that here.

    The rule we apply: an anchor stays clickable iff the visible text
    is a faithful representation of its href. That means

    - ``text == href`` (autolinked bare URLs like ``https://example.com``
      and self-form ``[https://x.test](https://x.test)``); and
    - the GFM autolink shapes where the parser added a scheme prefix
      to text the user typed without one — ``foo@example.com`` rendered
      with href ``mailto:foo@example.com``, and ``www.example.com``
      rendered with href ``http://www.example.com`` /
      ``https://www.example.com``.

    Every other anchor is replaced with the literal markdown source
    form ``[text](href)``, with default character formatting so it
    carries no link colour, no underline, and no clickability. The user
    sees both halves of the deception (``click here`` *and*
    ``https://scamsite.example``) and decides what to do.

    Implementation note: we collect target fragments first and then
    mutate from the end of the document backwards. Inserting longer
    text shifts every position to its right, so processing in reverse
    keeps the not-yet-rewritten anchor positions valid as we go.
    """
    targets: list[tuple[int, int, str, str]] = []
    block = doc.firstBlock()
    while block.isValid():
        it = block.begin()
        while not it.atEnd():
            frag = it.fragment()
            if frag.isValid():
                fmt = frag.charFormat()
                if fmt.isAnchor():
                    href = fmt.anchorHref()
                    text = frag.text()
                    if href and not _is_faithful_anchor(text, href):
                        targets.append(
                            (frag.position(), frag.position() + frag.length(), text, href)
                        )
            it += 1
        block = block.next()

    if not targets:
        return

    cursor = QTextCursor(doc)
    plain_format = QTextCharFormat()
    for start, end, text, href in reversed(targets):
        cursor.setPosition(start)
        cursor.setPosition(end, QTextCursor.MoveMode.KeepAnchor)
        cursor.insertText(f"[{text}]({href})", plain_format)


class _LinkPreviewCard(QFrame):
    """Single preview card rendered beneath a message body.

    Constructed from a server-provided :class:`LinkPreview` record, so
    the text half (title / description / site name / theme colour)
    lands on the first render pass without a network round-trip. If the
    record carries an ``image_id``, the card also asks the
    :class:`LinkPreviewImageCache` for the thumbnail; a hit paints
    synchronously, a miss schedules a fetch and the ``handle_preview_image_ready``
    callback patches the live thumbnail once the bytes land.

    Clicks route through a ``clicked(url)`` signal so ``MessagePane``
    can reuse the same ``_open_message_link`` scheme-allowlist it uses
    for in-body link activations. That keeps card-click semantics
    identical to clicking the link inside the message itself.
    """

    clicked = Signal(str)

    # Object-name so ``_size_message_item`` can find and width-pin every
    # card in a row without having to know about ``_LinkPreviewCard``
    # through ``findChild(type)``, which is more expensive than
    # ``findChildren(QWidget, name)``.
    OBJECT_NAME = "messageLinkPreviewCard"

    def __init__(
        self,
        preview: LinkPreview,
        image_cache: LinkPreviewImageCache,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._preview = preview
        self._image_cache = image_cache
        self.setObjectName(self.OBJECT_NAME)
        self.setFrameShape(QFrame.Shape.NoFrame)
        self.setCursor(Qt.CursorShape.PointingHandCursor)
        # A muted, slightly-lighter-than-bubble background plus a
        # full-height accent bar on the left visually separates the card
        # from the message body above it while echoing the Aspen palette.
        # The bar colour is driven by the server-supplied ``theme_color``
        # when present, falling back to Aspen's own ``COLOR_ACCENT``.
        if preview.theme_color and QColor(preview.theme_color).isValid():
            accent = preview.theme_color
        else:
            accent = COLOR_ACCENT
        self._apply_accent(accent)

        row = QHBoxLayout(self)
        row.setContentsMargins(10, 6, 10, 6)
        row.setSpacing(8)

        text_column = QWidget(self)
        text_column.setStyleSheet("background: transparent;")
        text_layout = QVBoxLayout(text_column)
        text_layout.setContentsMargins(0, 0, 0, 0)
        text_layout.setSpacing(2)

        if preview.site_name:
            site_label = QLabel(preview.site_name, text_column)
            site_label.setStyleSheet(
                f"color: {COLOR_TEXT_MUTED}; font-size: 10px;"
            )
            site_label.setWordWrap(False)
            text_layout.addWidget(site_label)

        if preview.title:
            title_label = QLabel(preview.title, text_column)
            title_label.setStyleSheet(
                f"color: {COLOR_TEXT_MAIN}; font-weight: 600;"
            )
            title_label.setWordWrap(True)
            text_layout.addWidget(title_label)

        if preview.description:
            description_label = QLabel(preview.description, text_column)
            description_label.setStyleSheet(f"color: {COLOR_TEXT_MUTED};")
            description_label.setWordWrap(True)
            text_layout.addWidget(description_label)

        row.addWidget(text_column, 1)

        # Thumbnail slot. Only materialised when the server says there
        # is an image to show; otherwise the layout stays tight against
        # the text column and doesn't reserve space for a picture that
        # will never arrive.
        self._thumbnail_label: QLabel | None = None
        if preview.image_id is not None:
            self._thumbnail_label = QLabel(self)
            self._thumbnail_label.setObjectName("messageLinkPreviewThumbnail")
            self._thumbnail_label.setFixedSize(PREVIEW_IMAGE_SIZE, PREVIEW_IMAGE_SIZE)
            self._thumbnail_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            self._thumbnail_label.setStyleSheet("background: transparent;")
            row.addWidget(
                self._thumbnail_label,
                0,
                alignment=Qt.AlignmentFlag.AlignTop,
            )
            self._apply_thumbnail()
            if not image_cache.has_settled(preview.image_id):
                image_cache.request_image(preview.image_id)

    @property
    def url(self) -> str:
        return self._preview.url

    @property
    def image_id(self) -> str | None:
        return self._preview.image_id

    def refresh_thumbnail(self) -> None:
        """Repaint the thumbnail from the cache.

        Called by ``MessagePane.handle_preview_image_ready`` once the
        background fetch for ``self.image_id`` lands. No-op if the card
        was built without a thumbnail slot (the server published no
        image for this URL); a settled-but-failed fetch leaves the
        QLabel's pixmap null so the slot stays empty rather than
        occupying space with a broken placeholder.
        """
        if self._thumbnail_label is None:
            return
        self._apply_thumbnail()

    def _apply_thumbnail(self) -> None:
        assert self._thumbnail_label is not None
        assert self._preview.image_id is not None
        pixmap = self._image_cache.get_pixmap(self._preview.image_id)
        if pixmap is not None:
            self._thumbnail_label.setPixmap(pixmap)
        else:
            self._thumbnail_label.clear()

    def _apply_accent(self, color: str) -> None:
        """Install the card's stylesheet with ``color`` as the left bar.

        Only the border-left colour is parameterised; the background,
        rounded-corner geometry, and text colours stay anchored to the
        Aspen palette so a pathological ``theme-color`` cannot destroy
        the card's visual identity.
        """
        self.setStyleSheet(
            f"QFrame#{self.OBJECT_NAME} {{"
            f" background-color: {COLOR_BG_MAIN};"
            f" border-left: 3px solid {color};"
            " border-top-left-radius: 0px;"
            " border-bottom-left-radius: 0px;"
            " border-top-right-radius: 4px;"
            " border-bottom-right-radius: 4px;"
            "}"
        )

    def mousePressEvent(self, event: QMouseEvent) -> None:  # type: ignore[override]
        if event.button() == Qt.MouseButton.LeftButton:
            self.clicked.emit(self._preview.url)
            event.accept()
            return
        super().mousePressEvent(event)


class MessagePane(QWidget):
    """Message list, scroll-driven pagination, and the jump-to-latest button.

    The five sliding-window invariants from ``client/AGENTS.md`` are realised
    here together with ``ClientState``:

    1. ``has_newer``-True channels drop live events (enforced by
       ``ClientState.upsert_message``; ``handle_message_event`` defers to
       state for the gating decision).
    2. ``MESSAGE_WINDOW_CAP`` is enforced on both paged reads and live
       appends — ``_on_message_page_loaded`` calls
       ``evict_older_to_cap`` / ``evict_newer_to_cap`` after every page,
       and live appends in ``handle_message_event`` follow up with
       ``_evict_ui_rows_not_in_window`` to drop matching rows.
    3. ``_capture_top_anchor`` / ``_restore_top_anchor`` pin the topmost
       visible message across older-prepends so the user's scroll position
       does not jump.
    4. The jump-to-latest button is shown iff ``window.has_newer`` is True
       (see ``_update_jump_to_latest_button``).
    5. Sending while scrolled back resets the window via
       ``handle_message_sent`` — the cached slice is dropped and a fresh
       initial fetch is dispatched so the user's outbound message lands at
       the new tip rather than in a non-contiguous slice.

    The pane keeps no copy of profile or avatar state — the chat window
    owns those caches and supplies the four callbacks to query/refresh
    them, which keeps the AGENTS.md ``TaskSpawner`` rule local to the
    object that holds the API client reference.
    """

    _SIZED_AT_WIDTH_ROLE = Qt.ItemDataRole.UserRole + 100

    def __init__(
        self,
        state: ClientState,
        api: AspenApiClient,
        tasks: TaskSpawner,
        icons: IconCache,
        link_preview_images: LinkPreviewImageCache,
        *,
        profile_resolver: Callable[[list[Message]], None],
        header_formatter: Callable[[Message], str],
        avatar_pixmap: Callable[[str, int], QPixmap],
        status_setter: Callable[[str], None],
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._state = state
        self._api = api
        self._tasks = tasks
        self._icons = icons
        self._link_preview_images = link_preview_images
        self._profile_resolver = profile_resolver
        self._format_header = header_formatter
        self._avatar_pixmap = avatar_pixmap
        self._set_status = status_setter

        self._current_channel_id: str | None = None
        self._message_items_by_id: dict[str, QListWidgetItem] = {}
        self._loading_header_item: QListWidgetItem | None = None
        self._loading_footer_item: QListWidgetItem | None = None
        self._pending_fetches: dict[str, set[str]] = {}
        # imageId → set of message ids whose rows currently have a
        # thumbnail slot expecting this image. Populated when a row is
        # built with a preview that carries an ``image_id`` and
        # consumed by ``handle_preview_image_ready`` so a landed
        # thumbnail only touches the rows that actually asked for it
        # rather than walking the entire window. Entries are pruned
        # when rows are evicted (``_evict_ui_rows_not_in_window``,
        # ``remove_message_row``) so the bookkeeping stays bounded.
        self._image_subscribers: dict[str, set[str]] = {}

        self.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)

        self._messages_list = QListWidget(self)
        self._messages_list.setStyleSheet(
            f"background-color: {COLOR_BG_PANE}; color: {COLOR_TEXT_MAIN}; border: none;"
        )
        self._messages_list.setVerticalScrollMode(QListWidget.ScrollMode.ScrollPerPixel)
        self._messages_list.verticalScrollBar().valueChanged.connect(self._on_messages_scrolled)
        layout.addWidget(self._messages_list, 1)

        # Jump-to-latest is parented to ``self`` (not the list widget) so it
        # can float over the bottom of the message viewport without being
        # clipped by the list's own paint pass. Its position is recomputed
        # in ``resizeEvent`` whenever it's visible.
        self._jump_to_latest_button = QToolButton(self)
        self._jump_to_latest_button.setText("Jump to latest")
        self._jump_to_latest_button.setIcon(material_icon("arrow-down-circle", color=COLOR_BG_PANE))
        self._jump_to_latest_button.setIconSize(QSize(16, 16))
        self._jump_to_latest_button.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        self._jump_to_latest_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self._jump_to_latest_button.setStyleSheet(
            f"QToolButton {{ background-color: {COLOR_TEXT_MAIN}; color: {COLOR_BG_PANE};"
            f"border: 1px solid {COLOR_BG_PANE}; border-radius: 12px; padding: 4px 12px; }}"
        )
        self._jump_to_latest_button.clicked.connect(self._jump_to_latest_clicked)
        self._jump_to_latest_button.hide()

    # ---------- public surface used by ChatWindow ----------

    def set_active_channel(self, channel_id: str | None) -> None:
        """Switch the visible channel.

        If a window is already cached for the channel, it's replayed
        without a network round-trip; otherwise an initial fetch is
        dispatched. The jump-to-latest visibility is recomputed at the end
        so the affordance always matches the new ``window.has_newer``.
        """
        self._current_channel_id = channel_id
        self._clear_message_view()
        if channel_id is None:
            self._update_jump_to_latest_button()
            return
        window = self._state.channel_windows.get(channel_id)
        if window is not None and window.ordered_ids:
            self._render_cached_window()
        else:
            self._dispatch_message_fetch(channel_id, "initial", None, INITIAL_MESSAGE_LOAD)
        self._update_jump_to_latest_button()

    def handle_message_event(self, message: Message, event_type: str) -> None:
        """Apply a server message event to the visible list.

        The state layer's ``has_newer`` gate has already filtered out
        creates the user shouldn't see (they're reading older history); by
        the time we're here, the message has been appended to the current
        window.
        """
        if event_type == "create":
            if message.id in self._message_items_by_id:
                self._update_message_row(message)
            else:
                self._append_message_row(message)
                # State may have evicted older rows when the append pushed
                # the window past the cap; drop matching UI rows to stay
                # in sync.
                self._evict_ui_rows_not_in_window()
        elif event_type == "update":
            self._update_message_row(message)

    def handle_message_sent(self, message: Message) -> None:
        """Local echo for the user's own outbound message.

        Sliding-window invariant 5: if the user was reading older history,
        their own message only makes sense as the new tip. Drop the
        cached window and re-fetch from the server so we don't end up with
        a non-contiguous slice.
        """
        window = self._state.channel_windows.get(message.channel_id)
        if window is not None and window.has_newer:
            self._state.clear_channel_window(message.channel_id)
            if message.channel_id == self._current_channel_id:
                self._clear_message_view()
                self._dispatch_message_fetch(
                    message.channel_id,
                    "initial",
                    None,
                    INITIAL_MESSAGE_LOAD,
                )
                self._update_jump_to_latest_button()
        else:
            self._state.upsert_message(message)
            # Show the outbound message immediately via the incremental
            # path; when the matching server event echoes back,
            # handle_message_event will find the row already present and
            # fall through to a cheap update.
            if message.channel_id == self._current_channel_id:
                if message.id in self._message_items_by_id:
                    self._update_message_row(message)
                else:
                    self._append_message_row(message)
                    self._evict_ui_rows_not_in_window()

    def remove_message_row(self, message_id: str) -> None:
        item = self._message_items_by_id.pop(message_id, None)
        if item is None:
            return
        row = self._messages_list.row(item)
        if row >= 0:
            self._messages_list.takeItem(row)

    def relayout(self) -> None:
        """Recompute every row's width-based size hint.

        Called from the chat window's ``resizeEvent`` so the splitter or
        sidebar collapse can resize this pane and the message rows wrap to
        the new viewport width on the same frame.
        """
        for index in range(self._messages_list.count()):
            item = self._messages_list.item(index)
            if item is None:
                continue
            self._size_message_item(item)

    def clear_for_resync(self) -> None:
        """Drop UI bookkeeping — called from ``ChatWindow._reset_client_state``."""
        self._message_items_by_id.clear()
        self._loading_header_item = None
        self._loading_footer_item = None
        self._pending_fetches.clear()
        self._image_subscribers.clear()

    def handle_link_previews_ready(self, message_id: str) -> None:
        """Rebuild a row's preview cards after a ``messageLinkPreviewsReady`` event.

        The state layer has just replaced the cached message's
        ``link_previews`` with the authoritative list the server
        published for it; here we translate that into new
        :class:`_LinkPreviewCard` children on the row, tearing down any
        cards that belonged to the previous preview set. Any row that
        isn't currently materialised (evicted past the window cap, or
        for a channel the user isn't viewing) is silently skipped —
        the cache update is what matters; the next time the row is
        built, it reads the updated ``link_previews`` directly.
        """
        item = self._message_items_by_id.get(message_id)
        if item is None:
            return
        widget = self._messages_list.itemWidget(item)
        if not isinstance(widget, QWidget):
            return
        previews_container = widget.findChild(QWidget, "messagePreviewsContainer")
        if not isinstance(previews_container, QWidget):
            return
        message = self._state.messages.get(message_id)
        if message is None:
            return
        self._reset_preview_cards(previews_container, message_id, message.link_previews)
        # Preview rows shift the card stack height, so force a re-sizing
        # pass: without this the row keeps whatever height it had
        # before the cards landed and the last card gets clipped.
        item.setData(self._SIZED_AT_WIDTH_ROLE, None)
        self._size_message_item(item)

    def handle_preview_image_ready(self, image_id: str) -> None:
        """Patch every row whose preview card subscribed to ``image_id``.

        The callback wire runs :class:`LinkPreviewImageCache` →
        ``ChatWindow._on_link_preview_image_ready`` → here. We consult
        ``_image_subscribers`` to find only the rows that built a
        thumbnail slot for this id and ask each card to repaint itself
        from the cache; the card already knows whether the fetch
        succeeded (paint the pixmap) or failed (leave the slot empty).
        We still invalidate the row's width marker so a newly-taller
        thumbnail slot doesn't clip against a stale cached height.
        """
        subscribers = self._image_subscribers.get(image_id)
        if not subscribers:
            return
        for message_id in list(subscribers):
            item = self._message_items_by_id.get(message_id)
            if item is None:
                subscribers.discard(message_id)
                continue
            container = self._messages_list.itemWidget(item)
            if not isinstance(container, QWidget):
                continue
            for card in container.findChildren(
                _LinkPreviewCard, _LinkPreviewCard.OBJECT_NAME
            ):
                if card.image_id == image_id:
                    card.refresh_thumbnail()
            item.setData(self._SIZED_AT_WIDTH_ROLE, None)
            self._size_message_item(item)

    def refresh_author_row(self, user_id: str) -> None:
        """Re-paint visible message rows authored by ``user_id``.

        Called when a user profile load completes so the header gets the
        real display name and the avatar gets a fresh render in place.
        """
        for msg_id, item in self._message_items_by_id.items():
            message = self._state.messages.get(msg_id)
            if message is None or message.author != user_id:
                continue
            widget = self._messages_list.itemWidget(item)
            if widget is None:
                continue
            header_label = widget.findChild(QLabel, "messageHeaderLabel")
            if isinstance(header_label, QLabel):
                header_label.setText(self._format_header(message))
            avatar_label = widget.findChild(QLabel, "messageAvatarLabel")
            if isinstance(avatar_label, QLabel):
                avatar_label.setPixmap(self._avatar_pixmap(user_id, 28))

    # ---------- Qt event hooks ----------

    def resizeEvent(self, event) -> None:  # type: ignore[override]
        super().resizeEvent(event)
        if self._jump_to_latest_button.isVisible():
            self._position_jump_to_latest_button()
        self.relayout()

    # ---------- private helpers (moved verbatim from ChatWindow) ----------

    def _clear_message_view(self) -> None:
        """Drop all row widgets and sentinel rows for the message list."""
        self._messages_list.clear()
        self._message_items_by_id.clear()
        self._loading_header_item = None
        self._loading_footer_item = None
        # Every row that could have been subscribed to a preview image
        # is now gone, so the subscriber map is as stale as it gets.
        # Wiping it wholesale is cheaper and safer than iterating what
        # used to be there.
        self._image_subscribers.clear()

    def _render_cached_window(self) -> None:
        """Rebuild the message view from the currently-cached window."""
        if self._current_channel_id is None:
            return
        messages = self._state.get_messages_for_channel(self._current_channel_id)
        self._profile_resolver(messages)
        for message in messages:
            self._add_message_item(message)
        self.relayout()
        window = self._state.channel_windows.get(self._current_channel_id)
        if window is None or not window.has_newer:
            self._messages_list.scrollToBottom()

    def _add_message_item(self, message: Message) -> None:
        """Append a message row at the end of the list.

        Used by full re-renders (``_render_cached_window``). For
        incremental inserts that need to land in the middle, use
        ``_insert_message_item_at``.
        """
        row = self._messages_list.count()
        if self._loading_footer_item is not None:
            row -= 1
        self._insert_message_item_at(max(row, 0), message)

    def _dispatch_message_fetch(
        self,
        channel_id: str,
        direction: str,
        anchor_id: str | None,
        count: int,
    ) -> None:
        """Kick off a page fetch if one isn't already in flight in this direction."""
        in_flight = self._pending_fetches.setdefault(channel_id, set())
        if direction in in_flight:
            return
        in_flight.add(direction)
        if direction == "older":
            self._show_loading_sentinel("header")
        elif direction == "newer":
            self._show_loading_sentinel("footer")
        if direction == "older":
            coro = self._api.read_channel_messages(channel_id, before=anchor_id, count=count)
        elif direction == "newer":
            coro = self._api.read_channel_messages(channel_id, after=anchor_id, count=count)
        else:
            # "initial" — no anchor means "most recent page".
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
        """Apply a paginated page onto the window and the message list."""
        in_flight = self._pending_fetches.get(channel_id)
        if in_flight is not None:
            in_flight.discard(direction)

        # Use the raw (pre-dedupe) count to decide whether the server has
        # more history in this direction. A full page means "probably more";
        # a short page means "we hit the end".
        raw_count = len(messages)
        hit_end = raw_count < requested_count

        # The anchor id itself is echoed back inclusively by the server's
        # le/ge filters; strip it so we don't double-render.
        window = self._state.channel_windows.get(channel_id)
        if window is not None:
            known = set(window.ordered_ids)
            messages = [m for m in messages if m.id not in known]

        if direction == "initial":
            # Replace the window entirely. An initial fetch at the tip
            # implies we are sitting on the newest message, so ``has_newer``
            # is False. ``has_older`` depends on whether the page was full.
            self._state.set_channel_window(
                channel_id,
                messages,
                has_older=not hit_end,
                has_newer=False,
            )
        elif direction == "older":
            self._state.merge_channel_page(channel_id, messages)
            window = self._state.channel_windows.get(channel_id)
            if window is not None and hit_end:
                window.has_older = False
        elif direction == "newer":
            self._state.merge_channel_page(channel_id, messages)
            window = self._state.channel_windows.get(channel_id)
            if window is not None and hit_end:
                window.has_newer = False

        # If the user switched channels while the fetch was in flight, state
        # was still updated above so a return trip to this channel is warm.
        # The message list is now showing a different channel, and its
        # sentinels were cleared by set_active_channel, so there's nothing
        # left to do on the UI side.
        if channel_id != self._current_channel_id:
            return

        self._hide_loading_sentinel("header")
        self._hide_loading_sentinel("footer")

        if direction == "initial":
            # Full re-render from the fresh window.
            self._clear_message_view()
            self._render_cached_window()
            self._update_jump_to_latest_button()
            return

        top_anchor = self._capture_top_anchor() if direction == "older" else None

        self._profile_resolver(messages)
        for message in messages:
            if message.id in self._message_items_by_id:
                continue
            row = self._compute_insert_row_for(message.id)
            self._insert_message_item_at(row, message)

        self.relayout()

        # Enforce the window cap from the end opposite to the one we just
        # loaded. State stays authoritative about which ids are retained;
        # we mirror by removing UI rows for ids that fell out.
        if direction == "older":
            self._state.evict_newer_to_cap(channel_id, MESSAGE_WINDOW_CAP)
        else:
            self._state.evict_older_to_cap(channel_id, MESSAGE_WINDOW_CAP)
        self._evict_ui_rows_not_in_window()

        if top_anchor is not None:
            self._restore_top_anchor(top_anchor)

        self._update_jump_to_latest_button()

    def _open_message_link(self, url: str) -> None:
        """Click handler for links inside rendered message bodies.

        Connected to every body ``QLabel.linkActivated``. By the time we
        get here, ``_disarm_misleading_links`` has guaranteed that the
        anchor's visible text is a faithful representation of ``url``,
        so the user has read where they're going. We still gate on a
        scheme allowlist before handing off to ``QDesktopServices.openUrl``
        so that, e.g., a ``[file:///etc/passwd](file:///etc/passwd)``
        self-form link — which has matching text and href and would
        therefore survive the disarm pass — cannot trick the user into
        opening a local file just because the destination was visible.
        ``QDesktopServices.openUrl`` may also call out to scheme handlers
        the user has registered for arbitrary custom protocols, and we
        do not want a chat message to be a launcher for those either.
        """
        parsed = QUrl(url)
        scheme = parsed.scheme().lower()
        if scheme not in _SAFE_LINK_SCHEMES:
            self._set_status(
                f"Refused to open link with unsupported scheme '{scheme}': {url}"
            )
            return
        if not QDesktopServices.openUrl(parsed):
            self._set_status(f"Failed to open link in default browser: {url}")

    def _on_message_page_failed(self, channel_id: str, direction: str, error: str) -> None:
        in_flight = self._pending_fetches.get(channel_id)
        if in_flight is not None:
            in_flight.discard(direction)
        if direction == "older":
            self._hide_loading_sentinel("header")
        elif direction == "newer":
            self._hide_loading_sentinel("footer")
        if channel_id == self._current_channel_id:
            self._set_status(f"Message fetch failed: {error}")

    def _on_messages_scrolled(self, _value: int) -> None:
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        window = self._state.channel_windows.get(channel_id)
        if window is None:
            return
        bar = self._messages_list.verticalScrollBar()
        if window.has_older and bar.value() <= SCROLL_EDGE_PIXELS:
            oldest = window.ordered_ids[0] if window.ordered_ids else None
            if oldest is not None:
                self._dispatch_message_fetch(channel_id, "older", oldest, MESSAGE_PAGE_SIZE)
        if window.has_newer and (bar.maximum() - bar.value()) <= SCROLL_EDGE_PIXELS:
            newest = window.ordered_ids[-1] if window.ordered_ids else None
            if newest is not None:
                self._dispatch_message_fetch(channel_id, "newer", newest, MESSAGE_PAGE_SIZE)

    def _compute_insert_row_for(self, message_id: str) -> int:
        """Find the widget-row index for ``message_id`` based on the current window."""
        if self._current_channel_id is None:
            return self._messages_list.count()
        window = self._state.channel_windows.get(self._current_channel_id)
        if window is None:
            return self._messages_list.count()
        try:
            window_index = window.ordered_ids.index(message_id)
        except ValueError:
            return self._messages_list.count()
        # Walk the window in order and find the Nth already-materialized row.
        widget_row = 0
        if self._loading_header_item is not None:
            widget_row += 1
        for window_pos, mid in enumerate(window.ordered_ids):
            if window_pos == window_index:
                return widget_row
            if mid in self._message_items_by_id:
                widget_row += 1
        return widget_row

    def _insert_message_item_at(self, row: int, message: Message) -> None:
        item = QListWidgetItem()
        item.setFlags(Qt.ItemFlag.NoItemFlags)
        container = QWidget(self._messages_list)
        container.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        container_layout = QHBoxLayout(container)
        container_layout.setContentsMargins(6, 3, 6, 3)
        container_layout.setSpacing(8)

        avatar_label = QLabel(container)
        avatar_label.setObjectName("messageAvatarLabel")
        avatar_label.setFixedSize(28, 28)
        avatar_label.setPixmap(self._avatar_pixmap(message.author, 28))
        avatar_label.setAlignment(Qt.AlignmentFlag.AlignTop)
        container_layout.addWidget(avatar_label, 0, alignment=Qt.AlignmentFlag.AlignTop)

        # Right column: rich-text header line + markdown body + optional
        # link-preview cards. Aspen message content is markdown; we
        # render it through ``_render_markdown_to_html`` (which sets
        # ``MarkdownNoHTML``) so the parser strips raw HTML from the
        # source before producing the rich-text output. Link previews
        # are populated from ``message.link_previews`` below rather
        # than scanned out of the body here — the server is the
        # authoritative source for which URLs get a card.
        text_column = QWidget(container)
        text_column.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        text_column_layout = QVBoxLayout(text_column)
        text_column_layout.setContentsMargins(0, 0, 0, 0)
        text_column_layout.setSpacing(2)

        header_label = QLabel(self._format_header(message), text_column)
        header_label.setObjectName("messageHeaderLabel")
        header_label.setTextFormat(Qt.TextFormat.RichText)
        header_label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        header_label.setWordWrap(False)
        header_label.setContentsMargins(0, 0, 0, 0)
        text_column_layout.addWidget(header_label)

        body_html = _render_markdown_to_html(message.content)
        body_label = QLabel(text_column)
        body_label.setObjectName("messageBodyLabel")
        body_label.setTextFormat(Qt.TextFormat.RichText)
        body_label.setText(body_html)
        body_label.setTextInteractionFlags(
            Qt.TextInteractionFlag.TextSelectableByMouse
            | Qt.TextInteractionFlag.LinksAccessibleByMouse
            | Qt.TextInteractionFlag.LinksAccessibleByKeyboard
        )
        body_label.setWordWrap(True)
        body_label.setContentsMargins(0, 0, 0, 0)
        body_label.setStyleSheet(f"color: {COLOR_TEXT_MAIN};")
        # ``openExternalLinks`` stays False so Qt routes link clicks to
        # our own ``linkActivated`` slot rather than passing the URL
        # straight to ``QDesktopServices.openUrl``. The slot enforces a
        # scheme allowlist (``_SAFE_LINK_SCHEMES``) before dispatching,
        # which is the second half of the safe-link contract: the disarm
        # pass guarantees the URL is *visible* to the reader, and the
        # click handler guarantees only schemes the OS-default handler
        # is safe for (http/https/mailto) actually launch.
        body_label.linkActivated.connect(self._open_message_link)
        text_column_layout.addWidget(body_label)

        # Preview container: always created (so ``messageLinkPreviewsReady``
        # events arriving after the row is materialised can swap its
        # children without restructuring the layout), empty when the
        # server published no preview-worthy links for this message.
        # ``_build_preview_cards`` populates it from
        # ``message.link_previews`` and registers the message id as a
        # subscriber for each card's ``image_id`` so a landed thumbnail
        # fetch only repaints the rows that asked for it.
        previews_container = QWidget(text_column)
        previews_container.setObjectName("messagePreviewsContainer")
        previews_container.setStyleSheet(f"background-color: {COLOR_BG_PANE};")
        previews_layout = QVBoxLayout(previews_container)
        previews_layout.setContentsMargins(0, 4, 0, 0)
        previews_layout.setSpacing(4)
        text_column_layout.addWidget(previews_container)
        self._build_preview_cards(
            previews_container, message.id, message.link_previews
        )

        container_layout.addWidget(text_column, 1)

        self._messages_list.insertItem(row, item)
        self._messages_list.setItemWidget(item, container)
        self._message_items_by_id[message.id] = item

    def _build_preview_cards(
        self,
        previews_container: QWidget,
        message_id: str,
        previews: list[LinkPreview],
    ) -> None:
        """Materialize preview cards for a row from server-provided records.

        Attaches one :class:`_LinkPreviewCard` per entry in
        ``previews`` (the server already enforces the per-message
        preview cap, so no client-side limit is applied here). Each
        card is populated synchronously from the ``LinkPreview`` record
        itself, so the first paint shows the final text layout without
        any loading placeholder; only the thumbnail image may still be
        outstanding, in which case the card kicks off a fetch via
        :class:`LinkPreviewImageCache` and ``handle_preview_image_ready``
        patches the thumbnail slot once the bytes land.

        The method also registers ``message_id`` as a subscriber for
        each card's ``image_id``, so a later-arriving thumbnail (or a
        thumbnail shared by a *different* row referencing the same id)
        only repaints the rows that actually need updating.
        """
        layout = previews_container.layout()
        for preview in previews:
            card = _LinkPreviewCard(preview, self._link_preview_images, previews_container)
            card.clicked.connect(self._open_message_link)
            layout.addWidget(card)
            if preview.image_id is not None:
                self._image_subscribers.setdefault(preview.image_id, set()).add(
                    message_id
                )

    def _evict_ui_rows_not_in_window(self) -> None:
        if self._current_channel_id is None:
            return
        window = self._state.channel_windows.get(self._current_channel_id)
        if window is None:
            return
        valid = set(window.ordered_ids)
        stale_ids = [mid for mid in self._message_items_by_id if mid not in valid]
        for mid in stale_ids:
            item = self._message_items_by_id.pop(mid, None)
            if item is None:
                continue
            self._drop_preview_subscriptions(item, mid)
            row = self._messages_list.row(item)
            if row >= 0:
                self._messages_list.takeItem(row)

    def _drop_preview_subscriptions(self, item: QListWidgetItem, message_id: str) -> None:
        """Remove ``message_id`` from the subscriber list of each card's image.

        Called just before a row is evicted so a later-arriving
        ``handle_preview_image_ready`` doesn't try to repaint a card on
        a widget Qt has already freed. Walks the row's cards via
        object-name rather than tracking image ids separately on the
        row, which keeps the subscription set in exactly one place
        (the widget tree). Cards without a thumbnail never entered the
        subscriber map, so we just skip them.
        """
        container = self._messages_list.itemWidget(item)
        if not isinstance(container, QWidget):
            return
        for card in container.findChildren(_LinkPreviewCard, _LinkPreviewCard.OBJECT_NAME):
            image_id = card.image_id
            if image_id is None:
                continue
            subscribers = self._image_subscribers.get(image_id)
            if subscribers is None:
                continue
            subscribers.discard(message_id)
            if not subscribers:
                self._image_subscribers.pop(image_id, None)

    def _capture_top_anchor(self) -> tuple[str, int] | None:
        """Record the id + pixel offset of the topmost visible message row.

        Used to preserve the user's visual position across a prepend: after
        inserting older rows, the content shifts downward and we compensate
        by advancing the scrollbar by exactly that delta.
        """
        viewport_top = 0
        for row in range(self._messages_list.count()):
            item = self._messages_list.item(row)
            if item is None:
                continue
            rect = self._messages_list.visualItemRect(item)
            if rect.bottom() < viewport_top:
                continue
            # Sentinel rows don't correspond to a real message; skip them.
            msg_id = next(
                (mid for mid, it in self._message_items_by_id.items() if it is item),
                None,
            )
            if msg_id is None:
                continue
            return msg_id, rect.top()
        return None

    def _restore_top_anchor(self, anchor: tuple[str, int]) -> None:
        msg_id, saved_top = anchor
        item = self._message_items_by_id.get(msg_id)
        if item is None:
            return
        new_top = self._messages_list.visualItemRect(item).top()
        bar = self._messages_list.verticalScrollBar()
        bar.setValue(bar.value() + (new_top - saved_top))

    def _show_loading_sentinel(self, position: str) -> None:
        """Insert a non-interactive "loading..." row at top or bottom."""
        if position == "header" and self._loading_header_item is not None:
            return
        if position == "footer" and self._loading_footer_item is not None:
            return

        item = QListWidgetItem()
        item.setFlags(Qt.ItemFlag.NoItemFlags)
        label = QLabel(
            "Loading older messages..." if position == "header" else "Loading newer messages...",
            self._messages_list,
        )
        label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        label.setContentsMargins(6, 6, 6, 6)
        label.setStyleSheet(f"color: {COLOR_TEXT_MUTED}; font-style: italic;")

        if position == "header":
            self._messages_list.insertItem(0, item)
            self._loading_header_item = item
        else:
            self._messages_list.addItem(item)
            self._loading_footer_item = item
        self._messages_list.setItemWidget(item, label)
        item.setSizeHint(label.sizeHint())

    def _hide_loading_sentinel(self, position: str) -> None:
        if position == "header":
            item = self._loading_header_item
            self._loading_header_item = None
        else:
            item = self._loading_footer_item
            self._loading_footer_item = None
        if item is None:
            return
        row = self._messages_list.row(item)
        if row >= 0:
            self._messages_list.takeItem(row)

    def _update_jump_to_latest_button(self) -> None:
        visible = False
        if self._current_channel_id is not None:
            window = self._state.channel_windows.get(self._current_channel_id)
            if window is not None and window.has_newer:
                visible = True
        self._jump_to_latest_button.setVisible(visible)
        if visible:
            self._position_jump_to_latest_button()

    def _position_jump_to_latest_button(self) -> None:
        button = self._jump_to_latest_button
        button.adjustSize()
        size = button.sizeHint()
        margin = 16
        x = (self.width() - size.width()) // 2
        y = self.height() - size.height() - margin
        button.move(max(x, margin), max(y, margin))
        button.raise_()

    def _jump_to_latest_clicked(self) -> None:
        channel_id = self._current_channel_id
        if channel_id is None:
            return
        # Nuke the current window (plus its cached message records) and
        # start from the server tip. This matches the behavior the user
        # expects from "catch me up to now" chips in other chat clients.
        self._state.clear_channel_window(channel_id)
        self._clear_message_view()
        self._dispatch_message_fetch(channel_id, "initial", None, INITIAL_MESSAGE_LOAD)
        self._update_jump_to_latest_button()

    def _append_message_row(self, message: Message) -> None:
        # Fill in the author profile (and its avatar-cache entry) before we
        # paint the row so the first render shows the real display name.
        self._profile_resolver([message])
        self._add_message_item(message)
        item = self._message_items_by_id.get(message.id)
        if item is not None:
            self._size_message_item(item)
        self._messages_list.scrollToBottom()

    def _update_message_row(self, message: Message) -> None:
        item = self._message_items_by_id.get(message.id)
        if item is None:
            return
        widget = self._messages_list.itemWidget(item)
        if widget is None:
            return
        header_label = widget.findChild(QLabel, "messageHeaderLabel")
        if isinstance(header_label, QLabel):
            header_label.setText(self._format_header(message))
        body_label = widget.findChild(QLabel, "messageBodyLabel")
        # Re-render through the ``MarkdownNoHTML`` pipeline so the same
        # HTML-stripping guarantee holds for edited messages. Preview
        # cards are driven separately by ``message.link_previews`` —
        # the server clears them synchronously on an edit and republishes
        # a fresh set via ``messageLinkPreviewsReady`` once its
        # async fetch lands, so we just rebuild from whatever the cached
        # record currently carries.
        body_html = _render_markdown_to_html(message.content)
        if isinstance(body_label, QLabel):
            body_label.setText(body_html)
        previews_container = widget.findChild(QWidget, "messagePreviewsContainer")
        if isinstance(previews_container, QWidget):
            self._reset_preview_cards(
                previews_container, message.id, message.link_previews
            )
        # Content changed — invalidate the cached-at-width marker so the
        # next sizing pass actually recomputes heightForWidth.
        item.setData(self._SIZED_AT_WIDTH_ROLE, None)
        self._size_message_item(item)

    def _reset_preview_cards(
        self,
        previews_container: QWidget,
        message_id: str,
        previews: list[LinkPreview],
    ) -> None:
        """Tear down the row's existing cards and rebuild from ``previews``.

        An edit or a ``messageLinkPreviewsReady`` event can introduce a
        completely different set of links (a user might replace a URL,
        strip them all, or the server might resolve new metadata), so
        the cheapest correct answer is to wipe the row's preview slots
        and rebuild from the authoritative list. Image subscribers for
        the old cards are pruned here so a late-arriving thumbnail
        fetch for an image the row no longer references doesn't touch
        its widgets.
        """
        for card in list(
            previews_container.findChildren(_LinkPreviewCard, _LinkPreviewCard.OBJECT_NAME)
        ):
            image_id = card.image_id
            if image_id is not None:
                subscribers = self._image_subscribers.get(image_id)
                if subscribers is not None:
                    subscribers.discard(message_id)
                    if not subscribers:
                        self._image_subscribers.pop(image_id, None)
            card.setParent(None)
            card.deleteLater()
        self._build_preview_cards(previews_container, message_id, previews)

    def _size_message_item(self, item: QListWidgetItem) -> None:
        """Apply width + size-hint to one message row.

        Shared by the incremental-append path (``_append_message_row`` /
        ``_update_message_row``) and the bulk relayout path (``relayout``).
        Keeping a single implementation guarantees the outbound user
        message lands with the same margins, the same x-placement of its
        avatar/body, and the same wrapping as messages arriving via any
        other path.

        Critically, we pin both the container widget and the row sizeHint
        to the viewport's full width. If we let ``adjustSize`` shrink the
        widget to its children's natural width, QListView can end up
        placing a narrower widget at a different x offset than its
        siblings (this is what made a freshly-sent message appear shifted
        right of all the previously-rendered rows). Forcing every row to
        the same width eliminates that class of drift entirely.

        The per-row viewport-width cache is the main lever for very long
        messages: computing ``heightForWidth`` on a word-wrapped QLabel
        with, say, 14 kB of content costs real CPU time, and doing it
        redundantly every time a page is inserted adds up. We only
        re-measure when the viewport width actually changed from the last
        time we sized this row; content edits invalidate the cache via
        ``_update_message_row``.
        """
        widget = self._messages_list.itemWidget(item)
        if not isinstance(widget, QWidget):
            return
        body_label = widget.findChild(QLabel, "messageBodyLabel")
        if not isinstance(body_label, QLabel):
            return
        viewport_width = max(self._messages_list.viewport().width(), 200)
        cached_width = item.data(self._SIZED_AT_WIDTH_ROLE)
        if cached_width == viewport_width:
            return
        # 16px subtracts a conservative scrollbar/inner-padding allowance;
        # 44px subtracts the avatar column (28) + layout spacing (8) + left
        # container margin (6) + right container margin (~2 of slop).
        body_width = max(viewport_width - 16 - 44, 120)
        body_label.setFixedWidth(body_width)
        # Link-preview cards share the body's column, so they have to
        # get the same width pin — otherwise a wide description label
        # inside a card can stretch the row past ``viewport_width`` and
        # disagree with the row's own sizeHint (which was computed
        # against the pinned body width), producing visible ghost
        # scrollbars on the list.
        for card in widget.findChildren(_LinkPreviewCard, _LinkPreviewCard.OBJECT_NAME):
            card.setFixedWidth(body_width)
        widget.setFixedWidth(viewport_width)
        widget.ensurePolished()
        layout = widget.layout()
        if layout is not None:
            layout.activate()
        item.setSizeHint(QSize(viewport_width, widget.sizeHint().height()))
        item.setData(self._SIZED_AT_WIDTH_ROLE, viewport_width)
