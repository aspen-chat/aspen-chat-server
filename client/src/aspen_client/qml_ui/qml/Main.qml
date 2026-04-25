import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    id: root
    width: 1200
    height: 800
    visible: true
    title: qsTr("Aspen Chat (Quick)")

    color: theme.bgMain

    // Two-phase close, mirroring ChatWindow.closeEvent. The first
    // close attempt schedules the async cleanup (which sets
    // app_close_event from inside the still-running qasync loop) and
    // refuses the close so the window stays alive while teardown
    // runs. The Python side does not need to call back here to
    // accept the close: once app_close_event is set, main's
    // run_until_complete returns and the process exits, which closes
    // the window through Qt's normal teardown path.
    property bool _shutdownRequested: false
    onClosing: function(close) {
        if (_shutdownRequested)
            return
        _shutdownRequested = true
        close.accepted = false
        chat.requestShutdown()
    }

    StackView {
        id: stack
        anchors.fill: parent
        initialItem: loginPageComponent
    }

    Component {
        id: loginPageComponent
        LoginView {
            onLoggedIn: stack.replace(chatPageComponent)
        }
    }

    Component {
        id: chatPageComponent
        ChatView {}
    }

    Connections {
        target: login
        function onLoggedIn() {
            if (stack.currentItem && stack.currentItem.objectName !== "chatView") {
                stack.replace(chatPageComponent)
            }
        }
    }
}
