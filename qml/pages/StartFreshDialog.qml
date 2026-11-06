// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// The stored data can't be opened (a forgotten passphrase, a missing key, damage): what starting
// fresh deletes. The caller runs it once this dialog is gone.
Dialog {
    id: dialog
    objectName: "startFresh"
    allowedOrientations: Orientation.All

    canAccept: understood.checked

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Start fresh"
                acceptText: qsTrId("threesea-he-start-fresh")
            }

            InfoLabel {
                //% "This deletes Threesea's own data on this phone: chats, contacts, media, settings and your Threema ID. Backups and pictures saved to your folders stay."
                text: qsTrId("threesea-la-start-fresh-goes")
            }

            Repeater {
                model: [
                    //% "You can then restore your ID from Threema Safe, an ID backup or a Data backup, and your chats from a Data backup. Without one of them, the ID is lost."
                    qsTrId("threesea-la-start-fresh-comes-back"),
                    //% "Devices linked to your ID stay linked to it."
                    qsTrId("threesea-la-start-fresh-linked")
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

            TextSwitch {
                id: understood
                objectName: "understood"
                //% "Delete the stored data"
                text: qsTrId("threesea-la-start-fresh-understood")
            }
        }

        VerticalScrollDecorator { }
    }
}
