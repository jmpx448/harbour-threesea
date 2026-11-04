// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Code entry like Sailfish's device lock: the title in the upper part, Silica's keypad (with
// letters) at the bottom, the dots right-aligned above it with backspace next to them, cancel
// bottom left and accept bottom right once the code is long enough. The keyboard button top left
// switches to a passphrase field (`allowKeyboard`), like the device lock's; the keypad and its
// cancel are hidden then, so cancel moves to the top right, above the keyboard.
// `entered(pin, keyboard)` on accept; `clear()` after a wrong code. Fills its parent.
Item {
    id: root

    property string title
    // Shown in place of the dots while nothing is entered (hints, "Wrong PIN").
    property string errorText
    property int minimumLength: 4
    property bool keyboard: false
    property bool allowKeyboard: false
    property bool cancellable: false
    property bool busy: false
    readonly property string pin: _pin
    property string _pin

    signal entered(string pin, bool keyboard)
    signal canceled()

    function clear() {
        _pin = ""
        field.text = ""
    }

    function accept() {
        var value = keyboard ? field.text : _pin
        if (!busy && value.length >= minimumLength) entered(value, keyboard)
    }

    onKeyboardChanged: {
        clear()
        if (keyboard) field.forceActiveFocus()
    }
    // The page calls it once it is shown (created in the background, focus would not stick).
    function focusField() {
        if (keyboard) focusLater.restart()
    }
    // Right after the window is shown the keyboard doesn't open by itself for a field that
    // already has focus: ask for it once the window has settled.
    Timer {
        id: focusLater
        interval: 300
        onTriggered: {
            if (!root.keyboard || Qt.application.state !== Qt.ApplicationActive) return
            field.forceActiveFocus()
            Qt.inputMethod.show()
        }
    }
    Connections {
        target: Qt.application
        onStateChanged: if (Qt.application.state === Qt.ApplicationActive) root.focusField()
    }

    CutoutInset { id: cutout }

    IconButton {
        visible: root.allowKeyboard && !root.busy
        x: Theme.horizontalPageMargin
        y: cutout.inset + Theme.paddingLarge
        icon.source: root.keyboard ? "image://theme/icon-m-dialpad" : "image://theme/icon-m-keyboard"
        onClicked: root.keyboard = !root.keyboard
    }
    IconButton {
        visible: root.cancellable && root.keyboard && !root.busy
        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
        }
        y: cutout.inset + Theme.paddingLarge
        icon.source: "image://theme/icon-m-cancel"
        onClicked: root.canceled()
    }

    Label {
        id: titleLabel
        x: Theme.horizontalPageMargin
        y: cutout.inset + Theme.itemSizeLarge + Theme.paddingLarge
        width: parent.width - 2 * x
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: root.title
        color: Theme.highlightColor
        font.pixelSize: Theme.fontSizeExtraLarge
    }

    // Passphrase on the keyboard.
    PasswordField {
        id: field
        visible: root.keyboard
        anchors {
            top: titleLabel.bottom
            topMargin: Theme.itemSizeLarge
        }
        width: parent.width
        enabled: !root.busy
        label: root.errorText
        placeholderText: root.errorText
        EnterKey.enabled: text.length >= root.minimumLength
        EnterKey.iconSource: "image://theme/icon-m-enter-accept"
        EnterKey.onClicked: root.accept()
    }

    // Digits: the dots (or the hint) right-aligned above the keypad, backspace next to them.
    Label {
        visible: !root.keyboard && root._pin.length === 0
        anchors {
            bottom: keypad.top
            bottomMargin: Theme.itemSizeMedium
        }
        x: Theme.horizontalPageMargin
        width: parent.width - 2 * x
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: root.errorText
        color: Theme.secondaryHighlightColor
        font.pixelSize: Theme.fontSizeMedium
    }
    Label {
        visible: !root.keyboard && root._pin.length > 0
        anchors {
            right: backspace.left
            rightMargin: Theme.paddingLarge
            verticalCenter: backspace.verticalCenter
        }
        text: new Array(root._pin.length + 1).join("●")
        color: Theme.highlightColor
        font.pixelSize: Theme.fontSizeMedium
        font.letterSpacing: Theme.paddingSmall / 2
    }
    IconButton {
        id: backspace
        visible: !root.keyboard && root._pin.length > 0
        anchors {
            bottom: keypad.top
            bottomMargin: Theme.itemSizeMedium - (height - Theme.itemSizeExtraSmall) / 2
        }
        x: keypad.x + keypad._horizontalPadding + 2 * (keypad._buttonWidth + keypad._horizontalSpacing)
           + (keypad._buttonWidth - width) / 2
        icon.source: "image://theme/icon-m-backspace-keypad"
        onClicked: root._pin = root._pin.slice(0, -1)
        onPressAndHold: root._pin = ""
    }

    BusyIndicator {
        anchors {
            bottom: keypad.top
            bottomMargin: Theme.itemSizeMedium
            horizontalCenter: parent.horizontalCenter
        }
        size: BusyIndicatorSize.Medium
        running: root.busy
    }

    Keypad {
        id: keypad
        visible: !root.keyboard
        enabled: !root.busy
        anchors {
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        symbolsVisible: false
        onClicked: if (root._pin.length < 16) root._pin += number

        // The empty "*" and "#" slots of the bottom row: cancel and accept.
        IconButton {
            visible: root.cancellable
            x: keypad._horizontalPadding
            y: keypad.height - height
            width: keypad._buttonWidth
            height: keypad._buttonHeight
            icon.source: "image://theme/icon-m-cancel"
            onClicked: root.canceled()
        }
        IconButton {
            visible: root._pin.length >= root.minimumLength
            x: keypad._horizontalPadding + 2 * (keypad._buttonWidth + keypad._horizontalSpacing)
            y: keypad.height - height
            width: keypad._buttonWidth
            height: keypad._buttonHeight
            icon.source: "image://theme/icon-m-accept"
            onClicked: root.accept()
        }
    }
}
