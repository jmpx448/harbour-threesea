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

    function restore() {
        if (working || !identityField.acceptableInput || passwordField.text.length === 0) {
            return
        }
        working = true
        errorLabel.text = ""
        backend.call("safeRestore", { identity: identityField.text, password: passwordField.text },
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

        PullDownMenu {
            enabled: !page.working
            MenuItem {
                //% "Find my ID"
                text: qsTrId("threesea-me-safe-find-id")
                onClicked: {
                    var finder = pageStack.push(Qt.resolvedUrl("SafeFindIdPage.qml"))
                    finder.chosen.connect(function(identity) { identityField.text = identity })
                }
            }
        }

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Restore from Threema Safe"
                title: qsTrId("threesea-he-restore-safe")
            }

            InfoLabel {
                //% "Enter your Threema ID and your Threema Safe password. Your profile, contacts, groups and settings are restored with it. Chats come from a Data backup, which Settings, Backup can import afterwards."
                text: qsTrId("threesea-la-restore-safe-intro-data-backup")
            }

            TextField {
                id: identityField
                width: parent.width
                enabled: !page.working
                //% "Threema ID"
                label: qsTrId("threesea-la-threema-id")
                placeholderText: "ABCD1234"
                inputMethodHints: Qt.ImhUppercaseOnly | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                validator: RegExpValidator { regExp: /^\s*[A-Za-z0-9*][A-Za-z0-9]{7}\s*$/ }
                // Threema IDs are upper case; show them that way while typing.
                onTextChanged: {
                    var upper = text.toUpperCase()
                    if (upper !== text) {
                        var position = cursorPosition
                        text = upper
                        cursorPosition = position
                    }
                }
                EnterKey.enabled: acceptableInput
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: passwordField.focus = true
            }

            PasswordField {
                id: passwordField
                width: parent.width
                enabled: !page.working
                //% "Threema Safe password"
                label: qsTrId("threesea-la-safe-password")
                EnterKey.enabled: text.length > 0 && identityField.acceptableInput
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.restore()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                enabled: !page.working && identityField.acceptableInput && passwordField.text.length > 0
                //% "Restore"
                text: qsTrId("threesea-bt-restore")
                onClicked: page.restore()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.working
                visible: running
            }

            InfoLabel {
                visible: page.working
                horizontalAlignment: Text.AlignHCenter
                //% "Unlocking the backup takes a moment."
                text: qsTrId("threesea-la-safe-deriving")
            }
        }

        VerticalScrollDecorator { }
    }
}
