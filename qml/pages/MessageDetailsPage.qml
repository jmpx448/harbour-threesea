// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// A message's times and ids (Android's message details): created/sent/delivered/read for ours,
// sent/received/read for incoming ones, edit or deletion time, message id, file type and size.
// Times the app didn't record (older messages, a "read" receipt without "received") are left out.
// Then who reacted with what.
// Follows delivery receipts while open.
Page {
    id: page
    allowedOrientations: Orientation.All

    // Message JSON (core message JSON); refreshed from the core on opening.
    property var message: ({})
    readonly property bool outgoing: message.outgoing === true
    readonly property bool deleted: message.kind === "deleted"
    readonly property bool isFile: message.kind === "file" && !!message.body
    // The chat, which knows who is who there, also for whoever reacts while this is open.
    property var chat: null
    readonly property var reactions: Fmt.reactionSenders(message.reactions)

    function names(senders) {
        return senders.map(function(sender) {
            return sender === "" ? qsTrId("threesea-la-you") : page.chat ? page.chat.participantName(sender) : sender
        }).join(", ")
    }

    function when(ms) {
        var date = new Date(ms)
        return Format.formatDate(date, Formatter.DateMedium) + " " + Format.formatDate(date, Formatter.TimeValue)
    }

    function reload() {
        backend.call("message", { message: message.id }, function(ok, result) {
            if (ok && result) page.message = result
        })
    }

    Component.onCompleted: reload()

    Connections {
        target: backend
        onCoreEvent: {
            if (name !== "messagesChanged") return
            for (var i = 0; i < data.messages.length; ++i) {
                if (data.messages[i].id === page.message.id) page.reload()
            }
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Message details"
                title: qsTrId("threesea-he-message-details")
            }

            DetailItem {
                visible: page.outgoing && !!page.message.createdAt
                //% "Created"
                label: qsTrId("threesea-la-details-created")
                value: visible ? page.when(page.message.createdAt) : ""
            }
            DetailItem {
                // Incoming: the sender's time.
                readonly property var at: page.outgoing ? page.message.sentAt : page.message.createdAt
                visible: !!at
                //% "Sent"
                label: qsTrId("threesea-la-details-sent")
                value: visible ? page.when(at) : ""
            }
            DetailItem {
                visible: !page.outgoing && !!page.message.receivedAt
                //% "Received"
                label: qsTrId("threesea-la-details-received")
                value: visible ? page.when(page.message.receivedAt) : ""
            }
            DetailItem {
                visible: page.outgoing && !!page.message.deliveredAt
                //% "Delivered"
                label: qsTrId("threesea-la-details-delivered")
                value: visible ? page.when(page.message.deliveredAt) : ""
            }
            DetailItem {
                visible: !!page.message.readAt
                //% "Read"
                label: qsTrId("threesea-la-details-read")
                value: visible ? page.when(page.message.readAt) : ""
            }
            DetailItem {
                visible: !!page.message.editedAt
                label: page.deleted
                       //% "Deleted"
                       ? qsTrId("threesea-la-details-deleted")
                       //% "Edited"
                       : qsTrId("threesea-la-details-edited")
                value: visible ? page.when(page.message.editedAt) : ""
            }

            DetailItem {
                visible: page.isFile && !!page.message.body.mediaType
                //% "File type"
                label: qsTrId("threesea-la-details-file-type")
                value: visible ? page.message.body.mediaType : ""
            }
            DetailItem {
                visible: page.isFile && page.message.body.size > 0
                //% "File size"
                label: qsTrId("threesea-la-details-file-size")
                value: visible ? Format.formatFileSize(page.message.body.size) : ""
            }
            DetailItem {
                visible: !!page.message.messageId
                //% "Message ID"
                label: qsTrId("threesea-la-details-message-id")
                value: page.message.messageId || ""
            }

            SectionHeader {
                visible: page.reactions.length > 0
                //% "Reactions"
                text: qsTrId("threesea-he-reactions")
            }
            Repeater {
                model: page.reactions
                // Like the chips: Threema's picture (Settings → Appearance), else the system's glyph.
                Item {
                    width: column.width
                    height: Math.max(emoji.height, reactors.height) + Theme.paddingMedium
                    Label {
                        id: emoji
                        x: Theme.horizontalPageMargin
                        width: Theme.iconSizeMedium
                        textFormat: backend.threemaEmoji ? Text.StyledText : Text.PlainText
                        text: backend.threemaEmoji ? Fmt.markEmoji(modelData.emoji, Math.round(font.pixelSize * 1.2))
                                                   : modelData.emoji
                    }
                    Label {
                        id: reactors
                        anchors {
                            left: emoji.right
                            leftMargin: Theme.paddingMedium
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin
                        }
                        wrapMode: Text.Wrap
                        text: page.names(modelData.senders)
                        color: Theme.highlightColor
                    }
                }
            }
        }

        VerticalScrollDecorator {}
    }
}
