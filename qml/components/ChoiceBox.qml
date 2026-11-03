// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A single choice like Jolla Settings (Display → Orientation, Blank display after): Silica's
// ComboBox, applied at once; Silica shows few options inline and many on a page of their own.
// `options`: texts; `choice`: the current index; `chosen(index)` when the user picks one (also
// the current one: it may lead somewhere, e.g. editing the selected contacts).
// `valueText` replaces the shown value (e.g. "Until 15:30").
ComboBox {
    id: root

    property var options: []
    property int choice: 0
    property string valueText

    signal chosen(int index)

    // Always the real setting: a pick shows once it is applied (it may continue elsewhere, e.g.
    // setting up a PIN, and be cancelled there).
    onChoiceChanged: currentIndex = choice
    Component.onCompleted: currentIndex = choice
    on_MenuOpenChanged: if (!_menuOpen) currentIndex = choice
    value: valueText.length > 0 ? valueText : (options[choice] || "")

    menu: ContextMenu {
        Repeater {
            model: root.options
            MenuItem {
                text: modelData
                onClicked: root.chosen(index)
            }
        }
    }
}
