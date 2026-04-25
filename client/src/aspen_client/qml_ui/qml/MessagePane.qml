import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: root
    color: theme.bgMain

    // Threshold (in pixels from the respective edge) at which we issue
    // an older/newer-page fetch. Matches the chunk-on-scroll behaviour
    // of MessagePane._on_message_scroll: the goal is to start the
    // round-trip while there's still scrollback left to render so the
    // user never sees a blank gap.
    readonly property int paginationTriggerPx: 120

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 32
            color: theme.bgPane
            Label {
                anchors.fill: parent
                anchors.leftMargin: 8
                verticalAlignment: Text.AlignVCenter
                text: messagePane.title
                color: theme.textMain
                elide: Text.ElideRight
                font.bold: true
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            DesktopListView {
                id: messages
                anchors.fill: parent
                clip: true
                model: chat.messageModel
                spacing: 4
                cacheBuffer: 600
                // Re-anchor the view to the bottom on first show so the
                // most recent message is visible without the user having
                // to scroll. ``positionViewAtEnd`` is cheaper than
                // ``contentY = contentHeight - height`` because it
                // accounts for delegate height variance.
                onCountChanged: {
                    if (autoScrollPending) {
                        positionViewAtEnd()
                        autoScrollPending = false
                    }
                }
                property bool autoScrollPending: true

                // Scroll-driven pagination. ``contentY`` near the top
                // means the user is reading old history; near the
                // bottom means they're catching up. The
                // ``hasOlder``/``hasNewer`` gates avoid hammering the
                // server with empty pages once we've reached the end
                // in either direction.
                onContentYChanged: {
                    if (!model || count === 0)
                        return
                    if (contentY - originY < root.paginationTriggerPx
                        && messagePane.hasOlder)
                        messagePane.requestOlder()
                    if ((contentHeight - (contentY + height)) < root.paginationTriggerPx
                        && messagePane.hasNewer)
                        messagePane.requestNewer()
                }

                delegate: MessageDelegate {
                    width: ListView.view.width
                }

                ScrollBar.vertical: ScrollBar {}
            }

            // Top-anchor preservation across older-prepend. When the
            // model emits ``pagePrepended(n)`` we capture the height
            // of the n newly-inserted delegates (by inspecting the
            // post-insert ``contentHeight``) and shift ``contentY``
            // by that amount so the user's reading position stays put.
            Connections {
                target: messagePane
                function onPagePrepended(insertedRows) {
                    // Defer one event-loop tick so the ListView has
                    // finished sizing the new delegates. ``Qt.callLater``
                    // posts a metaobject invocation that runs after the
                    // current paint pass.
                    Qt.callLater(function() {
                        let acc = 0
                        for (let i = 0; i < insertedRows; ++i) {
                            const item = messages.itemAtIndex(i)
                            if (item)
                                acc += item.height + messages.spacing
                        }
                        if (acc > 0)
                            messages.contentY += acc
                    })
                }
                function onLiveMessageAppended() {
                    // Live message arrived at the tip; auto-scroll only
                    // if the user was already pinned to the bottom (so
                    // we never yank them out of scrollback they're
                    // actively reading). Margin matches the pagination
                    // trigger threshold for symmetry.
                    const wasAtBottom = (messages.contentHeight
                        - (messages.contentY + messages.height))
                        < root.paginationTriggerPx
                    if (wasAtBottom)
                        messages.positionViewAtEnd()
                }
                function onActiveChannelChanged() {
                    messages.autoScrollPending = true
                }
            }

            // Floating jump-to-latest button. Visible only when the
            // window has unseen newer messages (``hasNewer``). Clicking
            // resets the window to the tip and rebinds the model.
            Button {
                visible: messagePane.hasNewer
                text: qsTr("Jump to latest")
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: 12
                onClicked: messagePane.jumpToLatest()
            }
        }

        // Composer. Enter sends, Shift+Enter inserts a newline; same
        // semantics as the Widgets composer eventFilter.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: composerColumn.implicitHeight + 12
            color: theme.bgPane

            ColumnLayout {
                id: composerColumn
                anchors.fill: parent
                anchors.margins: 6

                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.min(120, Math.max(40, composer.implicitHeight + 12))

                    TextArea {
                        id: composer
                        wrapMode: TextEdit.Wrap
                        placeholderText: qsTr("Write a message\u2026 (Enter to send, Shift+Enter for newline)")
                        color: theme.textMain
                        // ``Keys.onPressed`` runs *before* the default
                        // handler, so ``event.accepted = true`` here
                        // suppresses the newline insert.
                        Keys.onReturnPressed: function(event) {
                            if (event.modifiers & Qt.ShiftModifier) {
                                event.accepted = false
                                return
                            }
                            event.accepted = true
                            sendCurrentDraft()
                        }
                        Keys.onEnterPressed: function(event) {
                            if (event.modifiers & Qt.ShiftModifier) {
                                event.accepted = false
                                return
                            }
                            event.accepted = true
                            sendCurrentDraft()
                        }
                        function sendCurrentDraft() {
                            const draft = text
                            if (!draft || draft.trim().length === 0)
                                return
                            // Snapshot-and-clear sequence mirrors
                            // MessagePane._on_send_clicked: clear the
                            // composer first so any keystrokes the
                            // user types while the send is in flight
                            // land on the now-empty buffer.
                            text = ""
                            messagePane.sendMessage(draft)
                        }
                    }
                }
            }
        }
    }
}
