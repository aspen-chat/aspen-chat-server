import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: root
    color: theme.bgPane
    property bool collapsed: false

    // Two ListView siblings sharing the same model. We toggle which one
    // is visible based on ``collapsed`` so the avatar strip can have a
    // very different delegate from the full text+icon row without
    // forcing a model swap on the controller side.
    ListView {
        id: fullList
        anchors.fill: parent
        anchors.margins: 4
        clip: true
        visible: !root.collapsed
        model: chat.communities
        spacing: 2
        currentIndex: -1
        acceptedButtons: Qt.NoButton

        delegate: ItemDelegate {
            width: ListView.view.width
            highlighted: chat.currentCommunityId === model.id
            onClicked: chat.selectCommunity(model.id)
            contentItem: RowLayout {
                spacing: 8
                Image {
                    width: 28; height: 28
                    sourceSize.width: 28; sourceSize.height: 28
                    source: "image://aspen/community/" + model.id + "?v=" + model.iconEpoch
                    fillMode: Image.PreserveAspectCrop
                }
                Label {
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    text: model.name
                    color: theme.textMain
                }
            }
        }
    }

    ListView {
        id: avatarStrip
        anchors.fill: parent
        anchors.margins: 4
        clip: true
        visible: root.collapsed
        model: chat.communities
        spacing: 4
        acceptedButtons: Qt.NoButton

        delegate: Item {
            width: avatarStrip.width
            height: 44
            Rectangle {
                anchors.centerIn: parent
                width: 32; height: 32
                radius: 16
                color: "transparent"
                border.color: chat.currentCommunityId === model.id
                              ? theme.accent
                              : "transparent"
                border.width: 2
                Image {
                    anchors.centerIn: parent
                    width: 28; height: 28
                    sourceSize.width: 28; sourceSize.height: 28
                    source: "image://aspen/community/" + model.id + "?v=" + model.iconEpoch
                    fillMode: Image.PreserveAspectCrop
                }
            }
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: chat.selectCommunity(model.id)
            }
        }
    }
}
