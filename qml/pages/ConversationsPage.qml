// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components"
import "../components/Format.js" as Fmt
import "../components/Errors.js" as Errors

Page {
    id: page
    // The same page shows the archive (`archived`) and the private chats (`privateList`, after
    // the PIN; ADR 0009), pushed from the menu under the chats.
    property bool archived: false
    property bool privateList: false
    readonly property bool mainList: !archived && !privateList
    objectName: privateList ? "private" : archived ? "archive" : "conversations"
    allowedOrientations: Orientation.All
    property int archivedCount: 0
    property int privateCount: 0

    // Leaving the private chats hides them again; the core still takes the files a chat of theirs
    // left on their way to it.
    Component.onDestruction: if (privateList) backend.lockWithFilesOnTheirWay()

    // The remorse callback for a chat command. Made here: it runs after the context menu, and with
    // it the menu's scope, is gone.
    function chatAction(command, identity) {
        return function() {
            backend.call(command, { identity: identity }, function(ok, result, kind, message) {
                if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                page.reload()
            })
        }
    }

    function openPrivate() {
        // Marked by a restore or a linked device with no lock here yet: a PIN comes first.
        if (!(window.lockInfo && window.lockInfo.lock)) {
            Notices.show(qsTrId("threesea-la-private-needs-lock"), Notice.Long, Notice.Center)
            return
        }
        var unlock = pageStack.push(Qt.resolvedUrl("UnlockPage.qml"), {
            privateChats: true,
            //% "Private chats"
            title: qsTrId("threesea-he-private-chats")
        })
        unlock.unlocked.connect(function() {
            pageStack.replace(Qt.resolvedUrl("ConversationsPage.qml"), { privateList: true })
        })
    }

    function setPrivate(identity, isPrivate) {
        backend.call("setPrivate", { identity: identity, private: isPrivate }, function(ok, result, kind, message) {
            if (ok) {
                page.reload()
            } else if (kind === "lockNeeded") {
                //% "Private chats need an app lock. Set up a PIN in Settings, App lock."
                Notices.show(qsTrId("threesea-la-private-needs-lock"), Notice.Long, Notice.Center)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    JsonListModel {
        id: conversations
        keyField: "identity"
        roles: ["identity", "displayName", "unread", "markedUnread", "lastMessage", "verification", "section",
                "group", "senderName", "pictureVersion", "muted", "pinned", "distributionList", "draft",
                "draftMentions"]
    }

    // New group: name and members in one dialog; its chat opens once the dialog has closed.
    function newList() {
        var dialog = pageStack.push(Qt.resolvedUrl("MemberPickerDialog.qml"), {
            askName: true,
            distributionList: true,
            //% "Create"
            acceptText: qsTrId("threesea-he-create"),
            //% "New distribution list"
            title: qsTrId("threesea-he-new-list")
        })
        dialog.accepted.connect(function() {
            backend.call("createDistributionList", { name: dialog.name.trim(), members: dialog.selected },
                         function(ok, result, kind, message) {
                if (ok) {
                    openWhenIdle.identity = result.key
                    openWhenIdle.start()
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    function newGroup() {
        var dialog = pageStack.push(Qt.resolvedUrl("MemberPickerDialog.qml"), {
            askName: true,
            //% "Create"
            acceptText: qsTrId("threesea-he-create"),
            //% "New group"
            title: qsTrId("threesea-he-new-group")
        })
        dialog.accepted.connect(function() {
            backend.call("createGroup", { name: dialog.name.trim(), members: dialog.selected },
                         function(ok, result, kind, message) {
                if (ok) {
                    openWhenIdle.identity = result.key
                    openWhenIdle.start()
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    Timer {
        id: openWhenIdle
        property string identity
        interval: 50
        repeat: true
        onTriggered: {
            if (pageStack.busy) return
            stop()
            pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: identity })
        }
    }

    // Reloads asked for in a burst (many messages arriving at once, opening the page) run once.
    function reload() {
        if (!reloadTimer.running) reloadTimer.start()
    }
    Timer {
        id: reloadTimer
        interval: 50
        onTriggered: page.load()
    }

    // Reloads are for someone looking at the list. Hidden under another page, or with the app in
    // the background or its window closed, a change only marks it; it reloads on coming back.
    readonly property bool seen: page.status === PageStatus.Active && Qt.application.state === Qt.ApplicationActive
    property bool stale: false
    onSeenChanged: if (seen && stale) reload()

    function reloadWhenSeen() {
        if (seen) reload()
        else stale = true
    }

    // Whether changed messages show in this list: of a row, only the newest message does (its
    // delivery icon, its text). While a reload is under way the list can't tell yet which is the
    // newest: a send's ack may overtake the reply that brings the new message.
    function showsAnyOf(messages) {
        if (reloadTimer.running || loadsRunning > 0) return true
        for (var i = 0; i < messages.length; ++i) {
            var row = conversations.indexOfKey(messages[i].conversation)
            if (row >= 0 && conversations.get(row).lastMessage.id === messages[i].id) return true
        }
        return false
    }
    property int loadsRunning: 0

    function load() {
        stale = false
        loadsRunning++
        backend.call("conversations", { archived: page.archived, private: page.privateList },
                     function(ok, result, kind, message) {
            page.loadsRunning--
            // Locked: the page goes anyway, and has nothing to say about that.
            if (!ok) {
                if (kind !== "locked") page.loadError = Errors.text(kind, message)
                return
            }
            page.loadError = ""
            page.archivedCount = result.archivedCount || 0
            page.privateCount = result.privateCount || 0
            if (page.privateList) window.clearNotification("private")
            // Day sections like Jolla Messages ("Yesterday", "Monday", "Wednesday, 14 October"),
            // by the same time the core orders the list by.
            // Pinned chats come first (core), under their own section.
            var list = result.conversations
            page.pinnedCount = 0
            for (var i = 0; i < list.length; ++i) {
                if (list[i].pinned !== null && list[i].pinned !== undefined) {
                    //% "Pinned"
                    list[i].section = qsTrId("threesea-se-pinned")
                    page.pinnedCount++
                } else {
                    // A restored group can come without a creation date: no day section then.
                    list[i].section = list[i].lastMessage.arrivedAt > 0
                                      ? Fmt.day(list[i].lastMessage.arrivedAt) : ""
                }
            }
            conversations.update(list)
        })
    }

    // Why the list could not be read: no chats and "no chats" are not the same.
    property string loadError
    property int pinnedCount: 0

    function pin(identity, pinned) {
        backend.call("setPinned", { identity: identity, pinned: pinned }, function() { page.reload() })
    }

    function movePinned(identity, offset) {
        backend.call("movePinned", { identity: identity, offset: offset }, function() { page.reload() })
    }

    function openChat(identity) {
        pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: identity })
    }

    // A new or restored ID is asked for a nickname once. If the update question is up first, at
    // the next showing.
    property bool nicknameChecked: false
    function askNickname() {
        backend.call("profile", {}, function(ok, profile) {
            if (!ok || nicknameChecked) return
            if (profile.askNickname) {
                if (status !== PageStatus.Active || window.stackHeld || pageStack.busy) return
                pageStack.push(Qt.resolvedUrl("NicknameDialog.qml"))
            }
            nicknameChecked = true
        })
    }

    // Shown at once when created; coming back to it (it was just loaded the first time) reloads.
    property bool wasActive: false
    Component.onCompleted: load()
    onStatusChanged: {
        if (status === PageStatus.Active) {
            if (wasActive) reload()
            // The main list's first showing, after setup or unlocking: the one-time update question.
            else if (mainList) window.askUpdateCheck()
            wasActive = true
            if (mainList && !nicknameChecked) askNickname()
            // Contacts are attached: swipe left (forward indicator top right), like Jolla apps.
            if (mainList && pageStack.nextPage(page) === null) pageStack.pushAttached(Qt.resolvedUrl("ContactsPage.qml"))
        }
    }

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "messageAdded" || name === "contactsChanged"
                    || name === "conversationRead" || name === "groupsChanged" || name === "chatsChanged") {
                page.reloadWhenSeen()
            } else if (name === "privateChanged") {
                // Seen or not: a chat that went private must not be drawn again on coming back,
                // not even until the reply is here. Gone from this list at once.
                if (data.private !== page.privateList) conversations.remove(data.identity)
                if (data.private) page.reload()
                else page.reloadWhenSeen()
            } else if (name === "messagesChanged") {
                if (!page.seen) page.stale = true
                // A private chat's message from another device comes only as a change, with an id
                // the list doesn't know yet, and no conversationRead follows it.
                else if (page.privateList || page.showsAnyOf(data.messages)) page.reload()
            }
        }
    }

    FontMetrics {
        id: nameMetrics
        font.family: Theme.fontFamily
        font.pixelSize: Theme.fontSizeLarge
    }
    FontMetrics {
        id: previewMetrics
        font.family: Theme.fontFamily
        font.pixelSize: Theme.fontSizeSmall
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: conversations

        PullDownMenu {
            visible: page.mainList
            // Like Jolla Mail while it syncs: the pulley glows while connecting.
            busy: backend.connectionState === "connecting"
            MenuItem {
                //% "Settings"
                text: qsTrId("threesea-me-settings")
                onClicked: pageStack.push(Qt.resolvedUrl("SettingsPage.qml"))
            }
            MenuItem {
                // Makes room for Connect now (four items at most). The Contacts page has it too.
                visible: backend.connectionState !== "disconnected"
                //% "New distribution list"
                text: qsTrId("threesea-me-new-list")
                onClicked: page.newList()
            }
            MenuItem {
                //% "New group"
                text: qsTrId("threesea-me-new-group")
                onClicked: page.newGroup()
            }
            MenuItem {
                //% "Search"
                text: qsTrId("threesea-me-search")
                onClicked: pageStack.push(Qt.resolvedUrl("SearchPage.qml"))
            }
            // Offline: try now instead of waiting for the next attempt (like Jolla Mail's Sync).
            MenuItem {
                visible: backend.connectionState === "disconnected"
                //% "Connect now"
                text: qsTrId("threesea-me-connect-now")
                onClicked: backend.call("reconnect", {})
            }
        }

        header: Column {
            width: listView.width

            PageHeader {
                title: page.privateList
                       ? qsTrId("threesea-he-private-chats")
                       : page.archived
                       //% "Archived chats"
                       ? qsTrId("threesea-he-archived-chats")
                       //% "Chats"
                       : qsTrId("threesea-he-chats")
                description: window.storageProblem ? qsTrId("threesea-la-storage-problem-title")
                             : backend.connectionState === "connected" ? ""
                             : backend.connectionState === "connecting"
                               //% "Connecting…"
                               ? qsTrId("threesea-la-connecting")
                               //% "Offline"
                               : qsTrId("threesea-la-offline")
            }

            // What the server said: too long for the header's own line, and this is the place the
            // user looks at to find out what is going on. The server's words stay plain text.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                visible: text.length > 0
                height: visible ? implicitHeight + Theme.paddingMedium : 0
                text: window.offlineReasonText
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignRight
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
            }
        }

        section {
            property: "section"
            delegate: SectionHeader {
                text: section
                visible: section.length > 0
                height: section.length > 0 ? implicitHeight : 0
            }
        }

        // Like Jolla Messages: name and clock time, then up to three lines of the last message.
        // Unread chats are shown in the highlight colour.
        delegate: ListItem {
            id: item
            // The same for every row, whatever its preview: the name and three lines.
            contentHeight: Math.max(Theme.itemSizeExtraLarge,
                                    nameMetrics.height + 3 * previewMetrics.lineSpacing + 2 * Theme.paddingSmall)
            onClicked: page.openChat(model.identity)

            // Unread look also for a chat the user marked unread by hand (the count stays away;
            // there is nothing to count).
            readonly property bool unread: model.unread > 0 || model.markedUnread === true

            readonly property bool pinned: model.pinned !== null && model.pinned !== undefined

            menu: Component {
                ContextMenu {
                    MenuItem {
                        visible: page.mainList
                        text: item.pinned
                              //% "Unpin"
                              ? qsTrId("threesea-me-unpin")
                              //% "Pin to top"
                              : qsTrId("threesea-me-pin")
                        onClicked: page.pin(model.identity, !item.pinned)
                    }
                    MenuItem {
                        visible: item.pinned && model.pinned > 0
                        //% "Move up"
                        text: qsTrId("threesea-me-move-up")
                        onClicked: page.movePinned(model.identity, -1)
                    }
                    MenuItem {
                        visible: item.pinned && model.pinned < page.pinnedCount - 1
                        //% "Move down"
                        text: qsTrId("threesea-me-move-down")
                        onClicked: page.movePinned(model.identity, 1)
                    }
                    MenuItem {
                        // Android's "mark as unread": a mark on the chat, taken away as soon
                        // as it is read or a new message arrives.
                        visible: model.lastMessage.kind !== "none" && model.distributionList !== true
                        text: item.unread
                              //% "Mark as read"
                              ? qsTrId("threesea-me-mark-read")
                              //% "Mark as unread"
                              : qsTrId("threesea-me-mark-unread")
                        onClicked: {
                            var identity = model.identity
                            if (item.unread) {
                                backend.call("markRead", { identity: identity },
                                             function() { page.reload() })
                            } else {
                                backend.call("setConversationUnread", { identity: identity, unread: true },
                                             function() { page.reload() })
                            }
                        }
                    }
                    MenuItem {
                        visible: !page.privateList
                        text: page.archived
                              //% "Move back to chats"
                              ? qsTrId("threesea-me-unarchive")
                              //% "Archive"
                              : qsTrId("threesea-me-archive")
                        onClicked: {
                            var identity = model.identity
                            backend.call("setArchived", { identity: identity, archived: !page.archived },
                                         function() { page.reload() })
                        }
                    }
                    MenuItem {
                        text: page.privateList
                              //% "Remove from private chats"
                              ? qsTrId("threesea-me-unmark-private")
                              //% "Make private"
                              : qsTrId("threesea-me-mark-private")
                        onClicked: page.setPrivate(model.identity, !page.privateList)
                    }
                    MenuItem {
                        // Nothing to empty in a chat without messages.
                        visible: model.lastMessage.kind !== "empty" && model.lastMessage.kind !== "none"
                        //% "Empty chat"
                        text: qsTrId("threesea-me-empty-chat")
                        //% "Emptying chat"
                        onClicked: item.remorseAction(qsTrId("threesea-la-emptying-chat"),
                                                      page.chatAction("emptyConversation", model.identity))
                    }
                    MenuItem {
                        //% "Delete chat"
                        text: qsTrId("threesea-me-delete-chat")
                        //% "Deleting chat"
                        onClicked: item.remorseAction(qsTrId("threesea-la-deleting-chat"),
                                                      page.chatAction("deleteConversation", model.identity))
                    }
                }
            }

            // Picture at the left edge like Jolla Media's lists.
            Thumbnail {
                id: avatar
                height: item.contentHeight
                group: model.group === true
                list: model.distributionList === true
                highlighted: item.highlighted || pressed
                pressed: pictureArea.pressed && pictureArea.containsMouse
                source: !model.pictureVersion ? ""
                        : (model.group ? "image://threesea/group/" : "image://threesea/contact/")
                          + model.identity + "?" + model.pictureVersion
                // The picture opens the contact's or group's details, the row the chat.
                MouseArea {
                    id: pictureArea
                    anchors.fill: parent
                    onClicked: pageStack.push(Qt.resolvedUrl(model.group ? "GroupPage.qml"
                                                             : model.distributionList ? "DistributionListPage.qml"
                                                             : "ContactPage.qml"),
                                              { identity: model.identity })
                    onPressAndHold: item.openMenu()
                }
            }

            Column {
                id: column
                x: avatar.width + Theme.paddingLarge
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - x - Theme.horizontalPageMargin

                Item {
                    width: parent.width
                    height: nameLabel.height

                    // The name stays white; unread chats get their count after it in the highlight
                    // colour (user 2026-10-26).
                    readonly property real nameSpace: width - timeLabel.width - Theme.paddingMedium
                                                      - (mutedIcon.visible ? mutedIcon.width + Theme.paddingSmall : 0)
                                                      - (unreadLabel.visible ? unreadLabel.width + Theme.paddingMedium : 0)
                    Label {
                        id: nameLabel
                        width: Math.min(implicitWidth, parent.nameSpace)
                        text: model.displayName
                        font.pixelSize: Theme.fontSizeLarge
                        truncationMode: TruncationMode.Fade
                        color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
                    }
                    Label {
                        id: unreadLabel
                        // A chat marked unread by hand has nothing to count.
                        visible: model.unread > 0
                        anchors {
                            left: nameLabel.right
                            leftMargin: Theme.paddingMedium
                            baseline: nameLabel.baseline
                        }
                        text: model.unread
                        font.pixelSize: Theme.fontSizeLarge
                        color: Theme.highlightColor
                    }

                    // Notifications off for this chat (or mentions only).
                    Image {
                        id: mutedIcon
                        visible: model.muted === true
                        anchors {
                            right: timeLabel.left
                            rightMargin: Theme.paddingSmall
                            verticalCenter: timeLabel.verticalCenter
                        }
                        width: Theme.iconSizeExtraSmall
                        height: width
                        sourceSize.width: width
                        sourceSize.height: height
                        source: "image://theme/icon-status-silent?" + timeLabel.color
                    }

                    Label {
                        id: timeLabel
                        anchors {
                            right: parent.right
                            baseline: nameLabel.baseline
                        }
                        // Under a day's section the clock is enough; "Pinned" says no day.
                        text: model.lastMessage.arrivedAt <= 0 ? ""
                              : item.pinned ? Fmt.time(model.lastMessage.arrivedAt, Format, Formatter)
                              : Fmt.clock(model.lastMessage.arrivedAt, Format, Formatter)
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: item.highlighted || item.unread ? Theme.highlightColor : Theme.primaryColor
                    }
                }

                Row {
                    width: parent.width
                    spacing: Theme.paddingSmall

                    DeliveryState {
                        id: tick
                        visible: model.lastMessage.outgoing && !model.draft
                        state: model.lastMessage.state
                        // The preview's colour (failed: the error colour).
                        color: state > 3 ? Theme.errorColor : preview.color
                        // Centred on the first line's letters (not its box, which has room below).
                        y: preview.baselineOffset - Math.round(preview.font.pixelSize * 0.36) - height / 2
                    }

                    Label {
                        id: preview
                        width: parent.width - (tick.visible ? tick.width + parent.spacing : 0)
                        // Groups: who wrote it, like Jolla Messages' group conversations.
                        // An unsent draft first, like Jolla Messages.
                        // About as many characters as the lines hold, from the width and
                        // the font (measured on the emulator: a line of 534 px at a font size
                        // of 36 holds some 33 characters), a little under so the ellipsis we
                        // add still has room. What is longer is cut by us, see below.
                        readonly property int limit: Math.max(24, Math.round(
                            width / (font.pixelSize * 0.47)) * maximumLineCount)
                        readonly property string plain: Fmt.previewLine(
                            model.draft
                              //% "Draft: %1"
                              ? qsTrId("threesea-la-draft").arg(Fmt.withMentions(model.draft, model.draftMentions))
                              : (model.group && model.senderName && !model.lastMessage.outgoing
                                 && model.lastMessage.kind !== "status"
                                 ? model.senderName + ": " : "") + Fmt.preview(model.lastMessage),
                            limit)
                        // Threema's emoji pictures (Settings → Appearance) need styled text.
                        readonly property bool pictures: backend.threemaEmoji && Fmt.mayHaveEmoji(plain)
                        textFormat: pictures ? Text.StyledText : Text.PlainText
                        text: pictures ? Fmt.markEmoji(Fmt.escapeHtml(plain), Math.round(font.pixelSize * 1.2)) : plain
                        maximumLineCount: 3
                        wrapMode: Text.Wrap
                        // Qt puts the pictures a line away from their place when it elides
                        // text that contains them (Text.StyledText with <img>, user
                        // 2026-10-30), so the text is cut above instead and clipped here.
                        elide: pictures ? Text.ElideNone : Text.ElideRight
                        clip: true
                        font.pixelSize: Theme.fontSizeSmall
                        color: item.highlighted || item.unread ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    }
                }
            }
        }

        // At the end of the chats, only when there are some: the archive and the private chats.
        // Only the entries that apply are created. With the entries switched by `visible`
        // bindings this menu opened empty (seen in the emulator once private chats were added;
        // the chat's push-up menu with a hidden item works, so the exact Silica cause is unclear).
        PushUpMenu {
            visible: page.mainList && (page.archivedCount > 0 || page.privateCount > 0)
            Repeater {
                model: {
                    var entries = []
                    //% "Archived chats"
                    if (page.archivedCount > 0) entries.push({ text: qsTrId("threesea-he-archived-chats"), action: "archive" })
                    if (page.privateCount > 0) entries.push({ text: qsTrId("threesea-he-private-chats"), action: "private" })
                    return entries
                }
                MenuItem {
                    text: modelData.text
                    onClicked: {
                        if (modelData.action === "archive") {
                            pageStack.push(Qt.resolvedUrl("ConversationsPage.qml"), { archived: true })
                        } else {
                            page.openPrivate()
                        }
                    }
                }
            }
        }

        ViewPlaceholder {
            enabled: listView.count === 0 && page.loadError.length > 0
            //% "The chats could not be loaded"
            text: qsTrId("threesea-la-chats-not-loaded")
            hintText: page.loadError
        }
        ViewPlaceholder {
            enabled: listView.count === 0 && page.loadError.length === 0 && page.archived
            //% "No archived chats"
            text: qsTrId("threesea-la-no-archived")
        }
        ViewPlaceholder {
            enabled: listView.count === 0 && page.loadError.length === 0 && page.privateList
            //% "No private chats"
            text: qsTrId("threesea-la-no-private")
        }
        ViewPlaceholder {
            enabled: listView.count === 0 && page.loadError.length === 0 && page.mainList
            //% "No chats yet"
            text: qsTrId("threesea-la-no-chats")
            //% "Pull down to write a message"
            hintText: qsTrId("threesea-la-no-chats-hint")
        }

        VerticalScrollDecorator { }
    }
}
