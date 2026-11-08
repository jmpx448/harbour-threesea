// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Link an e-mail address or phone number to the Threema ID (Android's identity links), so people
// who have it in their address book find you. `kind`: "email" or "mobile". The address or number
// itself goes to Threema, which sends a confirmation link or an SMS code. A number must have its
// country code, and the dialog shows what will be sent: `normalized` (digits, no "+").
// `unlinkRequested()` from the pulley when something is linked.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property string kind: "email"
    property string current
    property alias value: field.text
    readonly property bool email: kind === "email"
    // The core's reading of the number, and of which text: it answers later, the field may have
    // moved on by then. The hint keeps the last answer so it doesn't flicker while typing.
    property string normalized
    property string normalizedFrom

    signal unlinkRequested()

    canAccept: email ? /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(field.text.trim())
                     : normalized.length > 0 && normalizedFrom === field.text

    function normalize() {
        if (email) return
        var text = field.text
        backend.call("normalizeMobile", { number: text }, function(ok, result) {
            if (!ok || field.text !== text) return
            dialog.normalized = result.number || ""
            dialog.normalizedFrom = text
        })
    }

    Component.onCompleted: normalize()

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
                onTextChanged: dialog.normalize()
            }

            Label {
                visible: !dialog.email
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: dialog.normalized.length > 0
                      //% "The code goes to +%1."
                      ? qsTrId("threesea-la-link-mobile-target").arg(dialog.normalized)
                      : qsTrId("threesea-la-error-phone-invalid")
            }
        }

        VerticalScrollDecorator { }
    }
}
