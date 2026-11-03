// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Format.js" as Fmt

// Search all chats (Chats pulley → Search), like People's search: a search field under the
// header, results newest first with the chat's name, time and the matching text; tapping one
// opens the chat at that message.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var results: []
    property string query
    // Search one chat (from its contact/group page): results go back to that chat page.
    property string identity
    property string chatName
    // Starred messages instead of a search (no field; all of them, newest first).
    property bool starred: false

    Component.onCompleted: if (starred) loadStarred()
    function loadStarred() {
        var args = {}
        if (identity.length > 0) args.identity = identity
        backend.call("starredMessages", args, function(ok, result) {
            if (ok) page.results = result.results
        })
    }

    function search() {
        var text = searchFieldItem.text.trim()
        if (text.length < 2) {
            results = []
            query = ""
            return
        }
        var args = { query: text }
        if (identity.length > 0) args.identity = identity
        backend.call("searchMessages", args, function(ok, result) {
            // Only the latest query's answer counts.
            if (ok && text === page.searchFieldItem.text.trim()) {
                page.query = text
                // A filled model makes its first row the list's current item, which takes the
                // focus from the field in the list's header and closes the keyboard on every
                // keystroke (user report 2026-10-28). Give it back once the field's own
                // focus-loss handling (a 1 ms timer in Silica) has run.
                var typing = page.searchFieldItem.activeFocus
                page.results = result.results
                listView.currentIndex = -1
                if (typing) refocusField.restart()
            }
        })
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: page.results
        currentIndex: -1

        header: Column {
            width: parent.width
            PageHeader {
                title: page.starred
                       //% "Starred messages"
                       ? qsTrId("threesea-he-starred")
                       //% "Search"
                       : qsTrId("threesea-he-search")
                description: page.chatName
            }
            SearchField {
                visible: !page.starred
                height: visible ? implicitHeight : 0
                id: searchField
                width: parent.width
                //% "Search messages"
                placeholderText: qsTrId("threesea-la-search-messages")
                inputMethodHints: Qt.ImhNoAutoUppercase | window.incognitoHints
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
                onTextChanged: searchDelay.restart()
                Component.onCompleted: page.searchFieldItem = searchField
            }
            // Before searching (all chats): the starred messages, one tap away.
            BackgroundItem {
                visible: !page.starred && page.identity.length === 0 && page.query.length === 0
                width: parent.width
                onClicked: pageStack.push(Qt.resolvedUrl("SearchPage.qml"), { starred: true })
                Image {
                    id: starIcon
                    x: Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    source: "image://theme/icon-m-favorite?" + (parent.highlighted ? Theme.highlightColor : Theme.primaryColor)
                }
                Label {
                    anchors {
                        left: starIcon.right
                        leftMargin: Theme.paddingMedium
                        verticalCenter: parent.verticalCenter
                    }
                    //% "Starred messages"
                    text: qsTrId("threesea-he-starred")
                    color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }
        }

        delegate: ListItem {
            id: item
            contentHeight: column.height + 2 * Theme.paddingMedium
            onClicked: {
                var chat = pageStack.find(function(p) { return p.objectName === "chat" && p.identity === modelData.conversation })
                if (chat) {
                    chat.showMessage(modelData.id)
                    pageStack.pop(chat)
                } else {
                    pageStack.push(Qt.resolvedUrl("ChatPage.qml"),
                                   { identity: modelData.conversation, focusMessage: modelData.id })
                }
            }

            Column {
                id: column
                x: Theme.horizontalPageMargin
                y: Theme.paddingMedium
                width: parent.width - 2 * Theme.horizontalPageMargin

                Item {
                    width: parent.width
                    height: nameLabel.height
                    Label {
                        id: nameLabel
                        width: parent.width - timeLabel.width - Theme.paddingMedium
                        // One chat: who wrote it; all chats: which chat.
                        text: page.identity.length > 0
                              //% "You"
                              ? (modelData.outgoing ? qsTrId("threesea-la-you")
                                                    : modelData.senderName || modelData.displayName)
                              : modelData.outgoing
                              //% "You → %1"
                              ? qsTrId("threesea-la-search-you-to").arg(modelData.displayName)
                              : modelData.displayName
                        truncationMode: TruncationMode.Fade
                        color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
                    }
                    Label {
                        id: timeLabel
                        anchors { right: parent.right; baseline: nameLabel.baseline }
                        text: Fmt.time(modelData.createdAt, Format, Formatter)
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    }
                }
                Label {
                    width: parent.width
                    // The matching text (caption, file or place name…), mentions as names.
                    text: Fmt.highlightMatch((modelData.snippet ? Fmt.withMentions(modelData.snippet, modelData.mentions)
                                                                 : Fmt.preview(modelData))
                                             .replace(/\s+/g, " "), page.query, Theme.highlightColor)
                    textFormat: Text.StyledText
                    font.pixelSize: Theme.fontSizeSmall
                    color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    wrapMode: Text.Wrap
                    maximumLineCount: 3
                    elide: Text.ElideRight
                }
            }
        }

        ViewPlaceholder {
            enabled: page.starred && page.results.length === 0
            //% "No starred messages"
            text: qsTrId("threesea-la-no-starred")
            //% "Long press a message and choose Star to keep it here."
            hintText: qsTrId("threesea-la-no-starred-hint")
        }
        ViewPlaceholder {
            enabled: !page.starred && page.query.length > 0 && page.results.length === 0
            //% "No messages found"
            text: qsTrId("threesea-la-search-nothing")
        }

        VerticalScrollDecorator { }
    }

    // The search field lives in the list's header; it gets the keyboard when the page opens.
    property Item searchFieldItem
    onStatusChanged: {
        if (status === PageStatus.Active && page.starred) loadStarred()
        if (!page.starred && status === PageStatus.Active && searchFieldItem && searchFieldItem.text.length === 0) {
            searchFieldItem.forceActiveFocus()
        }
    }

    Timer {
        id: refocusField
        interval: 20
        onTriggered: {
            page.searchFieldItem.forceActiveFocus()
            Qt.inputMethod.show()
        }
    }

    Timer {
        id: searchDelay
        interval: 300
        onTriggered: page.search()
    }
}
