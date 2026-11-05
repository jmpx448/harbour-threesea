// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Name a browser in Settings → Threema Web. The name stays on this phone; empty, the browser goes
// by what it says it is again.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All
    property alias name: field.text
    // What the browser is called without a name of ours.
    property string ownName

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width
            DialogHeader {
                acceptText: qsTrId("threesea-he-save")
                //% "Rename browser"
                title: qsTrId("threesea-he-rename-browser")
            }
            TextField {
                id: field
                inputMethodHints: window.incognitoHints
                width: parent.width
                focus: true
                //% "Browser name"
                label: qsTrId("threesea-la-browser-name")
                placeholderText: dialog.ownName
                //% "Only this phone shows this name. Left empty, the browser's own name is shown."
                description: qsTrId("threesea-la-browser-name-description")
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
