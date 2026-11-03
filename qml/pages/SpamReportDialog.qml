// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// What a spam report sends, and whether to block and delete too. Accepted by swipe; the chat page
// reports afterwards.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property string name
    property bool isGroup: false
    property alias block: blockSwitch.checked

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Report"
                acceptText: qsTrId("threesea-he-spam-report")
                //% "Report %1"
                title: qsTrId("threesea-he-spam-report-name").arg(dialog.name)
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                font.pixelSize: Theme.fontSizeSmall
                text: dialog.isGroup
                      //% "This sends your Threema ID to Threema, with the group's ID and number of members and the ID and nickname of its creator. No messages and not the group's name."
                      ? qsTrId("threesea-la-spam-explain-group")
                      //% "This sends your Threema ID to Threema, with the ID and nickname of this contact. No messages."
                      : qsTrId("threesea-la-spam-explain-contact")
            }

            TextSwitch {
                id: blockSwitch
                text: dialog.isGroup
                      //% "Also block the creator, leave the group and delete the chat"
                      ? qsTrId("threesea-la-spam-block-group")
                      //% "Also block this contact and delete the chat"
                      : qsTrId("threesea-la-spam-block-contact")
            }
        }

        VerticalScrollDecorator { }
    }
}
