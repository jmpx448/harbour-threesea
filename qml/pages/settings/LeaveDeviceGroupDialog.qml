// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Removing the linked devices failed. Before this device leaves the group, the choice: remove them
// first (try again; this device leaves only once they are gone, so a lost computer can still be
// removed), or leave on this device only, which also works without a connection. `leave` says
// which was chosen.
Dialog {
    id: dialog
    // The window leaves a failed removal to this and the page beneath.
    objectName: "linkedDevices"
    allowedOrientations: Orientation.All

    property bool leave: false

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: dialog.leave
                            //% "Leave"
                            ? qsTrId("threesea-he-leave-device-group")
                            //% "Remove"
                            : qsTrId("threesea-he-remove-devices-again")
                //% "Your linked devices"
                title: qsTrId("threesea-he-leave-device-group-choice")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "Removing your linked devices did not finish, so they may still be linked."
                text: qsTrId("threesea-la-leave-device-group-not-finished")
            }

            Column {
                width: parent.width

                TextSwitch {
                    objectName: "removeFirst"
                    automaticCheck: false
                    checked: !dialog.leave
                    //% "Remove the linked devices first"
                    text: qsTrId("threesea-la-remove-devices-first")
                    //% "Tries again now, best once you are online. This device stays in the group until they are removed, so it can still remove a lost or stolen computer."
                    description: qsTrId("threesea-la-remove-devices-first-description")
                    onClicked: dialog.leave = false
                }

                TextSwitch {
                    objectName: "leaveOnly"
                    automaticCheck: false
                    checked: dialog.leave
                    //% "Leave on this device only"
                    text: qsTrId("threesea-la-leave-device-group-only")
                    //% "This device connects directly again. The other devices keep your ID and stay in their group, and only they can remove themselves from then on."
                    description: qsTrId("threesea-la-leave-device-group-text")
                    onClicked: dialog.leave = true
                }
            }
        }
    }
}
