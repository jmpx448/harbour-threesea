// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A self-hosted Threema Web server (ADR 0013) shows another certificate than the one remembered
// when the user allowed it (WebPairDialog).
Dialog {
    allowedOrientations: Orientation.All
    property string host

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Trust"
                acceptText: qsTrId("threesea-he-web-trust")
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
                //% "This Threema Web server shows a different certificate than last time. Trust it only if you know its certificate was renewed; otherwise go back."
                text: qsTrId("threesea-la-web-certificate-changed")
            }
        }
        VerticalScrollDecorator { }
    }
}
