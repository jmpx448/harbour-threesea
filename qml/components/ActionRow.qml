// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A context menu row of actions side by side, each a theme icon over a small label (the
// user's layout for a bubble's menu). `actions`: [{id, icon, text}].
Row {
    id: row

    property var actions: []
    property color color: Theme.primaryColor
    signal triggered(string id)

    visible: actions.length > 0
    width: parent ? parent.width : 0

    Repeater {
        model: row.actions
        BackgroundItem {
            id: action
            width: row.width / row.actions.length
            height: Theme.itemSizeLarge
            onClicked: row.triggered(modelData.id)
            Column {
                anchors.centerIn: parent
                width: parent.width
                Image {
                    anchors.horizontalCenter: parent.horizontalCenter
                    source: "image://theme/" + modelData.icon + "?"
                            + (action.highlighted ? Theme.highlightColor : row.color)
                }
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    text: modelData.text
                    font.pixelSize: Theme.fontSizeExtraSmall
                    truncationMode: TruncationMode.Fade
                    color: action.highlighted ? Theme.highlightColor : row.color
                }
            }
        }
    }
}
