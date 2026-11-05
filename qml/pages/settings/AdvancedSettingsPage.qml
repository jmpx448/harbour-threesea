// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"

// Settings → Advanced: background running, IPv6, the update check (builds for GitHub, ADR 0014),
// how much is logged.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var network: ({ ipv6: true })
    property var updates: ({ available: false })
    readonly property bool checkingUpdates: page.updates.checking === true

    Connections {
        target: backend
        onCoreEvent: {
            if (name !== "updateChecked") return
            backend.call("updateSettings", {}, function(ok, result) {
                if (ok) page.updates = result
            })
            // The pulley's own check, not the daily one; nothing on the page changes on a failure.
            if (data.manual && !data.ok) {
                //% "Could not check for updates. Try again later."
                Notices.show(qsTrId("threesea-la-update-check-failed"), Notice.Long, Notice.Center)
            }
        }
    }

    Component.onCompleted: {
        backend.call("networkSettings", {}, function(ok, result) {
            if (ok) page.network = result
        })
        backend.call("updateSettings", {}, function(ok, result) {
            if (ok) page.updates = result
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            visible: page.updates.enabled === true
            busy: page.checkingUpdates
            MenuItem {
                //% "Check for updates now"
                text: qsTrId("threesea-me-check-updates-now")
                // The core checks in the background and says when it is done (updateChecked).
                onClicked: backend.call("checkForUpdate", {}, function(ok, result) {
                    if (ok) page.updates = result
                })
            }
        }

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Advanced"
                title: qsTrId("threesea-se-advanced")
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.keepRunning
                //% "Keep running in the background"
                text: qsTrId("threesea-la-keep-running")
                description: backend.full
                             //% "Threesea starts when you turn on the phone and stays connected after you close it, so new messages arrive right away. Uses more battery."
                             ? qsTrId("threesea-la-keep-running-description-full")
                             //% "Threesea stays connected after you close it, so new messages arrive right away. After you turn on the phone, it connects once you open it. Uses more battery."
                             : qsTrId("threesea-la-keep-running-description")
                onClicked: backend.keepRunning = !checked
            }

            SectionHeader {
                visible: page.updates.available === true
                //% "Updates"
                text: qsTrId("threesea-se-updates")
            }

            TextSwitch {
                visible: page.updates.available === true
                automaticCheck: false
                checked: page.updates.enabled === true
                //% "Check for updates"
                text: qsTrId("threesea-la-update-check")
                //% "Once a day, Threesea asks GitHub whether a newer version has been released, and tells you. GitHub sees your IP address; nothing else about you is sent."
                description: qsTrId("threesea-la-update-check-description")
                onClicked: backend.call("setUpdateCheck", { enabled: !checked }, function(ok, result) {
                    if (ok) page.updates = result
                })
            }

            // Like the version on the About page: what is current, when it was last asked.
            DetailItem {
                visible: page.updates.available === true && page.updates.enabled === true && !!page.updates.checkedAt
                //% "Newest version"
                label: qsTrId("threesea-la-update-newest")
                value: page.updates.latest ? page.updates.latest.version : (page.updates.current || "")
            }

            DetailItem {
                visible: page.updates.available === true && page.updates.enabled === true && !!page.updates.checkedAt
                //% "Installed"
                label: qsTrId("threesea-la-update-installed")
                value: page.updates.current || ""
            }

            DetailItem {
                visible: page.updates.available === true && page.updates.enabled === true && !!page.updates.checkedAt
                //% "Last checked"
                label: qsTrId("threesea-la-update-checked")
                value: page.updates.checkedAt
                       ? Format.formatDate(new Date(page.updates.checkedAt), Formatter.DateMedium) + " "
                         + Format.formatDate(new Date(page.updates.checkedAt), Formatter.TimeValue)
                       : ""
            }

            Item {
                visible: !!page.updates.latest
                width: parent.width
                height: downloadButton.height + 2 * Theme.paddingLarge

                Button {
                    id: downloadButton
                    anchors.centerIn: parent
                    //% "Open download page"
                    text: qsTrId("threesea-la-update-open")
                    // Sailfish itself says so when no browser is installed.
                    onClicked: Qt.openUrlExternally(page.updates.latest.url)
                }
            }

            SectionHeader {
                //% "Network"
                text: qsTrId("threesea-se-network")
            }

            // Android's "IPv6 for messages", which here covers calls as well; changing it connects
            // again at once.
            TextSwitch {
                automaticCheck: false
                checked: page.network.ipv6 !== false
                //% "IPv6"
                text: qsTrId("threesea-la-ipv6")
                //% "Messages, files and calls use IPv6 where the network has it. Off: Threesea uses IPv4 for them."
                description: qsTrId("threesea-la-ipv6-description")
                onClicked: backend.call("setNetworkSettings", { ipv6: !checked }, function(ok, result) {
                    if (ok) page.network = result
                })
            }

            SectionHeader {
                //% "Troubleshooting"
                text: qsTrId("threesea-se-troubleshooting")
            }

            // Into the system log (journalctl) and the in-app log below. What is logged follows CLAUDE.md §7.3 at any level.
            ChoiceBox {
                //% "Log"
                label: qsTrId("threesea-la-log-level")
                //% "Problems only"
                options: [qsTrId("threesea-la-log-level-warn"),
                          //% "Normal"
                          qsTrId("threesea-la-log-level-info"),
                          //% "Detailed"
                          qsTrId("threesea-la-log-level-debug")]
                choice: Math.max(0, ["warn", "info", "debug"].indexOf(backend.logLevel))
                //% "How much Threesea logs, to help find the cause of a problem."
                description: qsTrId("threesea-la-log-level-description")
                onChosen: backend.logLevel = ["warn", "info", "debug"][index]
            }

            BackgroundItem {
                width: parent.width
                onClicked: pageStack.push(Qt.resolvedUrl("LogPage.qml"))

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    //% "View log"
                    text: qsTrId("threesea-la-view-log")
                    color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
