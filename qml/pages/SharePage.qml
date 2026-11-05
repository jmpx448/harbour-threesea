// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components"
import "../components/Format.js" as Fmt

// "Share" from another app: choose the chat (recent chats first, then all contacts). One file or
// text opens the chat ready to send; several files are sent right away.
Page {
    id: page
    objectName: "share"
    allowedOrientations: Orientation.All

    property var files: []
    property string text
    // Forwarding a message of ours (its local id) instead of shared content; `text` previews it.
    property int forwardMessage: -1
    // Or several (local ids, in chat order), from the selection page.
    property var forwardMessages: []
    readonly property bool forwarding: forwardMessage >= 0 || forwardMessages.length > 0

    // A tap waits for the core's answer below: one at a time.
    property bool opening: false

    // A private chat hidden right now takes nothing: the PIN comes first, above this page, so
    // that its X comes back to this choice with the content still here. `draft` answers `locked`
    // for such a chat and changes nothing.
    function open(identity) {
        if (opening) return
        opening = true
        if (identity.indexOf("g:") === 0 || identity.indexOf("d:") === 0) {
            openWritable(identity)
            return
        }
        // A blocked contact or a revoked ID has no input to take this, and the core would refuse
        // it: said here, with the content still on this page for another chat. (Left groups aren't
        // in the list.)
        backend.call("contact", { identity: identity }, function(ok, result) {
            if (ok && result.invalid === true) {
                page.opening = false
                Notices.show(qsTrId("threesea-la-revoked-input"), Notice.Long, Notice.Center)
            } else if (ok && result.blocked === true) {
                page.opening = false
                Notices.show(qsTrId("threesea-la-blocked-input"), Notice.Long, Notice.Center)
            } else {
                page.openWritable(identity)
            }
        })
    }

    function openWritable(identity) {
        backend.call("draft", { identity: identity }, function(ok, result, kind) {
            page.opening = false
            // The app locked or a call came in meanwhile: this stays for when they are over.
            if (pageStack.busy || pageStack.currentPage !== page) return
            if (ok || kind !== "locked") {
                page.deliver(identity)
                return
            }
            var unlock = pageStack.push(Qt.resolvedUrl("UnlockPage.qml"), {
                privateChats: true,
                //% "Private chats"
                title: qsTrId("threesea-he-private-chats")
            })
            unlock.unlocked.connect(function() { page.deliver(identity) })
        })
    }

    // Called from this page or from the PIN's above it.
    function deliver(identity) {
        if (forwarding) {
            // In order; this page is gone before the replies come.
            var ids = forwardMessage >= 0 ? [forwardMessage] : forwardMessages.slice()
            window.forwardInTurn(ids, identity)
            // Back to where the selection started, then into the target chat.
            var chats = pageStack.find(function(p) { return p.objectName === "conversations" })
            if (chats) pageStack.pop(chats, PageStackAction.Immediate)
            pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: identity, openAtNewest: true })
            return
        }
        // Waiting above the input, like picked files: sent in order with the next send.
        var chat = pageStack.replaceAbove(pageStack.previousPage(page), Qt.resolvedUrl("ChatPage.qml"),
                                          { identity: identity, openAtNewest: true })
        chat.attachAll(files)
        if (text.length > 0 && chat.inputField) chat.setInput(text)
    }

    JsonListModel {
        id: chats
        keyField: "identity"
        roles: ["identity", "displayName", "group", "pictureVersion", "left"]
    }

    Component.onCompleted: backend.call("conversations", {}, function(ok, result) {
        if (!ok) return
        var list = []
        for (var i = 0; i < result.conversations.length; ++i) {
            if (!result.conversations[i].left) list.push(result.conversations[i])
        }
        chats.update(list)
    })

    SilicaListView {
        anchors.fill: parent
        model: chats

        header: Column {
            width: parent.width
            PageHeader {
                title: page.forwarding
                       //% "Forward"
                       ? qsTrId("threesea-he-forward")
                       //% "Share"
                       : qsTrId("threesea-he-share")
                description: page.files.length > 1
                             //% "%n file(s)"
                             ? qsTrId("threesea-la-share-files", page.files.length)
                             : page.files.length === 1 ? page.files[0].name : page.text
            }
            SectionHeader {
                //% "Chats"
                text: qsTrId("threesea-he-chats")
            }
        }

        // Entries like the chat and contact lists (Jolla Media style), a bit lower.
        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeLarge
            onClicked: page.open(model.identity)
            Thumbnail {
                id: avatar
                height: item.contentHeight
                group: model.group === true
                highlighted: item.highlighted
                source: !model.pictureVersion ? ""
                        : (model.group ? "image://threesea/group/" : "image://threesea/contact/")
                          + model.identity + "?" + model.pictureVersion
            }
            Label {
                anchors {
                    left: avatar.right
                    leftMargin: Theme.paddingLarge
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                text: model.displayName
                font.pixelSize: Theme.fontSizeLarge
                truncationMode: TruncationMode.Fade
                color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
            }
        }

        footer: Column {
            width: parent.width
            SectionHeader {
                //% "Contacts"
                text: qsTrId("threesea-he-contacts")
            }
            Repeater {
                model: backend.contacts
                ListItem {
                    id: contactItem
                    contentHeight: Theme.itemSizeLarge
                    onClicked: page.open(model.identity)
                    Thumbnail {
                        id: contactPicture
                        height: contactItem.contentHeight
                        highlighted: contactItem.highlighted
                        source: model.pictureVersion > 0
                                ? "image://threesea/contact/" + model.identity + "?" + model.pictureVersion : ""
                    }
                    Label {
                        anchors {
                            left: contactPicture.right
                            leftMargin: Theme.paddingLarge
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }
                        text: model.displayName
                        font.pixelSize: Theme.fontSizeLarge
                        truncationMode: TruncationMode.Fade
                        color: contactItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                    }
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
