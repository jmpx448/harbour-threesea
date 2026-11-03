// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A self-hosted Threema Web server (ADR 0013): allowed once by the user, its certificate
// remembered; asked again if that certificate changes.
Dialog {
    allowedOrientations: Orientation.All
    property string host
    property bool certificateChanged

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: certificateChanged
                            //% "Trust"
                            ? qsTrId("threesea-he-web-trust")
                            //% "Allow"
                            : qsTrId("threesea-he-web-allow")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                color: Theme.highlightColor
                text: host
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: certificateChanged
                      //% "This Threema Web server shows a different certificate than last time. Trust it only if you know its certificate was renewed; otherwise go back."
                      ? qsTrId("threesea-la-web-certificate-changed")
                      //% "This code is for a Threema Web server run by someone other than Threema. Your chats go through it end-to-end encrypted. Allow it only if you run it yourself or trust who does. Its certificate is remembered and you'll be asked if it changes."
                      : qsTrId("threesea-la-web-self-hosted")
            }
        }
        VerticalScrollDecorator { }
    }
}
