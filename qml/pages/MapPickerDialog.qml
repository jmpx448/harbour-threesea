// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtPositioning 5.4
import Sailfish.Silica 1.0
import "../components"
import "../components/Format.js" as Fmt

// Choosing the point to send on Threema's map (from SendLocationDialog, ADR 0032): the point under
// the pin in the middle, moved by moving the map, or a place of Threema's nearby list, tapped on
// the map. The map server learns the area shown and the places server the middle of it, so the row
// that opens this says so, and nothing is asked before.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    // The dialog below: its `place`, `fix`, `center` and `choosePlace`.
    property var positioning: null
    // A private chat's map keeps its tiles in memory only.
    property bool privateChat: true
    // Where the map opens: the chosen place, else our position, else the last one known.
    readonly property var start: positioning ? (positioning.place || positioning.fix || positioning.center) : null

    // A tapped place, while the map still shows it in the middle.
    property var chosen: null
    property var places: []
    // Where places were last asked around, and the latest request (only its answer counts).
    property var askedAt: null
    property int requests: 0

    // The map's middle, its longitude brought back into -180…180 (panned across the date line
    // it runs on beyond).
    readonly property var point: wrapped(map.center)
    function wrapped(c) {
        if (!c || !isFinite(c.latitude) || !isFinite(c.longitude)) return null
        return QtPositioning.coordinate(c.latitude, ((c.longitude + 180) % 360 + 360) % 360 - 180)
    }
    // The map takes the swipes while it is moved: no accepting or going back by accident.
    backNavigation: !map.map.gestureInProgress
    forwardNavigation: !map.map.gestureInProgress
    readonly property bool pointValid: !!point && point.isValid && (start !== null || map.map.zoomLevel > 3)

    canAccept: pointValid
    onAccepted: {
        if (!positioning) return
        if (chosen) {
            positioning.choosePlace(chosen)
        } else {
            positioning.choosePlace({ name: "", latitude: point.latitude, longitude: point.longitude })
        }
    }

    // A tapped place within a finger's reach of the tap.
    function placeAt(coordinate, degLat, degLon) {
        var reach = Theme.itemSizeSmall / 2
        var best = null
        var bestDistance = reach * reach
        for (var i = 0; i < places.length; ++i) {
            var dy = (places[i].latitude - coordinate.latitude) / degLat
            var dx = (places[i].longitude - coordinate.longitude) / degLon
            var d = dx * dx + dy * dy
            if (d <= bestDistance) {
                best = places[i]
                bestDistance = d
            }
        }
        return best
    }

    // The middle moved away from a tapped place: a point by hand again.
    onPointChanged: {
        if (chosen && point) {
            var moved = QtPositioning.coordinate(chosen.latitude, chosen.longitude).distanceTo(point)
            if (moved > map.map.metersPerPixel * Theme.paddingLarge) chosen = null
        }
        askLater.restart()
    }

    function askPlaces() {
        if (window.locked || !point || !point.isValid || map.map.zoomLevel < 13 || map.map.gestureInProgress) return
        // Again only after the middle moved by a good part of the list's radius (750 m).
        if (askedAt && QtPositioning.coordinate(askedAt.latitude, askedAt.longitude).distanceTo(point) < 300) return
        var args = { latitude: point.latitude, longitude: point.longitude }
        askedAt = args
        var request = ++requests
        backend.call("findPlaces", args, function(ok, result) {
            if (request !== dialog.requests || !ok) return
            dialog.places = result.places
        })
    }

    Timer {
        id: askLater
        // As the place list's search does: once the map has rested a moment.
        interval: 1000
        onTriggered: dialog.askPlaces()
    }

    Connections {
        target: window
        onLockedChanged: if (!window.locked) askLater.restart()
    }

    DialogHeader {
        id: header
        //% "Choose"
        acceptText: qsTrId("threesea-he-choose")
        title: dialog.chosen ? dialog.chosen.name
                             : dialog.pointValid ? Fmt.coordinates({ latitude: dialog.point.latitude,
                                                                     longitude: dialog.point.longitude })
                                                 : ""
    }

    ThreemaMap {
        id: map
        anchors {
            top: header.bottom
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        start: dialog.start
        privateChat: dialog.privateChat
        places: dialog.places
        onTapped: {
            var place = dialog.placeAt(coordinate, degLatPerPixel, degLonPerPixel)
            if (!place || !map.valid(place)) return
            map.map.center = QtPositioning.coordinate(place.latitude, place.longitude)
            dialog.chosen = place
        }

        // The point to send: the theme's location mark, its tip in the middle of the map.
        Image {
            x: (parent.width - width) / 2
            y: parent.height / 2 - height
            source: "image://theme/icon-m-location?" + Theme.highlightColor
        }
    }
}
