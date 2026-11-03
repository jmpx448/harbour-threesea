// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A multi-select dialog (Silica's glowing switches, accepted by swipe): which kinds the media
// gallery shows, or which download by themselves. `kinds`/`names`: the choices; `selected`:
// kinds in and out; `allowNone`: accepting with nothing chosen.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property var kinds: []
    property var names: []
    property var selected: []
    //% "Show"
    property string title: qsTrId("threesea-me-media-show")
    property bool allowNone: false

    canAccept: allowNone || kinds.some(function(k) { return selected.indexOf(k) >= 0 })

    function toggle(kind) {
        var list = selected.slice()
        var index = list.indexOf(kind)
        if (index >= 0) list.splice(index, 1)
        else list.push(kind)
        selected = list
    }

    SilicaListView {
        anchors.fill: parent
        model: dialog.kinds

        header: DialogHeader {
            title: dialog.title
        }

        delegate: TextSwitch {
            automaticCheck: false
            checked: dialog.selected.indexOf(modelData) >= 0
            text: dialog.names[index]
            onClicked: dialog.toggle(modelData)
        }

        VerticalScrollDecorator { }
    }
}
