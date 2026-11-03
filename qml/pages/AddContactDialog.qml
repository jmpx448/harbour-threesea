// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property alias identity: identityField.text
    // Reused to block an ID: other header and intro.
    property string acceptText
    property string intro

    canAccept: identityField.acceptableInput

    Column {
        width: parent.width
        spacing: Theme.paddingLarge

        DialogHeader {
            //% "Add"
            acceptText: dialog.acceptText || qsTrId("threesea-he-add")
        }

        InfoLabel {
            //% "Enter the Threema ID of the person you want to add. Meet in person and scan each other's QR code later to fully verify the contact."
            text: dialog.intro || qsTrId("threesea-la-add-contact-intro")
        }

        TextField {
            id: identityField
            width: parent.width
            focus: true
            //% "Threema ID"
            label: qsTrId("threesea-la-threema-id")
            placeholderText: "ABCD1234"
            inputMethodHints: Qt.ImhUppercaseOnly | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
            validator: RegExpValidator { regExp: /^\s*[A-Za-z0-9*][A-Za-z0-9]{7}\s*$/ }
            onTextChanged: {
                var upper = text.toUpperCase()
                if (upper !== text) {
                    var position = cursorPosition
                    text = upper
                    cursorPosition = position
                }
            }
            EnterKey.enabled: acceptableInput
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
    }
}
