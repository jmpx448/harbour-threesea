// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "../components"
import "../components/Errors.js" as Errors

// The first start, with a Data backup: the ID comes out of it, with contacts, groups and pictures.
// The state then puts the chat list here, and the window continues the import over it with the
// messages and media, ending on the usual results page (pendingDataImport).
Page {
    id: page
    allowedOrientations: Orientation.All
    // The core finishes the restore anyway, and meanwhile the page below offers a new ID.
    backNavigation: !working

    property string filePath
    property string fileName
    property bool working: false

    readonly property bool canRestore: !working && filePath.length > 0 && password.text.length > 0

    Component.onDestruction: password.text = ""

    function restore() {
        if (!canRestore) return
        working = true
        errorLabel.text = ""
        backend.call("androidBackupRestore", { path: filePath, password: password.text },
                     function(ok, result, kind, message) {
            page.working = false
            if (ok) {
                window.pendingDataImport = {
                    filePath: page.filePath,
                    password: password.text,
                    imported: result.imported
                }
                backend.refresh()
            } else {
                errorLabel.text = Errors.text(kind, message)
            }
        })
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
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Restore from Data backup"
                title: qsTrId("threesea-he-restore-data-backup")
            }

            InfoLabel {
                //% "Choose a Data backup from Threema for Android or Threesea and enter its password. It brings your ID, contacts, groups, chats and media. The password stays on this device."
                text: qsTrId("threesea-la-restore-data-backup-intro")
            }

            ValueButton {
                enabled: !page.working
                label: qsTrId("threesea-la-android-backup-file")
                value: page.fileName.length > 0 ? page.fileName : qsTrId("threesea-la-select")
                onClicked: pageStack.push(filePicker)
            }

            PasswordField {
                id: password
                width: parent.width
                enabled: !page.working
                label: qsTrId("threesea-la-android-backup-password")
                placeholderText: label
                EnterKey.enabled: page.canRestore
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.restore()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                enabled: page.canRestore
                text: qsTrId("threesea-bt-restore")
                onClicked: page.restore()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.working
                visible: running
            }
        }

        VerticalScrollDecorator { }
    }
}
