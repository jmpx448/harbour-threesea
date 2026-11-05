// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"
import "../../components/Format.js" as Fmt

// Settings → Security: the app lock, and the passphrase for the stored data (Android's
// "Encryption of locally stored data"), which are independent of each other (ADR 0009).
Page {
    id: page
    allowedOrientations: Orientation.All

    property bool working: false

    // The bridge (re-)encrypts the database key in Sailfish Secrets (or stores it plain again).
    function encrypt(on, passphrase, keyboard) {
        working = true
        backend.setKeyEncryption(on, passphrase, keyboard, function(ok) {
            page.working = false
            if (!ok) {
                //% "Could not change the encryption of the stored data."
                Notices.show(qsTrId("threesea-la-key-encryption-failed"), Notice.Long, Notice.Center)
            }
        })
    }

    // A new passphrase, entered twice; then encrypt with it.
    function choosePassphrase(replacing) {
        var setup = replacing
                ? pageStack.replace(Qt.resolvedUrl("../PinSetupPage.qml"), { minimumLength: 8, startKeyboard: true })
                : pageStack.push(Qt.resolvedUrl("../PinSetupPage.qml"), { minimumLength: 8, startKeyboard: true })
        setup.chosen.connect(function(passphrase, keyboard) {
            pageStack.pop(page)
            page.encrypt(true, passphrase, keyboard)
        })
    }

    // The current passphrase first, then `done`.
    function askPassphrase(done) {
        var check = pageStack.push(Qt.resolvedUrl("../UnlockPage.qml"), { passphraseCheck: true })
        check.unlocked.connect(function() { done() })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Security"
                title: qsTrId("threesea-se-security")
            }

            // Behind the PIN once one is set, like the device lock settings.
            ValueButton {
                //% "App lock"
                label: qsTrId("threesea-he-app-lock")
                value: Fmt.lockValue(window.lockInfo && window.lockInfo.lock)
                onClicked: {
                    if (!(window.lockInfo && window.lockInfo.lock)) {
                        pageStack.push(Qt.resolvedUrl("../AppLockPage.qml"))
                        return
                    }
                    var unlock = pageStack.push(Qt.resolvedUrl("../UnlockPage.qml"))
                    unlock.unlocked.connect(function(pin) {
                        pageStack.replace(Qt.resolvedUrl("../AppLockPage.qml"), { pin: pin })
                    })
                }
            }

            SectionHeader {
                //% "Encryption of stored data"
                text: qsTrId("threesea-se-data-encryption")
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.dataEncrypted
                busy: page.working
                enabled: !page.working
                //% "Passphrase"
                text: qsTrId("threesea-la-lock-passphrase")
                //% "Your messages are stored encrypted with a key that is kept encrypted with this passphrase. Threesea asks for it every time it starts, also after restarting the phone or deleting its data folder, and only this passphrase opens it. It receives messages once it is entered. If you forget it, everything stored on this phone is lost. Threema Safe or an ID backup keeps your ID, a Data backup your chats."
                description: qsTrId("threesea-la-data-passphrase-about")
                onClicked: {
                    if (checked) page.askPassphrase(function() {
                        pageStack.pop(page)
                        page.encrypt(false, "", false)
                    })
                    else page.choosePassphrase(false)
                }
            }

            SettingsButton {
                enabled: backend.dataEncrypted && !page.working
                //% "Change passphrase"
                text: qsTrId("threesea-la-change-passphrase")
                onClicked: page.askPassphrase(function() { page.choosePassphrase(true) })
            }
        }

        VerticalScrollDecorator { }
    }
}
