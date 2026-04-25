import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root
    objectName: "chatView"

    Rectangle {
        anchors.fill: parent
        color: theme.bgMain

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // Top toolbar carrying the global actions ChatWindow puts in
            // the menu bar (refresh, create community, create channel,
            // create invite, sidebar collapse toggle).
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 36
                color: theme.bgPane

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 6

                    ToolButton {
                        text: chat.sidebarCollapsed ? qsTr(">") : qsTr("<")
                        ToolTip.visible: hovered
                        ToolTip.text: chat.sidebarCollapsed
                                      ? qsTr("Show communities")
                                      : qsTr("Hide communities")
                        onClicked: chat.setSidebarCollapsed(!chat.sidebarCollapsed)
                    }

                    ToolButton {
                        text: qsTr("Refresh")
                        onClicked: chat.refresh()
                    }
                    ToolButton {
                        text: qsTr("New Community")
                        onClicked: createCommunityDialog.open()
                    }
                    ToolButton {
                        text: qsTr("New Channel")
                        enabled: chat.currentCommunityId !== ""
                        onClicked: createChannelDialog.open()
                    }
                    ToolButton {
                        text: qsTr("Invite")
                        enabled: chat.currentCommunityId !== ""
                        onClicked: chat.createInvite()
                    }

                    Connections {
                        target: chat
                        function onInviteCreated(code) {
                            inviteCodeField.text = code
                            inviteDialog.open()
                        }
                    }

                    Item { Layout.fillWidth: true }

                    Label {
                        text: chat.status
                        color: theme.textMuted
                        elide: Text.ElideRight
                        Layout.maximumWidth: parent.width / 2
                    }
                }
            }

            // The main shell: communities on the left, then a nested
            // SplitView with channels + chat + users.
            SplitView {
                id: outerSplit
                Layout.fillWidth: true
                Layout.fillHeight: true
                orientation: Qt.Horizontal

                CommunityPanel {
                    id: communityPanel
                    SplitView.preferredWidth: chat.sidebarCollapsed ? 56 : 200
                    SplitView.minimumWidth: chat.sidebarCollapsed ? 56 : 120
                    collapsed: chat.sidebarCollapsed
                }

                SplitView {
                    id: innerSplit
                    orientation: Qt.Horizontal
                    SplitView.fillWidth: true

                    ChannelPanel {
                        id: channelPanel
                        SplitView.preferredWidth: 200
                        SplitView.minimumWidth: 120
                    }

                    MessagePane {
                        // Intentionally unnamed: a local id of
                        // ``messagePane`` would shadow the engine's
                        // ``messagePane`` context property in the
                        // inherited binding context inside MessagePane.qml,
                        // breaking ``messagePane.title`` /
                        // ``messagePane.sendMessage`` / the Connections
                        // signal lookups there.
                        SplitView.fillWidth: true
                        SplitView.minimumWidth: 320
                    }

                    UsersPanel {
                        id: usersPanel
                        SplitView.preferredWidth: 200
                        SplitView.minimumWidth: 140
                    }
                }
            }
        }
    }

    // Inline create-community / create-channel / invite dialogs
    // replacing ChatWindow's QInputDialog popups. ``Dialog`` derives
    // its width from its ``contentItem``'s ``implicitWidth``; we set
    // an explicit width on the dialog and make the input children fill
    // it, otherwise a bare ``TextField`` with ``width: 280`` overflows
    // the dialog frame because the dialog doesn't pick that up as a
    // size hint.
    Dialog {
        id: createCommunityDialog
        title: qsTr("Create community")
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        anchors.centerIn: parent
        width: 360
        contentItem: ColumnLayout {
            spacing: 8
            TextField {
                id: createCommunityField
                Layout.fillWidth: true
                placeholderText: qsTr("Community name")
            }
        }
        onAccepted: {
            if (createCommunityField.text.trim().length > 0) {
                chat.createCommunity(createCommunityField.text)
                createCommunityField.text = ""
            }
        }
    }

    Dialog {
        id: createChannelDialog
        title: qsTr("Create channel")
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        anchors.centerIn: parent
        width: 360
        contentItem: ColumnLayout {
            spacing: 8
            TextField {
                id: createChannelField
                Layout.fillWidth: true
                placeholderText: qsTr("Channel name")
            }
        }
        onAccepted: {
            if (createChannelField.text.trim().length > 0) {
                chat.createChannel(createChannelField.text)
                createChannelField.text = ""
            }
        }
    }

    // Surfaced from ``ChatController.inviteCreated``. The code goes
    // into a read-only ``TextField`` so the user can triple-click the
    // value or rely on the explicit ``Copy`` button which routes the
    // string through ``QGuiApplication.clipboard()``. Putting it in the
    // status bar (the previous behaviour) was easy to miss and forced
    // the user to retype the code by hand.
    Dialog {
        id: inviteDialog
        title: qsTr("Invite code")
        modal: true
        standardButtons: Dialog.Close
        anchors.centerIn: parent
        width: 420
        contentItem: ColumnLayout {
            spacing: 10
            Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: theme.textMuted
                text: qsTr(
                    "Share this invite code with someone you want to add "
                    + "to this community. The code is one-time use."
                )
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                TextField {
                    id: inviteCodeField
                    Layout.fillWidth: true
                    readOnly: true
                    selectByMouse: true
                    color: theme.textMain
                    font.family: "monospace"
                    onFocusChanged: if (focus) selectAll()
                }
                Button {
                    id: copyInviteButton
                    text: qsTr("Copy")
                    onClicked: {
                        chat.copyToClipboard(inviteCodeField.text)
                        copyFeedbackTimer.restart()
                        copyInviteButton.text = qsTr("Copied")
                    }
                    Timer {
                        id: copyFeedbackTimer
                        interval: 1500
                        onTriggered: copyInviteButton.text = qsTr("Copy")
                    }
                }
            }
        }
        onOpened: {
            inviteCodeField.forceActiveFocus()
            inviteCodeField.selectAll()
        }
    }
}
