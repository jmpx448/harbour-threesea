// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// "Export chat" on a contact, group or list page: the whole chat as a ZIP in Downloads (Android's
// "Export chat"). The window runs it: it can take longer than this page stays open.
ValueButton {
    id: root

    property string identity
    // Shown in the dialog.
    property string name

    //% "Export chat"
    label: qsTrId("threesea-me-export-chat")
    onClicked: {
        var dialog = pageStack.push(Qt.resolvedUrl("../pages/ExportChatDialog.qml"),
                                    { chatName: root.name || root.identity })
        dialog.accepted.connect(function() { window.exportChat(root.identity, dialog.media, dialog.password) })
    }
}
