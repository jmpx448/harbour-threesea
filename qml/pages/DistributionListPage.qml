// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components"
import "../components/Errors.js" as Errors

// A distribution list, attached to its chat (swipe left): who gets its messages (each as a
// normal 1:1 message); rename, add members, long press to remove one, delete the list.
Page {
    id: page
    allowedOrientations: Orientation.All

    property string identity
    property var list: ({ members: [] })

    function load() {
        backend.call("distributionList", { key: identity }, function(ok, result) {
            if (ok) page.list = result
        })
    }

    function update(args) {
        args.key = identity
        backend.call("updateDistributionList", args, function(ok, result, kind, message) {
            if (ok) page.list = result
            else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
        })
    }

    function addMembers() {
        var current = memberIdentities()
        var dialog = pageStack.push(Qt.resolvedUrl("MemberPickerDialog.qml"), {
            //% "Add"
            acceptText: qsTrId("threesea-he-add"),
            //% "Add recipients"
            title: qsTrId("threesea-me-add-recipients"),
            distributionList: true,
            excluded: current
        })
        dialog.accepted.connect(function() {
            if (dialog.selected.length > 0) page.update({ members: current.concat(dialog.selected) })
        })
    }

    // Here, not in the row: its scope is gone when the remorse runs out.
    function removeMember(item, identity, name) {
        //% "Removing %1"
        item.remorseAction(qsTrId("threesea-la-removing-member").arg(name), function() {
            var ids = page.memberIdentities()
            ids.splice(ids.indexOf(identity), 1)
            page.update({ members: ids })
        })
    }

    function memberIdentities() {
        var ids = []
        for (var i = 0; i < list.members.length; ++i) ids.push(list.members[i].identity)
        return ids
    }

    // Keyed, so a reload keeps the rows, and a running remorse with them.
    JsonListModel {
        id: recipients
        keyField: "identity"
        roles: ["identity", "displayName"]
    }
    onListChanged: recipients.update(list.members)

    Component.onCompleted: load()

    Connections {
        target: backend
        onCoreEvent: if (name === "contactsChanged") page.load()
    }

    RemorsePopup { id: remorse }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: recipients

        StayAtTop { target: listView }

        PullDownMenu {
            MenuItem {
                //% "Delete list"
                text: qsTrId("threesea-me-delete-list")
                //% "Deleting list"
                onClicked: remorse.execute(qsTrId("threesea-la-deleting-list"), function() {
                    backend.call("deleteConversation", { identity: page.identity }, function() {
                        pageStack.pop(pageStack.find(function(p) { return p.objectName === "conversations" }))
                    })
                })
            }
            MenuItem {
                //% "Rename"
                text: qsTrId("threesea-me-rename")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("GroupNameDialog.qml"),
                                                { name: page.list.name, distributionList: true })
                    dialog.accepted.connect(function() { page.update({ name: dialog.name }) })
                }
            }
        }

        header: Column {
            width: parent.width
            ProfileHeader {
                list: true
                pinned: page.list.pinned === true
                onPinToggled: backend.call("setPinned", { identity: page.identity, pinned: !pinned },
                                           function() { page.load() })
                //% "Distribution list"
                title: page.list.displayName || qsTrId("threesea-he-distribution-list")
                //% "%n recipient(s)"
                description: qsTrId("threesea-la-recipient-count", page.list.members.length)
            }
            Item { width: 1; height: Theme.paddingLarge }
            MediaButton {
                identity: page.identity
                name: page.list.displayName || ""
            }
            ExportChatButton {
                identity: page.identity
                name: page.list.displayName || ""
            }
            InfoLabel {
                //% "Each recipient gets the messages as a normal message from you; they don't see who else got them."
                text: qsTrId("threesea-la-distribution-list-info")
                font.pixelSize: Theme.fontSizeSmall
            }
            SectionHeader {
                //% "Recipients"
                text: qsTrId("threesea-he-recipients")
            }
            // Like Jolla Settings → Lock screen → Select shortcuts.
            AddRow {
                //% "Add recipients"
                text: qsTrId("threesea-me-add-recipients")
                onClicked: page.addMembers()
            }
        }

        // Tap: message the recipient; the round × removes them (remorse).
        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeSmall
            onClicked: pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: model.identity })

            Label {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: removeButton.left
                    rightMargin: Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                text: model.displayName
                truncationMode: TruncationMode.Fade
                color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
            }
            RoundIconButton {
                id: removeButton
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
