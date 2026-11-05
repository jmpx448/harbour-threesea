// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Our contact QR code (Threema's 3mid: format): someone scanning it verifies us in person.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var code: ({ size: 0, bits: "" })

    Component.onCompleted: backend.call("myQrCode", {}, function(ok, result) {
        if (ok) page.code = result
    })

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                id: header
                //% "My QR code"
                title: qsTrId("threesea-he-my-qr-code")
                description: backend.identity
            }

            QrCodeView {
                anchors.horizontalCenter: parent.horizontalCenter
                // All of it on screen at once in landscape too, under the header.
                width: Math.min(page.width - 2 * Theme.horizontalPageMargin,
                                page.height - header.height - 2 * column.spacing)
                code: page.code
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                //% "When someone scans this code with Threema, you are verified for them in person (three dots)."
                text: qsTrId("threesea-la-my-qr-code-description")
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
            }
        }

        VerticalScrollDecorator { }
    }
}
