// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A closed poll's results (Android: the results view of a closed ballot): every choice as Silica's
// progress bar (its share of the voters, the count above it) and who chose it; the most voted
// highlighted. Read-only; voting is over.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var poll: ({})
    // identity -> display name of the voters (ours is "You").
    property var names: ({})

    readonly property string me: backend.identity

    function nameOf(identity) {
        if (identity === me) return qsTrId("threesea-la-you")
        return names[identity] || identity
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

    readonly property int voterCount: {
        var n = 0
        for (var voter in (poll.votes || {})) if (poll.votes[voter].length > 0) n++
        // Results without who voted: at least the largest count.
        var choices = poll.choices || []
        for (var i = 0; i < choices.length; ++i) n = Math.max(n, count(choices[i]))
        return n
    }

    readonly property int topCount: {
        var top = 0
        var choices = poll.choices || []
        for (var i = 0; i < choices.length; ++i) top = Math.max(top, count(choices[i]))
        return top
    }

    SilicaListView {
        anchors.fill: parent
        model: page.poll.choices || []

        header: Column {
            width: parent.width
            PageHeader {
                //% "Results"
                title: qsTrId("threesea-he-poll-results")
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
                text: [page.poll.multiple ? qsTrId("threesea-la-poll-multiple") : qsTrId("threesea-la-poll-single"),
                       qsTrId("threesea-la-poll-closed"),
                       qsTrId("threesea-la-poll-votes", page.voterCount)].join(" · ")
            }
        }

        // Silica's progress bar: the count above the bar, the choice under it, then who chose it.
        delegate: Column {
            readonly property int votes: page.count(modelData)
            readonly property var voters: page.votersOf(modelData)
            width: parent.width
            bottomPadding: Theme.paddingLarge

            ProgressBar {
                width: parent.width
                leftMargin: Theme.horizontalPageMargin
                rightMargin: Theme.horizontalPageMargin
                minimumValue: 0
                maximumValue: Math.max(1, page.voterCount)
                value: votes
                valueText: votes
                label: modelData.name
                // The most voted in the highlight colour.
                highlighted: votes > 0 && votes === page.topCount
            }
            Label {
                visible: voters.length > 0
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                horizontalAlignment: Text.AlignHCenter
                text: voters.map(page.nameOf).join(", ")
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
            }
        }

        VerticalScrollDecorator { }
    }
}
