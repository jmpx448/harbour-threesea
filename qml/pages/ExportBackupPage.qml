// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

Page {
    id: page
    allowedOrientations: Orientation.All

    property bool working: false
    property string backup: ""

    function exportBackup() {
        working = true
        errorLabel.text = ""
        backend.call("exportIdentityBackup", { password: passwordField.text }, function(ok, result, kind, message) {
            working = false
            if (ok) {
                backup = result.backup
                passwordField.text = ""
                repeatField.text = ""
                Qt.inputMethod.hide()
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
                //% "Export ID backup"
                title: qsTrId("threesea-he-export-backup")
            }

            InfoLabel {
                //% "An ID backup lets you restore your Threema ID on another device. Keep it and its password safe: anyone with both can use your ID."
                text: qsTrId("threesea-la-export-intro")
            }

            PasswordField {
                id: passwordField
                width: parent.width
                visible: page.backup.length === 0
                enabled: !page.working
                //% "Password (at least 8 characters)"
                label: qsTrId("threesea-la-new-backup-password")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: repeatField.focus = true
            }

            PasswordField {
                id: repeatField
                width: parent.width
                visible: page.backup.length === 0
                enabled: !page.working
                //% "Repeat password"
                label: qsTrId("threesea-la-repeat-password")
                errorHighlight: text.length > 0 && text !== passwordField.text
                EnterKey.enabled: passwordField.text.length >= 8 && text === passwordField.text
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.exportBackup()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.backup.length === 0
                enabled: !page.working && passwordField.text.length >= 8 && repeatField.text === passwordField.text
                //% "Create backup"
                text: qsTrId("threesea-bt-create-backup")
                onClicked: page.exportBackup()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.working
                visible: running
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.backup.length > 0
                // Zero-width spaces after the dashes let lines break between groups only.
                text: page.backup.replace(/-/g, "-\u200b")
                wrapMode: Text.Wrap
                font.family: "monospace"
                color: Theme.highlightColor
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.backup.length > 0
                //% "Copy to clipboard"
                text: qsTrId("threesea-bt-copy")
                onClicked: Clipboard.text = page.backup
            }
        }

        VerticalScrollDecorator { }
    }
}
