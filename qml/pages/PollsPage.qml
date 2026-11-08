// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

// A chat's polls (its pulley → Polls, or the reminder under its header; Android's "All polls"):
// the open ones first, then the closed, newest first in each. Tapping one opens it, the menu finds
// it in the chat.
Page {
    id: page
    allowedOrientations: Orientation.All

    // The chat page it was opened from, which opens the poll and shows it.
    property Item chat
    property string identity
    property string chatName

    property var polls: []
    property string loadError
    property bool loaded: false

    function load() {
        backend.call("polls", { identity: identity }, function(ok, result, kind, message) {
            page.loaded = true
            if (ok) {
                page.polls = result.polls
                page.loadError = ""
            } else {
                page.loadError = Errors.text(kind, message)
            }
        })
    }
    Component.onCompleted: load()
    // A vote or a closing changes the row, and its section; also someone else's while this is open.
    Connections {
        target: page.chat
        onPollsRefreshed: page.polls = polls
    }

    SilicaListView {
        id: list
        anchors.fill: parent
        model: page.polls

        header: PageHeader {
            //% "Polls"
            title: qsTrId("threesea-he-polls")
            description: page.chatName
        }

        ViewPlaceholder {
            enabled: page.loaded && list.count === 0
            //% "No polls in this chat"
            text: page.loadError || qsTrId("threesea-la-no-polls")
        }

        delegate: ListItem {
            id: item
            readonly property var poll: modelData.body || ({})
            readonly property bool closed: poll.state === 1
            readonly property var myVote: (poll.votes && poll.votes[backend.identity]) || []
            readonly property bool firstOfSection: index === 0
                                                   || (page.polls[index - 1].body.state === 1) !== closed
            contentHeight: column.height + 2 * Theme.paddingMedium
                           + (firstOfSection ? sectionHeader.height : 0)
            onClicked: if (page.chat) page.chat.openPoll(modelData.id, poll)
            menu: Component {
                ContextMenu {
                    MenuItem {
                        //% "Show in chat"
                        text: qsTrId("threesea-me-show-in-chat")
                        onClicked: {
                            page.chat.showMessage(modelData.id)
                            pageStack.pop(page.chat)
                        }
                    }
                }
            }

            SectionHeader {
                id: sectionHeader
                visible: item.firstOfSection
                text: item.closed
                      //% "Closed"
                      ? qsTrId("threesea-se-polls-closed")
                      //% "Open"
                      : qsTrId("threesea-se-polls-open")
            }
            Column {
                id: column
                x: Theme.horizontalPageMargin
                y: Theme.paddingMedium + (item.firstOfSection ? sectionHeader.height : 0)
                width: parent.width - 2 * x
                Label {
                    width: parent.width
                    wrapMode: Text.Wrap
                    maximumLineCount: 3
                    elide: Text.ElideRight
                    color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
                    text: item.poll.description || ""
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    text: {
                        var time = Fmt.time(modelData.createdAt, Format, Formatter)
                        if (item.closed) return time
                        return time + " · " + (item.myVote.length > 0
                                               //% "You voted"
                                               ? qsTrId("threesea-la-poll-you-voted")
                                               //% "Waiting for your vote"
                                               : qsTrId("threesea-la-poll-waiting"))
                    }
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
