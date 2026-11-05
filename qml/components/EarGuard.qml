// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6

// Black and deaf to touch while a call is held to the ear, so that a cheek doesn't press hang up
// or mute. (The sensor can't turn the display off; this is what Android apps do with the proximity
// wake lock.) The page it is on says when that can be with its `atEar`, for the window's sensor.
Rectangle {
    property bool atEar
    readonly property bool shown: atEar && proximity.covered

    anchors.fill: parent
    z: 1000
    color: "black"
    visible: shown

    MouseArea {
        anchors.fill: parent
        enabled: parent.shown
    }
}
