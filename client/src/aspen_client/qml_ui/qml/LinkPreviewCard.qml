import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: root
    // ``preview`` is a plain dict from MessageListModel.LinkPreviewsRole.
    // Keys: ``url``, ``title``, ``description``, ``siteName``,
    // ``imageUrl``, ``themeColor``. Strings are always present (empty
    // string for absent fields) so we can use them in bindings without
    // null guards.
    property var preview: ({})

    color: theme.bgPane
    border.color: theme.textMuted
    border.width: 1
    radius: 4
    implicitHeight: contentRow.implicitHeight + 12

    RowLayout {
        id: contentRow
        anchors.fill: parent
        anchors.margins: 6
        spacing: 8

        // Accent bar drawn from ``themeColor`` if the server provided
        // one; falls back to the global accent so the card is still
        // visually distinguishable from plain message text.
        Rectangle {
            Layout.preferredWidth: 4
            Layout.fillHeight: true
            radius: 2
            color: preview && preview.themeColor && preview.themeColor.length > 0
                   ? preview.themeColor
                   : theme.accent
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            Label {
                visible: preview && preview.siteName && preview.siteName.length > 0
                text: preview ? preview.siteName : ""
                color: theme.textMuted
                font.pixelSize: 11
            }

            Label {
                visible: preview && preview.title && preview.title.length > 0
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: preview ? "<a href='" + preview.url + "' style='color:" + theme.accent + "'>"
                                + preview.title + "</a>"
                              : ""
                textFormat: Text.RichText
                color: theme.textMain
                onLinkActivated: function(link) {
                    messagePane.openLink(link)
                }
            }

            Label {
                visible: preview && preview.description && preview.description.length > 0
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: preview ? preview.description : ""
                color: theme.textMain
                font.pixelSize: 12
            }
        }

        Image {
            visible: preview && preview.imageUrl && preview.imageUrl.length > 0
            Layout.preferredWidth: 72
            Layout.preferredHeight: 72
            sourceSize.width: 72; sourceSize.height: 72
            fillMode: Image.PreserveAspectCrop
            // The provider expects a percent-encoded URL in the path
            // segment so the colons and slashes from the server-supplied
            // ``imageUrl`` don't collide with image://aspen/<kind>/<key>.
            source: preview && preview.imageUrl && preview.imageUrl.length > 0
                    ? "image://aspen/preview/" + encodeURIComponent(preview.imageUrl)
                    : ""
        }
    }
}
