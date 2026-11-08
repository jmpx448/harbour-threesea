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
    property bool linkedDevices

    Component.onCompleted: backend.call("multiDevice", {}, function(ok, result) {
        if (ok) dialog.linkedDevices = result.active === true
    })

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
                //% "This removes your Threema ID, contacts, chats, media and settings from this device."
                text: qsTrId("threesea-la-delete-data-goes")
            }

            Repeater {
                model: [
                    //% "Your ID itself is not revoked: you can restore it later from Threema Safe, an ID backup or a Data backup — without one of them, the ID is lost. Chats and media come back only from a Data backup."
                    qsTrId("threesea-la-delete-data-comes-back-data-backup"),
                    //% "Pictures saved to the Threesea folder in Pictures, and backups and files you exported, stay on this device. A Threema Safe backup stays on the server."
                    qsTrId("threesea-la-delete-data-stays")
                ]
                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * x
                    wrapMode: Text.Wrap
                    color: Theme.highlightColor
                    font.pixelSize: Theme.fontSizeSmall
                    text: modelData
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                visible: dialog.linkedDevices
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                font.pixelSize: Theme.fontSizeSmall
                //% "Your linked devices are removed from this ID first, which takes a connection. If this phone can't reach the server then, they stay linked and keep the ID."
                text: qsTrId("threesea-la-delete-data-linked")
            }

            TextSwitch {
                id: understood
                //% "I have a Threema Safe, ID or Data backup, or no longer need this ID"
                text: qsTrId("threesea-la-delete-data-have-a-backup")
            }
        }

        VerticalScrollDecorator { }
    }
}
