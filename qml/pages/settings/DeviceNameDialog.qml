// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Rename a device of the device group (Settings → Linked devices). For this device the name
// goes out to the others (its label); for another device it stays here (an alias), since a
// device's own label is encrypted with the device group key.
Dialog {
    id: dialog
    property alias name: field.text
    property bool current: false
    // Empty is allowed: the device goes back to its default name (ours) or to its own (theirs).

    Column {
        width: parent.width
        DialogHeader {
            //% "Save"
            acceptText: qsTrId("threesea-he-save")
            //% "Rename device"
            title: qsTrId("threesea-he-rename-device")
        }
        TextField {
            id: field
            inputMethodHints: window.incognitoHints
            width: parent.width
            focus: true
            //% "Device name"
            label: qsTrId("threesea-la-device-name")
            placeholderText: label
            //% "The other devices show this name for this phone."
            description: dialog.current ? qsTrId("threesea-la-device-name-own-description")
                                          //% "Only this phone shows this name; the device keeps its own name everywhere else."
                                        : qsTrId("threesea-la-device-name-alias-description")
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
    }
}
