// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// A new PIN or passphrase (keyboard button top left), entered twice like the device lock's
// "Enter new security code". `chosen(pin, keyboard)` once both match; the caller saves it.
// `minimumLength`: 8 while the stored data is encrypted with it.
Page {
    id: page
    allowedOrientations: Orientation.Portrait
    backNavigation: false
    forwardNavigation: false

    property int minimumLength: 4
    // Start on the keyboard (a passphrase) rather than the keypad.
    property bool startKeyboard: false
    property string first
    property bool firstKeyboard

    signal chosen(string pin, bool keyboard)

    onStatusChanged: if (status === PageStatus.Active) entry.focusField()

    PinEntry {
        id: entry
        anchors.fill: parent
        allowKeyboard: page.first.length === 0
        Component.onCompleted: keyboard = page.startKeyboard
        cancellable: true
        minimumLength: page.minimumLength
        title: page.first.length === 0
               ? (keyboard
                  //% "Enter new passphrase"
                  ? qsTrId("threesea-he-new-passphrase")
                  //% "Enter new PIN"
                  : qsTrId("threesea-he-new-pin"))
               : (keyboard
                  //% "Enter the passphrase again"
                  ? qsTrId("threesea-he-repeat-passphrase")
                  //% "Enter the PIN again"
                  : qsTrId("threesea-he-repeat-pin"))
        errorText: page.first.length > 0 ? "" : page.minimumLength > 4
                   //% "At least %1 characters"
                   ? qsTrId("threesea-la-passphrase-hint").arg(page.minimumLength)
                   : keyboard
                     ? qsTrId("threesea-la-passphrase-hint").arg(page.minimumLength)
                     //% "At least 4 digits"
                     : qsTrId("threesea-la-pin-hint")
        onCanceled: pageStack.pop()
        onEntered: {
            if (page.first.length === 0) {
                page.first = pin
                page.firstKeyboard = keyboard
            } else if (pin === page.first) {
                page.chosen(pin, keyboard)
            } else {
                page.first = ""
                //% "They did not match. Try again."
                errorText = qsTrId("threesea-la-pin-mismatch")
            }
            clear()
        }
    }
}
