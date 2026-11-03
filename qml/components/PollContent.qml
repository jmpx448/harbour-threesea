// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A poll inside a message bubble, compact: the question, its state and what to do; tapping the
// bubble opens the poll page (PollPage.qml) to vote, or its results (PollResultsPage.qml) once it
// is closed. Also the bubble of a poll's closing (`results`).
Row {
    id: root

    property var poll: ({})
    // The poll's closing (its results), not the poll itself.
    property bool results
    property string myIdentity
    property real maxWidth
    property bool highlighted
    property color textColor: Theme.primaryColor

    readonly property bool closed: poll.state === 1
    readonly property bool voted: ((poll.votes && poll.votes[myIdentity]) || []).length > 0
    readonly property int voters: {
        var n = 0
        for (var voter in (poll.votes || {})) if (poll.votes[voter].length > 0) n++
        return n
    }
    readonly property bool countsKnown: closed || poll.announce === true || poll.creator === myIdentity

    spacing: Theme.paddingMedium
    width: Math.min(maxWidth, Screen.width * 2 / 3)

    // The theme has no poll icon; its pie chart reads as "results".
    Image {
        id: icon
        anchors.verticalCenter: parent.verticalCenter
        source: "image://theme/icon-m-storage?" + (root.highlighted ? Theme.highlightColor : root.textColor)
    }

    Column {
        width: root.width - icon.width - root.spacing
        anchors.verticalCenter: parent.verticalCenter

        Label {
            width: parent.width
            text: root.poll.description || ""
            wrapMode: Text.Wrap
            color: root.textColor
        }
        Label {
            width: parent.width
            wrapMode: Text.Wrap
            text: root.results
                  //% "Poll closed. Tap to view results"
                  ? qsTrId("threesea-la-poll-tap-results")
                  : [
                root.closed
                //% "Closed"
                ? qsTrId("threesea-la-poll-closed")
                : root.voted
                  //% "You voted"
                  ? qsTrId("threesea-la-poll-you-voted")
                  //% "Tap to vote"
                  : qsTrId("threesea-la-poll-tap-to-vote"),
                //% "%n vote(s)"
                root.countsKnown ? qsTrId("threesea-la-poll-votes", root.voters) : ""
            ].filter(function(s) { return s.length > 0 }).join(" · ")
            font.pixelSize: Theme.fontSizeExtraSmall
            color: root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
        }
    }
}
