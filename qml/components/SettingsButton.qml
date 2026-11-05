// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A centred button under a switch that opens that feature's own settings, like "Favorite
// ambiences" under "Show ambiences in top menu" in Sailfish's Settings → Top menu. Bind `enabled`
// to the switch it belongs to. `description` explains it below, like a TextSwitch's.
Item {
    id: root

    property alias text: button.text
    property string description
    signal clicked()

    width: parent ? parent.width : 0
    height: button.height + 2 * Theme.paddingLarge
            + (description.length > 0 ? descriptionLabel.height + Theme.paddingSmall : 0)

    Button {
        id: button
        y: Theme.paddingLarge
        anchors.horizontalCenter: parent.horizontalCenter
        enabled: root.enabled
        onClicked: root.clicked()
    }

    Label {
        id: descriptionLabel
        anchors.top: button.bottom
        anchors.topMargin: Theme.paddingLarge
        x: Theme.horizontalPageMargin
        width: root.width - 2 * Theme.horizontalPageMargin
        visible: root.description.length > 0
        opacity: root.enabled ? 1.0 : Theme.opacityLow
        text: root.description
        wrapMode: Text.Wrap
        font.pixelSize: Theme.fontSizeExtraSmall
        color: Theme.secondaryColor
    }
}
