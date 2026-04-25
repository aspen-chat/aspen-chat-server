import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: root
    color: theme.bgPane

    ListView {
        id: list
        anchors.fill: parent
        anchors.margins: 4
        clip: true
        model: chat.channels
        spacing: 1
        acceptedButtons: Qt.NoButton

        delegate: ItemDelegate {
            width: ListView.view.width
            highlighted: chat.currentChannelId === model.id
            onClicked: chat.selectChannel(model.id)
            contentItem: Label {
                text: "#" + model.name
                color: theme.textMain
                elide: Text.ElideRight
            }
        }
    }
}
