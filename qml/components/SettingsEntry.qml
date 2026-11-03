// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A row of the main settings page, like Sailfish Settings' lists: the category's icon, its name
// and (optionally) its current state below; opens `page` (a file in pages/settings/).
BackgroundItem {
    id: root

    property string icon
    property string text
    property string description
    property string page

    width: parent ? parent.width : 0
    height: Theme.itemSizeMedium
    onClicked: pageStack.push(Qt.resolvedUrl("../pages/settings/" + page))

    Icon {
        id: image
        x: Theme.horizontalPageMargin
        anchors.verticalCenter: parent.verticalCenter
        // Theme icons by name; anything else is one of ours in qml/icons/ (128 px, so it
        // needs the explicit icon-m size).
        width: Theme.iconSizeMedium
        height: Theme.iconSizeMedium
        sourceSize.width: width
        sourceSize.height: height
        source: root.icon.indexOf("icon-") === 0 ? "image://theme/" + root.icon
                                                 : Qt.resolvedUrl("../icons/" + root.icon)
        highlighted: root.highlighted
    }
    Column {
        anchors {
            left: image.right
            leftMargin: Theme.paddingLarge
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
            verticalCenter: parent.verticalCenter
        }
        Label {
            width: parent.width
            text: root.text
            truncationMode: TruncationMode.Fade
            color: root.highlighted ? Theme.highlightColor : Theme.primaryColor
        }
        Label {
            visible: text.length > 0
            width: parent.width
            text: root.description
            truncationMode: TruncationMode.Fade
            font.pixelSize: Theme.fontSizeExtraSmall
            color: root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
        }
    }
}
