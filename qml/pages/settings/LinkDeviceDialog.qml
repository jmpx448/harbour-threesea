// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Linking Threema for Desktop, step 1: what it means (Android's PFS warning), then the scanner.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    // Already scanned (the contacts page's scanner takes any code): straight to linking.
    property string code

    acceptDestination: code ? Qt.resolvedUrl("LinkDevicePage.qml") : Qt.resolvedUrl("../ScanQrPage.qml")
    acceptDestinationAction: PageStackAction.Replace
    // The scanner continues to LinkDevicePage with the scanned code.
    acceptDestinationProperties: code ? { "code": code } : { "deviceLink": true }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Link"
                acceptText: qsTrId("threesea-he-link")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                visible: !dialog.code
                //% "Open Threema for Desktop on your computer. It shows a QR code; scan it on the next page."
                text: qsTrId("threesea-la-link-intro")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                //% "While a device is linked, chats with your contacts use end-to-end encryption without Perfect Forward Secrecy. It comes back when you remove the last linked device."
                text: qsTrId("threesea-la-link-pfs")
            }
        }
    }
}
