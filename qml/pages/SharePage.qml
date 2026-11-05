// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components"
import "../components/Format.js" as Fmt

// "Share" from another app, or forwarding: choose the chat (recent chats first, then all contacts),
// or several from the pull-down menu. One chat opens with shared content ready to send. Several
// get it at once, after a look at what goes, as does anything forwarded but a single text.
Page {
    id: page
    objectName: "share"
    allowedOrientations: Orientation.All

    property var files: []
    property string text
    // Forwarding messages of ours instead of shared content, in chat order, as ChatPage.forwardItem
    // makes them; `text` previews them.
    property var forwardItems: []
    readonly property bool forwarding: forwardItems.length > 0

    // Chosen so far, kept for the picker to come back to; and those being sent to.
    property var selected: []
    property var targets: []
    // The forward's own caption, once the user has been asked: nothing for what the messages have.
    property var caption
    // A private chat among them was unlocked for this, so it is hidden again after it.
    property bool unlockedHere: false

    // A tap waits for the core's answer below: one at a time.
    property bool opening: false

    // A dialog above this page has answered: what follows waits for this page to show again.
    property var whenShowing: null
    function whenBack(next) {
        if (status === PageStatus.Active && !pageStack.busy) next()
        else whenShowing = next
    }
    onStatusChanged: {
        if (status === PageStatus.Active && whenShowing) {
            var next = whenShowing
            whenShowing = null
            next()
        }
    }

    // A blocked contact or a revoked ID has no input to take this, and the core would refuse it:
    // said here, with the content still on this page for another chat. (Left groups aren't in the
    // list.)
    function choose(identities, names) {
        if (opening || identities.length === 0) return
        opening = true
        function check(at) {
            if (at >= identities.length) {
                page.opening = false
                page.targets = identities
                page.confirm(names)
                return
            }
            if (identities[at].indexOf("g:") === 0 || identities[at].indexOf("d:") === 0) {
                check(at + 1)
                return
            }
            backend.call("contact", { identity: identities[at] }, function(ok, result) {
                if (ok && (result.invalid === true || result.blocked === true)) {
                    page.opening = false
                    page.selected = page.selected.filter(function(other) { return other !== identities[at] })
                    var why = result.invalid === true ? qsTrId("threesea-la-revoked-input")
                                                      : qsTrId("threesea-la-blocked-input")
                    Notices.show(identities.length > 1 ? names[at] + ": " + why : why, Notice.Long, Notice.Center)
                } else {
                    check(at + 1)
                }
            })
        }
        check(0)
    }

    // Like Android: a look first at anything forwarded but a single text, here also before sending
    // to several chats. A single file's caption can be changed, several messages' left out.
    function confirm(names) {
        var oneText = forwardItems.length === 1 && forwardItems[0].kind === "text"
        var goesAtOnce = forwarding ? oneText : targets.length === 1
        caption = undefined
        if (goesAtOnce) {
            openTargets()
            return
        }
        var oneFile = forwardItems.length === 1 && forwardItems[0].kind === "file"
        var withCaptions = forwardItems.length > 1 && forwardItems.some(function(item) { return item.caption.length > 0 })
        var shown = oneFile ? Fmt.withMentions(forwardItems[0].caption, forwardItems[0].mentions) : text
        var dialog = pageStack.push(Qt.resolvedUrl("ConfirmSendDialog.qml"), {
            names: names.join(", "),
            forwarding: forwarding,
            askContent: oneFile || !forwarding,
            content: shown,
            contentIsCaption: oneFile || files.length > 0,
            askCaptions: withCaptions
        })
        dialog.accepted.connect(function() {
            if (oneFile) {
                page.caption = dialog.content === shown ? forwardItems[0].caption
                                                        : page.withMentionTags(dialog.content, forwardItems[0])
            } else if (withCaptions && !dialog.includeCaptions) {
                page.caption = ""
            } else if (!page.forwarding) {
                page.text = dialog.content
            }
            page.whenBack(page.openTargets)
        })
    }

    // What the dialog showed as names back to the tags it had.
    function withMentionTags(shown, item) {
        var tags = item.caption.match(/@\[[0-9A-Z*@]{8}\]/g) || []
        tags.sort(function(a, b) {
            return Fmt.withMentions(b, item.mentions).length - Fmt.withMentions(a, item.mentions).length
        })
        for (var i = 0; i < tags.length; ++i) {
            shown = shown.split(Fmt.withMentions(tags[i], item.mentions)).join(tags[i])
        }
        return shown
    }

    // A private chat hidden right now takes nothing: the PIN comes first, above this page, so
    // that its X comes back to this choice with the content still here. `draft` answers `locked`
    // for such a chat and changes nothing. One PIN shows them all.
    function openTargets() {
        if (opening) return
        opening = true
        var identities = targets
        function check(at) {
            if (at >= identities.length) {
                page.opening = false
                page.deliver()
                return
            }
            backend.call("draft", { identity: identities[at] }, function(ok, result, kind) {
                // The app locked or a call came in meanwhile: this stays for when they are over.
                if (pageStack.busy || pageStack.currentPage !== page) {
                    page.opening = false
                    return
                }
                if (ok || kind !== "locked") {
                    check(at + 1)
                    return
                }
                page.opening = false
                var unlock = pageStack.push(Qt.resolvedUrl("UnlockPage.qml"), {
                    privateChats: true,
                    //% "Private chats"
                    title: qsTrId("threesea-he-private-chats")
                })
                unlock.unlocked.connect(function() {
                    page.unlockedHere = true
                    page.deliver()
                })
            })
        }
        check(0)
    }

    // Called from this page or from the PIN's above it.
    function deliver() {
        // Several: everything goes from here, without a chat, and the user is back where this began.
        var hide = unlockedHere
        unlockedHere = false
        if (targets.length > 1) {
            if (forwarding) window.forwardInTurn(forwardIds(), targets, caption, hide)
            else window.shareInTurn(files, text, targets, hide)
            pageStack.pop(pageStack.previousPage(page))
            return
        }
        var identity = targets[0]
        if (forwarding) {
            // In order; this page is gone before the replies come.
            window.forwardInTurn(forwardIds(), targets, caption, false)
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

    function forwardIds() {
        return forwardItems.map(function(item) { return item.id })
    }

    function chooseSeveral() {
        var dialog = pageStack.push(Qt.resolvedUrl("ChatPickerDialog.qml"), { selected: page.selected })
        dialog.accepted.connect(function() {
            page.selected = dialog.selected
            var names = dialog.selectedNames()
            page.whenBack(function() { page.choose(page.selected, names) })
        })
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

    // What the search field in the header narrows both lists to. The contacts model is shared.
    property string query
    ListFilter {
        id: foundChats
        sourceModel: chats
        query: page.query
    }
    ListFilter {
        id: foundContacts
        sourceModel: backend.contacts
        query: page.query
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: foundChats
        // Or the first row becomes the current item and takes the focus from the search field.
        currentIndex: -1

        // "Chats" goes while no chat matches.
        StayAtTop { target: listView }

        PullDownMenu {
            MenuItem {
                //% "Choose several chats"
                text: qsTrId("threesea-me-choose-several-chats")
                onClicked: page.chooseSeveral()
            }
        }

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
            SearchField {
                width: parent.width
                //% "Search chats and contacts"
                placeholderText: qsTrId("threesea-la-search-chats-contacts")
                inputMethodHints: Qt.ImhNoAutoUppercase | window.incognitoHints
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
                onTextChanged: page.query = text
            }
            SectionHeader {
                visible: foundChats.count > 0
                //% "Chats"
                text: qsTrId("threesea-he-chats")
            }
        }

        // Entries like the chat and contact lists (Jolla Media style), a bit lower.
        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeLarge
            onClicked: page.choose([model.identity], [model.displayName])
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
                visible: foundContacts.count > 0
                //% "Contacts"
                text: qsTrId("threesea-he-contacts")
            }
            Repeater {
                model: foundContacts
                ListItem {
                    id: contactItem
                    contentHeight: Theme.itemSizeLarge
                    onClicked: page.choose([model.identity], [model.displayName])
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

        ViewPlaceholder {
            enabled: page.query.trim().length > 0 && foundChats.count === 0 && foundContacts.count === 0
            //% "No chats or contacts found"
            text: qsTrId("threesea-la-search-no-chats-contacts")
        }

        VerticalScrollDecorator { }
    }
}
