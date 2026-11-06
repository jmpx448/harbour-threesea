// SPDX-License-Identifier: AGPL-3.0-only
// One Silica page with the bundled map renderer, drawing a style from this directory (no network:
// a GeoJSON source inline, no tiles, glyphs or sprite). scripts/test-mapview.sh takes a picture.
import QtQuick 2.6
import QtPositioning 5.4
import Sailfish.Silica 1.0
import MapboxMap 1.0

ApplicationWindow {
    initialPage: Component {
        Page {
            PageHeader {
                id: header
                title: "Location"
                description: "Map test, offline fixture"
            }
            MapboxMap {
                id: map
                anchors {
                    top: header.bottom
                    left: parent.left
                    right: parent.right
                    bottom: parent.bottom
                }
                cacheDatabasePath: "/tmp/threesea-mapview-cache.db"
                center: QtPositioning.coordinate(47.3769, 8.5417)
                zoomLevel: 13
                pixelRatio: Theme.pixelRatio * 1.5
                styleJson: fixtureStyle
            }
            Label {
                anchors {
                    right: parent.right
                    bottom: parent.bottom
                    margins: Theme.paddingSmall
                }
                font.pixelSize: Theme.fontSizeTiny
                color: "black"
                text: "© OpenMapTiles © OpenStreetMap contributors"
            }
        }
    }
}
