// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Errors.js" as Errors

Page {
    id: page
    allowedOrientations: Orientation.All

    // Not for an upgrade that is over before anyone could read this.
    property bool upgradeTakesLong: false
    Timer {
        interval: 1500
        running: backend.upgrading
        onTriggered: page.upgradeTakesLong = true
    }

    // Trying again doesn't make an older version newer.
    readonly property bool canRetry: backend.state === "error"
                                     && (backend.dataRemovalStuck || backend.errorKind !== "storeTooNew")

    // The data is there but can't be opened (its key, damage): starting without it is the way
    // on. Not for a newer version's data, a full disk or any other store error such as a failed
    // upgrade, which a fixed release opens again (the bridge's startFresh has the same list).
    readonly property bool canStartFresh: backend.state === "error" && !backend.dataRemovalStuck
                                          && ["storeKey", "storeDamaged", "keyMissing"].indexOf(backend.errorKind) >= 0

    function askStartFresh() {
        var dialog = pageStack.push(Qt.resolvedUrl("StartFreshDialog.qml"))
        dialog.accepted.connect(function() {
            // Last chance to undo, like Remove ID.
            //% "Deleting the stored data"
            Remorse.popupAction(page, qsTrId("threesea-la-starting-fresh"), function() { backend.startFresh() })
        })
    }

    function failureHint() {
        var lines = [Errors.text(backend.errorKind, "")]
        //% "Pull down to try again, or to start fresh without the stored data."
        if (canRetry && canStartFresh) lines.push(qsTrId("threesea-la-startup-try-again-or-fresh"))
        //% "Pull down to try again."
        else if (canRetry) lines.push(qsTrId("threesea-la-startup-try-again"))
        //% "Code: %1"
        lines.push(qsTrId("threesea-la-startup-code").arg(backend.errorText))
        return lines.join("\n")
    }

    SilicaFlickable {
        anchors.fill: parent

        PullDownMenu {
            visible: canRetry || canStartFresh
            MenuItem {
                objectName: "startFresh"
                visible: page.canStartFresh
                //% "Start fresh"
                text: qsTrId("threesea-me-start-fresh")
                onClicked: page.askStartFresh()
            }
            MenuItem {
                visible: page.canRetry
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
            visible: backend.state === "removing" || (backend.upgrading && page.upgradeTakesLong)
            text: backend.state === "removing"
                  //% "Removing ID and data"
                  ? qsTrId("threesea-la-removing-data")
                  : Errors.upgradingHint()
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
