// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Link an e-mail address or phone number to the Threema ID (Android's identity links), so people
// who have it in their address book find you. `kind`: "email" or "mobile". The address or number
// itself goes to Threema, which sends a confirmation link or an SMS code. `unlinkRequested()` from
// the pulley when something is linked.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property string kind: "email"
    property string current
    property alias value: field.text
    readonly property bool email: kind === "email"

    signal unlinkRequested()

    canAccept: email ? /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(field.text.trim())
                     : field.text.replace(/[^0-9]/g, "").length >= 6

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            visible: dialog.current.length > 0
            MenuItem {
                //% "Remove link"
                text: qsTrId("threesea-me-unlink")
                onClicked: {
                    dialog.unlinkRequested()
                    dialog.reject()
                }
            }
        }

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Link"
                acceptText: qsTrId("threesea-he-link")
            }

            InfoLabel {
                text: dialog.email
                      //% "People who have this e-mail address in their address book can find you on Threema. It is sent to Threema, which e-mails you a link to confirm it."
                      ? qsTrId("threesea-la-link-email-info")
                      //% "People who have this number in their address book can find you on Threema. It is sent to Threema, which texts you a code to confirm it."
                      : qsTrId("threesea-la-link-mobile-info")
            }

            TextField {
                id: field
                width: parent.width
                focus: true
                text: dialog.current
                inputMethodHints: dialog.email ? Qt.ImhEmailCharactersOnly | Qt.ImhNoAutoUppercase
                                               : Qt.ImhDialableCharactersOnly
                label: dialog.email
                       //% "E-mail address"
                       ? qsTrId("threesea-la-email")
                       //% "Phone number"
                       : qsTrId("threesea-la-phone")
                placeholderText: label
                EnterKey.enabled: dialog.canAccept
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
