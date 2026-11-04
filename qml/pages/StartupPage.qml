// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Page {
    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent

        PullDownMenu {
            visible: backend.dataRemovalStuck
            MenuItem {
                //% "Try again"
                text: qsTrId("threesea-me-try-again")
                onClicked: backend.retryDataRemoval()
            }
        }

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
            text: backend.dataRemovalStuck
                  //% "Removing the ID did not finish"
                  ? qsTrId("threesea-la-removal-stuck")
                  //% "Threesea could not start"
                  : qsTrId("threesea-la-startup-failed")
            hintText: (backend.dataRemovalStuck
                       //% "Pull down to try again, or close Threesea and open it again (%1)."
                       ? qsTrId("threesea-la-removal-stuck-hint")
                       //% "The encrypted storage could not be opened (%1)."
                       : qsTrId("threesea-la-startup-failed-hint")).arg(backend.errorText)
        }
    }
}
