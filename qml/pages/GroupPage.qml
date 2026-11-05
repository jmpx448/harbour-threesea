// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import harbour.threesea 1.0
import "../components"
import "../components/Errors.js" as Errors

// Group details, attached to the group's chat (swipe left): members; the creator can rename and
// change members; everyone can leave (the creator dissolves the group instead).
Page {
    id: page
    allowedOrientations: Orientation.All

    property string identity
    property var group: ({ members: [] })

    // Our own picture, for our row among the members (a millisecond timestamp: not an int).
    property real myPictureVersion: 0

    function load() {
        backend.call("group", { key: identity }, function(ok, result) {
            if (ok) page.group = result
        })
        backend.call("profile", {}, function(ok, result) {
            if (ok) page.myPictureVersion = result.pictureVersion || 0
        })
    }

    function run(command, args) {
        args.key = identity
        backend.call(command, args, function(ok, result, kind, message) {
            if (ok) page.group = result
            else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
        })
    }

    function addMembers() {
        var current = memberIdentities()
        var dialog = pageStack.push(Qt.resolvedUrl("MemberPickerDialog.qml"), {
            //% "Add"
            acceptText: qsTrId("threesea-he-add"),
            //% "Add members"
            title: qsTrId("threesea-he-add-members"),
            excluded: current
        })
        dialog.accepted.connect(function() {
            if (dialog.selected.length > 0) {
                page.run("setGroupMembers", { members: current.concat(dialog.selected) })
            }
        })
    }

    // Here, not in the row: its scope is gone when the remorse runs out.
    function removeMember(item, identity, name) {
        //% "Removing %1"
        item.remorseAction(qsTrId("threesea-la-removing-member").arg(name), function() {
            var ids = page.memberIdentities()
            ids.splice(ids.indexOf(identity), 1)
            page.run("setGroupMembers", { members: ids })
        })
    }

    function memberIdentities() {
        var ids = []
        for (var i = 0; i < group.members.length; ++i) {
            if (!group.members[i].creator) ids.push(group.members[i].identity)
        }
        return ids
    }

    // Keyed, so a reload keeps the rows, and a running remorse with them.
    JsonListModel {
        id: members
        keyField: "identity"
        roles: ["identity", "displayName", "creator", "pictureVersion"]
    }
    onGroupChanged: members.update(group.members)

    Component.onCompleted: load()

    Connections {
        target: backend
        onCoreEvent: if (name === "groupsChanged" || name === "contactsChanged") page.load()
    }

    RemorsePopup { id: remorse }

    Component {
        id: picturePicker
        ImagePickerPage {
            onSelectedContentPropertiesChanged: {
                backend.setGroupPicture(page.identity, selectedContentProperties.filePath,
                                        function(ok, result, kind, message) {
                    if (ok) page.group = result
                    else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                })
            }
        }
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: members

        StayAtTop { target: listView }

        PullDownMenu {
            MenuItem {
                visible: page.group.left === true
                //% "Delete group"
                text: qsTrId("threesea-me-delete-group")
                //% "Deleting group"
                onClicked: remorse.execute(qsTrId("threesea-la-deleting-group"), function() {
                    backend.call("deleteConversation", { identity: page.identity }, function() {
                        pageStack.pop(pageStack.find(function(p) { return p.objectName === "conversations" }))
                    })
                })
            }
            MenuItem {
                visible: page.group.left === false
                text: page.group.isCreator
                      //% "Dissolve group"
                      ? qsTrId("threesea-me-dissolve-group")
                      //% "Leave group"
                      : qsTrId("threesea-me-leave-group")
                onClicked: remorse.execute(page.group.isCreator
                                           //% "Dissolving group"
                                           ? qsTrId("threesea-la-dissolving-group")
                                           //% "Leaving group"
                                           : qsTrId("threesea-la-leaving-group"),
                                           function() { page.run("leaveGroup", {}) })
            }
            MenuItem {
                visible: page.group.isCreator === true && !page.group.left && page.group.pictureVersion > 0
                //% "Remove picture"
                text: qsTrId("threesea-me-remove-picture")
                onClicked: remorse.execute(qsTrId("threesea-la-removing-picture"), function() {
                    page.run("removeGroupPicture", {})
                })
            }
            MenuItem {
                visible: page.group.isCreator === true && !page.group.left
                //% "Set picture"
                text: qsTrId("threesea-me-set-picture")
                onClicked: pageStack.push(picturePicker)
            }
            MenuItem {
                visible: page.group.isCreator === true && !page.group.left
                //% "Rename"
                text: qsTrId("threesea-me-rename")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("GroupNameDialog.qml"), { name: page.group.name })
                    dialog.accepted.connect(function() { page.run("renameGroup", { name: dialog.name }) })
                }
            }
        }

        header: Column {
            width: parent.width
            ProfileHeader {
                pinned: page.group.pinned === true
                onPinToggled: backend.call("setPinned", { identity: page.identity, pinned: !pinned },
                                           function() { page.load() })
                group: true
                //% "Group"
                title: page.group.title || qsTrId("threesea-he-group")
                description: page.group.left
                             //% "You are no longer in this group"
                             ? qsTrId("threesea-la-group-left-input")
                             //% "%n member(s)"
                             : qsTrId("threesea-la-member-count", page.group.members.length + 1)
                source: page.group.pictureVersion > 0
                        ? "image://threesea/group/" + page.identity + "?" + page.group.pictureVersion : ""
            }
            SectionHeader {
                text: qsTrId("threesea-se-chat")
            }
            MediaButton {
                identity: page.identity
                name: page.group.title || ""
            }
            ExportChatButton {
                identity: page.identity
                name: page.group.title || ""
            }
            ChatNotificationsButton {
                visible: !page.group.left
                identity: page.identity
                group: true
                settings: page.group.notifications || ({})
            }
            // Device-local, like the tone of all groups.
            ToneButton {
                visible: !page.group.left
                property int revision: 0
                tone: revision >= 0 ? backend.chatTone(page.identity) : ""
                //% "Same as all groups"
                defaultText: qsTrId("threesea-la-tone-all-groups")
                onChosen: {
                    backend.setChatTone(page.identity, tone)
                    revision++
                }
            }
            SectionHeader {
                //% "Members"
                text: qsTrId("threesea-he-members")
            }
            // Like Jolla Settings → Lock screen → Select shortcuts: add at the top, remove beside
            // each member (creator only).
            AddRow {
                visible: page.group.isCreator === true && !page.group.left
                //% "Add members"
                text: qsTrId("threesea-me-add-members")
                onClicked: page.addMembers()
            }
            // We are a member too: our picture, our profile on tap.
            ListItem {
                id: meItem
                contentHeight: Theme.itemSizeSmall
                onClicked: pageStack.push(Qt.resolvedUrl("settings/ProfileSettingsPage.qml"))
                Thumbnail {
                    id: myPicture
                    height: meItem.contentHeight
                    highlighted: meItem.highlighted
                    source: page.myPictureVersion > 0 ? "image://threesea/contact/me?" + page.myPictureVersion : ""
                }
                Label {
                    anchors {
                        left: myPicture.right
                        leftMargin: Theme.paddingLarge
                        right: parent.right
                        rightMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: page.group.isCreator
                          //% "You (creator)"
                          ? qsTrId("threesea-la-you-creator")
                          //% "You"
                          : qsTrId("threesea-la-you")
                    color: meItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }
        }

        // Tap: message the member, its picture: the contact's details (like the contact list); the
        // creator removes members with the round × (remorse).
        delegate: ListItem {
            id: item
            readonly property bool removable: page.group.isCreator === true && !page.group.left
                                              && !model.creator
            contentHeight: Theme.itemSizeSmall
            onClicked: pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: model.identity })

            Thumbnail {
                id: memberPicture
                height: item.contentHeight
                highlighted: item.highlighted || pressed
                pressed: memberPictureArea.pressed && memberPictureArea.containsMouse
                source: model.pictureVersion > 0
                        ? "image://threesea/contact/" + model.identity + "?" + model.pictureVersion : ""
                MouseArea {
                    id: memberPictureArea
                    anchors.fill: parent
                    onClicked: pageStack.push(Qt.resolvedUrl("ContactPage.qml"), { identity: model.identity })
                }
            }
            Label {
                anchors {
                    left: memberPicture.right
                    leftMargin: Theme.paddingLarge
                    right: removeButton.visible ? removeButton.left : parent.right
                    rightMargin: removeButton.visible ? Theme.paddingMedium : Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                text: model.creator
                      //% "%1 (creator)"
                      ? qsTrId("threesea-la-member-creator").arg(model.displayName)
                      : model.displayName
                truncationMode: TruncationMode.Fade
                color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
            }
            RoundIconButton {
                id: removeButton
                visible: item.removable
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                icon.source: "image://theme/icon-m-clear"
                onClicked: page.removeMember(item, model.identity, model.displayName)
            }
        }

        VerticalScrollDecorator { }
    }
}
