// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Pick group members from the contacts (multi-select, like Jolla's contact pickers): tap to
// select, the dialog header accepts. With `askName` (new groups) the name field comes first.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All
    canAccept: distributionList ? selected.length > 0 : (!askName || name.trim().length > 0)

    property bool askName: false
    // Labels for a distribution list instead of a group.
    property bool distributionList: false
    // Identities not to offer (e.g. current members).
    property var excluded: []
    property string name

    // Identities, in and out.
    property var selected: []
    property string acceptText
    property string title

    function isSelected(identity) {
        return selected.indexOf(identity) >= 0
    }

    function toggle(identity) {
        var list = selected.slice()
        var index = list.indexOf(identity)
        if (index >= 0) list.splice(index, 1)
        else list.push(identity)
        selected = list
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: backend.contacts

        header: Column {
            width: parent.width
            DialogHeader {
                acceptText: dialog.acceptText
                title: dialog.title
            }
            TextField {
                inputMethodHints: window.incognitoHints
                visible: dialog.askName
                width: parent.width
                focus: dialog.askName
                text: dialog.name
                onTextChanged: dialog.name = text
                label: dialog.distributionList
                       //% "List name"
                       ? qsTrId("threesea-la-list-name")
                       //% "Group name"
                       : qsTrId("threesea-la-group-name")
                placeholderText: label
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
            }
            SectionHeader {
                visible: dialog.askName
                text: dialog.distributionList
                      //% "Recipients"
                      ? qsTrId("threesea-he-recipients")
                      //% "Members"
                      : qsTrId("threesea-he-members")
            }
        }

        section {
            property: "section"
            delegate: SectionHeader { text: section }
        }

        // Like Jolla's top menu pickers (IconTextSwitch): the glowing switch and the name, the
        // contact's square picture at the right edge.
        delegate: TextSwitch {
            id: item
            readonly property bool hiddenHere: dialog.excluded.indexOf(model.identity) >= 0
            visible: !hiddenHere
            height: hiddenHere ? 0 : Math.max(implicitHeight, Theme.itemSizeMedium)
            automaticCheck: false
            checked: dialog.isSelected(model.identity)
            text: model.displayName
            _rightPadding: picture.width + Theme.paddingMedium
            onClicked: dialog.toggle(model.identity)

            Thumbnail {
                id: picture
                anchors.right: parent.right
                height: Theme.itemSizeMedium
                highlighted: item.down
                source: model.pictureVersion > 0
                        ? "image://threesea/contact/" + model.identity + "?" + model.pictureVersion : ""
            }
        }

        ViewPlaceholder {
            enabled: listView.count === 0
            //% "No contacts yet"
            text: qsTrId("threesea-la-no-contacts")
        }

        VerticalScrollDecorator { }
    }
}
