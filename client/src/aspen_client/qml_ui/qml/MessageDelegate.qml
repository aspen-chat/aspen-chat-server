import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root
    height: outerColumn.implicitHeight + 8

    // Pin the per-row link-preview list to a delegate-scoped property so
    // the Repeater binding doesn't shadow ``model`` against itself when
    // the QML engine resolves the inner ``model:`` assignment.
    property var linkPreviews: model && model.linkPreviews ? model.linkPreviews : []

    RowLayout {
        id: outerColumn
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 8

        Image {
            Layout.preferredWidth: 28
            Layout.preferredHeight: 28
            // Pin the avatar to the top of the row so it lines up with
            // the author header rather than drifting to the row's
            // vertical centre as bodies grow taller.
            Layout.alignment: Qt.AlignTop
            sourceSize.width: 28; sourceSize.height: 28
            fillMode: Image.PreserveAspectCrop
            // ``avatarEpoch`` is a per-author monotonic counter that
            // the controller bumps whenever an avatar fetch (or
            // profile load) lands. Embedding it as a query string
            // forces QML to re-resolve the URL through the image
            // provider so the cache hit replaces the placeholder.
            source: "image://aspen/user/" + model.author + "/28?v=" + model.avatarEpoch
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            RowLayout {
                spacing: 8
                Label {
                    text: model.authorName
                    color: theme.textMain
                    font.bold: true
                }
                Label {
                    text: model.timestamp
                    color: theme.textMuted
                    font.pixelSize: 11
                }
            }

            // Body. ``markdown.render`` runs the same MarkdownNoHTML
            // + _disarm_misleading_links pipeline the Widgets path
            // uses, so raw HTML in a message can never render as an
            // active element. ``onLinkActivated`` routes through the
            // controller's allow-list (http/https/mailto) before
            // handing the URL to ``QDesktopServices``.
            //
            // ``TextEdit`` (rather than ``Label`` / ``Text``) is what
            // unlocks click-and-drag selection plus the platform's
            // standard copy keyboard shortcut. ``readOnly: true``
            // suppresses the editing surface, ``cursorVisible: false``
            // hides the blinking caret that would otherwise appear on
            // focus, and the explicit ``activeFocusOnPress`` keeps
            // Ctrl+C wired up so users can copy a selected snippet
            // without having to right-click.
            TextEdit {
                id: bodyLabel
                Layout.fillWidth: true
                wrapMode: TextEdit.Wrap
                color: theme.textMain
                textFormat: TextEdit.RichText
                text: markdown.render(model.content)
                readOnly: true
                selectByMouse: true
                selectByKeyboard: true
                persistentSelection: true
                cursorVisible: false
                activeFocusOnPress: true
                selectionColor: theme.accent
                onLinkActivated: function(link) {
                    messagePane.openLink(link)
                }
            }

            // Server-authoritative link previews. The model surfaces a
            // QVariantList of plain dicts (URL, title, description,
            // image id, theme colour) that LinkPreviewCard reads
            // directly. We deliberately don't rescan the body for
            // URLs.
            Repeater {
                model: root.linkPreviews
                LinkPreviewCard {
                    Layout.fillWidth: true
                    Layout.topMargin: 4
                    preview: modelData
                }
            }
        }
    }
}
