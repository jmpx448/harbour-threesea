// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Page {
    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent

        BusyIndicator {
            anchors.centerIn: parent
            size: BusyIndicatorSize.Large
            running: backend.state === "starting"
        }

        ViewPlaceholder {
            enabled: backend.state === "error"
            //% "Threesea could not start"
            text: qsTrId("threesea-la-startup-failed")
            //% "The encrypted storage could not be opened (%1)."
            hintText: qsTrId("threesea-la-startup-failed-hint").arg(backend.errorText)
        }
    }
}
