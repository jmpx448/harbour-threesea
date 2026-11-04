// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "../components"
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

// Settings → Backup → Data backup: encrypted full backups in the format of Threema for
// Android. Import one (checked locally first, then the import flow in its own pages) or
// create backups into a folder, by hand or on a schedule.
Page {
    id: page
    objectName: "holdsPin"
    allowedOrientations: Orientation.All

    property string filePath
    property string fileName
    property bool working: false
    // Creating backups: the stored schedule settings and the last written file.
    property var exportStatus: ({})
    property bool exportRunning: false
    property string exportResult
    // The app lock's PIN, while this page lives (window.pushWithPin).
    property string pin

    function refreshExportStatus() {
        backend.call("androidBackupExportStatus", {}, function(ok, result) {
            if (ok) page.exportStatus = result
        })
    }
    Component.onCompleted: refreshExportStatus()

    // Called from the folder picker, which is destroyed while the call runs: the reply must
    // come back to this page's context, not the picker's.
    function setExportFolder(path) {
        if (!path) return
        backend.call("androidBackupSchedule", { directory: path, pin: page.pin }, function(ok, result, kind, message) {
            if (ok) page.exportStatus = result
            else exportError.text = Errors.text(kind, message)
        })
    }

    // The password stays only in this field while the page lives.
    Component.onDestruction: {
        password.text = ""
        pin = ""
    }

    // Checking only reads the file; what it holds goes to the import dialog, which imports on
    // accept (offline, nothing is sent, existing local data stays) and ends on a results page.
    function check() {
        if (working || filePath.length === 0 || password.text.length === 0) return
        working = true
        importError.text = ""
        backend.call("androidBackupInspect", { path: filePath, password: password.text },
                     function(ok, result, kind, message) {
            page.working = false
            if (ok) {
                pageStack.push(Qt.resolvedUrl("DataBackupImportDialog.qml"), {
                    "preview": result,
                    "filePath": page.filePath,
                    "fileName": page.fileName,
                    "password": password.text
                })
            } else {
                importError.text = Errors.text(kind, message)
            }
        })
    }

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "androidExportDone") {
                page.exportRunning = false
                if (data.ok) {
                    page.exportResult = data.path.split("/").pop()
                } else {
                    exportError.text = Errors.text(data.kind, "")
                }
                page.refreshExportStatus()
            }
        }
    }

    Component {
        id: filePicker
        FilePickerPage {
            nameFilters: ["*.zip"]
            onSelectedContentPropertiesChanged: {
                page.filePath = selectedContentProperties.filePath || ""
                page.fileName = selectedContentProperties.fileName || page.filePath.split("/").pop()
            }
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Data backup"
                title: qsTrId("threesea-he-data-backup")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.highlightColor
                //% "Encrypted backups of chats, contacts and settings, in the format of Threema for Android: they restore here and there."
                text: qsTrId("threesea-la-data-backup-description")
            }

            SectionHeader {
                //% "Import a backup"
                text: qsTrId("threesea-se-data-backup-import")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                //% "Checking a backup only reads the file and shows what it holds. The password never leaves this device."
                text: qsTrId("threesea-la-data-backup-import-description")
            }

            Item { width: 1; height: Theme.paddingMedium }

            ValueButton {
                //% "Backup file"
                label: qsTrId("threesea-la-android-backup-file")
                value: page.fileName.length > 0 ? page.fileName
                       //% "Select"
                       : qsTrId("threesea-la-select")
                onClicked: pageStack.push(filePicker)
            }

            PasswordField {
                id: password
                //% "Backup password"
                label: qsTrId("threesea-la-android-backup-password")
                placeholderText: label
                EnterKey.enabled: text.length > 0 && page.filePath.length > 0
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.check()
            }

            Item { width: 1; height: Theme.paddingLarge }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                //% "Check backup"
                text: qsTrId("threesea-bu-android-backup-check")
                visible: !page.working
                enabled: page.filePath.length > 0 && password.text.length > 0
                onClicked: page.check()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: page.working
                visible: running
            }

            ErrorLabel { id: importError }

            SectionHeader {
                //% "Create backups"
                text: qsTrId("threesea-se-android-backup-create")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                //% "Writes a backup into the chosen folder, now or on a schedule. The password is kept in the encrypted storage so automatic backups can run."
                text: qsTrId("threesea-la-data-backup-create-description")
            }

            Item { width: 1; height: Theme.paddingMedium }

            Component {
                id: folderPicker
                FolderPickerPage {
                    //% "Backup folder"
                    dialogTitle: qsTrId("threesea-la-android-export-folder")
                    onSelectedPathChanged: page.setExportFolder(selectedPath)
                }
            }

            ValueButton {
                label: qsTrId("threesea-la-android-export-folder")
                value: page.exportStatus.directory
                       ? page.exportStatus.directory.split("/").pop()
                       : qsTrId("threesea-la-select")
                onClicked: pageStack.push(folderPicker)
            }

            PasswordField {
                id: exportPassword
                label: qsTrId("threesea-la-android-backup-password")
                placeholderText: page.exportStatus.passwordSet
                                 //% "Password saved"
                                 ? qsTrId("threesea-la-android-export-password-saved")
                                 : qsTrId("threesea-la-android-backup-password")
            }

            ChoiceBox {
                //% "Automatic backup"
                label: qsTrId("threesea-la-android-export-auto")
                //% "Off"
                options: [qsTrId("threesea-la-android-export-off"),
                          //% "Daily"
                          qsTrId("threesea-la-android-export-daily"),
                          //% "Weekly"
                          qsTrId("threesea-la-android-export-weekly")]
                choice: page.exportStatus.interval === "daily" ? 1
                        : page.exportStatus.interval === "weekly" ? 2 : 0
                onChosen: backend.call("androidBackupSchedule", {
                    interval: index === 1 ? "daily" : index === 2 ? "weekly" : "off",
                    password: exportPassword.text,
                    pin: page.pin
                }, function(ok, result, kind, message) {
                    if (!ok) exportError.text = Errors.text(kind, message)
                    page.refreshExportStatus()
                })
            }

            ChoiceBox {
                //% "Keep"
                label: qsTrId("threesea-la-android-export-keep")
                //% "%n backup(s)"
                options: [qsTrId("threesea-la-n-backups", 1), qsTrId("threesea-la-n-backups", 3),
                          qsTrId("threesea-la-n-backups", 5), qsTrId("threesea-la-n-backups", 10)]
                choice: page.exportStatus.keep >= 10 ? 3 : page.exportStatus.keep >= 5 ? 2
                        : page.exportStatus.keep >= 3 ? 1 : 0
                onChosen: backend.call("androidBackupSchedule",
                                       { keep: [1, 3, 5, 10][index], pin: page.pin },
                                       function() { page.refreshExportStatus() })
            }

            Item { width: 1; height: Theme.paddingLarge }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                //% "Create backup now"
                text: qsTrId("threesea-bu-android-export-now")
                visible: !page.exportRunning
                enabled: (page.exportStatus.directory || "").length > 0
                         && (exportPassword.text.length > 0 || page.exportStatus.passwordSet === true)
                onClicked: {
                    exportError.text = ""
                    page.exportResult = ""
                    var args = { password: exportPassword.text, pin: page.pin }
                    backend.call("androidBackupExport", args, function(ok, result, kind, message) {
                        if (ok) {
                            page.exportRunning = true
                            // Saved alongside, so the schedule can reuse it.
                            if (exportPassword.text.length > 0) {
                                backend.call("androidBackupSchedule",
                                             { password: exportPassword.text, pin: page.pin },
                                             function() { page.refreshExportStatus() })
                            }
                        } else {
                            exportError.text = Errors.text(kind, message)
                        }
                    })
                }
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: page.exportRunning
                visible: running
            }

            ErrorLabel { id: exportError }

            Item { width: 1; height: Theme.paddingMedium }

            Label {
                visible: page.exportResult.length > 0
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                //% "Backup written: %1"
                text: qsTrId("threesea-la-android-export-written").arg(page.exportResult)
            }

            DetailItem {
                visible: !!page.exportStatus.lastBackup
                label: qsTrId("threesea-la-safe-last-backup")
                value: page.exportStatus.lastBackup
                       ? Fmt.day(Number(page.exportStatus.lastBackup)) + " "
                         + Fmt.clock(Number(page.exportStatus.lastBackup), Format, Formatter)
                       : ""
            }

            DetailItem {
                visible: !!page.exportStatus.lastError
                //% "Last attempt"
                label: qsTrId("threesea-la-safe-last-attempt")
                value: page.exportStatus.lastError
                       ? Errors.text(page.exportStatus.lastError, "") : ""
            }
        }

        VerticalScrollDecorator { }
    }
}
