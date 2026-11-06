// SPDX-License-Identifier: AGPL-3.0-only
// The app's ThreemaMap on a Silica page, with a style given by scripts/test-mapview.sh: the offline
// fixture (inline GeoJSON: no tiles, glyphs or sprite) or a URL of the test's local server.
import QtQuick 2.6
import Sailfish.Silica 1.0
import "components"

ApplicationWindow {
    initialPage: Component {
        Page {
            PageHeader {
                id: header
                title: "Location"
                description: "Map test"
            }
            ThreemaMap {
                id: mapView
                anchors {
                    top: header.bottom
                    left: parent.left
                    right: parent.right
                    bottom: parent.bottom
                }
                start: ({ latitude: 47.3769, longitude: 8.5417 })
                startZoom: 13
                privateChat: testPrivate
                styleJson: testStyleJson
                styleUrl: testStyleUrl
                places: [{ name: "", latitude: 47.385, longitude: 8.53 }]
                marked: ({ latitude: 47.37, longitude: 8.555 })
            }
            // A position coming in later (as the send dialog's does every second) must not move
            // the map: it starts where it was told once.
            Timer {
                interval: 3000
                running: true
                onTriggered: {
                    mapView.start = { latitude: 10, longitude: 10 }
                    var c = mapView.map.center
                    if (Math.abs(c.latitude - 47.3769) > 1e-6 || Math.abs(c.longitude - 8.5417) > 1e-6)
                        console.warn("a new start moved the map to " + c.latitude + ", " + c.longitude)
                }
            }
        }
    }
}
