from __future__ import annotations

from PySide6.QtGui import (
    QTextCharFormat,
    QTextCursor,
    QTextDocument,
)

# Tunable limits for the bidirectional message window. INITIAL_MESSAGE_LOAD
# fills the viewport with one round-trip on channel switch; MESSAGE_PAGE_SIZE
# is used for subsequent older/newer loads.
INITIAL_MESSAGE_LOAD = 50
MESSAGE_PAGE_SIZE = 50

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

def render_markdown_to_html(content: str) -> str:
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
