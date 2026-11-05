// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Format.js" as Fmt

// Choosing a place to send (from SendLocationDialog), from Threema's map server: nearby places
// around the position, or by name, nearest first. The server gets the position and the search
// text, so nothing is asked before this page opens.
Page {
    id: page
    allowedOrientations: Orientation.All

    // The dialog below: its `center`, `place`, `waitingText` and `choosePlace`.
    property var positioning: null
    readonly property var center: positioning ? positioning.center : null

    property var places: []
    property bool waiting: false
    property bool failed: false
    // The text searched for, once it is long enough to ask; else the list shows what is nearby.
    readonly property int minQueryChars: 3
    property string query
    property var nearby: []
    // How good a position the nearby list is from: 0 none yet, 1 an old one, 2 a recent one. Asked
    // again only for a better one: the position changes every second.
    property int nearbyRank: 0
    property int requests: 0
    property Item searchFieldItem

    onCenterChanged: askNearby()
    Component.onCompleted: askNearby()

    function centerRank() {
        return !center ? 0 : center.recent ? 2 : 1
    }

    // Not while asking, or after a failure until the user tries again.
    function askNearby() {
        if (query.length === 0 && !waiting && !failed && centerRank() > nearbyRank) ask()
    }

    // One answer counts, the one to the latest question.
    function ask() {
        if (!center) return
        var args = { latitude: center.latitude, longitude: center.longitude }
        if (query.length > 0) args.query = query
        var rank = centerRank()
        var request = ++requests
        waiting = true
        failed = false
        backend.call("findPlaces", args, function(ok, result) {
            if (request !== page.requests) return
            page.waiting = false
            page.failed = !ok
            var found = ok ? result.places : []
            if (ok && args.query === undefined) {
                page.nearby = found
                page.nearbyRank = rank
            }
            page.show(found)
        })
    }

    function show(found) {
        // A filled model makes its first row the list's current item, which takes the focus from
        // the field in the list's header and closes the keyboard (see SearchPage).
        var typing = searchFieldItem && searchFieldItem.activeFocus
        places = found
        listView.currentIndex = -1
        if (typing) refocusField.restart()
    }

    function searchTextChanged(text) {
        var trimmed = text.trim()
        query = trimmed.length >= minQueryChars ? trimmed : ""
        // Back to what is nearby at once; a search once typing has paused.
        if (query.length === 0) {
            searchDelay.stop()
            requests++
            waiting = false
            failed = false
            show(nearby)
            askNearby()
        } else {
            searchDelay.restart()
        }
    }

    function choose(place) {
        positioning.choosePlace(place)
        pageStack.pop()
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: page.places
        currentIndex: -1

        // Its header grows and shrinks with the row for going back to one's own position.
        StayAtTop { target: listView }

        PullDownMenu {
            visible: page.failed
            MenuItem {
                text: qsTrId("threesea-me-try-again")
                onClicked: page.ask()
            }
        }

        header: Column {
            width: parent.width
            PageHeader {
                //% "Choose a place"
                title: qsTrId("threesea-he-choose-place")
            }
            SearchField {
                id: searchField
                width: parent.width
                //% "Search by name"
                placeholderText: qsTrId("threesea-la-place-search")
                inputMethodHints: Qt.ImhNoAutoUppercase | window.incognitoHints
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
                onTextChanged: page.searchTextChanged(text)
                Component.onCompleted: page.searchFieldItem = searchField
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                //% "To find places, Threema's map server gets your position and what you search for."
                text: qsTrId("threesea-la-place-server")
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                bottomPadding: Theme.paddingMedium
            }
            BackgroundItem {
                id: ownPosition
                visible: page.positioning !== null && !!page.positioning.place
                width: parent.width
                onClicked: page.choose(null)
                Image {
                    id: ownIcon
                    x: Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    source: "image://theme/icon-m-location?" + (ownPosition.highlighted ? Theme.highlightColor : Theme.primaryColor)
                }
                Label {
                    anchors {
                        left: ownIcon.right
                        leftMargin: Theme.paddingMedium
                        right: parent.right
                        rightMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    //% "Use your position"
                    text: qsTrId("threesea-la-place-use-own")
                    truncationMode: TruncationMode.Fade
                    color: ownPosition.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }
            SectionHeader {
                visible: page.places.length > 0
                text: page.query.length > 0
                      //% "Places found"
                      ? qsTrId("threesea-he-places-found")
                      //% "Places nearby"
                      : qsTrId("threesea-he-places-nearby")
            }
        }

        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeSmall
            onClicked: page.choose(modelData)
            Label {
                id: nameLabel
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: distanceLabel.left
                    rightMargin: Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                text: modelData.name
                textFormat: Text.PlainText
                truncationMode: TruncationMode.Fade
                color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
            }
            Label {
                id: distanceLabel
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                text: Fmt.distance(modelData.distance, Qt.locale().decimalPoint)
                font.pixelSize: Theme.fontSizeSmall
                color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            }
        }

        ViewPlaceholder {
            enabled: page.places.length === 0 && !page.waiting && !searchDelay.running
            verticalOffset: listView.headerItem ? listView.headerItem.height / 2 : 0
            text: !page.center
                  //% "Places are found around your position"
                  ? qsTrId("threesea-la-places-need-position")
                  : page.failed
                  //% "Threema's map server did not answer"
                    ? qsTrId("threesea-la-places-failed")
                  //% "No places found"
                    : qsTrId("threesea-la-places-none")
            hintText: !page.center && page.positioning ? page.positioning.waitingText
                      : page.failed
                      //% "Check your internet connection and pull down to try again."
                        ? qsTrId("threesea-la-places-failed-hint")
                        : page.query.length > 0 ? ""
                      //% "Search by name to look further away."
                        : qsTrId("threesea-la-places-none-hint")
        }

        BusyIndicator {
            parent: listView.contentItem
            size: BusyIndicatorSize.Large
            running: page.waiting
            visible: running
            anchors.horizontalCenter: parent.horizontalCenter
            y: (listView.headerItem ? listView.headerItem.height : 0) + Theme.paddingLarge
        }

        VerticalScrollDecorator { }
    }

    Timer {
        id: searchDelay
        // As on Android.
        interval: 1000
        onTriggered: page.ask()
    }

    Timer {
        id: refocusField
        interval: 20
        onTriggered: {
            page.searchFieldItem.forceActiveFocus()
            Qt.inputMethod.show()
        }
    }
}
