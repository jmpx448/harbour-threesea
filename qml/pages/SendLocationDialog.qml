// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtPositioning 5.4
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// Send the current location (Threema's location message): waits for a recent position from the
// device's positioning (GPS / network as set in Settings → Location), shows how accurate it is,
// and lets the user name the place. Android finds addresses with the platform's geocoder, which
// has no counterpart here.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property var location: null
    property alias placeName: nameField.text

    canAccept: location !== null

    // Qt starts a source off with the last position it knows, which may be hours old and from
    // somewhere else. `valid` only says that a positioning plugin exists; location being off
    // shows in `sourceError`, if the platform reports it at all, hence the wait's own text.
    readonly property bool unavailable: !positionSource.valid
                                        || positionSource.sourceError !== PositionSource.NoError
    property int secondsWaited: 0
    readonly property int patience: 30

    PositionSource {
        id: positionSource
        active: dialog.status === PageStatus.Active || dialog.status === PageStatus.Activating
        updateInterval: 1000
        onPositionChanged: {
            var coordinate = position.coordinate
            if (!position.latitudeValid || !position.longitudeValid) return
            if (!Fmt.isRecentFix(position.timestamp, Date.now())) return
            var accuracy = position.horizontalAccuracyValid ? position.horizontalAccuracy : null
            // Keep the most accurate fix while the dialog is open, as long as it is recent.
            if (dialog.location && dialog.location.accuracy !== null && accuracy !== null
                    && accuracy > dialog.location.accuracy
                    && Fmt.isRecentFix(dialog.location.timestamp, Date.now())) return
            dialog.location = { latitude: coordinate.latitude, longitude: coordinate.longitude,
                                accuracy: accuracy, timestamp: position.timestamp }
        }
    }

    Timer {
        interval: 1000
        repeat: true
        running: positionSource.active
        onTriggered: {
            // No new fix for a while: where the kept one was taken says nothing about now.
            if (dialog.location && !Fmt.isRecentFix(dialog.location.timestamp, Date.now())) {
                dialog.location = null
            }
            dialog.secondsWaited = dialog.location ? 0 : dialog.secondsWaited + 1
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
                    text: dialog.location ? Fmt.coordinates(dialog.location)
                          //% "Positioning is not available. Turn on location in Settings → Location."
                          : dialog.unavailable ? qsTrId("threesea-la-location-unavailable")
                          //% "No position yet. Check that location is on in Settings → Location. Outdoors this goes faster."
                          : dialog.secondsWaited >= dialog.patience ? qsTrId("threesea-la-location-waiting")
                          //% "Finding your location…"
                          : qsTrId("threesea-la-location-finding")
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
