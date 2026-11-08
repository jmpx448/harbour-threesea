// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A scanned link to a Threema ID: asked before it becomes a contact. It carries no key, so this
// adds the contact and verifies nothing.
Dialog {
    allowedOrientations: Orientation.All

    property string identity

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: qsTrId("threesea-me-add-contact")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                color: Theme.highlightColor
                text: identity
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "Adds this Threema ID to your contacts with the key from Threema's directory. To verify it, scan the QR code in the other person's app."
                text: qsTrId("threesea-la-add-scanned-contact")
            }
        }
        VerticalScrollDecorator { }
    }
}
