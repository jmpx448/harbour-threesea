// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A look at what is about to go, forwarded or shared, and to whom, like Android's question.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All
    canAccept: !askContent || contentIsCaption || content.trim().length > 0

    // The chats, by name.
    property string names
    property bool forwarding
    // A caption or message, to change or take away.
    property bool askContent
    property string content
    property bool contentIsCaption
    // Several messages with captions: all of them go along or none.
    property bool askCaptions
    property bool includeCaptions: true

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Send"
                acceptText: qsTrId("threesea-he-confirm-send")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                color: Theme.highlightColor
                text: (dialog.forwarding
                       //% "Forward to %1"
                       ? qsTrId("threesea-la-confirm-forward-to")
                       //% "Send to %1"
                       : qsTrId("threesea-la-confirm-send-to")).arg(dialog.names)
            }

            TextArea {
                visible: dialog.askContent
                width: parent.width
                inputMethodHints: window.incognitoHints
                text: dialog.content
                onTextChanged: dialog.content = text
                label: dialog.contentIsCaption
                       //% "Caption"
                       ? qsTrId("threesea-la-confirm-caption")
                       //% "Message"
                       : qsTrId("threesea-la-confirm-message")
                placeholderText: label
            }

            TextSwitch {
                visible: dialog.askCaptions
                checked: dialog.includeCaptions
                //% "Include captions"
                text: qsTrId("threesea-la-confirm-include-captions")
                onCheckedChanged: dialog.includeCaptions = checked
            }
        }

        VerticalScrollDecorator { }
    }
}
