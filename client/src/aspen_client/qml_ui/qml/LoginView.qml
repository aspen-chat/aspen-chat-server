import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root
    objectName: "loginView"
    signal loggedIn

    Connections {
        target: login
        function onLoggedIn() { root.loggedIn() }
    }

    Rectangle {
        anchors.fill: parent
        color: theme.bgMain

        ColumnLayout {
            anchors.centerIn: parent
            width: 360
            spacing: 12

            Label {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Aspen Chat")
                color: theme.textMain
                font.pixelSize: 22
                font.bold: true
            }

            Label {
                text: qsTr("Username")
                color: theme.textMuted
            }
            TextField {
                id: usernameField
                Layout.fillWidth: true
                placeholderText: qsTr("Your username")
                enabled: !login.busy
                color: theme.textMain
                onAccepted: passwordField.forceActiveFocus()
            }

            Label {
                text: qsTr("Password")
                color: theme.textMuted
            }
            TextField {
                id: passwordField
                Layout.fillWidth: true
                placeholderText: qsTr("Your password")
                echoMode: TextInput.Password
                enabled: !login.busy
                color: theme.textMain
                onAccepted: loginButton.clicked()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Button {
                    id: loginButton
                    text: qsTr("Log In")
                    enabled: !login.busy
                    Layout.fillWidth: true
                    onClicked: login.login(usernameField.text, passwordField.text)
                }

                Button {
                    text: qsTr("Create User")
                    enabled: !login.busy
                    Layout.fillWidth: true
                    onClicked: login.createUser(usernameField.text, passwordField.text)
                }
            }

            Label {
                Layout.fillWidth: true
                Layout.minimumHeight: 24
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                color: theme.textMuted
                text: login.status
            }
        }
    }
}
