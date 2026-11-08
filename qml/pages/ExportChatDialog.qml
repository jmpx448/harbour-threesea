// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Export chat (Android has the same): the messages as a text file in a ZIP, with the pictures
// and files of the chat if they are wanted, protected by a password if one is given. The
// archive goes to Downloads; nothing is sent anywhere.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property string chatName
    property alias media: mediaSwitch.checked
    property alias password: passwordField.text

    // No password, or a real one: the core asks the same (at least 8 characters).
    canAccept: password.length === 0 || password.length >= 8

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            DialogHeader {
                //% "Export"
                acceptText: qsTrId("threesea-he-export")
                //% "Export chat"
                title: qsTrId("threesea-he-export-chat")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                font.pixelSize: Theme.fontSizeSmall
                //% "The messages of %1 are written as a text file into a ZIP archive in your Downloads folder."
                text: qsTrId("threesea-la-export-chat-description").arg(dialog.chatName)
            }

            TextSwitch {
                id: mediaSwitch
                checked: true
                //% "Include pictures and files"
                text: qsTrId("threesea-la-export-chat-media")
                //% "Everything that has been downloaded to this phone goes into the archive."
                description: qsTrId("threesea-la-export-chat-media-description")
            }

            PasswordField {
                id: passwordField
                width: parent.width
                //% "Password"
                label: qsTrId("threesea-la-password")
                errorHighlight: !dialog.canAccept
                //% "At least 8 characters. With a password the messages and files in the archive are encrypted with AES-256, and it opens in ZIP programs with AES support, such as 7-Zip; the names of the files stay readable. Empty: not encrypted, anyone who gets the file can read it."
                description: qsTrId("threesea-la-export-chat-password-note")
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.enabled: dialog.canAccept
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
