// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Before a scanned Threema Web code connects anything: what it grants. Also the place to allow
// a self-hosted server (ADR 0013), once by the user, its certificate remembered.
Dialog {
    allowedOrientations: Orientation.All

    // Only for a server that has to be allowed.
    property string host

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Allow"
                acceptText: qsTrId("threesea-he-web-allow")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "This code connects a browser to your Threema ID. That browser can read your contacts and chats and send messages in your name."
                text: qsTrId("threesea-la-web-pair-grants")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "Allow it only if you opened Threema Web yourself and the code is on your own screen. If someone sent you this code, go back."
                text: qsTrId("threesea-la-web-pair-only-yours")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                // Always: the code's "remember me" bit is the sender's word, and the browser asks
                // later, whatever the code said.
                //% "If the browser asks to be remembered, it can connect again without a new code until you remove it from Threema Web in the settings."
                text: qsTrId("threesea-la-web-pair-remembered")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                visible: host.length > 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                color: Theme.highlightColor
                text: host
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                visible: host.length > 0
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "This code is for a Threema Web server run by someone other than Threema. Your chats go through it end-to-end encrypted. Allow it only if you run it yourself or trust who does. Its certificate is remembered and you'll be asked if it changes."
                text: qsTrId("threesea-la-web-self-hosted")
            }
        }
        VerticalScrollDecorator { }
    }
}
