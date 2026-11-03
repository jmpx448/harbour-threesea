// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Dialog {
    id: dialog
    property alias name: field.text
    canAccept: field.text.trim().length > 0

    Column {
        width: parent.width
        DialogHeader {
            //% "Save"
            acceptText: qsTrId("threesea-he-save")
            //% "Rename group"
            title: qsTrId("threesea-he-rename-group")
        }
        TextField {
            inputMethodHints: window.incognitoHints
            id: field
            width: parent.width
            focus: true
            label: qsTrId("threesea-la-group-name")
            placeholderText: label
            EnterKey.enabled: text.trim().length > 0
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
    }
}
