// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A video or picture whose metadata can't be taken out: sent as it is, or not at all (ADR 0025).
Dialog {
    allowedOrientations: Orientation.All

    property string name
    property bool video: true

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Send as it is"
                acceptText: qsTrId("threesea-he-video-keeps-metadata-send")
                //% "Don't send"
                cancelText: qsTrId("threesea-he-video-keeps-metadata-cancel")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                color: Theme.highlightColor
                text: video
                      //% "This video keeps what its file says about it"
                      ? qsTrId("threesea-la-video-keeps-metadata")
                      //% "This picture keeps what its file says about it"
                      : qsTrId("threesea-la-picture-keeps-metadata")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: (video
                       //% "Threesea can't remove the metadata of this file. It can hold where and when the video was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file."
                       ? qsTrId("threesea-la-video-keeps-metadata-description")
                       //% "Threesea can't remove the metadata of this file. It can hold where and when the picture was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file."
                       : qsTrId("threesea-la-picture-keeps-metadata-description")).arg(name)
            }
        }
    }
}
