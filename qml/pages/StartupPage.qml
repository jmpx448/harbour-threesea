// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Page {
    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent

        BusyIndicator {
            id: busy
            anchors.centerIn: parent
            size: BusyIndicatorSize.Large
            running: backend.state === "starting" || backend.state === "removing"
        }

        InfoLabel {
            anchors {
                bottom: busy.top
                bottomMargin: Theme.paddingLarge
            }
            visible: backend.state === "removing"
            //% "Removing ID and data"
            text: qsTrId("threesea-la-removing-data")
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
