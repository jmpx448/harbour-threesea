// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A line of text to put on a photo (EditPhotoPage).
Dialog {
    id: dialog
    allowedOrientations: Orientation.All
    property alias text: field.text
    canAccept: field.text.trim().length > 0

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width
            DialogHeader {
                //% "Add"
                acceptText: qsTrId("threesea-he-add")
                //% "Text on the photo"
                title: qsTrId("threesea-he-photo-text")
            }
            TextField {
                inputMethodHints: window.incognitoHints
                id: field
                width: parent.width
                focus: true
                //% "Text"
                label: qsTrId("threesea-la-photo-text")
                placeholderText: label
                EnterKey.enabled: text.trim().length > 0
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
