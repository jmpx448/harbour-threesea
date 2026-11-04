// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Removing the linked devices failed. What's left to choose: try again later (back), or take only
// this device out of the group, which also works without a connection.
Dialog {
    // The window leaves a failed removal to this and the page beneath.
    objectName: "linkedDevices"
    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Leave"
                acceptText: qsTrId("threesea-he-leave-device-group")
                //% "Leave on this device only?"
                title: qsTrId("threesea-he-leave-device-group-title")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "Removing your linked devices did not finish, so they may still be linked. Go back to try again once you are online."
                text: qsTrId("threesea-la-leave-device-group-failed")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                //% "If you leave, this device connects directly again. The other devices keep your ID and stay in their group, and only they can remove themselves from then on."
                text: qsTrId("threesea-la-leave-device-group-text")
            }
        }
    }
}
