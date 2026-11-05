// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property alias password: passwordField.text
    property string quality: ""   // tooShort | tooLong | weak | ok, from the core
    // Asked of a new ID (the chat list): going on without Threema Safe takes a choice of its own.
    property bool firstRun: false
    readonly property bool skip: firstRun && skipSwitch.checked

    canAccept: skip || ((quality === "ok" || quality === "weak") && repeatField.text === passwordField.text)

    Timer {
        id: checkLater
        interval: 300
        onTriggered: backend.call("safeCheckPassword", { password: passwordField.text },
                                  function(ok, result) { if (ok) dialog.quality = result.result })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: dialog.skip
                            //% "Continue"
                            ? qsTrId("threesea-he-continue")
                            //% "Enable"
                            : qsTrId("threesea-he-enable")
            }

            InfoLabel {
                //% "Threema Safe stores an encrypted backup of your ID, profile, contacts, groups and settings on Threema's servers, updated when something changes. For your chats, use Data backup. Only your password (at least 8 characters) unlocks it — if you forget it, the backup cannot be recovered."
                text: qsTrId("threesea-la-safe-password-intro-changes")
            }

            PasswordField {
                id: passwordField
                width: parent.width
                visible: !dialog.skip
                //% "Password"
                label: qsTrId("threesea-la-password")
                onTextChanged: { dialog.quality = ""; checkLater.restart() }
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: repeatField.focus = true
            }

            PasswordField {
                id: repeatField
                width: parent.width
                visible: !dialog.skip
                //% "Repeat password"
                label: qsTrId("threesea-la-repeat-password")
                errorHighlight: text.length > 0 && text !== passwordField.text
                EnterKey.enabled: dialog.canAccept
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }

            ErrorLabel {
                text: dialog.skip ? ""
                      : dialog.quality === "weak"
                      //% "This password is easy to guess. Choose a stronger one, or accept to use it anyway."
                      ? qsTrId("threesea-la-safe-password-weak")
                      : dialog.quality === "tooLong"
                        //% "The password is too long."
                        ? qsTrId("threesea-la-error-password-long")
                        : ""
            }

            TextSwitch {
                id: skipSwitch
                visible: dialog.firstRun
                //% "Continue without a backup"
                text: qsTrId("threesea-la-safe-setup-skip")
                //% "Your ID, contacts and groups then exist on this phone alone, and go with it. Settings, Backup can turn Threema Safe on or export an ID backup at any time."
                description: qsTrId("threesea-la-safe-setup-skip-description")
            }
        }

        VerticalScrollDecorator { }
    }
}
