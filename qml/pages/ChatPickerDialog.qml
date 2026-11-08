// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components"

// Pick several chats to send something to: recent chats first, then the contacts without one, like
// the share page; tap to select, the dialog header accepts.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All
    canAccept: selected.length > 0

    // Identities, in and out.
    property var selected: []
    // Identity → name of everything offered.
    property var names: ({})

    function toggle(identity) {
        var list = selected.slice()
        var index = list.indexOf(identity)
        if (index >= 0) list.splice(index, 1)
        else list.push(identity)
        selected = list
    }

    function selectedNames() {
        return selected.map(function(identity) { return names[identity] || identity })
    }

    JsonListModel {
        id: offered
        keyField: "identity"
        roles: ["identity", "displayName", "group", "pictureVersion", "section"]
    }

    Component.onCompleted: backend.call("conversations", {}, function(ok, result) {
        if (!ok) return
        var list = []
        var known = {}
        function add(row, section) {
            known[row.identity] = row.displayName
            list.push({ identity: row.identity, displayName: row.displayName, group: row.group === true,
                        pictureVersion: row.pictureVersion, section: section })
        }
        var chats = qsTrId("threesea-he-chats")
        result.conversations.forEach(function(chat) { if (!chat.left) add(chat, chats) })
        offered.update(list)
        backend.call("contacts", {}, function(ok, result) {
            if (ok) {
                var contacts = qsTrId("threesea-he-contacts")
                result.contacts.forEach(function(contact) {
                    if (known[contact.identity] === undefined) add(contact, contacts)
                })
                offered.update(list)
            }
            dialog.names = known
        })
        dialog.names = known
    })

    // What the search field narrows the offer to. Something selected stays selected, seen or not.
    property string query
    ListFilter {
        id: found
        sourceModel: offered
        query: dialog.query
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: found
        // Or the first row becomes the current item and takes the focus from the search field.
        currentIndex: -1

        StayAtTop { target: listView }

        header: Column {
            width: parent.width
            DialogHeader {
                //% "Choose"
                acceptText: qsTrId("threesea-he-choose-chats-accept")
                title: qsTrId("threesea-he-chats")
            }
            SearchField {
                width: parent.width
                placeholderText: qsTrId("threesea-la-search-chats-contacts")
                inputMethodHints: Qt.ImhNoAutoUppercase | window.incognitoHints
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
                onTextChanged: dialog.query = text
            }
        }

        section {
            property: "section"
            delegate: SectionHeader { text: section }
        }

        // Like the member picker: the glowing switch and the name, the square picture at the right edge.
        delegate: TextSwitch {
            id: item
            height: Math.max(implicitHeight, Theme.itemSizeMedium)
            automaticCheck: false
            checked: dialog.selected.indexOf(model.identity) >= 0
            text: model.displayName
            _rightPadding: picture.width + Theme.paddingMedium
            onClicked: dialog.toggle(model.identity)

            Thumbnail {
                id: picture
                anchors.right: parent.right
                height: Theme.itemSizeMedium
                group: model.group === true
                highlighted: item.down
                source: !model.pictureVersion ? ""
                        : (model.group ? "image://threesea/group/" : "image://threesea/contact/")
                          + model.identity + "?" + model.pictureVersion
            }
        }

        ViewPlaceholder {
            enabled: offered.count > 0 && found.count === 0
            text: qsTrId("threesea-la-search-no-chats-contacts")
        }

        VerticalScrollDecorator { }
    }
}
