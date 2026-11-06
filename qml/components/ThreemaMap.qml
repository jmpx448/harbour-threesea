// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtPositioning 5.4
import Sailfish.Silica 1.0
import MapboxMap 1.0

// Threema's map, as on Android (ADR 0032): drawn by the bundled MapLibre from Threema's map server
// only, whose keys are pinned (the bridge hands MapLibre the core's pins; any other address is
// refused before it is looked up). It loads as soon as it exists, so pages create it only when
// the user asked for a map. The server learns which area is shown, as with Android. Tiles are
// cached in the cache directory (emptied by Delete ID and data); a private chat's map keeps them
// in memory only.
Item {
    id: root

    // Where the map starts: {latitude, longitude}.
    property var start: null
    property real startZoom: 16
    property bool privateChat: false
    // Threema's style; tests/mapview points it at a local server.
    property string styleUrl: "https://map.threema.ch/styles/threema/style.json"
    property string styleJson: ""
    // Places to mark, [{name, latitude, longitude}], each a dot with its name.
    property var places: []
    // One point to mark (a received location), or null.
    property var marked: null
    // Taps on the map, with what one screen pixel is in degrees there.
    signal tapped(var coordinate, real degLatPerPixel, real degLonPerPixel)

    readonly property alias map: map
    readonly property var center: map.center

    // MapLibre ends the process on a coordinate outside the earth (a C++ exception), and a
    // location comes from another user: only valid ones reach it.
    function valid(point) {
        return !!point && isFinite(point.latitude) && isFinite(point.longitude)
               && Math.abs(point.latitude) <= 90 && Math.abs(point.longitude) <= 180
    }
    function usable(list) {
        var result = []
        for (var i = 0; list && i < list.length; ++i) {
            if (valid(list[i])) result.push(list[i])
        }
        return result
    }
    function showPlaces() {
        var shown = usable(places)
        var coordinates = [], names = []
        for (var i = 0; i < shown.length; ++i) {
            coordinates.push(QtPositioning.coordinate(shown[i].latitude, shown[i].longitude))
            names.push(shown[i].name || "")
        }
        map.updateSourcePoints("threesea-places", coordinates, names)
    }
    function showMarked() {
        map.updateSourcePoints("threesea-marked", valid(marked)
                               ? [QtPositioning.coordinate(marked.latitude, marked.longitude)] : [], [])
    }

    onPlacesChanged: showPlaces()
    onMarkedChanged: showMarked()

    MapboxMap {
        id: map
        anchors.fill: parent

        center: root.valid(root.start) ? QtPositioning.coordinate(root.start.latitude, root.start.longitude)
                                       : QtPositioning.coordinate(0, 0)
        zoomLevel: root.valid(root.start) ? root.startZoom : 1
        minimumZoomLevel: 0
        // Threema's tiles go to zoom 14 and are drawn larger beyond.
        maximumZoomLevel: 19
        pixelRatio: Theme.pixelRatio * 1.5
        cacheDatabasePath: root.privateChat ? ":memory:" : backend.mapCache
        cacheDatabaseMaximalSize: 20 * 1024 * 1024

        Component.onCompleted: {
            // One or the other: an empty style text would replace the URL's.
            if (root.styleJson.length > 0) styleJson = root.styleJson
            else styleUrl = root.styleUrl
            // Our marks over the style, in the theme's highlight colour; names in the style's font.
            addSourcePoints("threesea-places", [], [])
            addSourcePoints("threesea-marked", [], [])
            addLayer("threesea-places-dot", { type: "circle", source: "threesea-places" })
            setPaintProperty("threesea-places-dot", "circle-radius", 6 * pixelRatio / Theme.pixelRatio)
            setPaintProperty("threesea-places-dot", "circle-color", String(Theme.highlightColor))
            setPaintProperty("threesea-places-dot", "circle-stroke-width", 2)
            setPaintProperty("threesea-places-dot", "circle-stroke-color", "#ffffff")
            addLayer("threesea-places-name", { type: "symbol", source: "threesea-places" })
            setLayoutProperty("threesea-places-name", "text-field", "{name}")
            setLayoutPropertyList("threesea-places-name", "text-font", ["Noto Sans Regular"])
            setLayoutProperty("threesea-places-name", "text-size", 12)
            setLayoutProperty("threesea-places-name", "text-anchor", "top")
            setLayoutPropertyList("threesea-places-name", "text-offset", [0, 0.8])
            setLayoutProperty("threesea-places-name", "text-optional", true)
            setPaintProperty("threesea-places-name", "text-color", "#333333")
            setPaintProperty("threesea-places-name", "text-halo-color", "#ffffff")
            setPaintProperty("threesea-places-name", "text-halo-width", 1.5)
            addLayer("threesea-marked-dot", { type: "circle", source: "threesea-marked" })
            setPaintProperty("threesea-marked-dot", "circle-radius", 9 * pixelRatio / Theme.pixelRatio)
            setPaintProperty("threesea-marked-dot", "circle-color", String(Theme.highlightColor))
            setPaintProperty("threesea-marked-dot", "circle-stroke-width", 3)
            setPaintProperty("threesea-marked-dot", "circle-stroke-color", "#ffffff")
            root.showPlaces()
            root.showMarked()
        }

        MapboxMapGestureArea {
            map: map
            activeClickedGeo: true
            onClickedGeo: root.tapped(geocoordinate, degLatPerPixel, degLonPerPixel)
        }
    }

    // OpenStreetMap's licence (ODbL) and OpenMapTiles' ask for this line wherever the map shows;
    // Android leaves it out, we don't.
    Rectangle {
        anchors {
            right: parent.right
            bottom: parent.bottom
        }
        width: attribution.implicitWidth + 2 * Theme.paddingSmall
        height: attribution.implicitHeight + Theme.paddingSmall
        color: Theme.rgba(Theme.highlightDimmerColor, Theme.opacityHigh)

        Label {
            id: attribution
            anchors.centerIn: parent
            font.pixelSize: Theme.fontSizeTiny
            color: Theme.primaryColor
            text: "© OpenMapTiles © OpenStreetMap contributors"
        }
    }
}
