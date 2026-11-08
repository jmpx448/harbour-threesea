// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// A poll as a Silica dialog: set the choices (switches; single choice replaces, multiple toggles),
// then swipe left / "Vote" to send them, swipe right / "Cancel" to leave them unchanged. Counts
// and who voted for what where that is known; in groups who has not voted yet. Pulley: close the
// poll (creator), clear the choices. Results update live.
Dialog {
    id: page
    allowedOrientations: Orientation.All

    property int messageId
    // The conversation (contact id or group key), for member names.
    property string conversation
    property var poll: ({})
    // A chat where nothing goes out: the poll can be looked at, not voted in or closed.
    property bool readOnly: false
    // identity -> display name; members (groups) to show who hasn't voted.
    property var names: ({})
    property var members: []
    // The chat's names of those who voted, for someone who has left the group since.
    property var formerNames: ({})

    // The poll message changed by our vote or closing (the chat updates its bubble).
    signal changed(var message)
    signal voteRequested(var choices)

    readonly property string me: backend.identity
    readonly property bool closed: poll.state === 1
    readonly property bool mine: poll.creator === me
    readonly property var myVote: (poll.votes && poll.votes[me]) || []
    // The choices being set here (sent on accept); starts as our current vote.
    property var selection: []
    property bool edited: false
    onMyVoteChanged: if (!edited) selection = myVote.map(Number)

    function sameAsVote() {
        if (selection.length !== myVote.length) return false
        for (var i = 0; i < selection.length; ++i) if (myVote.map(Number).indexOf(selection[i]) < 0) return false
        return true
    }
    canAccept: !closed && !readOnly && !sameAsVote()
    // The chat sends it: this dialog is gone before the reply arrives.
    onAccepted: voteRequested(selection.slice())
    readonly property bool countsKnown: closed || poll.announce === true || mine
    readonly property var voters: {
        var list = []
        for (var voter in (poll.votes || {})) if (poll.votes[voter].length > 0) list.push(voter)
        return list
    }

    function nameOf(identity) {
        if (identity === me) {
            //% "You"
            return qsTrId("threesea-la-you")
        }
        return names[identity] || formerNames[identity] || identity
    }

    function votersOf(choice) {
        var list = []
        for (var voter in (poll.votes || {})) {
            if (poll.votes[voter].indexOf(choice.id) >= 0) list.push(voter)
        }
        return list
    }

    function count(choice) {
        if (choice.total !== undefined && choice.total !== null) return choice.total
        return votersOf(choice).length
    }


    function toggle(id) {
        var chosen = selection.slice()
        var index = chosen.indexOf(id)
        if (index >= 0) chosen.splice(index, 1)
        else if (poll.multiple) chosen.push(id)
        else chosen = [id]
        edited = true
        selection = chosen
    }

    function loadNames() {
        if (conversation.indexOf("g:") === 0) {
            backend.call("group", { key: conversation }, function(ok, result) {
                if (!ok) return
                var map = {}
                var list = []
                for (var i = 0; i < result.members.length; ++i) {
                    map[result.members[i].identity] = result.members[i].displayName
                    list.push(result.members[i].identity)
                }
                list.push(page.me)
                page.names = map
                page.members = list
            })
        } else {
            backend.call("contact", { identity: conversation }, function(ok, result) {
                if (!ok) return
                var map = {}
                map[result.identity] = result.displayName
                page.names = map
                page.members = [result.identity, page.me]
            })
        }
    }

    Component.onCompleted: loadNames()

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "messagesChanged") {
                for (var i = 0; i < data.messages.length; ++i) {
                    if (data.messages[i].id === page.messageId) page.poll = data.messages[i].body
                }
            }
        }
    }

    RemorsePopup { id: remorse }

    SilicaListView {
        id: list
        anchors.fill: parent
        model: page.poll.choices || []

        // The DialogHeader's height arrives after the header's first layout.
        StayAtTop { target: list }

        // Silica opens a menu with nothing in it too.
        PullDownMenu {
            visible: !page.closed && !page.readOnly && (page.mine || page.selection.length > 0)
            MenuItem {
                visible: page.mine
                //% "Close poll"
                text: qsTrId("threesea-me-close-poll")
                //% "Closing poll"
                onClicked: remorse.execute(qsTrId("threesea-la-closing-poll"), function() {
                    backend.call("closePoll", { message: page.messageId }, function(ok, result, kind, message) {
                        // The results are a message of their own now; the voting is over.
                        // No result: already closed elsewhere, and the voting is just as over.
                        if (ok) {
                            if (result) page.changed(result)
                            pageStack.pop()
                        }
                        else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                    })
                })
            }
            MenuItem {
                visible: page.selection.length > 0
                //% "Clear choices"
                text: qsTrId("threesea-me-clear-choices")
                onClicked: {
                    page.edited = true
                    page.selection = []
                }
            }
        }

        header: Column {
            width: parent.width
            DialogHeader {
                //% "Vote"
                acceptText: qsTrId("threesea-he-vote")
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                text: page.poll.description || ""
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                font.family: Theme.fontFamilyHeading
                color: Theme.highlightColor
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                bottomPadding: Theme.paddingLarge
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: [
                page.poll.multiple
                //% "Multiple choice"
                ? qsTrId("threesea-la-poll-multiple")
                //% "Single choice"
                : qsTrId("threesea-la-poll-single"),
                //% "Closed"
                page.closed ? qsTrId("threesea-la-poll-closed") : "",
                //% "%n vote(s)"
                page.countsKnown ? qsTrId("threesea-la-poll-votes", page.voters.length) : ""
            ].filter(function(s) { return s.length > 0 }).join(" · ")
            }
        }

        // Silica's switches (single choice: switching one on switches the others off); the count
        // and who voted in the description.
        delegate: TextSwitch {
            readonly property var choiceVoters: page.countsKnown ? page.votersOf(modelData) : []
            text: modelData.name
            automaticCheck: false
            checked: page.selection.indexOf(Number(modelData.id)) >= 0
            enabled: !page.closed && !page.readOnly
            description: page.countsKnown
                         ? [qsTrId("threesea-la-poll-votes", page.count(modelData))]
                               .concat(choiceVoters.length > 0 ? [choiceVoters.map(page.nameOf).join(", ")] : [])
                               .join(" · ")
                         : ""
            onClicked: page.toggle(Number(modelData.id))
        }

        footer: Column {
            width: parent.width
            readonly property var pending: {
                if (!page.countsKnown || page.closed) return []
                var list = []
                for (var i = 0; i < page.members.length; ++i) {
                    if (page.voters.indexOf(page.members[i]) < 0) list.push(page.members[i])
                }
                return list
            }
            visible: pending.length > 0
            SectionHeader {
                //% "Not voted yet"
                text: qsTrId("threesea-he-poll-not-voted")
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                text: parent.pending.map(page.nameOf).join(", ")
                color: Theme.secondaryColor
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: !page.poll.announce && page.mine
                wrapMode: Text.Wrap
                topPadding: Theme.paddingLarge
                //% "Only you see the votes until you close the poll."
                text: qsTrId("threesea-la-poll-only-you")
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
            }
        }

        VerticalScrollDecorator { }
    }
}
