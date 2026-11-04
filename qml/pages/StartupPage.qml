// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Errors.js" as Errors

Page {
    allowedOrientations: Orientation.All

    // Trying again doesn't make an older version newer.
    readonly property bool canRetry: backend.state === "error"
                                     && (backend.dataRemovalStuck || backend.errorKind !== "storeTooNew")

    function failureHint() {
        var lines = [Errors.text(backend.errorKind, "")]
        //% "Pull down to try again."
        if (canRetry) lines.push(qsTrId("threesea-la-startup-try-again"))
        //% "Code: %1"
        lines.push(qsTrId("threesea-la-startup-code").arg(backend.errorText))
        return lines.join("\n")
    }

    SilicaFlickable {
        anchors.fill: parent

        PullDownMenu {
            visible: canRetry
            MenuItem {
                //% "Try again"
                text: qsTrId("threesea-me-try-again")
                onClicked: backend.dataRemovalStuck ? backend.retryDataRemoval() : backend.retryStart()
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
            hintText: backend.dataRemovalStuck
                      //% "Pull down to try again, or close Threesea and open it again (%1)."
                      ? qsTrId("threesea-la-removal-stuck-hint").arg(backend.errorText)
                      : (enabled ? failureHint() : "")
        }
    }
}
