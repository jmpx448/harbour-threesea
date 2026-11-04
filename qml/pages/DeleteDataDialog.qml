// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

Dialog {
    id: dialog
    objectName: "holdsPin"
    allowedOrientations: Orientation.All

    // The caller runs the actual removal (with a remorse timer) once this dialog is gone:
    // anything started from here would outlive the dialog.
    canAccept: understood.checked

    // The app lock's PIN for the caller's removal (window.pushWithPin).
    property string pin

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Remove"
                acceptText: qsTrId("threesea-he-remove")
            }

            InfoLabel {
                //% "This removes your Threema ID, contacts, chats and settings from this device. Your ID itself is not revoked: you can restore it later from Threema Safe or an ID backup — without one of them, the ID is lost."
                text: qsTrId("threesea-la-delete-data-intro")
            }

            TextSwitch {
                id: understood
                //% "I have a backup or no longer need this ID"
                text: qsTrId("threesea-la-delete-data-confirm")
            }
        }
    }
}
