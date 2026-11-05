// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
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

    // How many contacts there are at all, and whether we know yet.
    property int contactCount
    property bool loaded

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

    // The contacts to offer: with the excluded ones gone from the model, the view has no section
    // for a letter that has nobody left under it.
    function load() {
        backend.call("contacts", {}, function(ok, result) {
            if (!ok) return
            offered.update(result.contacts.filter(function(contact) {
                return dialog.excluded.indexOf(contact.identity) < 0
            }))
            dialog.contactCount = result.contacts.length
            dialog.loaded = true
        })
    }

    JsonListModel {
        id: offered
        keyField: "identity"
        roles: ["identity", "displayName", "firstName", "lastName", "nickname", "section", "pictureVersion"]
    }

    // What the search field narrows the offer to. Someone selected stays selected, seen or not.
    property string query
    ListFilter {
        id: found
        sourceModel: offered
        query: dialog.query
    }

    Component.onCompleted: load()

    Connections {
        target: backend
        onCoreEvent: if (name === "contactsChanged") dialog.load()
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: found
        // Or the first row becomes the current item and takes the focus from the name field.
        currentIndex: -1

        StayAtTop { target: listView }

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
            SearchField {
                width: parent.width
                placeholderText: qsTrId("threesea-la-search-contacts")
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

        // Like Jolla's top menu pickers (IconTextSwitch): the glowing switch and the name, the
        // contact's square picture at the right edge.
        delegate: TextSwitch {
            id: item
            height: Math.max(implicitHeight, Theme.itemSizeMedium)
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
            enabled: dialog.loaded && offered.count === 0
            text: dialog.contactCount === 0
                  //% "No contacts"
                  ? qsTrId("threesea-la-no-contacts")
                  : dialog.distributionList
                    //% "All your contacts are on this list"
                    ? qsTrId("threesea-la-all-contacts-on-list")
                    //% "All your contacts are in this group"
                    : qsTrId("threesea-la-all-contacts-in-group")
        }
        ViewPlaceholder {
            enabled: offered.count > 0 && found.count === 0
            text: qsTrId("threesea-la-search-no-contacts")
        }

        VerticalScrollDecorator { }
    }
}
