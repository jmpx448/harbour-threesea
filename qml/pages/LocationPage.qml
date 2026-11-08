// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Format.js" as Fmt

// A location from a message on Threema's map (ADR 0032), opened by tapping its bubble: nothing of
// the map is fetched before that. The place's name on top, its address or coordinates under it,
// the map filling the rest; a maps app is still one pull away. Back by the header (the map takes
// the swipes for itself, like a maps app's).
Page {
    id: page
    allowedOrientations: Orientation.All

    // {latitude, longitude, name?, address?}
    property var location: ({})
    // A private chat's map keeps its tiles in memory only.
    property bool privateChat: true

    // The map takes the swipes while it is moved: no going back or pulling the menu by accident.
    backNavigation: !mapView.map.gestureInProgress

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: height
        interactive: !mapView.map.gestureInProgress

        PullDownMenu {
            MenuItem {
                //% "Open in maps app"
                text: qsTrId("threesea-me-open-in-maps")
                onClicked: {
                    if (!Qt.openUrlExternally(Fmt.geoUri(page.location))) {
                        //% "No maps app installed"
                        Notices.show(qsTrId("threesea-la-no-maps-app-installed"), Notice.Short, Notice.Center)
                    }
                }
            }
            MenuItem {
                //% "Copy coordinates"
                text: qsTrId("threesea-me-copy-coordinates")
                onClicked: Clipboard.text = Fmt.copyText("location", "", page.location)
            }
        }

        PageHeader {
            id: header
            //% "Location"
            title: page.location.name || qsTrId("threesea-la-location")
            description: page.location.address || Fmt.coordinates(page.location)
        }

        ThreemaMap {
            id: mapView
            anchors {
                top: header.bottom
                left: parent.left
                right: parent.right
                bottom: parent.bottom
            }
            start: page.location
            privateChat: page.privateChat
            marked: page.location
        }
    }
}
