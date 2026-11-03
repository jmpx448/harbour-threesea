// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A centred button under a switch that opens that feature's own settings, like "Favorite
// ambiences" under "Show ambiences in top menu" in Sailfish's Settings → Top menu. Bind `enabled`
// to the switch it belongs to.
Item {
    id: root

    property alias text: button.text
    signal clicked()

    width: parent ? parent.width : 0
    height: button.height + 2 * Theme.paddingLarge

    Button {
        id: button
        anchors.centerIn: parent
        enabled: root.enabled
        onClicked: root.clicked()
    }
}
