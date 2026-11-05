// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

Page {
    id: page
    allowedOrientations: Orientation.All
    // The core finishes the restore anyway, and meanwhile the page below offers a new ID.
    backNavigation: !working

    property bool working: false
    // A code was scanned: on to the password when back here.
    property bool passwordNext: false
    onStatusChanged: {
        if (status === PageStatus.Active && passwordNext) {
            passwordNext = false
            passwordField.forceActiveFocus()
        }
    }

    function scan() {
        var scanner = pageStack.push(Qt.resolvedUrl("ScanQrPage.qml"), { idBackup: true })
        scanner.idBackupScanned.connect(function(text) {
            backupField.text = text
            page.passwordNext = true
        })
    }

    function restore() {
        working = true
        errorLabel.text = ""
        backend.call("restoreIdentityBackup", { backup: backupField.text, password: passwordField.text },
                     function(ok, result, kind, message) {
            working = false
            if (ok) {
                backend.refresh()
            } else {
                errorLabel.text = Errors.text(kind, message)
            }
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Restore ID backup"
                title: qsTrId("threesea-he-restore-backup")
            }

            InfoLabel {
                //% "Scan the QR code of the ID backup or paste its text (80 characters in groups of four, like XXXX-XXXX-…), and enter the password you chose when you created it. Threema for Android calls it an ID export. It brings your ID. Chats come from a Data backup, which Settings, Backup can import afterwards."
                text: qsTrId("threesea-la-restore-intro-scan")
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                enabled: !page.working
                text: qsTrId("threesea-he-scan-qr")
                onClicked: page.scan()
            }

            TextArea {
                id: backupField
                width: parent.width
                enabled: !page.working
                //% "ID backup"
                label: qsTrId("threesea-la-id-backup")
                placeholderText: "XXXX-XXXX-XXXX-…"
                inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                font.family: "monospace"
            }

            PasswordField {
                id: passwordField
                width: parent.width
                enabled: !page.working
                //% "Backup password"
                label: qsTrId("threesea-la-backup-password")
                EnterKey.enabled: text.length > 0 && backupField.text.length > 0
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.restore()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                enabled: !page.working && passwordField.text.length > 0 && backupField.text.length > 0
                //% "Restore"
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
