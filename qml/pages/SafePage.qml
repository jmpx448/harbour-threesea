// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

Page {
    id: page
    objectName: "holdsPin"
    readonly property bool safePage: true
    allowedOrientations: Orientation.All

    property var status: ({ enabled: false })
    property bool working: false
    // The app lock's PIN, while this page lives (window.pushWithPin).
    property string pin

    // After a restore: what still waits to be imported (it needs the directory server), what the
    // import skipped and why, and the last failure. Shown only while something needs attention.
    readonly property var restore: status.restore || ({})
    readonly property var importSummary: restore.summary || ({})
    readonly property int pendingImports: (restore.pendingContacts || 0) + (restore.pendingGroups || 0)
                                          + (restore.pendingLists || 0)
    readonly property var skippedGroups: importSummary.skippedGroups || ({})
    readonly property int skippedTotal: (skippedGroups.deleted || 0) + (skippedGroups.unknownCreator || 0)
                                        + (skippedGroups.invalid || 0)

    // "%1 contacts, %1 groups, %1 distribution lists" from per-kind counts, kinds of 0 left out.
    function importParts(contacts, groups, lists) {
        var parts = []
        //% "%n contact(s)"
        if (contacts > 0) parts.push(qsTrId("threesea-la-n-contacts", contacts))
        //% "%n group(s)"
        if (groups > 0) parts.push(qsTrId("threesea-la-n-groups", groups))
        //% "%n distribution list(s)"
        if (lists > 0) parts.push(qsTrId("threesea-la-n-lists", lists))
        return parts.join(", ")
    }

    function refresh() {
        backend.call("safeStatus", {}, function(ok, result) { if (ok) page.status = result })
    }

    function run(command, args) {
        working = true
        errorLabel.text = ""
        backend.call(command, args || {}, function(ok, result, kind, message) {
            working = false
            if (ok) {
                page.status = result
                //% "Threema Safe is off. The backup could not be deleted from Threema's server, so it stays there until it expires."
                if (result.serverCopyDeleted === false) errorLabel.text = qsTrId("threesea-la-safe-server-copy-left")
            } else {
                errorLabel.text = Errors.text(kind, message)
                refresh()
            }
        })
    }

    function askPassword() {
        var dialog = pageStack.push(Qt.resolvedUrl("SafePasswordDialog.qml"))
        dialog.accepted.connect(function() { page.run("safeEnable", { password: dialog.password, pin: page.pin }) })
    }

    Component.onCompleted: refresh()

    Connections {
        target: backend
        onCoreEvent: if (name === "safeChanged") page.refresh()
    }

    RemorsePopup { id: remorse }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            visible: page.status.enabled || page.pendingImports > 0
            busy: page.working
            MenuItem {
                visible: page.status.enabled
                //% "Change password"
                text: qsTrId("threesea-me-safe-change-password")
                enabled: !page.working
                onClicked: page.askPassword()
            }
            MenuItem {
                visible: page.pendingImports > 0
                //% "Import the rest now"
                text: qsTrId("threesea-me-safe-retry-import")
                enabled: !page.working
                onClicked: page.run("safeRetryImport")
            }
            MenuItem {
                visible: page.status.enabled
                //% "Back up now"
                text: qsTrId("threesea-me-safe-backup-now")
                enabled: !page.working
                onClicked: page.run("safeBackupNow")
            }
        }

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Threema Safe"
                title: qsTrId("threesea-la-threema-safe")
            }

            TextSwitch {
                automaticCheck: false
                checked: page.status.enabled
                busy: page.working
                //% "Threema Safe"
                text: qsTrId("threesea-la-threema-safe")
                //% "Encrypted backup of your ID, profile, contacts, groups and settings, updated when something changes. For your chats, use Data backup."
                description: qsTrId("threesea-la-safe-description-changes")
                onClicked: {
                    if (page.working) return
                    if (page.status.enabled) {
                        //% "Turning off and deleting the backup"
                        remorse.execute(qsTrId("threesea-la-safe-disabling-delete"), function() { page.run("safeDisable", { pin: page.pin }) })
                    } else {
                        page.askPassword()
                    }
                }
            }

            ErrorLabel { id: errorLabel }

            SectionHeader {
                visible: page.status.enabled
                //% "Status"
                text: qsTrId("threesea-se-status")
            }

            DetailItem {
                visible: page.status.enabled
                //% "Last backup"
                label: qsTrId("threesea-la-safe-last-backup")
                value: page.status.lastBackup
                       // "Today 14:49": Silica's TimepointRelative ignores the 24-hour setting.
                       ? Fmt.day(page.status.lastBackup * 1000) + " " + Fmt.clock(page.status.lastBackup * 1000, Format, Formatter)
                       //% "Not yet"
                       : qsTrId("threesea-la-safe-not-yet")
            }

            DetailItem {
                visible: page.status.enabled && !!page.status.lastError
                //% "Last attempt failed"
                label: qsTrId("threesea-la-safe-last-attempt-failed")
                value: page.status.lastError ? Errors.text(page.status.lastError, "") : ""
            }

            DetailItem {
                visible: page.status.enabled && !!page.status.size
                //% "Size"
                label: qsTrId("threesea-la-safe-size")
                value: page.status.size ? Format.formatFileSize(page.status.size) : ""
            }

            DetailItem {
                visible: page.status.enabled
                //% "Server"
                label: qsTrId("threesea-la-safe-server")
                value: page.status.server ? page.status.server.replace(/^https:\/\//, "").replace(/\/$/, "") : ""
            }

            DetailItem {
                visible: page.status.enabled && !!page.status.retentionDays
                //% "Kept on the server"
                label: qsTrId("threesea-la-safe-retention")
                // Retention is always many days (180 on Threema's servers): no plural forms needed.
                //% "%1 days"
                value: qsTrId("threesea-la-days").arg(page.status.retentionDays || 0)
            }

            SectionHeader {
                visible: page.pendingImports > 0 || page.skippedTotal > 0
                         || !!page.restore.importError || (page.importSummary.revokedContacts || 0) > 0
                //% "Restore"
                text: qsTrId("threesea-se-safe-restore")
            }

            // Contacts, groups and lists wait for a directory lookup; "Import the rest now"
            // retries, and so do reconnecting and the hourly maintenance.
            DetailItem {
                visible: page.pendingImports > 0
                //% "Waiting to import"
                label: qsTrId("threesea-la-safe-to-import")
                value: page.importParts(page.restore.pendingContacts, page.restore.pendingGroups,
                                        page.restore.pendingLists)
            }

            DetailItem {
                visible: page.pendingImports > 0 && !!page.restore.importError
                //% "Import attempt"
                label: qsTrId("threesea-la-safe-import-attempt")
                value: page.restore.importError ? Errors.text(page.restore.importError, "") : ""
            }

            // Groups the backup cannot restore, like on Android: already deleted there, their
            // creator unknown, or unreadable entries.
            DetailItem {
                visible: (page.skippedGroups.deleted || 0) > 0
                //% "Deleted groups"
                label: qsTrId("threesea-la-safe-skipped-deleted")
                //% "%n group(s)"
                value: qsTrId("threesea-la-n-groups", page.skippedGroups.deleted || 0)
            }

            DetailItem {
                visible: (page.skippedGroups.unknownCreator || 0) > 0
                //% "Creator not available"
                label: qsTrId("threesea-la-safe-skipped-creator")
                value: qsTrId("threesea-la-n-groups", page.skippedGroups.unknownCreator || 0)
            }

            DetailItem {
                visible: (page.skippedGroups.invalid || 0) > 0
                //% "Unreadable entries"
                label: qsTrId("threesea-la-safe-skipped-invalid")
                value: qsTrId("threesea-la-n-groups", page.skippedGroups.invalid || 0)
            }

            DetailItem {
                visible: (page.importSummary.revokedContacts || 0) > 0
                //% "No longer registered"
                label: qsTrId("threesea-la-safe-revoked")
                //% "%n contact(s)"
                value: qsTrId("threesea-la-n-contacts", page.importSummary.revokedContacts || 0)
            }
        }

        VerticalScrollDecorator { }
    }
}
