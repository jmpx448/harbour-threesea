// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// The code Threema texted to confirm a phone number (Android's SMS verification). The pulley asks
// for a call that reads the code instead, or cancels the link.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property string number
    property alias code: field.text

    signal cancelRequested()

    canAccept: field.text.trim().length >= 4

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            MenuItem {
                //% "Cancel linking"
                text: qsTrId("threesea-me-cancel-link")
                onClicked: {
                    dialog.cancelRequested()
                    dialog.reject()
                }
            }
            MenuItem {
                //% "Call me instead"
                text: qsTrId("threesea-me-call-me")
                onClicked: backend.call("callMobile", {}, function(ok, result, kind, message) {
                    Notices.show(ok
                                 //% "You will get a call with the code."
                                 ? qsTrId("threesea-la-call-coming")
                                 : Errors.text(kind, message), Notice.Long, Notice.Center)
                })
            }
        }

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Confirm"
                acceptText: qsTrId("threesea-he-confirm")
            }

            InfoLabel {
                //% "Enter the code Threema sent to +%1."
                text: qsTrId("threesea-la-enter-sms-code").arg(dialog.number)
            }

            TextField {
                id: field
                width: parent.width
                focus: true
                inputMethodHints: Qt.ImhDigitsOnly
                //% "Code"
                label: qsTrId("threesea-la-sms-code")
                placeholderText: label
                EnterKey.enabled: dialog.canAccept
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
