// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Android's storage management: what to delete (media files, or whole messages) and how old,
// in every chat. Accepted by swipe; the caller deletes after a remorse timer.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property bool messages: false
    property int days: 365
    readonly property var periods: [365, 180, 90, 30, 7, 0]
    readonly property var periodNames: [
        //% "Older than 1 year"
        qsTrId("threesea-me-older-1-year"),
        //% "Older than 6 months"
        qsTrId("threesea-me-older-6-months"),
        //% "Older than 3 months"
        qsTrId("threesea-me-older-3-months"),
        //% "Older than 1 month"
        qsTrId("threesea-me-older-1-month"),
        //% "Older than 1 week"
        qsTrId("threesea-me-older-1-week"),
        //% "All"
        qsTrId("threesea-me-older-all")
    ]

    //% "Groups with only you in them are cleaned up too."
    readonly property string scopeText: qsTrId("threesea-me-clean-up-scope")

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            DialogHeader {
                //% "Delete"
                acceptText: qsTrId("threesea-he-delete")
                //% "Clean up all chats"
                title: qsTrId("threesea-he-clean-up-all-chats")
            }

            SectionHeader {
                //% "What"
                text: qsTrId("threesea-se-clean-up-what")
            }
            TextSwitch {
                automaticCheck: false
                checked: !dialog.messages
                //% "Media files"
                text: qsTrId("threesea-me-clean-up-media")
                //% "Photos, videos, voice messages and files are deleted from this device, usually for good; the messages and their previews stay. Starred messages and messages still on their way keep their files."
                description: qsTrId("threesea-me-clean-up-media-description") + " " + dialog.scopeText
                onClicked: dialog.messages = false
            }
            TextSwitch {
                automaticCheck: false
                checked: dialog.messages
                //% "Messages and media"
                text: qsTrId("threesea-me-clean-up-messages")
                //% "Starred messages, open polls and messages still on their way stay."
                description: qsTrId("threesea-me-clean-up-messages-description") + " " + dialog.scopeText
                onClicked: dialog.messages = true
            }

            SectionHeader {
                //% "Age"
                text: qsTrId("threesea-se-clean-up-age")
            }
            Repeater {
                model: dialog.periods
                TextSwitch {
                    automaticCheck: false
                    checked: dialog.days === modelData
                    text: dialog.periodNames[index]
                    onClicked: dialog.days = modelData
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
