// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// "Add …" at the top of an editable list, like Jolla Settings → Lock screen → Select shortcuts:
// the text and a round plus at the right edge; the whole row reacts.
BackgroundItem {
    id: root
    property alias text: label.text
    width: parent ? parent.width : 0
    height: Theme.itemSizeSmall

    Label {
        id: label
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            right: button.left
            verticalCenter: parent.verticalCenter
        }
        truncationMode: TruncationMode.Fade
        color: root.highlighted ? Theme.highlightColor : Theme.primaryColor
    }
    RoundIconButton {
        id: button
        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
            verticalCenter: parent.verticalCenter
        }
        icon.source: "image://theme/icon-m-add"
        onClicked: root.clicked(null)
    }
}
