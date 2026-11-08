// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Format.js" as Fmt

// A location inside a message bubble: the theme's location icon, the place's name and address
// (if the sender gave them) and the coordinates; tapping the bubble opens the location on
// Threema's map (LocationPage, ADR 0032). No map in the bubble: nothing is fetched before the tap.
Row {
    id: root

    property var location: ({})
    property real maxWidth
    property bool highlighted
    property color textColor: Theme.primaryColor

    spacing: Theme.paddingMedium
    width: Math.min(maxWidth, Screen.width * 2 / 3)

    Image {
        id: icon
        anchors.verticalCenter: parent.verticalCenter
        source: "image://theme/icon-m-location?" + (root.highlighted ? Theme.highlightColor : root.textColor)
    }

    Column {
        width: root.width - icon.width - root.spacing
        anchors.verticalCenter: parent.verticalCenter

        Label {
            width: parent.width
            //% "Location"
            text: root.location.name || qsTrId("threesea-la-location")
            color: root.textColor
            wrapMode: Text.Wrap
        }
        Label {
            width: parent.width
            visible: !!root.location.address
            text: root.location.address || ""
            font.pixelSize: Theme.fontSizeSmall
            color: root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            wrapMode: Text.Wrap
        }
        Label {
            width: parent.width
            text: Fmt.coordinates(root.location)
            font.pixelSize: Theme.fontSizeExtraSmall
            color: root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            truncationMode: TruncationMode.Fade
        }
    }
}
