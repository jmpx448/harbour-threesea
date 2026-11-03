// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Asked once (ADR 0014), in builds for GitHub: whether Threesea may check there for a newer
// version. Accept: yes; back or cancel: no. Either can be changed in Settings → Advanced.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    onAccepted: backend.call("setUpdateCheck", { enabled: true })
    onRejected: backend.call("setUpdateCheck", { enabled: false })

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                //% "Check"
                acceptText: qsTrId("threesea-he-update-ask-accept")
                //% "Don't check"
                cancelText: qsTrId("threesea-he-update-ask-cancel")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraLarge
                font.family: Theme.fontFamilyHeading
                color: Theme.highlightColor
                //% "Check for updates?"
                text: qsTrId("threesea-la-update-ask-title")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "This version of Threesea comes from GitHub, so it isn't updated by a store. Once a day, Threesea can ask GitHub whether a newer version has been released and tell you; you install it yourself. GitHub sees your IP address; nothing else about you is sent. You can change this in Settings, Advanced."
                text: qsTrId("threesea-la-update-ask-text")
            }
        }

        VerticalScrollDecorator { }
    }
}
