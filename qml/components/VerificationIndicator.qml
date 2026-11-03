// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Threema's verification level as three dots (1 = unverified, 2 = verified by the server,
// 3 = fully verified in person). Drawn in Theme colours instead of Threema's red/orange/green
// so it fits every ambience; the meaning is the same.
Row {
    property int level: 0           // 0, 1, 2 like the protocol
    property bool highlighted: false
    property real dotSize: Theme.paddingSmall + Theme.paddingSmall / 2

    spacing: Theme.paddingSmall / 2

    Repeater {
        model: 3
        Rectangle {
            width: dotSize
            height: width
            radius: width / 2
            color: index <= level
                   ? (highlighted ? Theme.highlightColor : Theme.primaryColor)
                   : "transparent"
            border.width: Math.max(1, Theme.paddingSmall / 4)
            border.color: highlighted ? Theme.highlightColor : Theme.secondaryColor
        }
    }
}
