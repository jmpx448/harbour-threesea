// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtPositioning 5.4
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// Send a location (Threema's location message): by default a recent position from the device's
// positioning (GPS / network as set in Settings → Location) with how accurate it is, or a place
// chosen from Threema's map server (PlacePickerPage); the user can name it. No address: Android
// finds those with the platform's geocoder, which has no counterpart here. No map (ADR 0030).
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    // What goes out, read by the chat when the dialog is accepted.
    readonly property var location: place || fix
    // Our own recent position.
    property var fix: null
    // A chosen place `{name, latitude, longitude}`, instead of the fix.
    property var place: null
    // Where to search around: the last position known, of any age, as it only orders the results.
    property var center: null
    // Keeps the position coming while the place list is on top.
    property bool picking: false

    onStatusChanged: if (status === PageStatus.Active) picking = false

    function choosePlace(chosen) {
        if (chosen) {
            nameField.text = chosen.name
        } else if (place && nameField.text === place.name) {
            nameField.text = ""
        }
        place = chosen ? { name: chosen.name, latitude: chosen.latitude, longitude: chosen.longitude,
                           accuracy: null } : null
    }
    property alias placeName: nameField.text

    canAccept: location !== null

    // Qt starts a source off with the last position it knows, which may be hours old and from
    // somewhere else. `valid` only says that a positioning plugin exists; location being off
    // shows in `sourceError`, if the platform reports it at all, hence the wait's own text.
    readonly property bool unavailable: !positionSource.valid
                                        || positionSource.sourceError !== PositionSource.NoError
    property int secondsWaited: 0
    readonly property int patience: 30
    // How things stand while there is no fix.
    readonly property string waitingText: dialog.unavailable
          //% "Positioning is not available. Turn on location in Settings → Location."
          ? qsTrId("threesea-la-location-unavailable")
          : dialog.secondsWaited >= dialog.patience
          //% "No position yet. Check that location is on in Settings → Location. Outdoors this goes faster."
            ? qsTrId("threesea-la-location-waiting")
          //% "Finding your location…"
            : qsTrId("threesea-la-location-finding")

    PositionSource {
        id: positionSource
        active: dialog.picking
                || (!dialog.place && (dialog.status === PageStatus.Active || dialog.status === PageStatus.Activating))
        updateInterval: 1000
        onPositionChanged: {
            var coordinate = position.coordinate
            if (!position.latitudeValid || !position.longitudeValid) return
            var recent = Fmt.isRecentFix(position.timestamp, Date.now())
            dialog.center = { latitude: coordinate.latitude, longitude: coordinate.longitude, recent: recent }
            if (!recent) return
            var accuracy = position.horizontalAccuracyValid ? position.horizontalAccuracy : null
            // Keep the most accurate fix while the dialog is open, as long as it is recent.
            if (dialog.fix && dialog.fix.accuracy !== null && accuracy !== null
                    && accuracy > dialog.fix.accuracy
                    && Fmt.isRecentFix(dialog.fix.timestamp, Date.now())) return
            dialog.fix = { latitude: coordinate.latitude, longitude: coordinate.longitude,
                           accuracy: accuracy, timestamp: position.timestamp }
        }
    }

    Timer {
        interval: 1000
        repeat: true
        running: positionSource.active
        // The wait counts while looking; a fix can't go out from behind a chosen place.
        onTriggered: {
            // No new fix for a while: where the kept one was taken says nothing about now.
            if (dialog.fix && !Fmt.isRecentFix(dialog.fix.timestamp, Date.now())) {
                dialog.fix = null
            }
            dialog.secondsWaited = dialog.fix ? 0 : dialog.secondsWaited + 1
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Send"
                acceptText: qsTrId("threesea-he-send")
            }

            Row {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingMedium
                BusyIndicator {
                    id: busy
                    size: BusyIndicatorSize.Medium
                    running: dialog.location === null && !dialog.unavailable
                    visible: running
                    anchors.verticalCenter: parent.verticalCenter
                }
                Image {
                    visible: !busy.visible
                    anchors.verticalCenter: parent.verticalCenter
                    source: "image://theme/icon-m-location?" + Theme.highlightColor
                }
                Label {
                    width: parent.width - Theme.itemSizeSmall
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.Wrap
                    color: Theme.highlightColor
                    text: dialog.location ? Fmt.coordinates(dialog.location) : dialog.waitingText
                }
            }

            ValueButton {
                //% "Place"
                label: qsTrId("threesea-la-location-place")
                value: dialog.place ? dialog.place.name
                                    //% "Your position"
                                    : qsTrId("threesea-la-location-own")
                onClicked: {
                    dialog.picking = true
                    pageStack.push(Qt.resolvedUrl("PlacePickerPage.qml"), { positioning: dialog })
                }
            }

            TextField {

                inputMethodHints: window.incognitoHints
                id: nameField
                width: parent.width
                //% "Name of the place (optional)"
                label: qsTrId("threesea-la-location-name")
                placeholderText: label
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: if (dialog.canAccept) dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
