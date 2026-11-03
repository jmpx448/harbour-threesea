// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// The ID revocation password (Android "ID Revocation"): with it the ID can be revoked at
// myid.threema.ch/revoke if the phone is lost. Only a 4-byte hash of it goes to Threema.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property alias password: passwordField.text
    readonly property int length: passwordField.text.length

    canAccept: length >= 8 && length <= 256 && repeatField.text === passwordField.text

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Save"
                acceptText: qsTrId("threesea-he-save")
            }

            InfoLabel {
                //% "With this password you can revoke your Threema ID at myid.threema.ch/revoke in case you lose your phone or it gets stolen. Use at least 8 characters."
                text: qsTrId("threesea-la-revocation-intro")
            }

            PasswordField {
                id: passwordField
                width: parent.width
                focus: true
                label: qsTrId("threesea-la-password")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: repeatField.focus = true
            }

            PasswordField {
                id: repeatField
                width: parent.width
                label: qsTrId("threesea-la-repeat-password")
                errorHighlight: text.length > 0 && text !== passwordField.text
                EnterKey.enabled: dialog.canAccept
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
