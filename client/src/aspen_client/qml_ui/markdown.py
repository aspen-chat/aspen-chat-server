"""QML-side bridge for rendering message bodies through the safe pipeline.

Qt Quick's ``Text { textFormat: Text.MarkdownText }`` does not expose
the underlying MD4C ``MarkdownNoHTML`` flag, so a body containing raw
``<script>`` would render as a live element if we let QML parse the
markdown itself. To preserve the same guarantee the Widgets pane
gives, the QML delegate calls :meth:`MarkdownBridge.render`, which
delegates straight to :func:`aspen_client.ui_messages._render_markdown_to_html`
\u2014 the canonical implementation that already runs the markdown through
``MarkdownNoHTML`` and then through ``_disarm_misleading_links`` to
strip any anchor whose visible text doesn't match its href.

The bridge is exposed as a context property named ``markdown`` from
:func:`aspen_client.qml_ui.app.quick_main`; see the QML delegates for
how it's invoked. Link clicks are still routed through
``MessagePaneController.openLink`` so the ``_SAFE_LINK_SCHEMES``
allowlist applies identically to both UIs.
"""

from __future__ import annotations

from PySide6.QtCore import QObject, Slot

from aspen_client.ui_messages import _render_markdown_to_html


class MarkdownBridge(QObject):
    """Stateless ``QObject`` exposing the markdown renderer to QML."""

    @Slot(str, result=str)
    def render(self, content: str) -> str:
        """Return sanitised rich-text HTML for a message body.

        Always safe to call with untrusted input \u2014 the underlying
        renderer strips raw HTML at parse time and disarms anchors
        whose visible text disagrees with their ``href``.
        """
        return _render_markdown_to_html(content)
