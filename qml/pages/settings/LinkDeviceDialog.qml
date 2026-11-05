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
                //% "Link Threema for Desktop"
                title: qsTrId("threesea-he-link-desktop")
                acceptText: dialog.code
                            //% "Link"
                            ? qsTrId("threesea-he-link")
                            //% "Next"
                            : qsTrId("threesea-he-link-next")
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
                color: Theme.highlightColor
                //% "Threema for Desktop gets your Threema ID with its private key, your contacts and groups, and can read and send messages as you. Link only your own computer."
                text: qsTrId("threesea-la-link-grant")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                //% "While a device is linked, chats with your contacts use end-to-end encryption without Perfect Forward Secrecy. If it was on, it comes back when the last linked device is removed."
                text: qsTrId("threesea-la-link-pfs-if-on")
            }
        }

        VerticalScrollDecorator { }
    }
}
