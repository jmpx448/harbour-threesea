// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// The name of a group or, with `distributionList`, of a distribution list.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All
    property alias name: field.text
    property bool distributionList: false
    canAccept: field.text.trim().length > 0

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width
            DialogHeader {
                //% "Save"
                acceptText: qsTrId("threesea-he-save")
                title: dialog.distributionList
                       //% "Rename list"
                       ? qsTrId("threesea-he-rename-list")
                       //% "Rename group"
                       : qsTrId("threesea-he-rename-group")
            }
            TextField {
                inputMethodHints: window.incognitoHints
                id: field
                width: parent.width
                focus: true
                label: dialog.distributionList ? qsTrId("threesea-la-list-name") : qsTrId("threesea-la-group-name")
                placeholderText: label
                EnterKey.enabled: text.trim().length > 0
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
