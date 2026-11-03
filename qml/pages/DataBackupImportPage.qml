// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// Data backup, step 3: the import itself. Contacts, groups, lists and pictures first (one
// call), then the message history as the core's background job, with its progress. When it
// ends (done, stopped or failed) the results page replaces this one.
Page {
    id: page
    allowedOrientations: Orientation.All
    // Leaving would lose the progress; the job ends with "Stop", or by itself.
    backNavigation: false

    property string filePath
    property string password

    // "metadata", "history", then "done"
    property string phase: ""
    property bool stopping: false
    property var imported: null
    property var progress: null
    // Overall progress: chats done of all chats, plus the media share of the current one.
    property int chatsDone: 0
    property int chatsTotal: 0
    property real mediaShare: 0

    readonly property real value: chatsTotal > 0
                                  ? Math.min(1, (chatsDone + mediaShare) / chatsTotal) : 0

    Component.onDestruction: password = ""

    // Started once the page is shown, not when the dialog preloads it.
    onStatusChanged: if (status === PageStatus.Active && phase === "") importMetadata()

    function importMetadata() {
        phase = "metadata"
        backend.call("androidBackupImport",
                     { path: filePath, password: password, scope: "metadata" },
                     function(ok, result, kind, message) {
            if (ok) {
                page.imported = result
                page.importHistory()
            } else {
                page.finish(null, Errors.text(kind, message))
            }
        })
    }

    function importHistory() {
        phase = "history"
        backend.call("androidBackupImport",
                     { path: filePath, password: password, scope: "history" },
                     function(ok, result, kind, message) {
            if (!ok) page.finish(null, Errors.text(kind, message))
        })
    }

    function finish(summary, error) {
        phase = "done"
        password = ""
        resultTimer.summary = summary
        resultTimer.error = error || ""
        resultTimer.start()
    }

    // To the results once the page stack is idle (a fast import may end during the push).
    Timer {
        id: resultTimer
        property var summary: null
        property string error
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy) return
            stop()
            pageStack.replace(Qt.resolvedUrl("DataBackupResultPage.qml"), {
                "imported": page.imported,
                "summary": summary,
                "error": error
            })
        }
    }

    Connections {
        target: backend
        onCoreEvent: {
            if (page.phase !== "history") return
            if (name === "androidBackupProgress") {
                page.progress = data
                if (data.step === "media") {
                    page.mediaShare = data.total > 0 ? data.done / data.total : 0
                } else {
                    page.chatsDone = data.done
                    page.chatsTotal = data.total
                    page.mediaShare = 0
                }
            } else if (name === "androidBackupDone") {
                if (data.ok) page.finish(data.summary, "")
                else page.finish(null, Errors.text(data.kind, ""))
            }
        }
    }

    Column {
        width: parent.width
        spacing: Theme.paddingLarge

        PageHeader {
            //% "Importing"
            title: qsTrId("threesea-he-importing")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * x
            wrapMode: Text.Wrap
            color: Theme.highlightColor
            text: {
                if (page.stopping) {
                    //% "Stopping…"
                    return qsTrId("threesea-la-android-import-stopping")
                }
                if (page.phase === "metadata" || page.phase === "") {
                    //% "Contacts, groups and pictures"
                    return qsTrId("threesea-la-android-import-step-metadata")
                }
                if (page.phase === "history" && page.progress === null) {
                    //% "Reading the backup"
                    return qsTrId("threesea-la-android-import-step-reading")
                }
                //% "Messages and media"
                return qsTrId("threesea-la-android-import-step-history")
            }
        }

        ProgressBar {
            width: parent.width
            indeterminate: page.phase !== "history" || page.progress === null
            minimumValue: 0
            maximumValue: 1
            value: page.value
            valueText: indeterminate ? "" : Math.round(page.value * 100) + " %"
            label: {
                var p = page.progress
                if (!p) return ""
                if (p.step === "media") {
                    //% "Chat %1 of %2, media %3 of %4"
                    return qsTrId("threesea-la-android-import-chat-media")
                           .arg(page.chatsDone + 1).arg(page.chatsTotal)
                           .arg(p.done + 1).arg(p.total)
                }
                //% "Chats %1 of %2"
                return qsTrId("threesea-la-android-import-chats").arg(p.done + 1).arg(p.total)
            }
        }

        DetailItem {
            visible: page.imported !== null
            //% "Contacts"
            label: qsTrId("threesea-la-android-backup-imported-contacts")
            value: page.imported ? page.imported.contacts.added : 0
        }

        DetailItem {
            visible: page.imported !== null
            //% "Groups"
            label: qsTrId("threesea-la-android-backup-imported-groups")
            value: page.imported ? page.imported.groups.added : 0
        }

        DetailItem {
            visible: page.progress !== null
            //% "Messages"
            label: qsTrId("threesea-la-android-import-messages")
            value: page.progress && page.progress.imported !== undefined ? page.progress.imported : 0
        }

        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: page.phase === "history"
            enabled: !page.stopping
            //% "Stop"
            text: qsTrId("threesea-bu-android-backup-stop")
            onClicked: {
                page.stopping = true
                backend.call("androidBackupCancel", {})
            }
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * x
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryHighlightColor
            //% "Keep the app open until the import is done. What is imported stays; importing again continues."
            text: qsTrId("threesea-la-android-import-keep-open")
        }
    }
}
