import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: root
    color: theme.bgPane

    DesktopListView {
        id: list
        anchors.fill: parent
        anchors.margins: 4
        clip: true
        model: chat.users
        spacing: 2

        delegate: Item {
            width: ListView.view.width
            height: 28

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                spacing: 8

                // Presence dot. Filled for online/away, hollow ring for
                // offline. Mirrors IconCache.presence_dot_pixmap.
                Rectangle {
                    Layout.preferredWidth: 10
                    Layout.preferredHeight: 10
                    radius: 5
                    border.color: theme.presenceRing(model.status)
                    border.width: 1
                    color: {
                        const fill = theme.presenceColor(model.status)
                        return fill === "" ? "transparent" : fill
                    }
                }

                Image {
                    Layout.preferredWidth: 20
                    Layout.preferredHeight: 20
                    sourceSize.width: 20; sourceSize.height: 20
                    source: "image://aspen/user/" + model.id + "/20?v=" + model.iconEpoch
                    fillMode: Image.PreserveAspectCrop
                }

                Label {
                    Layout.fillWidth: true
                    text: model.name
                    color: theme.textMain
                    elide: Text.ElideRight
                }
            }
        }
    }
}
