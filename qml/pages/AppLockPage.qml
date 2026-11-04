// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// App lock settings (ADR 0009), reached through the PIN like the device lock settings:
// `pin` is what was entered to get here (empty while there is no lock). Choices go
// through selection dialogs confirmed by swipe. (Encrypting the stored data has its own
// passphrase: Settings → Security.)
Page {
    id: page
    // The app locking closes this page and takes its PIN (window.closePinPage): with it, the
    // next person could set a PIN of their own.
    objectName: "holdsPin"
    allowedOrientations: Orientation.All

    property string pin
    property var status: ({})
    readonly property var lock: status.lock || null

    readonly property var graceTimes: [0, 30, 60, 120, 300, 600, 1800]
    readonly property var graceNames: [
        //% "Immediately"
        qsTrId("threesea-me-lock-immediately"),
        //% "After 30 seconds"
        qsTrId("threesea-me-lock-30-seconds"),
        //% "After 1 minute"
        qsTrId("threesea-me-lock-1-minute"),
        //% "After 2 minutes"
        qsTrId("threesea-me-lock-2-minutes"),
        //% "After 5 minutes"
        qsTrId("threesea-me-lock-5-minutes"),
        //% "After 10 minutes"
        qsTrId("threesea-me-lock-10-minutes"),
        //% "After 30 minutes"
        qsTrId("threesea-me-lock-30-minutes")
    ]

    function load() {
        backend.call("lockStatus", {}, function(ok, result) { if (ok) page.status = result })
    }

    function showError(kind, message) {
        Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
    }

    // Records the lock in the core. `options`: appLock, graceSecs, and a new `pin`/`keyboard`
    // (or `remove`).
    function save(options, done) {
        var args = options.remove ? { enabled: false } : {
            enabled: true,
            appLock: options.appLock === true,
            graceSecs: options.graceSecs || 0
        }
        if (options.pin) {
            args.pin = options.pin
            args.keyboard = options.keyboard === true
        }
        if (page.pin.length > 0) args.currentPin = page.pin
        backend.call("setLock", args, function(ok, result, kind, message) {
            if (ok) {
                page.status = result
                if (options.pin) page.pin = options.pin
                if (!result.lock) page.pin = ""
                window.lockChanged(result)
                if (done) done()
            } else {
                page.showError(kind, message)
            }
        })
    }

    function current() {
        return { appLock: lock ? lock.appLock : true, graceSecs: lock ? lock.graceSecs : 30 }
    }

    // A new PIN or passphrase (switching the lock on: the whole app locks after 30 s).
    function choosePin(setup) {
        setup = setup || pageStack.push(Qt.resolvedUrl("PinSetupPage.qml"))
        setup.chosen.connect(function(pin, keyboard) {
            pageStack.pop(page)
            var options = current()
            options.pin = pin
            options.keyboard = keyboard
            page.save(options)
        })
    }

    function removeLock() {
        save({ remove: true })
    }

    Component.onCompleted: load()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "App lock"
                title: qsTrId("threesea-he-app-lock")
            }

            ChoiceBox {
                options: [
                    //% "Off"
                    qsTrId("threesea-me-lock-off"),
                    //% "PIN or passphrase"
                    qsTrId("threesea-me-lock-pin")
                ]
                //% "App lock"
                label: qsTrId("threesea-he-app-lock")
                choice: page.lock ? 1 : 0
                valueText: !page.lock ? "" : page.lock.keyboard
                           //% "Passphrase"
                           ? qsTrId("threesea-la-lock-passphrase")
                           //% "PIN"
                           : qsTrId("threesea-la-lock-pin")
                onChosen: {
                    if (index === 1 && !page.lock) {
                        // Switching it on continues to choosing the PIN.
                        page.choosePin()
                    } else if (index === 0 && page.lock) {
                        //% "Removing the app lock"
                        Remorse.popupAction(page, qsTrId("threesea-la-removing-lock"), page.removeLock)
                    }
                }
            }

            ValueButton {
                visible: page.lock !== null
                label: page.lock && page.lock.keyboard
                       //% "Change passphrase"
                       ? qsTrId("threesea-la-change-passphrase")
                       //% "Change PIN"
                       : qsTrId("threesea-la-change-pin")
                onClicked: page.choosePin()
            }

            TextSwitch {
                visible: page.lock !== null
                automaticCheck: false
                checked: page.lock !== null && page.lock.appLock
                //% "Lock the app"
                text: qsTrId("threesea-la-lock-app")
                //% "Ask whenever Threesea is opened. Private chats always ask."
                description: qsTrId("threesea-la-lock-app-description")
                onClicked: {
                    var options = page.current()
                    options.appLock = !checked
                    page.save(options)
                }
            }

            ChoiceBox {
                visible: page.lock !== null && page.lock.appLock
                //% "Lock"
                label: qsTrId("threesea-la-lock-after")
                options: page.graceNames
                choice: page.lock ? Math.max(0, page.graceTimes.indexOf(page.lock.graceSecs)) : 0
                onChosen: {
                    var options = page.current()
                    options.graceSecs = page.graceTimes[index]
                    page.save(options)
                }
            }

            Item { width: 1; height: Theme.paddingLarge }

            InfoLabel {
                //% "With a lock you can also make chats private: they are hidden from the chat list and are opened from its bottom menu. If you forget the app lock's PIN or passphrase, the way back in is to delete Threesea's data folder .local/share/org.threesea/harbour-threesea with the chats and media in it, restart the phone and restore from a backup. A passphrase for the stored data stays set: Threesea asks for it first."
                text: qsTrId("threesea-la-app-lock-about")
                font.pixelSize: Theme.fontSizeSmall
            }
        }

        VerticalScrollDecorator {}
    }
}
