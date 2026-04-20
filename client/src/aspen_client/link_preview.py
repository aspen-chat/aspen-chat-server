"""Async fetch + caching of link-preview metadata for message bodies.

This module is the render-hot-path cache that ``MessagePane`` consults
when it inserts a message row containing URLs. The design mirrors
``IconCache`` and ``UserDirectory``: the GUI-thread caller asks for the
preview synchronously, a miss kicks off an ``await``-driven fetch via
``TaskSpawner``, and a ready-callback fires back on the GUI thread so
the caller can patch live widgets once the metadata lands.

Nothing here depends on PySide6 widgets — the ``LinkPreview`` dataclass
and the ``LinkPreviewCache`` are both pure model/network code. The UI
layer in ``ui_messages.py`` is solely responsible for turning a
``LinkPreview`` into a QWidget.

Scope deliberately kept narrow for the first iteration:

- Only ``http``/``https`` URLs are fetched. Any other scheme is cached
  as a failure so we don't re-try.
- Response bodies are streamed with a hard 256 KiB ceiling; we stop
  reading the moment we cross it. HTML's ``<head>`` is comfortably
  inside that envelope for any well-behaved site, and malicious or
  misconfigured sites can't stall the UI by dribbling a multi-megabyte
  response at us.
- ``content-type`` is checked before we bother parsing — non-HTML
  responses short-circuit to an empty preview.
- Metadata extraction uses the stdlib ``html.parser``; we pull
  Open Graph (``og:*``), Twitter Card (``twitter:*``), the ``<title>``
  element, ``<meta name="description">``, and ``<meta name="theme-color">``
  (so the UI can brand the card's accent bar to match the site). No
  external HTML-parsing dependency is introduced.
- No images are fetched. ``og:image`` URLs are captured in the dataclass
  so a future iteration can render thumbnails, but the UI does not
  currently request them — that keeps this change free of a second
  class of network round-trip and of the privacy-leak / tracker-pixel
  concerns image loading would introduce.
- Failed fetches are cached as ``None`` so the UI can distinguish
  "we tried and it didn't work" from "we haven't tried yet" and avoid
  re-requesting on every scroll.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from html.parser import HTMLParser
from typing import TYPE_CHECKING
from urllib.parse import urlsplit

import httpx

if TYPE_CHECKING:
    from aspen_client.api_client import TaskSpawner


# Only these two schemes are worth a network round-trip for a preview.
# The UI's own link click-handler (``_SAFE_LINK_SCHEMES`` in
# ``ui_messages.py``) also permits ``mailto:``; we don't preview those
# because there is no page to fetch.
_PREVIEWABLE_SCHEMES = frozenset({"http", "https"})

# Hard upper bound on how much of a response body we'll read before
# giving up on the preview. 256 KiB is deliberately generous — typical
# ``<head>`` sections are under 16 KiB — so badly-ordered pages that put
# ``<meta>`` tags well below the fold still work. Sites that want to
# starve us out of memory can't: once we cross the limit we stop reading
# and parse what we already have.
_MAX_RESPONSE_BYTES = 256 * 1024

# httpx timeouts: total 10s covers connect + read; beyond that the
# preview is simply not coming in time for the user to care about it.
# We don't want a slow upstream to tie up a ``TaskSpawner`` slot forever.
_FETCH_TIMEOUT_SECONDS = 10.0


@dataclass(frozen=True)
class LinkPreview:
    """Immutable bundle of metadata the UI layer renders as a card.

    Every field except ``url`` is optional; the UI decides whether the
    combination is "worth rendering" (currently: at least a title, a
    description, or a site name). ``image_url`` is captured for future
    use but is not rendered today.
    """

    url: str
    title: str | None
    description: str | None
    site_name: str | None
    image_url: str | None
    theme_color: str | None

    def has_content(self) -> bool:
        """Return True if the preview carries metadata worth showing.

        A preview must have at least a title or a description — those
        are the signals that come from the page's own ``<meta>`` /
        ``<title>`` tags. ``site_name`` alone doesn't qualify because
        we synthesise it from the URL's host when no ``og:site_name``
        is present, and a card that reads only ``"example.com"`` adds
        no information the user can't already see in the link itself.
        A ``theme_color`` on its own is also not enough — it's a
        decoration on top of real content, not a reason to render.
        """
        return bool(self.title or self.description)


class _MetaTagExtractor(HTMLParser):
    """Collect ``<meta>``, ``<title>`` and ``<head>``-boundary info.

    Stops accumulating as soon as ``<body>`` is encountered — a
    well-formed document has exhausted its useful metadata by that
    point, and continuing to parse a 200 KiB body of content is pure
    waste. We signal the stop via a flag checked in every handler
    rather than raising, because ``HTMLParser.feed`` traps exceptions
    and we'd lose whatever we'd already gathered.
    """

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.meta: dict[str, str] = {}
        # ``theme-color`` is kept out of ``self.meta`` because HTML5
        # allows multiple ``<meta name="theme-color">`` tags
        # disambiguated by a ``media`` attribute (typically a light /
        # dark colour-scheme split). The single-string ``self.meta``
        # dict would drop every variant after the first; we retain
        # them here in document order so ``_build_preview`` can pick
        # the one matching our UI.
        self.theme_colors: list[tuple[str | None, str]] = []
        self._title_chunks: list[str] = []
        self._in_title = False
        self._in_head = True
        self._done = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if self._done:
            return
        attrs_d = {k.lower(): (v or "") for k, v in attrs}
        if tag == "meta":
            name = attrs_d.get("name", "").lower()
            value = attrs_d.get("content")
            if name == "theme-color" and value:
                media = attrs_d.get("media") or None
                self.theme_colors.append((media, value))
                return
            # Prefer ``property`` (Open Graph / Twitter Card / Article schemas)
            # but fall back to ``name`` for HTML's baseline description tag.
            key = attrs_d.get("property") or attrs_d.get("name")
            if key and value and key not in self.meta:
                self.meta[key.lower()] = value
        elif tag == "title" and self._in_head:
            self._in_title = True
        elif tag == "body":
            self._done = True
            self._in_head = False

    def handle_endtag(self, tag: str) -> None:
        if self._done:
            return
        if tag == "title":
            self._in_title = False
        elif tag == "head":
            self._in_head = False

    def handle_data(self, data: str) -> None:
        if self._done:
            return
        if self._in_title:
            self._title_chunks.append(data)

    def title_text(self) -> str | None:
        if not self._title_chunks:
            return None
        joined = "".join(self._title_chunks).strip()
        return joined or None


def _build_preview(url: str, body_text: str) -> LinkPreview:
    """Parse ``body_text`` and select the best values for each field.

    Precedence matches what most chat clients do: Open Graph first (it's
    the format site authors write *for this purpose*), then Twitter's
    equivalents (sites that only speak Twitter Cards), then the HTML
    baseline ``<title>`` / ``<meta name="description">``. Falling back
    to the URL's host as ``site_name`` is intentional — it gives every
    preview card at least one identifying line even for sites that
    publish no metadata at all.
    """
    extractor = _MetaTagExtractor()
    try:
        extractor.feed(body_text)
    except Exception:
        # Malformed HTML shouldn't poison the preview; salvage whatever
        # we managed to collect before the parser balked.
        pass

    meta = extractor.meta
    title = (
        meta.get("og:title")
        or meta.get("twitter:title")
        or extractor.title_text()
    )
    description = (
        meta.get("og:description")
        or meta.get("twitter:description")
        or meta.get("description")
    )
    site_name = meta.get("og:site_name") or meta.get("application-name")
    if not site_name:
        # Last-resort label so the card is never completely empty.
        host = urlsplit(url).hostname
        if host:
            site_name = host
    image_url = meta.get("og:image") or meta.get("twitter:image")
    theme_color = _select_theme_color(extractor.theme_colors)

    return LinkPreview(
        url=url,
        title=_normalise(title),
        description=_normalise(description),
        site_name=_normalise(site_name),
        image_url=_normalise(image_url),
        theme_color=_normalise(theme_color),
    )


def _select_theme_color(candidates: list[tuple[str | None, str]]) -> str | None:
    """Pick the best ``theme-color`` for Aspen's dark UI.

    Precedence:

    1. A tag whose ``media`` attribute matches the dark colour scheme,
       so sites that ship a split light/dark pair land on the dark
       variant.
    2. A tag with no ``media`` attribute — the site's default, which
       the HTML5 spec treats as applying to every scheme.
    3. Anything else, in document order — last-resort fallback for a
       site that only ships a light-scheme variant; a muted bar is
       still better than no brand cue at all.
    """
    if not candidates:
        return None
    unqualified: str | None = None
    any_media: str | None = None
    for media, value in candidates:
        if media is None:
            if unqualified is None:
                unqualified = value
            continue
        if "prefers-color-scheme" in media.lower() and "dark" in media.lower():
            return value
        if any_media is None:
            any_media = value
    return unqualified or any_media


def _normalise(value: str | None) -> str | None:
    """Collapse whitespace and trim; return None for empty results."""
    if value is None:
        return None
    collapsed = " ".join(value.split())
    return collapsed or None


class LinkPreviewCache:
    """Owns the preview cache, its dedupe set, and the fetch HTTP client.

    Lives next to ``IconCache`` / ``UserDirectory`` on ``ChatWindow``;
    ``MessagePane`` receives it via its constructor and calls three
    methods on it:

    - ``get_preview(url)`` — synchronous hot-path lookup that returns
      the cached ``LinkPreview`` (or ``None`` for either "not yet
      fetched" or "fetch failed"; the UI doesn't need to distinguish).
    - ``is_failed(url)`` — synchronous probe for "fetch completed with
      no usable metadata", so the UI can hide the preview slot rather
      than leaving a perpetual spinner.
    - ``request_preview(url)`` — dedupe-guarded kick-off for the async
      fetch. Returns immediately; ``on_preview_ready`` fires on the GUI
      thread once the bytes land.

    ``aclose`` releases the HTTP client; it must be awaited from
    ``ChatWindow._async_shutdown`` so we don't leak sockets across
    teardown.
    """

    def __init__(
        self,
        tasks: "TaskSpawner",
        on_preview_ready: Callable[[str], None],
    ) -> None:
        self._tasks = tasks
        self._on_preview_ready = on_preview_ready
        # ``None`` in the cache means "fetch completed but yielded no
        # usable preview" — distinct from "not yet fetched" (key absent)
        # so the UI can settle on a final state and stop showing a
        # loading placeholder.
        self._previews: dict[str, LinkPreview | None] = {}
        self._pending: set[str] = set()
        # A dedicated HTTP client so our timeouts, headers, and TLS
        # defaults don't mix with the authenticated Aspen API client.
        # ``follow_redirects=True`` is important because the canonical
        # share URL of nearly every content site redirects to a
        # fully-qualified destination before the meta tags are served.
        self._client = httpx.AsyncClient(
            timeout=_FETCH_TIMEOUT_SECONDS,
            follow_redirects=True,
            headers={
                "User-Agent": "Aspen/0.1 (+link-preview)",
                "Accept": "text/html,application/xhtml+xml",
            },
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    def clear(self) -> None:
        """Drop every cached preview and forget pending fetches.

        Called from ``ChatWindow._reset_client_state`` on a post-grace
        reconnect, alongside the other render-state caches. Previews
        are derived from external sites and could in principle survive
        a server-side resync untouched, but keeping them in lockstep
        with the rest of the per-session caches keeps the reset
        invariant simple (everything render-adjacent is wiped together)
        and costs us at most a second fetch per visible URL.
        """
        self._previews.clear()
        self._pending.clear()

    def get_preview(self, url: str) -> LinkPreview | None:
        """Return the cached preview, or ``None`` if we don't have one yet."""
        return self._previews.get(url)

    def is_failed(self, url: str) -> bool:
        """Return True if we already tried and failed to build a preview.

        Used by the UI to tear down the preview placeholder when a fetch
        completes with nothing worth rendering (non-HTML content type,
        network error, or metadata-free page), rather than leaving the
        slot empty forever.
        """
        return url in self._previews and self._previews[url] is None

    def request_preview(self, url: str) -> None:
        """Kick off an async fetch if one isn't already settled or running.

        The dedupe set + cache check between them ensure we issue at
        most one fetch per URL per session, regardless of how many
        message rows mention it. Same idea as
        ``UserDirectory.request_profiles`` / ``IconCache`` — it's the
        standard shape for render-hot-path caches in this client.
        """
        if url in self._previews or url in self._pending:
            return
        if not self._is_previewable(url):
            # Cache the rejection so repeated render passes don't keep
            # asking.
            self._previews[url] = None
            return
        self._pending.add(url)
        self._tasks.run(
            self._fetch(url),
            on_success=lambda preview, u=url: self._on_fetched(u, preview),
            on_failure=lambda _exc, u=url: self._on_fetched(u, None),
        )

    @staticmethod
    def _is_previewable(url: str) -> bool:
        parts = urlsplit(url)
        return parts.scheme.lower() in _PREVIEWABLE_SCHEMES and bool(parts.netloc)

    async def _fetch(self, url: str) -> LinkPreview | None:
        """Do the HTTP round-trip and parse the response.

        Uses ``stream`` so we can apply ``_MAX_RESPONSE_BYTES`` as a
        true cap (a non-streaming ``get`` would buffer the entire body
        into memory before we had a chance to look at it). Any
        non-200-family status or non-HTML content-type aborts before
        we bother parsing.
        """
        try:
            async with self._client.stream("GET", url) as response:
                if response.status_code >= 400:
                    return None
                content_type = response.headers.get("content-type", "").lower()
                if "html" not in content_type and "xml" not in content_type:
                    return None
                chunks: list[bytes] = []
                total = 0
                async for chunk in response.aiter_bytes():
                    chunks.append(chunk)
                    total += len(chunk)
                    if total >= _MAX_RESPONSE_BYTES:
                        break
                encoding = response.encoding or "utf-8"
        except httpx.HTTPError:
            return None

        raw = b"".join(chunks)[:_MAX_RESPONSE_BYTES]
        try:
            body_text = raw.decode(encoding, errors="replace")
        except LookupError:
            # Unknown declared encoding; fall back to utf-8.
            body_text = raw.decode("utf-8", errors="replace")

        preview = _build_preview(url, body_text)
        return preview if preview.has_content() else None

    def _on_fetched(self, url: str, preview: LinkPreview | None) -> None:
        self._pending.discard(url)
        self._previews[url] = preview
        # Always fire the callback — the UI wants to know about failures
        # too, so it can collapse the placeholder.
        self._on_preview_ready(url)
