// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// Asks for the app lock PIN or passphrase (ADR 0009). The app lock can't be dismissed
// (`cancellable: false`); before the private chats or the lock settings, the X leaves (PinEntry
// has it on the keypad and, for a passphrase, above the keyboard).
// `unlocked(pin)` once the core accepted it; with `privateChats` the core then shows the private
// chats, without it this only checks (the PIN then goes to a command that needs it). With `database` it opens the encrypted database key instead (state
// "locked", before the core runs).
Page {
    id: page
    objectName: "unlock"
    allowedOrientations: Orientation.Portrait

    property bool cancellable: true
    property bool privateChats: false
    property bool database: false
    // Only check the stored data's passphrase (Settings → Security), changing nothing.
    property bool passphraseCheck: false
    property string title

    signal unlocked(string pin)

    // Like the system's code entry: no page navigation (its corner would be the keyboard button);
    // the X cancels.
    backNavigation: false
    forwardNavigation: false

    readonly property bool keyboard: database || passphraseCheck ? backend.passphraseKeyboard
                                              : !!(window.lockInfo && window.lockInfo.lock && window.lockInfo.lock.keyboard)

    onStatusChanged: {
        if (status !== PageStatus.Active) return
        entry.focusField()
        // A private chat reached from a notification, with no lock set up yet: say so at once.
        if (privateChats && !(window.lockInfo && window.lockInfo.lock))
            entry.errorText = Errors.text("lockNeeded", "")
    }

    // The right passphrase, and the data it opens are being brought up to date first.
    Connections {
        target: backend
        onUpgradingChanged: {
            if (page.database && backend.upgrading) entry.errorText = Errors.upgradingHint()
        }
    }

    // Too many wrong PINs: the core checks none until then (like Android, counting down).
    property double pausedUntil: 0
    function showPause() {
        var seconds = Math.ceil((pausedUntil - Date.now()) / 1000)
        //% "Too many wrong attempts. Try again in %n second(s)."
        entry.errorText = seconds > 0 ? qsTrId("threesea-la-pin-paused", seconds) : ""
        if (seconds <= 0) pauseTimer.stop()
    }
    Timer {
        id: pauseTimer
        interval: 1000
        repeat: true
        onTriggered: page.showPause()
    }

    // The check takes a moment. If the app locked meanwhile, the lock page is on top and what
    // was to follow must not replace it; this page, empty again, asks once the lock is gone.
    // The core has shown the private chats by then, and now nothing that wanted them follows.
    function accepted(pin) {
        if (database || pageStack.currentPage === page) unlocked(pin)
        else if (privateChats) backend.lockWithFilesOnTheirWay()
    }

    // A forgotten passphrase: the stored data can't be opened, and starting without it is the
    // way on (the bridge's startFresh, behind StartFreshDialog and a remorse timer).
    function askStartFresh() {
        var dialog = pageStack.push(Qt.resolvedUrl("StartFreshDialog.qml"))
        dialog.accepted.connect(function() {
            //% "Deleting the stored data"
            Remorse.popupAction(page, qsTrId("threesea-la-starting-fresh"), function() { backend.startFresh() })
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentWidth: width
        contentHeight: height
        // Only the stored data's prompt has a menu; the others stay still under the finger.
        interactive: page.database

        PullDownMenu {
            objectName: "startFreshMenu"
            visible: page.database && !entry.busy
            MenuItem {
                objectName: "startFresh"
                //% "Start fresh"
                text: qsTrId("threesea-me-start-fresh")
                onClicked: page.askStartFresh()
            }
        }

        PinEntry {
            id: entry
            anchors.fill: parent
            keyboard: page.keyboard
            cancellable: page.cancellable
            //% "Forgotten it? Pull down to start fresh without the stored data."
            note: page.database ? qsTrId("threesea-la-data-passphrase-forgotten") : ""
            // By what the code is for: the keypad or the keyboard is only how it's entered.
            title: page.title.length > 0 ? page.title
                   : page.database || page.passphraseCheck
                     //% "Enter the passphrase of your stored data"
                     ? qsTrId("threesea-he-enter-data-passphrase")
                   : page.keyboard
                     //% "Enter your passphrase"
                     ? qsTrId("threesea-he-enter-passphrase")
                     //% "Enter your PIN"
                     : qsTrId("threesea-he-enter-pin")
            onCanceled: pageStack.pop()
            onEntered: {
                if (page.database || page.passphraseCheck) {
                    busy = true
                    var check = page.database ? backend.unlockDatabase : backend.checkPassphrase
                    check(pin, function(ok) {
                        // The right passphrase: opening the data now, the state's page replaces this one.
                        entry.busy = ok && page.database
                        entry.clear()
                        if (ok) {
                            page.accepted(pin)
                        } else {
                            // The busy field gave up its focus, and the keyboard with it.
                            entry.focusField()
                            //% "Wrong passphrase"
                            entry.errorText = qsTrId("threesea-la-wrong-passphrase")
                        }
                    })
                    return
                }
                // Until the reply: a second tap would be a second attempt.
                busy = true
                backend.call("unlock", { pin: pin, privateChats: page.privateChats },
                             function(ok, result, kind, message) {
                    entry.busy = false
                    entry.clear()
                    if (ok) {
                        page.accepted(pin)
                        return
                    }
                    entry.focusField()
                    if (kind === "pinLockout") {
                        page.pausedUntil = Date.now() + parseInt(message, 10) * 1000
                        page.showPause()
                        pauseTimer.restart()
                    } else if (kind === "wrongPin") {
                        entry.errorText = page.keyboard ? qsTrId("threesea-la-wrong-passphrase")
                                                        //% "Wrong PIN"
                                                        : qsTrId("threesea-la-wrong-pin")
                    } else {
                        entry.errorText = Errors.text(kind, message)
                    }
                })
            }
        }
    }
}
