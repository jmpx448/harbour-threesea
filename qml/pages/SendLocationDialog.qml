// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtPositioning 5.4
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// Send the current location (Threema's location message): waits for a position from the
// device's positioning (GPS / network as set in Settings → Location), shows how accurate it is,
// and lets the user name the place. No map and no address lookup: both would need a
// third-party server (CLAUDE.md §7.6).
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property var location: null
    property alias placeName: nameField.text

    canAccept: location !== null

    PositionSource {
        id: positionSource
        active: dialog.status === PageStatus.Active || dialog.status === PageStatus.Activating
        updateInterval: 1000
        onPositionChanged: {
            var coordinate = position.coordinate
            if (!position.latitudeValid || !position.longitudeValid) return
            var accuracy = position.horizontalAccuracyValid ? position.horizontalAccuracy : null
            // Keep the most accurate fix while the dialog is open.
            if (dialog.location && dialog.location.accuracy !== null && accuracy !== null
                    && accuracy > dialog.location.accuracy) return
            dialog.location = { latitude: coordinate.latitude, longitude: coordinate.longitude,
                                accuracy: accuracy }
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
                    running: dialog.location === null && positionSource.valid
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
                          : !positionSource.valid ? qsTrId("threesea-la-location-unavailable")
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
