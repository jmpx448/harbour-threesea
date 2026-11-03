// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property alias firstName: firstField.text
    property alias lastName: lastField.text

    Column {
        width: parent.width

        DialogHeader {
            //% "Save"
            acceptText: qsTrId("threesea-he-save")
        }

        TextField {

            inputMethodHints: window.incognitoHints
            id: firstField
            width: parent.width
            focus: true
            //% "First name"
            label: qsTrId("threesea-la-first-name")
            placeholderText: label
            EnterKey.iconSource: "image://theme/icon-m-enter-next"
            EnterKey.onClicked: lastField.focus = true
        }

        TextField {

            inputMethodHints: window.incognitoHints
            id: lastField
            width: parent.width
            //% "Last name"
            label: qsTrId("threesea-la-last-name")
            placeholderText: label
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
    }
}
