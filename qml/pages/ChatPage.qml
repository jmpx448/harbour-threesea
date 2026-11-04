// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import Sailfish.Pickers 1.0
// Registers the image://nemoThumbnail provider (video frames for sending). Its Thumbnail item is
// not used: on SFOS 5.1 it repeats statusChanged without end (main thread at 100 %).
import Nemo.Thumbnailer 1.0 as Nemo
import Nemo.KeepAlive 1.2
import harbour.threesea 1.0
import "../components"
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

Page {
    id: page
    allowedOrientations: Orientation.All

    property string identity
    // Open at this message (local id), e.g. from search; loaded even if older than a page.
    property int focusMessage: -1
    objectName: "chat"

    // Selecting messages: the chat stays as it is, taps select bubbles, a panel at the bottom
    // deletes or forwards them.
    property bool selecting: false
    property var selected: ({})
    readonly property int selectedCount: {
        var n = 0
        for (var id in selected) if (selected[id]) n++
        return n
    }
    forwardNavigation: !selecting
    function toggleSelected(id) {
        var map = selected
        map[id] = !map[id]
        selected = map
    }
    function endSelection() {
        selecting = false
        selected = {}
    }
    // Selected messages, oldest first.
    function selectedMessages() {
        var list = []
        for (var i = messages.count - 1; i >= 0; --i) {
            var m = messages.get(i)
            if (selected[m.id]) list.push(m)
        }
        return list
    }
    function forwardable(m) {
        return m.kind === "text" || m.kind === "location"
               || (m.kind === "file" && m.body && m.body.fileState === 2)
    }
    function showMessage(id) {
        focusMessage = id
        load()
    }
    property var contact: ({})
    // A group chat (identity is the group's key "g:<creator>:<id>").
    readonly property bool isGroup: identity.indexOf("g:") === 0
    // A distribution list: its own chat, each message goes to every member as a 1:1 message.
    readonly property bool isList: identity.indexOf("d:") === 0
    readonly property bool groupLeft: isGroup && contact.left === true
    // Blocked by us: no input (unblock on the attached contact page).
    readonly property bool blocked: !isGroup && !isList && contact.blocked === true
    // The ID was revoked (found by the directory check): nobody to write to any more.
    readonly property bool revoked: !isGroup && !isList && contact.invalid === true
    readonly property bool readOnly: groupLeft || blocked || revoked
    property bool typing: false
    property string stickyDay: ""
    // Editing one of our messages in the input box (its message id), "" when writing a new one.
    property string editingMessageId: ""
    // What the input held when the edit started ({text, mentions, quoting}): it comes back after.
    property var beforeEdit: null
    // Quoting a message with the next text message: its message JSON, or null.
    property var quoting: null
    // Quoted messages not in the loaded list, fetched on demand (messageId -> message or false).
    property var quoteCache: ({})
    property int quoteCacheRevision: 0
    // Picked files waiting to be sent with the next send ([{path, photo, name, video}]), in order;
    // the typed text becomes the first one's caption.
    property var attachments: []
    readonly property var attachment: attachments.length > 0 ? attachments[0] : null
    // Files being sent, one after the other so they arrive in the picked order.
    property var sendQueue: []
    property bool sendingQueue: false

    // Jolla Messages: the day of the topmost visible message always shows in a band under the
    // name; nothing scrolls under it (the list starts below the band).
    // The sticky day steps aside while a day header of the list is at the top: that one says it.
    property bool stickyHidden: false
    // The message the band's day comes from; -1 forces the next update to read it again
    // (this runs on every scrolled pixel, and reading a row copies the whole message).
    property int stickyIndex: -1
    function updateStickyDay() {
        // In the bottom-to-top list the top of the view is contentY.
        var top = listView.contentY
        var index = listView.indexAt(listView.width / 2, top + Theme.paddingSmall)
        // Above the oldest message (short chats have a spacer there): the oldest message's day.
        if (index < 0) index = messages.count - 1
        if (index !== page.stickyIndex) {
            page.stickyIndex = index
            stickyDay = index >= 0 ? Fmt.day(messages.get(index).createdAt) : ""
        }
        var reach = dayBand.height * 2
        var hidden = false
        for (var y = top; y <= top + reach; y += dayBand.height / 2) {
            var item = listView.itemAt(listView.width / 2, y)
            if (item && item.showDay && item.y + item.topGap + dayBand.height > top
                    && item.y + item.topGap < top + reach) {
                hidden = true
                break
            }
        }
        stickyHidden = hidden
    }

    // Newest first: the view is laid out bottom to top, so index 0 is at the bottom.
    JsonListModel {
        id: messages
        keyField: "id"
        roles: ["id", "messageId", "outgoing", "kind", "text", "body", "createdAt", "state",
                "editedAt", "sender", "reactions", "quoteId", "mentions", "starred"]
    }

    // The contact, or for groups the group info shaped like one (title as name, member names).
    function loadContact() {
        if (isGroup) {
            backend.call("group", { key: identity }, function(ok, result) {
                if (!ok) return
                var names = []
                for (var i = 0; i < result.members.length; ++i) names.push(result.members[i].displayName)
                result.displayName = result.title
                result.memberNames = names.join(", ")
                result.reactions = true
                result.canEdit = true
                result.canDelete = true
                page.contact = result
            })
        } else if (isList) {
            backend.call("distributionList", { key: identity }, function(ok, result) {
                if (!ok) return
                var names = []
                for (var i = 0; i < result.members.length; ++i) names.push(result.members[i].displayName)
                result.memberNames = names.join(", ")
                // Only plain messages: no reactions, edits, deletions for everyone, quotes, polls.
                result.reactions = false
                page.contact = result
            })
        } else {
            backend.call("contact", { identity: identity }, function(ok, result) {
                if (ok) page.contact = result
            })
        }
    }

    // Display name of a group member (sender of an incoming message).
    // The voters' names for the poll pages.
    function pollNames(poll) {
        var names = {}
        for (var voter in (poll.votes || {})) {
            names[voter] = isGroup ? memberName(voter) : (voter === identity ? contact.displayName : voter)
        }
        return names
    }

    function memberName(identity) {
        var members = contact.members || []
        for (var i = 0; i < members.length; ++i) {
            if (members[i].identity === identity) return members[i].displayName
        }
        return identity
    }

    // Nothing of the chat is shown until its messages came (a private chat needs the PIN).
    property bool shown: false
    // Leaving a private chat hides the private chats again (unless their list is underneath).
    property bool isPrivate: false

    // A private chat opened while they are hidden (contacts, links): the PIN first, then the chat
    // (the window does it: this page is gone by then).
    function askPin() {
        window.unlockChat(page, identity)
    }

    function load() {
        loadContact()
        // This window replaces whatever is loaded, so a page that is still on its way must
        // not land in it (it would put old messages next to the newest ones): the generation
        // tells the stale replies apart, and nothing new starts until this one is here.
        page.loadGeneration++
        page.loadingOlder = true
        page.loadingNewer = true
        var generation = page.loadGeneration
        // A screenful and a bit: opening stays fast, scrolling back pages the rest in.
        var args = { identity: identity, limit: 60 }
        if (focusMessage >= 0) args.since = focusMessage
        backend.call("messages", args, function(ok, result, kind) {
            if (generation !== page.loadGeneration) return
            page.loadingOlder = false
            page.loadingNewer = false
            if (!ok) {
                if (kind === "locked" && !page.shown) askPinLater.start()
                return
            }
            var firstLoad = !page.shown
            page.shown = true
            page.isPrivate = result.private === true
            page.olderExhausted = false
            page.newerExhausted = true
            page.voiceRevision++
            // Where the new messages start: kept for this visit (they are marked read now).
            if (firstLoad && result.firstUnread !== null && result.firstUnread !== undefined) {
                page.unreadMarkerId = result.firstUnread
            }
            messages.update(result.messages.reverse())
            if (focusMessage >= 0) {
                page.showLoadedMessage(focusMessage, ListView.Center)
                focusMessage = -1
            } else if (firstLoad && page.unreadMarkerId >= 0) {
                // Open where the new messages start (their line at the top of the view).
                page.showLoadedMessage(page.unreadMarkerId, ListView.End)
            } else {
                // The newest message and the input, as the chat always opens.
                listView.positionViewAtBeginning()
            }
        })
        markRead()
    }

    // Settings → Advanced → Log → Detailed: notes when the interface stopped for longer than
    // a frame or two while this chat was open, with what it was doing — counts only, no
    // content (§7.3). The emulator cannot measure this (its software renderer stalls by
    // itself), so this is how a real phone can say where the time goes.
    Timer {
        running: page.status === PageStatus.Active && backend.logLevel === "debug"
        interval: 16
        repeat: true
        property double last: 0
        onTriggered: {
            var now = Date.now()
            var gap = now - last
            if (last > 0 && gap > 150) {
                console.debug("chat: interface stopped for", gap, "ms (" + messages.count,
                              "messages loaded, fetching:", page.loadingOlder + ")")
            }
            last = now
        }
    }

    // A message of this chat into the list. The view follows the newest message, like Jolla
    // Messages — but only when the message is really new: an update of one that is already
    // there (a receipt, an edit, a reaction) must leave the view where it is, and so must a
    // page of older messages. The view's own countChanged cannot tell them apart, and it
    // arrives after the handler that caused it (which is how scrolling back ended up jumping
    // to the newest message, user report 2026-10-29).
    function addMessage(message) {
        if (!newerExhausted) {
            // The window does not reach the newest message, so this one does not belong next
            // to what is loaded. Ours brings the reader back to it; someone else's waits
            // until they scroll forward (it is in the chat either way).
            if (message.outgoing) page.load()
            return
        }
        var before = messages.count
        messages.upsert(message, 0)
        if (messages.count !== before) listView.positionViewAtBeginning()
    }

    function showLoadedMessage(id, mode) {
        for (var i = 0; i < messages.count; ++i) {
            if (messages.get(i).id === id) {
                listView.positionViewAtIndex(i, mode)
                break
            }
        }
    }

    // The line above the first message that was unread when the chat was opened.
    property int unreadMarkerId: -1

    // The chat keeps a window of messages around what the reader is looking at, not the whole
    // history: scrolling back used to load page after page and never let one go, so a long
    // read grew by roughly 100 KB per message (rows, delegates, decoded pictures) until the
    // chat was left (measured, user question 2026-10-30). Both ends page in and out: the page
    // before the oldest loaded message while scrolling back, the page after the newest while
    // scrolling forward, and whatever falls out of the window is dropped in one go.
    readonly property int pageSize: 50
    // Room for a few screens on either side of what is shown.
    readonly property int windowSize: 250
    readonly property int windowSlack: 100

    property bool loadingOlder: false
    property bool olderExhausted: false
    property bool loadingNewer: false
    // False only after the newest messages were dropped from the window.
    property bool newerExhausted: true
    // Counts the windows: a page that was asked for in an earlier one is thrown away.
    property int loadGeneration: 0

    function loadOlder() {
        if (loadingOlder || olderExhausted || !shown || messages.count === 0) return
        loadingOlder = true
        var generation = page.loadGeneration
        backend.call("messages",
                     { identity: identity, before: messages.get(messages.count - 1).id, limit: pageSize },
                     function(ok, result) {
            if (generation !== page.loadGeneration) return
            if (ok) {
                var older = result.messages.reverse()
                if (older.length === 0) page.olderExhausted = true
                // The whole page as one insert: row by row, the view laid itself out again
                // for every single message while the reader was scrolling (user report
                // 2026-10-29).
                messages.insertAll(older, messages.count)
                page.dropNewest()
                page.updateStickyDay()
            }
            page.loadingOlder = false
        })
    }

    // Scrolling forward again after the newest messages were dropped.
    function loadNewer() {
        if (loadingNewer || newerExhausted || !shown || messages.count === 0) return
        loadingNewer = true
        var generation = page.loadGeneration
        backend.call("messages",
                     { identity: identity, after: messages.get(0).id, limit: pageSize },
                     function(ok, result) {
            if (generation !== page.loadGeneration) return
            if (ok) {
                var newer = result.messages.reverse()
                if (newer.length === 0) page.newerExhausted = true
                messages.insertAll(newer, 0)
                page.dropOldest()
                page.updateStickyDay()
            }
            page.loadingNewer = false
        })
    }

    // Out of the window at the newest end (the reader is scrolling back): they load again
    // from the chat's own history when the reader comes forward. Never anything in view or
    // just below it — the list is bottom to top, so the newest visible message has the
    // smallest index.
    function dropNewest() {
        if (messages.count <= windowSize + windowSlack) return
        var newestVisible = listView.indexAt(listView.width / 2, listView.contentY + listView.height - 1)
        if (newestVisible < 0) return
        var drop = Math.min(messages.count - windowSize, newestVisible - pageSize)
        if (drop <= 0) return
        messages.dropFirst(drop)
        page.newerExhausted = false
        page.stickyIndex = -1
    }

    // The same at the oldest end (the reader is scrolling forward).
    function dropOldest() {
        if (messages.count <= windowSize + windowSlack) return
        var oldestVisible = listView.indexAt(listView.width / 2, listView.contentY)
        if (oldestVisible < 0) return
        var keep = Math.max(windowSize, oldestVisible + pageSize)
        if (messages.count <= keep) return
        messages.truncate(keep)
        page.olderExhausted = false
        page.stickyIndex = -1
    }

    // The app put aside: Qt drops the window's scene graph by itself, and nothing is drawn
    // for a while, so this is the moment to let the JavaScript side go too.
    Connections {
        target: Qt.application
        onStateChanged: if (Qt.application.state !== Qt.ApplicationActive && page.status === PageStatus.Active) gc()
    }

    // Silica's quick scroll to the bottom, or any other way back to the newest message, while
    // the newest ones are not loaded: fetch them again — one page, which is what opening the
    // chat does anyway.
    function backToNewest() {
        if (newerExhausted || loadingNewer) return
        page.load()
    }

    function markRead() {
        if (!isList && Qt.application.state === Qt.ApplicationActive && page.status === PageStatus.Active) {
            backend.call("markRead", { identity: identity }, function() { backend.refreshUnread() })
        }
    }

    // The edit stays in the input, and the mode with it, until the core has it: a refused one
    // is not lost, and can be changed or cancelled.
    function sendEdit(text) {
        var messageId = editingMessageId
        var sent = inputField.text
        sendingText = true
        backend.call("editMessage", { identity: identity, messageId: messageId, text: text },
                     function(ok, result, kind, message) {
            page.sendingText = false
            if (!ok) {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                return
            }
            if (result) page.addMessage(result)
            if (page.editingMessageId === messageId && page.inputField.text === sent) page.endEdit()
        })
    }

    function deleteMessage(rowId, messageId, outgoing, everyone) {
        backend.call("deleteMessage", { identity: identity, messageId: messageId, outgoing: outgoing,
                                         everyone: everyone }, function(ok, result, kind, message) {
            if (!ok) {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            } else if (result.removed) {
                messages.remove(rowId)
            } else {
                page.addMessage(result)
            }
        })
    }

    // The whole chat as a ZIP in Downloads (Android's "Export chat"). The core writes the file;
    // the words in it — "Me" and the times' offset — come from here, like all texts.
    function exportChat(media, password) {
        //% "Exporting chat"
        Notices.show(qsTrId("threesea-la-exporting-chat"), Notice.Short, Notice.Center)
        backend.call("exportChat", {
            identity: page.identity,
            media: media,
            password: password,
            directory: StandardPaths.download,
            //% "Me"
            me: qsTrId("threesea-la-mention-me"),
            offsetMinutes: -new Date().getTimezoneOffset()
        }, function(ok, result, kind, message) {
            if (ok) {
                //% "Saved to Downloads"
                Notices.show(qsTrId("threesea-la-saved-downloads"), Notice.Short, Notice.Center)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    // Save a file to Downloads; then open it with the system's app, or tell where it is.
    function saveFile(id, open) {
        backend.call("saveFile", { message: id, directory: StandardPaths.download },
                     function(ok, result, kind, message) {
            if (!ok) {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            } else {
                //% "Saved to Downloads"
                Notices.show(qsTrId("threesea-la-saved-downloads"), Notice.Short, Notice.Center)
                // Documents and media open in the app registered for the type (e.g. Documents
                // for PDFs), if any; the core decides by the name the file has now.
                if (open && result.openable) Qt.openUrlExternally("file://" + result.path)
            }
        })
    }

    // A command whose result is the changed message.
    function run(command, args) {
        backend.call(command, args, function(ok, result, kind, message) {
            if (ok) {
                if (result) page.addMessage(result)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    function createPoll() {
        var dialog = pageStack.push(Qt.resolvedUrl("CreatePollDialog.qml"))
        dialog.accepted.connect(function() {
            backend.call("createPoll", { identity: identity, description: dialog.question.trim(),
                                         choices: dialog.choices, multiple: dialog.multiple,
                                         announce: dialog.announce }, function(ok, result, kind, message) {
                if (ok) {
                    page.addMessage(result)
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    function react(messageId, emoji, apply) {
        backend.call("react", { identity: identity, messageId: messageId, emoji: emoji, apply: apply },
                     function(ok, result, kind, message) {
            if (ok) {
                if (result) page.addMessage(result)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    // Who wrote a message, for quotes.
    function senderNameOf(message) {
        if (message.outgoing) {
            //% "You"
            return qsTrId("threesea-la-you")
        }
        if (isGroup) return memberName(message.sender)
        return contact.displayName || identity
    }

    function indexOfMessageId(messageId) {
        for (var i = 0; i < messages.count; ++i) {
            if (messages.get(i).messageId === messageId) return i
        }
        return -1
    }

    // The quoted message with its sender's name, or null (not here; then fetched once).
    function quotedFor(quoteId, revision) {
        if (!quoteId) return null
        var index = indexOfMessageId(quoteId)
        var message = index >= 0 ? messages.get(index) : quoteCache[quoteId]
        if (message === undefined) {
            quoteCache[quoteId] = false
            backend.call("quotedMessage", { identity: identity, messageId: quoteId }, function(ok, result) {
                if (ok && result) {
                    page.quoteCache[quoteId] = result
                    page.quoteCacheRevision++
                }
            })
            return null
        }
        if (!message) return null
        var copy = JSON.parse(JSON.stringify(message))
        copy.senderName = senderNameOf(message)
        return copy
    }

    function startQuote(message) {
        if (editingMessageId !== "") endEdit()
        var copy = JSON.parse(JSON.stringify(message))
        copy.senderName = senderNameOf(message)
        quoting = copy
        refocus.restart()
    }

    function showQuoted(quoteId) {
        var index = indexOfMessageId(quoteId)
        if (index >= 0) listView.positionViewAtIndex(index, ListView.Center)
    }

    // Mentions (groups): the input shows "@Name", sending turns it back into "@[ID]".
    property var composeMentions: ({})
    // The "@…" being typed before the cursor, or null.
    property var mentionQuery: null
    function updateMentionQuery() {
        if (!isGroup || !inputField || recordingHere) {
            mentionQuery = null
            return
        }
        var before = inputField.text.substring(0, inputField.cursorPosition)
        var match = /(^|\s)@([^\s@\[]*)$/.exec(before)
        mentionQuery = match ? { start: before.length - match[2].length - 1, text: match[2].toLowerCase() } : null
    }
    function mentionCandidates(query) {
        if (!query) return []
        var result = []
        var members = contact.members || []
        for (var i = 0; i < members.length; ++i) {
            var member = members[i]
            if (member.identity === backend.identity) continue
            var name = member.displayName.replace(/^~/, "")
            if (query.text.length === 0 || name.toLowerCase().indexOf(query.text) >= 0
                    || member.identity.toLowerCase().indexOf(query.text) === 0) {
                result.push({ identity: member.identity, name: member.displayName })
            }
        }
        result.sort(function(a, b) { return a.name.localeCompare(b.name) })
        //% "All"
        var all = qsTrId("threesea-la-mention-all")
        if (all.toLowerCase().indexOf(query.text) === 0) result.push({ identity: "@@@@@@@@", name: all })
        return result.slice(0, 5)
    }
    function insertMention(identity, name) {
        // The keyboard may still hold the typed letters as a prediction: take them first.
        Qt.inputMethod.commit()
        updateMentionQuery()
        if (!mentionQuery) return
        var text = inputField.text
        var tag = "@" + name
        mentionCursor.position = mentionQuery.start + tag.length + 1
        inputField.text = text.substring(0, mentionQuery.start) + tag + " " + text.substring(inputField.cursorPosition)
        mentionCursor.restart()
        var map = composeMentions
        map[tag] = identity
        composeMentions = map
        mentionQuery = null
    }
    Timer {
        // After the keyboard has caught up with the changed text.
        id: mentionCursor
        property int position
        interval: 1
        onTriggered: page.inputField.cursorPosition = position
    }
    function rememberMentions(raw, mentions) {
        var map = {}
        var tags = (raw || "").match(/@\[[0-9A-Z*@]{8}\]/g) || []
        for (var i = 0; i < tags.length; ++i) map[Fmt.withMentions(tags[i], mentions)] = tags[i].substring(2, 10)
        composeMentions = map
    }
    function encodeMentions(text, mentions) {
        var map = mentions || composeMentions
        var tags = Object.keys(map).sort(function(a, b) { return b.length - a.length })
        for (var i = 0; i < tags.length; ++i) {
            text = text.split(tags[i]).join("@[" + map[tags[i]] + "]")
        }
        return text
    }

    // Text put into the input by the app rather than typed: nobody is told "typing" for it, and
    // the cursor goes to its end, for typing on or the emoji panel.
    property bool settingInput: false
    function setInput(text) {
        settingInput = true
        inputField.text = text
        settingInput = false
        inputField.cursorPosition = text.length
    }

    // Editing pulls the message back into the input box; the draft comes back afterwards.
    function startEdit(messageId, raw, mentions) {
        if (sendingText) return
        stopTyping()
        if (editingMessageId === "") {
            beforeEdit = { text: inputField.text, mentions: composeMentions, quoting: quoting }
        }
        quoting = null
        editingMessageId = messageId
        rememberMentions(raw, mentions)
        setInput(Fmt.withMentions(raw, mentions))
        refocus.restart()
    }

    function endEdit() {
        var before = beforeEdit
        editingMessageId = ""
        beforeEdit = null
        composeMentions = before.mentions
        quoting = before.quoting
        setInput(before.text)
    }

    // The emoji panel in place of the keyboard (EmojiPanel). Its height is the keyboard's when
    // known, so switching between the two doesn't move the chat.
    property bool emojiPanelOpen: false
    property real keyboardHeight: Math.round(Screen.height * 0.38)
    // Inserting a message row (someone wrote while the user types) steals the input's focus
    // and closes the keyboard. That steal takes the focus with it, while the user's own
    // dismissal keeps it on the input (FocusBehavior.KeepFocus), so a close without focus
    // right after an insert is undone and the keyboard stays open (user report 2026-10-28).
    property double lastInsertAt: 0
    Connections {
        target: Qt.inputMethod
        onKeyboardRectangleChanged: {
            var h = Qt.inputMethod.keyboardRectangle.height
            if (Qt.inputMethod.visible && h > Theme.itemSizeLarge) {
                page.keyboardHeight = h
            } else if (h === 0 && !page.emojiPanelOpen && page.status === PageStatus.Active
                       && page.inputField && !page.inputField.activeFocus
                       && Date.now() - page.lastInsertAt < 1000) {
                refocus.restart()
            }
        }
    }
    // A bubble's menu → "+": the panel picks a reaction for that message instead of typing.
    property var reactionTarget: null
    onEmojiPanelOpenChanged: if (!emojiPanelOpen) reactionTarget = null
    function pickReaction(message) {
        reactionTarget = message
        emojiPanel.load()
        inputField.focus = false
        Qt.inputMethod.hide()
        emojiPanelOpen = true
    }
    function toggleEmojiPanel() {
        if (emojiPanelOpen) {
            emojiPanelOpen = false
            inputField.forceActiveFocus()
            Qt.inputMethod.show()
        } else {
            emojiPanel.load()
            inputField.focus = false
            Qt.inputMethod.hide()
            emojiPanelOpen = true
        }
    }
    function insertEmoji(emoji) {
        var at = inputField.cursorPosition
        inputField._editor.insert(at, emoji)
        inputField.cursorPosition = at + emoji.length
    }
    // Backspace in the panel: the emoji (or character) before the cursor.
    function deleteBeforeCursor() {
        var at = inputField.cursorPosition
        if (at <= 0) return
        var text = inputField.text
        var ranges = Fmt.emojiRanges(text)
        var from = at - 1
        for (var i = 0; i < ranges.length; ++i) {
            if (ranges[i].start + ranges[i].length === at) from = ranges[i].start
        }
        var code = text.charCodeAt(from)
        if (from === at - 1 && code >= 0xDC00 && code <= 0xDFFF && from > 0) from -= 1
        inputField._editor.remove(from, at)
        inputField.cursorPosition = from
    }

    // A favourite GIF picked in the emoji panel's GIF tab.
    function sendGif(id) {
        backend.call("sendGifFavourite", { identity: identity, favourite: id }, function(ok, result, kind, message) {
            if (ok) {
                page.addMessage(result)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    function attach(path, photo, name, video) {
        attachAll([{ path: path, photo: photo, name: name, video: video === true }])
    }
    // Adds to what is already picked.
    function attachAll(files) {
        var list = attachments.slice()
        for (var i = 0; i < files.length; ++i) {
            if (files[i].path) list.push(files[i])
        }
        if (list.length === attachments.length) return
        if (editingMessageId !== "") endEdit()
        attachments = list
        refocus.restart()
    }
    // Cut and turn a picked photo (EditPhotoPage); the edited copy takes its place.
    function editAttachment(index) {
        var file = page.attachments[index]
        if (!file || !file.photo) return
        var editor = pageStack.push(Qt.resolvedUrl("EditPhotoPage.qml"), { path: file.path })
        editor.accepted.connect(function() {
            if (!editor.editedPath) {
                //% "The photo could not be edited."
                Notices.show(qsTrId("threesea-la-edit-photo-failed"), Notice.Long, Notice.Center)
                return
            }
            // By its path, not by the index: what is attached may have changed meanwhile.
            var list = page.attachments.slice()
            var at = -1
            for (var i = 0; i < list.length; ++i) {
                if (list[i].path === file.path) at = i
            }
            if (at < 0) {
                // It was dropped while the editor was open: don't leave the copy behind.
                backend.removeCapture(editor.editedPath)
                return
            }
            // The original stays where it is; a capture of ours it replaces is removed.
            backend.removeCapture(list[at].path)
            list[at] = { path: editor.editedPath, photo: true, name: file.name, video: false }
            page.attachments = list
        })
    }

    function dropAttachments() {
        for (var i = 0; i < attachments.length; ++i) backend.removeCapture(attachments[i].path)
        attachments = []
    }
    // A multi-select picker's choice (its selectedContent): videos as videos, pictures as
    // pictures (from the image picker), everything else as files.
    function attachPicked(content, kind) {
        var files = []
        for (var i = 0; i < content.count; ++i) {
            var item = content.get(i)
            files.push({ path: item.filePath, name: item.fileName, photo: kind === "image",
                         video: kind === "video" })
        }
        attachAll(files)
    }

    function sendAttachment() {
        var caption = encodeMentions(page.inputField.text.trim())
        composeMentions = {}
        var queue = sendQueue.slice()
        for (var i = 0; i < attachments.length; ++i) {
            var file = attachments[i]
            queue.push({ path: file.path, photo: file.photo, video: file.video, caption: i === 0 ? caption : "" })
        }
        sendQueue = queue
        attachments = []
        setInput("")
        // Sent: no draft left, even if the app ends before the page saves it.
        saveDraft()
        if (!sendingQueue) sendNext()
    }
    function sendNow(file) {
        backend.sendFile(identity, file.path, file.photo, file.caption)
    }
    function sendNext() {
        if (sendQueue.length === 0) {
            sendingQueue = false
            return
        }
        sendingQueue = true
        var queue = sendQueue.slice()
        var file = queue.shift()
        sendQueue = queue
        if (file.video) {
            videoPreparer.start(file.path, file.caption)
            return
        }
        backend.sendFile(identity, file.path, file.photo, file.caption, function(ok, result, kind, message) {
            if (ok) {
                page.addMessage(result)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
            page.sendNext()
        })
    }

    // Voice messages: the send button records while the input is empty (Threema), a second tap
    // sends; the recording bar's ✕ throws it away. The recorder is the app's, the recording this
    // page's: the bar and the button, and sending it, are for the chat it was started in.
    readonly property bool recordingHere: voiceRecorder.owner === page
    readonly property bool canRecord: voiceRecorder.available && !voiceRecorder.owner && !window.anyCallRunning
                                      && !attachment && !quoting && editingMessageId === ""
                                      && inputField && inputField.text.trim().length === 0
    // A recording went while nobody was looking: said when the app is back.
    property bool recordingDiscarded

    // Talking for a long time without touching the phone must not blank it: that would leave the
    // app, and a lock (or a recording's end) has nobody to ask then. Once the app has been left,
    // the phone blanks and locks as usual.
    DisplayBlanking {
        preventBlanking: page.recordingHere && voiceRecorder.recording
                         && Qt.application.state === Qt.ApplicationActive
    }

    // ---- Voice messages at the ear, their queue and their "new" state (user requests
    // 2026-10-28, like Threema for Android) ----
    function isVoiceMessage(m) {
        return m.kind === "file" && m.body && m.body.rendering !== "file"
               && String(m.body.mediaType).indexOf("audio/") === 0
    }
    function isPlayableVoice(m) {
        return isVoiceMessage(m) && m.body.fileState === 2
    }
    // Play a voice message and remember that it is not new any more.
    function playVoice(id) {
        if (!voicePlayer.play(id)) {
            //% "Can't play voice messages on this device"
            Notices.show(qsTrId("threesea-la-cannot-play"), Notice.Long, Notice.Center)
            return
        }
        window.voiceFrom = page.isPrivate ? "" : (page.contact.displayName || page.identity)
        var index = indexOfLocalId(id)
        if (index < 0) return
        var m = messages.get(index)
        if (!m.outgoing && m.body && m.body.listened !== true) {
            backend.call("setVoiceListened", { message: id }, function(ok, result) {
                if (ok && result) {
                    page.addMessage(result)
                    page.voiceRevision++
                }
            })
        }
    }
    function indexOfLocalId(id) {
        for (var i = 0; i < messages.count; ++i) {
            if (messages.get(i).id === id) return i
        }
        return -1
    }
    // The chat ends in voice messages: the oldest of that run not listened to yet, or -1.
    // Any other message breaks the run (the queue never jumps past text).
    property int voiceRevision: 0
    readonly property int firstUnheardVoice: {
        voiceRevision
        messages.count
        var candidate = -1
        for (var i = 0; i < messages.count; ++i) {
            var m = messages.get(i)
            if (!isVoiceMessage(m)) break
            if (!m.outgoing && m.body.listened !== true && m.body.fileState === 2) candidate = m.id
        }
        return candidate
    }
    // The proximity sensor runs only while it could start or carry playback here, the setting
    // allows it, and this build can reach the earpiece at all (Full only).
    Binding {
        target: proximity
        property: "enabled"
        value: page.status === PageStatus.Active && Qt.application.state === Qt.ApplicationActive
               && page.shown && backend.earToPlay && voicePlayer.earpieceAvailable
               && (voicePlayer.messageId >= 0 || page.firstUnheardVoice >= 0)
        when: page.status === PageStatus.Active
    }
    Connections {
        // Only the chat the user is in reacts (another chat may sit below in the stack).
        target: page.status === PageStatus.Active ? proximity : null
        onCoveredChanged: {
            if (proximity.covered) {
                // At the ear: earpiece at call volume; start the unheard queue, or carry on.
                // A message paused by hand stays paused: this can be a pocket as well as an ear.
                if (voicePlayer.messageId >= 0) {
                    if (voicePlayer.playing) {
                        voicePlayer.earpiece = true
                    } else if (page.voicePausedAtEar) {
                        voicePlayer.earpiece = true
                        voicePlayer.resume()
                    }
                } else if (page.firstUnheardVoice >= 0) {
                    voicePlayer.earpiece = true
                    page.playVoice(page.firstUnheardVoice)
                }
            } else {
                // Taken off the ear: pause, back to the loudspeaker.
                if (voicePlayer.earpiece && voicePlayer.playing) {
                    voicePlayer.pause()
                    page.voicePausedAtEar = Qt.application.state === Qt.ApplicationActive
                }
                voicePlayer.earpiece = false
            }
        }
    }
    // Paused by taking the phone off the ear, so that bringing it back carries on.
    property bool voicePausedAtEar: false
    // One voice message ended: a soft chime, then the directly following one, if any.
    property int voiceQueueNext: -1
    Connections {
        target: voicePlayer
        onPlayingChanged: if (voicePlayer.playing) page.voicePausedAtEar = false
        // Another message was started by hand during the chime: that one goes first.
        onMessageIdChanged: if (voicePlayer.messageId >= 0) page.voiceQueueNext = -1
        onFinished: {
            // The next one waits behind the lock.
            if (window.locked) return
            var index = page.indexOfLocalId(message)
            if (index <= 0) return
            var next = messages.get(index - 1)
            if (!page.isPlayableVoice(next)) return
            page.voiceQueueNext = next.id
            voicePlayer.chime()
        }
        onChimeDone: {
            var id = page.voiceQueueNext
            page.voiceQueueNext = -1
            if (id >= 0) page.playVoice(id)
        }
    }
    // While the phone is at the ear the screen shows nothing and takes no taps (the sensor
    // cannot turn it off; this is what Android apps do with the proximity wake lock).
    Rectangle {
        z: 1000
        anchors.fill: parent
        color: "black"
        visible: proximity.covered && voicePlayer.earpiece
        MouseArea {
            anchors.fill: parent
            enabled: parent.visible
        }
    }

    function sendRecording(duration) {
        backend.sendVoice(page.identity, duration, function(ok, result, kind, message) {
            if (ok) {
                page.addMessage(result)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }
    function sayRecordingDiscarded() {
        recordingDiscarded = false
        //% "Recording stopped and discarded"
        Notices.show(qsTrId("threesea-la-recording-discarded"), Notice.Long, Notice.Center)
    }
    function recordingWentUnasked() {
        if (Qt.application.state === Qt.ApplicationActive) sayRecordingDiscarded()
        else recordingDiscarded = true
    }

    Connections {
        target: voiceRecorder
        onFinished: if (page.recordingHere) page.sendRecording(duration)
        onHeldChanged: {
            if (voiceRecorder.held && page.recordingHere) {
                //% "Recording stopped. Send it or discard it."
                Notices.show(qsTrId("threesea-la-recording-held"), Notice.Long, Notice.Center)
            }
        }
        onDiscarded: {
            if (page.recordingHere) page.recordingWentUnasked()
        }
        onFailed: {
            if (!page.recordingHere) return
            //% "Recording failed"
            Notices.show(qsTrId("threesea-la-recording-failed"), Notice.Long, Notice.Center)
        }
    }

    function sendLocation(location, name) {
        backend.call("sendLocation", { identity: identity, latitude: location.latitude,
                                       longitude: location.longitude, accuracy: location.accuracy,
                                       name: name || null }, function(ok, result, kind, message) {
            if (ok) {
                page.addMessage(result)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    // Received locations open in a maps app (geo: link); without one, say so.
    function openLocation(location) {
        if (!Qt.openUrlExternally(Fmt.geoUri(location))) {
            //% "No maps app installed. Long press to copy the coordinates."
            Notices.show(qsTrId("threesea-la-no-maps-app"), Notice.Long, Notice.Center)
        }
    }

    // A text on its way to the core. It stays in the input until the core has accepted it, as
    // does its quote, so a refused one is still there; one send at a time.
    property bool sendingText: false
    function textAccepted(sent, quoteId, sentDraft) {
        // The core took the draft along with it, if it is still that one.
        if (savedDraft === sentDraft) savedDraft = noDraft
        if (quoting && quoting.messageId === quoteId) quoting = null
        var text = inputField.text
        if (text === sent) {
            composeMentions = {}
            setInput("")
        } else if (text.indexOf(sent) === 0) {
            // Typed on meanwhile: that stays.
            setInput(text.substring(sent.length).replace(/^\s+/, ""))
        }
        // Keep writing, like Jolla Messages: refocus once the list has inserted the message.
        if (status === PageStatus.Active) refocus.restart()
    }

    function send() {
        // The keyboard may still hold the last word as a prediction.
        Qt.inputMethod.commit()
        if (sendingText) return
        if (recordingHere) {
            if (voiceRecorder.held) sendRecording(voiceRecorder.duration)
            else voiceRecorder.stop()
            return
        }
        if (canRecord) {
            voicePlayer.stop()
            voiceRecorder.start(page)
            return
        }
        var sent = page.inputField.text
        var text = encodeMentions(sent.trim())
        // An edit is never a caption, whatever waits to be sent.
        if (editingMessageId !== "") {
            if (text.length > 0) sendEdit(text)
            return
        }
        if (attachment) {
            sendAttachment()
            return
        }
        if (text.length === 0) return
        // In the draft first: if the send is refused, or the app ends, the words are still there.
        // The core takes the draft away when it accepts the text.
        saveDraft()
        var sentDraft = savedDraft
        var args = { identity: identity, text: text }
        var quoteId = quoting ? quoting.messageId : ""
        if (quoting) args.quote = quoteId
        sendingText = true
        backend.call("sendText", args, function(ok, result, kind, message) {
            page.sendingText = false
            if (ok) {
                page.addMessage(result)
                // Whoever gets an answer is no stranger any more.
                page.spamPotential = false
                page.textAccepted(sent, quoteId, sentDraft)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    Timer {
        id: refocus
        interval: 50
        onTriggered: {
            page.inputField.forceActiveFocus()
            Qt.inputMethod.show()
        }
    }

    // A group call runs in this group (Full: group calls, ADR 0012): the banner under the header.
    property bool groupCallRunning: false
    property var groupCall: ({})
    readonly property bool inGroupCall: groupCalls.conversation === identity && groupCalls.state !== "idle"
    property real now: Date.now()
    Connections {
        target: groupCalls
        onRunningChanged: {
            page.groupCallRunning = groupCalls.running(page.identity)
            page.groupCall = groupCalls.call(page.identity)
        }
    }
    // The _Group Call Refresh Steps_ every 10 s while a call runs and the chat shows; the
    // banner's time every second.
    Timer {
        running: page.isGroup && page.groupCallRunning && page.status === PageStatus.Active
                 && Qt.application.active && groupCalls.supported
                 && window.appSettings.groupCalls !== false
        interval: 10000
        repeat: true
        onTriggered: groupCalls.refresh(page.identity)
    }
    Timer {
        running: page.groupCallRunning && page.status === PageStatus.Active && Qt.application.active
        interval: 1000
        repeat: true
        triggeredOnStart: true
        onTriggered: page.now = Date.now()
    }
    function openGroupCall() {
        if (!page.inGroupCall) groupCalls.join(page.identity)
        pageStack.push(Qt.resolvedUrl("GroupCallPage.qml"), { conversation: page.identity })
    }

    // A chat that may be spam offers a report under the header (docs/protocol-notes.md).
    property bool spamPotential: false
    function checkSpam() {
        backend.call("spamPotential", { identity: identity }, function(ok, result) {
            page.spamPotential = ok && result.potential
        })
    }
    function openSpamReport() {
        var dialog = pageStack.push(Qt.resolvedUrl("SpamReportDialog.qml"), {
            name: page.contact.displayName || page.identity,
            isGroup: page.isGroup
        })
        dialog.accepted.connect(function() {
            if (dialog.block) {
                //% "Reporting and deleting"
                spamRemorse.execute(qsTrId("threesea-la-spam-reporting-deleting"), function() {
                    page.reportSpam(true)
                })
            } else {
                page.reportSpam(false)
            }
        })
    }
    function reportSpam(block) {
        backend.call("reportSpam", { identity: identity, block: block }, function(ok, result, kind, message) {
            if (!ok) {
                // The server's refusals (the ID is gone, say) say nothing a user can act on.
                Notices.show(kind === "network" ? Errors.text(kind, message)
                                                //% "The report could not be sent."
                                                : qsTrId("threesea-la-spam-report-failed"),
                             Notice.Long, Notice.Center)
                return
            }
            page.spamPotential = false
            //% "Reported to Threema as spam."
            Notices.show(qsTrId("threesea-la-spam-reported"), Notice.Long, Notice.Center)
            if (block) {
                pageStack.pop(pageStack.find(function(p) { return p.objectName === "conversations" }))
            }
        })
    }
    RemorsePopup { id: spamRemorse }

    Component.onCompleted: {
        window.openChat = identity
        load()
        checkSpam()
        if (isGroup && groupCalls.supported) {
            groupCallRunning = groupCalls.running(identity)
            groupCall = groupCalls.call(identity)
            groupCalls.refresh(identity)
        }
        // The unsent text from last time, as it was: with its mentions and what it quotes (a
        // shared/linked text set before wins).
        backend.call("draft", { identity: identity }, function(ok, result) {
            page.draftLoaded = ok && !!page.inputField
            if (!page.draftLoaded || !result.text) return
            var quote = result.quote
            page.savedDraft = page.draftKey(result.text, quote ? quote.messageId : "")
            if (page.inputField.text.length > 0) return
            page.rememberMentions(result.text, result.mentions)
            page.setInput(Fmt.withMentions(result.text, result.mentions))
            if (quote && !page.attachment) {
                quote.senderName = page.senderNameOf(quote)
                page.quoting = quote
            }
        })
    }
    // Keep what is typed but not sent, the way it would go out (not while editing a message:
    // that is not a draft), from the moment it is known what is stored: an empty input before
    // that would delete a draft that has not even been shown. Saved a moment after each change,
    // as an app killed or crashed in the background says no goodbye.
    property bool draftLoaded: false
    // What the core has stored, so that an unchanged input doesn't write again.
    readonly property string noDraft: draftKey("", "")
    property string savedDraft: noDraft
    function draftKey(text, quoteId) {
        return JSON.stringify([text, quoteId])
    }
    function saveDraft() {
        draftSaver.stop()
        if (!inputField) return
        var editing = editingMessageId !== ""
        var text = editing ? encodeMentions(beforeEdit.text, beforeEdit.mentions)
                           : encodeMentions(inputField.text)
        var quote = editing ? beforeEdit.quoting : quoting
        if (!draftLoaded && text.length === 0) return
        var key = draftKey(text, quote ? quote.messageId : "")
        if (key === savedDraft) return
        savedDraft = key
        var args = { identity: identity, text: text }
        if (quote) args.quote = quote.messageId
        backend.call("setDraft", args)
    }
    Timer {
        id: draftSaver
        interval: 700
        onTriggered: page.saveDraft()
    }
    onQuotingChanged: draftSaver.restart()
    Component.onDestruction: {
        // Playing on is fine, the system's media controls have it; a paused message would only
        // stay in the way, and a private chat's is hidden with the chat.
        if (!voicePlayer.playing || isPrivate) voicePlayer.stop()
        voicePlayer.earpiece = false
        dropAttachments()
        // Files still waiting to be sent go to the core at once; a video's thumbnail is made here,
        // so a waiting video goes as a file.
        for (var i = 0; i < sendQueue.length; ++i) sendNow(sendQueue[i])
        if (videoPreparer.path.length > 0) {
            sendNow({ path: videoPreparer.path, photo: false, caption: videoPreparer.caption })
        }
        saveDraft()
        stopTyping()
        // Going back to this same chat further down, that one is the open one already.
        var below = pageStack.currentPage
        var sameChat = below && below !== page && below.identity === identity && below.inputField
        if (window.openChat === identity && !sameChat) window.openChat = ""
        // The window of messages goes with the page; clearing it here gives those rows back
        // at once (user question 2026-10-30). No collection run: that would cost a moment
        // exactly while the chat is closing, and the app has an idle one anyway.
        messages.clear()
        // Leaving a private chat hides the private chats again (unless their list is underneath).
        if (isPrivate && !pageStack.find(function(p) { return p.objectName === "private" })) backend.call("lock")
    }
    onStatusChanged: {
        if (status === PageStatus.Deactivating) {
            stopTyping()
            saveDraft()
        }
        // Whatever comes on top, it isn't this chat any more: nothing here shows the microphone
        // is on, and nothing there could say where the recording would go. Nobody asked for that
        // here, so it is said, at once: going back, this page is about to go.
        if (status === PageStatus.Inactive && recordingHere) {
            voiceRecorder.cancel()
            recordingWentUnasked()
        }
        if (status === PageStatus.Active) {
            // Also when coming back from a chat opened on top of this one (from a group's
            // member list): that one's closing left no chat open, and messages here would ring.
            window.openChat = identity
            markRead()
            // Back from the contact page: blocked or not may have changed.
            if (!isGroup) loadContact()
            // Contact details are attached: swipe left (forward indicator top right).
            if (pageStack.nextPage(page) === null) {
                pageStack.pushAttached(Qt.resolvedUrl(isGroup ? "GroupPage.qml"
                                                      : isList ? "DistributionListPage.qml" : "ContactPage.qml"),
                                       { identity: page.identity })
            }
        }
    }

    Connections {
        target: Qt.application
        onStateChanged: {
            if (Qt.application.state !== Qt.ApplicationActive) {
                page.stopTyping()
                page.saveDraft()
            } else if (page.recordingDiscarded) {
                page.sayRecordingDiscarded()
            }
            page.markRead()
        }
    }

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "messageAdded" && data.message.conversation === page.identity) {
                page.lastInsertAt = Date.now()
                page.addMessage(data.message)
                page.typing = false
                page.markRead()
                page.checkSpam()
            } else if (name === "messagesChanged") {
                for (var i = 0; i < data.messages.length; ++i) {
                    if (data.messages[i].conversation === page.identity) {
                        page.addMessage(data.messages[i])
                    }
                }
            } else if (name === "typing" && data.identity === page.identity) {
                page.typing = data.typing
                typingTimeout.restart()
            } else if (name === "contactsChanged" || name === "groupsChanged") {
                page.loadContact()
            } else if (name === "chatsChanged") {
                // Bulk changes (a backup import): reload the whole page instead of patching.
                page.load()
            }
        }
    }

    // Our own typing indicator (Threema spec): "typing" on the first keystroke, repeated every
    // 10 s while typing goes on; "stopped" after 5 s without a keystroke, when the input is
    // emptied (also by sending) or when the chat is left.
    function userTyped() {
        // Editing a message must not show "typing" (spec); lists have nobody to tell, and a
        // chat without an input has nobody typing.
        if (editingMessageId !== "" || isList || readOnly) return
        if (page.inputField.text.length === 0) {
            stopTyping()
            return
        }
        if (settingInput) return
        typingIdle.restart()
        if (!typingRepeat.running) {
            backend.call("setTyping", { identity: identity, typing: true })
            typingRepeat.start()
        }
    }

    function stopTyping() {
        typingIdle.stop()
        if (!typingRepeat.running) return
        typingRepeat.stop()
        backend.call("setTyping", { identity: identity, typing: false })
    }

    Timer {
        id: typingRepeat
        interval: 10000
        repeat: true
        onTriggered: backend.call("setTyping", { identity: page.identity, typing: true })
    }

    Timer {
        id: typingIdle
        interval: 5000
        onTriggered: page.stopTyping()
    }

    // Typing indicators expire after 15 s without an update (libthreema provider contract).
    Timer {
        id: typingTimeout
        interval: 15000
        onTriggered: page.typing = false
    }

    // Jolla Messages: the name (and status) stays on top, the contact's avatar to its right.
    // The page itself carries the pulley: the message list runs bottom to top, so a pulley in it
    // would only appear above the oldest message. Pulling down on the header opens it (and
    // anywhere in a chat that does not fill the screen).
    Timer {
        id: askPinLater
        interval: 50
        repeat: true
        onTriggered: {
            if (pageStack.busy || window.stackHeld) return
            stop()
            page.askPin()
        }
    }

    // Chat background (Settings → Appearance), like Android's wallpaper: behind everything,
    // under the theme's own dimmer so the bubbles stay readable in every Ambience.
    Image {
        id: wallpaper
        visible: page.shown && backend.wallpaper !== ""
        anchors.fill: parent
        source: visible ? "file://" + backend.wallpaper : ""
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        clip: true
        Rectangle {
            anchors.fill: parent
            color: Theme.rgba(Theme.highlightDimmerColor, Fmt.alpha(0.55, backend.emulator))
        }
    }

    SilicaFlickable {
        visible: page.shown
        id: pageFlickable
        anchors.fill: parent
        contentHeight: height
        // Not while the emoji panel is open: the pulleys would take its swipe down (which closes it).
        interactive: !page.emojiPanelOpen

        PullDownMenu {
            MenuItem {
                visible: page.selecting
                //% "Cancel selection"
                text: qsTrId("threesea-me-cancel-selection")
                onClicked: page.endSelection()
            }
            MenuItem {
                visible: !page.selecting
                //% "Select messages"
                text: qsTrId("threesea-me-select-messages")
                onClicked: page.selecting = true
            }
            MenuItem {
                visible: !page.selecting
                //% "Search"
                text: qsTrId("threesea-me-search")
                onClicked: pageStack.push(Qt.resolvedUrl("SearchPage.qml"),
                                          { identity: page.identity,
                                            chatName: page.contact.displayName || page.identity })
            }
            MenuItem {
                visible: !page.selecting
                //% "Export chat"
                text: qsTrId("threesea-me-export-chat")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("ExportChatDialog.qml"),
                                                { chatName: page.contact.displayName || page.identity })
                    dialog.accepted.connect(function() {
                        page.exportChat(dialog.media, dialog.password)
                    })
                }
            }
            MenuItem {
                visible: !page.selecting && page.isGroup && !page.groupCallRunning && !page.readOnly
                         && groupCalls.supported && window.appSettings.threemaCalls !== false
                         && window.appSettings.groupCalls !== false
                enabled: groupCalls.state === "idle"
                //% "Start group call"
                text: qsTrId("threesea-me-start-group-call")
                onClicked: {
                    groupCalls.join(page.identity, true)
                    pageStack.push(Qt.resolvedUrl("GroupCallPage.qml"), { conversation: page.identity })
                }
            }
            MenuItem {
                visible: !page.selecting && page.isGroup && page.groupCallRunning && !page.inGroupCall
                         && window.appSettings.groupCalls !== false
                enabled: groupCalls.state === "idle"
                //% "Join group call"
                text: qsTrId("threesea-me-join-group-call")
                onClicked: page.openGroupCall()
            }
            MenuItem {
                visible: !page.selecting && !page.readOnly && calls.supported
                         && window.appSettings.threemaCalls !== false
                         && page.identity.length === 8 && page.contact.canCall === true
                enabled: calls.state === "idle"
                //% "Call"
                text: qsTrId("threesea-me-call")
                onClicked: calls.start(page.identity)
            }
            MenuItem {
                visible: !page.selecting && !page.readOnly && calls.supported
                         && window.appSettings.threemaCalls !== false
                         && window.appSettings.videoCalls !== false
                         && page.identity.length === 8 && page.contact.canVideoCall === true
                enabled: calls.state === "idle"
                //% "Video call"
                text: qsTrId("threesea-me-video-call")
                onClicked: calls.start(page.identity, true)
            }
        }

        // On the page too: at the newest message the page, not the list, gets the upward drag.
        // Away from it the menu cannot open (the drag scrolls the list first), so its glow at the
        // bottom edge is dimmed then, like a list's pulley that only shows at the end.
        PushUpMenu {
            id: pushUpMenu
            enabled: listView.atYEnd && !page.selecting
            opacity: enabled ? 1.0 : Theme.opacityFaint
            Behavior on opacity { FadeAnimation { } }
            MenuItem {
                //% "Take photo or video"
                text: qsTrId("threesea-me-take-photo")
                onClicked: {
                    var camera = pageStack.push(Qt.resolvedUrl("CameraPage.qml"))
                    camera.captured.connect(function(path, video) {
                        page.attach(path, !video, path.replace(/^.*\//, ""), video)
                    })
                }
            }
            MenuItem {
                //% "Send photo"
                text: qsTrId("threesea-me-send-photo")
                onClicked: pageStack.push(imagePicker)
            }
            MenuItem {
                //% "Send video"
                text: qsTrId("threesea-me-send-video")
                onClicked: pageStack.push(videoPicker)
            }
            MenuItem {
                //% "Send file"
                text: qsTrId("threesea-me-send-file")
                onClicked: pageStack.push(contentPicker)
            }
            MenuItem {
                //% "Send location"
                text: qsTrId("threesea-me-send-location")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("SendLocationDialog.qml"))
                    dialog.accepted.connect(function() {
                        page.sendLocation(dialog.location, dialog.placeName.trim())
                    })
                }
            }
            MenuItem {
                // Groups: any member with poll support counts (core checks).
                visible: page.isGroup || page.contact.canPoll === true
                //% "Create poll"
                text: qsTrId("threesea-me-create-poll")
                onClicked: page.createPoll()
            }
        }

        Item {
            id: chatHeader
            width: parent.width
            // Room for a display cutout above the usual header height; the content centres in
            // the space below it (a standard PageHeader pads the same way).
            height: Theme.itemSizeLarge + cutout.inset
            z: 1

            CutoutInset { id: cutout }

            Avatar {
                id: headerAvatar
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                    verticalCenterOffset: cutout.inset / 2
                }
                name: page.contact.displayName || page.identity
                group: page.isGroup
                list: page.isList
                source: !page.contact.pictureVersion ? ""
                        : (page.isGroup ? "image://threesea/group/" : "image://threesea/contact/")
                          + page.identity + "?" + page.contact.pictureVersion
            }

            Column {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: headerAvatar.left
                    rightMargin: Theme.paddingLarge
                    verticalCenter: parent.verticalCenter
                    verticalCenterOffset: cutout.inset / 2
                }
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignRight
                    text: page.contact.displayName || page.identity
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeLarge
                    font.family: Theme.fontFamilyHeading
                    color: Theme.primaryColor
                }
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignRight
                    visible: text.length > 0
                    text: page.typing
                          //% "typing…"
                          ? qsTrId("threesea-la-typing")
                          : (backend.connectionState !== "connected"
                             //% "Offline – messages will be sent later"
                             ? qsTrId("threesea-la-offline-queue")
                             // Groups: the members; people not saved as contacts: their Threema ID
                             // (saved ones show the verification dots instead, like Android).
                             : page.isGroup || page.isList ? (page.contact.memberNames || "")
                             : page.contact.saved === true ? "" : page.identity)
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.secondaryHighlightColor
                }
                // Saved contacts: how much they are trusted (Threema's three dots).
                Item {
                    width: parent.width
                    height: dots.height + Theme.paddingSmall
                    visible: !page.isGroup && page.contact.saved === true && !page.typing
                             && backend.connectionState === "connected"
                    VerificationIndicator {
                        id: dots
                        anchors.right: parent.right
                        y: Theme.paddingSmall
                        level: page.contact.verification || 0
                    }
                }
            }
        }

        // A running group call, like Android's banner: who is in it and for how long; tap to join
        // (or to return to the call we are in).
        BackgroundItem {
            id: callBanner
            anchors.top: chatHeader.bottom
            width: parent.width
            height: visible ? Theme.itemSizeMedium : 0
            visible: page.isGroup && groupCalls.supported && window.appSettings.groupCalls !== false
                     && (page.groupCallRunning || page.inGroupCall)
            enabled: page.inGroupCall || groupCalls.state === "idle"
            onClicked: page.openGroupCall()

            Rectangle {
                anchors.fill: parent
                color: Theme.rgba(Theme.highlightBackgroundColor, Theme.highlightBackgroundOpacity)
            }
            Image {
                id: callIcon
                x: Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                source: "image://theme/icon-m-call?" + Theme.highlightColor
            }
            Column {
                anchors {
                    left: callIcon.right
                    leftMargin: Theme.paddingMedium
                    right: callAction.left
                    rightMargin: Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    color: Theme.highlightColor
                    text: {
                        var people = page.groupCall.participants || []
                        if (people.length === 0) {
                            //% "Group call"
                            return qsTrId("threesea-la-group-call")
                        }
                        return people.map(function(p) { return p.name }).join(", ")
                    }
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryHighlightColor
                    visible: text.length > 0
                    text: {
                        var started = page.groupCall.startedAt || 0
                        if (started <= 0) return ""
                        var seconds = Math.max(0, Math.floor((page.now - started) / 1000))
                        var minutes = Math.floor(seconds / 60)
                        var time = minutes >= 60
                                ? Math.floor(minutes / 60) + ":" + ("0" + (minutes % 60)).slice(-2) + ":" + ("0" + (seconds % 60)).slice(-2)
                                : minutes + ":" + ("0" + (seconds % 60)).slice(-2)
                        var people = (page.groupCall.participants || []).length
                        //% "%n participant(s)"
                        return people > 0 ? qsTrId("threesea-la-group-call-participants", people) + " · " + time : time
                    }
                }
            }
            Label {
                id: callAction
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                color: callBanner.enabled ? Theme.highlightColor : Theme.secondaryHighlightColor
                text: page.inGroupCall
                      //% "Return"
                      ? qsTrId("threesea-la-group-call-return")
                      //% "Join"
                      : qsTrId("threesea-la-group-call-join")
            }
        }

        BackgroundItem {
            id: spamBanner
            anchors.top: callBanner.bottom
            width: parent.width
            height: visible ? Theme.itemSizeMedium : 0
            visible: page.spamPotential
            onClicked: page.openSpamReport()

            Rectangle {
                anchors.fill: parent
                color: Theme.rgba(Theme.highlightBackgroundColor, Theme.highlightBackgroundOpacity)
            }
            Column {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: spamDismiss.left
                    rightMargin: Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    color: Theme.highlightColor
                    text: page.isGroup
                          //% "This group is from an unknown contact"
                          ? qsTrId("threesea-la-spam-unknown-group")
                          //% "This chat is from an unknown contact"
                          : qsTrId("threesea-la-spam-unknown-contact")
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryHighlightColor
                    //% "Tap to report spam"
                    text: qsTrId("threesea-la-spam-tap-to-report")
                }
            }
            IconButton {
                id: spamDismiss
                anchors {
                    right: parent.right
                    rightMargin: Theme.paddingSmall
                    verticalCenter: parent.verticalCenter
                }
                icon.source: "image://theme/icon-m-clear"
                onClicked: {
                    page.spamPotential = false
                    backend.call("dismissSpam", { identity: page.identity }, null)
                }
            }
        }

        Item {
            id: dayBand
            anchors.top: spamBanner.bottom
            width: parent.width
            // Constant height: resizing the list while it lays out would loop through its spacer.
            height: stickyDate.height + Theme.paddingSmall

            Label {
                id: stickyDate
                anchors.horizontalCenter: parent.horizontalCenter
                text: page.stickyDay
                color: Theme.secondaryColor
                opacity: page.stickyHidden ? 0 : 1
                Behavior on opacity { FadeAnimation { duration: 150 } }
            }
        }

        EmojiPanel {
            id: emojiPanel
            anchors {
                left: parent.left
                right: parent.right
                bottom: parent.bottom
            }
            height: page.emojiPanelOpen ? page.keyboardHeight : 0
            visible: height > 0
            clip: true
            reactionMode: page.reactionTarget !== null
            onEmojiPicked: {
                var target = page.reactionTarget
                if (!target) {
                    page.insertEmoji(emoji)
                    return
                }
                // Like the quick reactions: an emoji we reacted with already is withdrawn.
                var index = page.indexOfLocalId(target.id)
                var reactions = index >= 0 ? messages.get(index).reactions : target.reactions
                page.react(target.messageId, emoji, !Fmt.reactedWith(reactions, emoji))
                page.emojiPanelOpen = false
            }
            onBackspace: page.deleteBeforeCursor()
            onCloseRequested: page.emojiPanelOpen = false
            onGifPicked: {
                page.emojiPanelOpen = false
                page.sendGif(id)
            }
        }

        SilicaListView {
            id: listView
            anchors {
                top: dayBand.bottom
                left: parent.left
                right: parent.right
                bottom: emojiPanel.top
            }
            clip: true
            verticalLayoutDirection: ListView.BottomToTop
            model: messages

            // The rows moved: the band has to look at the message under it again. Nothing
            // is scrolled here — who adds a message decides that (page.addMessage).
            onCountChanged: {
                page.stickyIndex = -1
                page.updateStickyDay()
            }
            // The quick scroll (and anything else) landing at the newest end while the
            // newest messages are out of the window: bring them back.
            onAtYEndChanged: if (atYEnd) page.backToNewest()
            onContentYChanged: {
                page.updateStickyDay()
                // The page beyond the end the reader is approaching, two screens ahead so it
                // is there by the time they arrive.
                if (contentY - originY < 2 * Screen.height) page.loadOlder()
                if ((originY + contentHeight) - (contentY + height) < 2 * Screen.height) page.loadNewer()
            }

            // Short chats sit at the bottom next to the input: a spacer at the top takes the
            // rest of the height (Qt 5.6 lays out a short bottom-to-top list from the top). Assigned
            // imperatively; it converges because the content without the spacer stays the same.
            property real spacer: 0
            function updateSpacer() {
                var wanted = Math.max(0, height - (contentHeight - spacer))
                if (Math.abs(wanted - spacer) > 0.5) spacer = wanted
            }
            onContentHeightChanged: updateSpacer()
            onHeightChanged: updateSpacer()
            // In a bottom-to-top view the header sits at the bottom: the input, like Jolla Messages.
            header: Item {
                // Space between the newest message and the input (the list runs bottom to top).
                readonly property real gap: Theme.paddingLarge
                width: listView.width
                height: gap + editBar.height + attachmentBar.height + quoteBar.height + mentionBar.height
                        + Math.max(input.height, sendButton.height + 2 * Theme.paddingMedium) + Theme.paddingSmall
                // Bars appearing above the input (mentions, quote…) must not push it off the screen.
                onHeightChanged: if (input.activeFocus) listView.positionViewAtBeginning()
                // While selecting, the selection panel takes the input's place.
                opacity: page.selecting ? 0.0 : 1.0
                enabled: !page.selecting
                Behavior on opacity { FadeAnimation { } }

                // The picked file above the input, with a button to drop it; the input's text becomes
                // its caption.
                Item {
                    id: attachmentBar
                    y: parent.gap
                    width: parent.width
                    height: page.attachment ? Theme.itemSizeMedium : 0
                    visible: height > 0

                    // The picked files' thumbnails (a file's: the attach icon), side by side.
                    SilicaListView {
                        id: attachmentPreview
                        x: Theme.horizontalPageMargin
                        anchors.verticalCenter: parent.verticalCenter
                        width: count > 1 ? dropAttachment.x - x - Theme.paddingMedium : Theme.itemSizeSmall
                        height: Theme.itemSizeSmall
                        orientation: ListView.Horizontal
                        spacing: Theme.paddingSmall
                        clip: true
                        model: page.attachments
                        delegate: Image {
                            width: Theme.itemSizeSmall
                            height: Theme.itemSizeSmall
                            fillMode: Image.PreserveAspectCrop
                            asynchronous: true
                            sourceSize.width: width
                            sourceSize.height: height
                            source: modelData.photo ? "file://" + modelData.path
                                  : modelData.video ? "image://nemoThumbnail/" + modelData.path
                                  : "image://theme/icon-m-attach?" + Theme.highlightColor
                            // Show the photo the way it will be sent (EXIF rotation applied).
                            autoTransform: true

                            // A photo is cut and turned before sending by tapping it, as the
                            // Android app's editor is opened from its preview.
                            MouseArea {
                                anchors.fill: parent
                                enabled: modelData.photo === true
                                onClicked: page.editAttachment(index)
                            }
                        }
                    }
                    Label {
                        visible: page.attachments.length === 1
                        anchors {
                            left: attachmentPreview.right
                            leftMargin: Theme.paddingMedium
                            right: dropAttachment.left
                            verticalCenter: parent.verticalCenter
                        }
                        text: page.attachment ? page.attachment.name : ""
                        font.pixelSize: Theme.fontSizeSmall
                        color: Theme.highlightColor
                        truncationMode: TruncationMode.Fade
                    }
                    IconButton {
                        id: dropAttachment
                        anchors {
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                            verticalCenter: parent.verticalCenter
                        }
                        icon.source: "image://theme/icon-m-clear"
                        onClicked: page.dropAttachments()
                    }
                }

                // "Editing message" with a cancel button above the input while editing.
                Item {
                    id: editBar
                    y: parent.gap + attachmentBar.height
                    width: parent.width
                    height: page.editingMessageId !== "" ? cancelEdit.height : 0
                    visible: height > 0

                    Label {
                        anchors {
                            left: parent.left
                            leftMargin: Theme.horizontalPageMargin
                            right: cancelEdit.left
                            verticalCenter: parent.verticalCenter
                        }
                        //% "Editing message"
                        text: qsTrId("threesea-la-editing-message")
                        font.pixelSize: Theme.fontSizeSmall
                        color: Theme.highlightColor
                        truncationMode: TruncationMode.Fade
                    }
                    IconButton {
                        id: cancelEdit
                        anchors {
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                        }
                        icon.source: "image://theme/icon-m-clear"
                        onClicked: page.endEdit()
                    }
                }

                // A group we are no longer in, a blocked contact or a revoked ID: no input.
                Label {
                    visible: page.readOnly
                    anchors.fill: input
                    z: 2
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: Text.AlignHCenter
                    text: page.revoked
                          //% "This Threema ID was revoked"
                          ? qsTrId("threesea-la-revoked-input")
                          : page.blocked
                            //% "You have blocked this contact"
                            ? qsTrId("threesea-la-blocked-input")
                            //% "You are no longer in this group"
                            : qsTrId("threesea-la-group-left-input")
                    color: Theme.secondaryHighlightColor
                    wrapMode: Text.Wrap
                }

                // While recording, or with a recording that stopped by itself: time and cancel
                // instead of the input.
                Item {
                    id: recordingBar
                    visible: page.recordingHere
                    anchors.fill: input
                    z: 1
                    Label {
                        anchors {
                            left: parent.left
                            leftMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }
                        text: (voiceRecorder.held
                               //% "Recorded %1"
                               ? qsTrId("threesea-la-recorded")
                               //% "Recording %1"
                               : qsTrId("threesea-la-recording")).arg(Fmt.duration(voiceRecorder.duration / 1000))
                        color: Theme.highlightColor
                    }
                    IconButton {
                        anchors {
                            right: parent.right
                            verticalCenter: parent.verticalCenter
                        }
                        icon.source: "image://theme/icon-m-clear"
                        onClicked: voiceRecorder.cancel()
                    }
                }

                // "Quoting <name>" with the beginning of the quoted message and a cancel button.
                Item {
                    id: quoteBar
                    y: parent.gap + attachmentBar.height + editBar.height
                    width: parent.width
                    height: page.quoting ? Math.max(cancelQuote.height, quoteColumn.height) : 0
                    visible: height > 0

                    Rectangle {
                        x: Theme.horizontalPageMargin
                        width: Math.round(Theme.paddingSmall / 2)
                        height: quoteColumn.height
                        anchors.verticalCenter: parent.verticalCenter
                        radius: width / 2
                        color: Theme.highlightColor
                    }
                    Column {
                        id: quoteColumn
                        anchors {
                            left: parent.left
                            leftMargin: Theme.horizontalPageMargin + Theme.paddingMedium
                            right: cancelQuote.left
                            verticalCenter: parent.verticalCenter
                        }
                        Label {
                            width: parent.width
                            //% "Quoting %1"
                            text: page.quoting ? qsTrId("threesea-la-quoting").arg(page.quoting.senderName) : ""
                            font.pixelSize: Theme.fontSizeSmall
                            color: Theme.highlightColor
                            truncationMode: TruncationMode.Fade
                        }
                        Label {
                            width: parent.width
                            text: page.quoting ? Fmt.preview(page.quoting) : ""
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: Theme.secondaryHighlightColor
                            truncationMode: TruncationMode.Fade
                        }
                    }
                    IconButton {
                        id: cancelQuote
                        anchors {
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                            verticalCenter: parent.verticalCenter
                        }
                        icon.source: "image://theme/icon-m-clear"
                        onClicked: page.quoting = null
                    }
                }

                // Group members matching the "@…" being typed (Android's mention popup).
                Column {
                    id: mentionBar
                    y: parent.gap + attachmentBar.height + editBar.height + quoteBar.height
                    width: parent.width
                    Repeater {
                        model: page.mentionCandidates(page.mentionQuery)
                        BackgroundItem {
                            width: mentionBar.width
                            onClicked: page.insertMention(modelData.identity, modelData.name)
                            Label {
                                anchors {
                                    left: parent.left
                                    leftMargin: Theme.horizontalPageMargin
                                    right: parent.right
                                    rightMargin: Theme.horizontalPageMargin
                                    verticalCenter: parent.verticalCenter
                                }
                                text: "@" + modelData.name
                                color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                                truncationMode: TruncationMode.Fade
                            }
                        }
                    }
                }

                TextArea {
                    id: input
                    inputMethodHints: window.incognitoHints
                    opacity: page.recordingHere || page.readOnly ? 0 : 1
                    enabled: !page.readOnly
                    y: parent.gap + attachmentBar.height + editBar.height + quoteBar.height + mentionBar.height
                       + Theme.paddingSmall
                    x: emojiButton.x + emojiButton.width
                    textLeftMargin: Theme.paddingSmall
                    width: parent.width - x - sendButton.width - Theme.horizontalPageMargin - Theme.paddingMedium
                    onActiveFocusChanged: if (activeFocus) page.emojiPanelOpen = false
                    placeholderText: page.attachment && page.editingMessageId === ""
                                     //% "Add a caption"
                                     ? qsTrId("threesea-la-add-caption")
                                     //% "Type message"
                                     : qsTrId("threesea-la-type-message")
                    labelVisible: false
                    focusOutBehavior: FocusBehavior.KeepFocus
                    Component.onCompleted: page.inputField = input
                    onTextChanged: {
                        page.userTyped()
                        draftSaver.restart()
                        page.updateMentionQuery()
                        emojiRanges = backend.threemaEmoji ? Fmt.emojiRanges(text) : []
                        emojiLayout.restart()
                    }
                    onCursorPositionChanged: page.updateMentionQuery()

                    // Enter sends (Settings, on by default) and Shift+Enter makes a new line, or
                    // the other way round. Shift only exists on a hardware keyboard, so with the
                    // setting on the on-screen keyboard's Enter always sends.
                    // Threema's emoji pictures while typing (Settings → Appearance): the
                    // highlighter hides the system's emoji glyphs, the pictures are drawn over
                    // them; the text itself stays plain.
                    property var emojiRanges: []
                    property var emojiPlaces: []
                    // The pictures' size: a bit more than the font's, most of a line.
                    readonly property real emojiSide: Math.round(font.pixelSize * 1.15)
                    function placeEmoji() {
                        var places = []
                        for (var i = 0; i < emojiRanges.length; ++i) {
                            var range = emojiRanges[i]
                            var from = input._editor.positionToRectangle(range.start)
                            var to = input._editor.positionToRectangle(range.start + range.length)
                            var width = to.y === from.y ? to.x - from.x : from.height
                            places.push({ x: from.x, y: from.y, width: width, height: from.height,
                                          cell: range.cell })
                        }
                        emojiPlaces = places
                    }
                    Timer {
                        id: emojiLayout
                        interval: 0
                        onTriggered: input.placeEmoji()
                    }
                    onWidthChanged: emojiLayout.restart()
                    EmojiHighlighter {
                        textDocument: input._editor.textDocument
                        ranges: input.emojiRanges
                        // The picture (see below) and some room on both sides.
                        emojiWidth: input.emojiSide + Theme.paddingMedium
                        // The glyphs' boxes moved: place the pictures again.
                        onEmojiWidthChanged: emojiLayout.restart()
                    }
                    Item {
                        parent: input._editor
                        anchors.fill: parent
                        Repeater {
                            model: input.emojiPlaces
                            Image {
                                // Square, centred on the glyph's box.
                                readonly property real side: input.emojiSide
                                x: modelData.x + (modelData.width - side) / 2
                                y: modelData.y + (modelData.height - side) / 2
                                width: side
                                height: side
                                sourceSize.width: side
                                sourceSize.height: side
                                source: "image://emoji/" + modelData.cell
                            }
                        }
                    }

                    EnterKeyFilter {
                        editor: input._editor
                        sendWithShift: !backend.enterToSend
                        // Never starts a voice recording: only text or an attachment.
                        onSendRequested: if (page.attachment || input.text.trim().length > 0) page.send()
                    }
                }

                // The emoji panel (Threema's emoji and favourite GIFs) or, while it is open, back to
                // the keyboard; at the input's start like Threema for Android.
                IconButton {
                    id: emojiButton
                    visible: !page.readOnly
                    x: Theme.horizontalPageMargin - Theme.paddingMedium
                    anchors.verticalCenter: input.verticalCenter
                    width: Theme.itemSizeExtraSmall
                    height: width
                    onClicked: page.toggleEmojiPanel()
                    // Our smiley in the theme's style (the theme has none; scripts/make-ui-icons.py).
                    icon.width: Theme.iconSizeMedium
                    icon.height: Theme.iconSizeMedium
                    icon.sourceSize.width: Theme.iconSizeMedium
                    icon.sourceSize.height: Theme.iconSizeMedium
                    icon.source: page.emojiPanelOpen ? "image://theme/icon-m-keyboard"
                                                     : Qt.resolvedUrl("../icons/emoji-smileys.png")
                }

                // Send button on a faint rounded background, like Jolla Messages.
                Rectangle {
                    id: sendButton
                    visible: !page.readOnly
                    anchors {
                        right: parent.right
                        rightMargin: Theme.horizontalPageMargin
                        verticalCenter: input.verticalCenter
                    }
                    width: Theme.itemSizeExtraSmall
                    height: width
                    radius: Theme.paddingSmall
                    color: sendArea.pressed ? Theme.rgba(Theme.highlightBackgroundColor, Theme.highlightBackgroundOpacity)
                                            : Theme.rgba(Theme.primaryColor, Fmt.alpha(0.1, backend.emulator))

                    Image {
                        anchors.centerIn: parent
                        width: Theme.iconSizeSmallPlus
                        height: width
                        sourceSize.width: width
                        sourceSize.height: height
                        source: "image://theme/" + (page.canRecord ? "icon-m-mic" : "icon-m-send")
                                + "?" + (sendArea.pressed || page.recordingHere ? Theme.highlightColor : Theme.primaryColor)
                        opacity: (input.text.trim().length > 0 || page.attachment || page.canRecord
                                  || page.recordingHere) && !page.sendingText ? 1.0 : Theme.opacityLow
                    }
                    MouseArea {
                        id: sendArea
                        anchors.fill: parent
                        enabled: (input.text.trim().length > 0 || page.attachment || page.canRecord
                                  || page.recordingHere) && !page.sendingText
                        onClicked: page.send()
                    }
                }
            }

            // Short chats sit at the bottom next to the input (see spacer above).
            footer: Item { width: 1; height: listView.spacer }

            delegate: MessageDelegate {
                width: listView.width
                olderMessage: index + 1 < messages.count ? messages.get(index + 1) : null
                showUnread: model.id === page.unreadMarkerId
                anyReaction: page.contact.reactions !== false
                senderName: page.isGroup && !model.outgoing && model.sender ? page.memberName(model.sender) : ""
                canEdit: page.contact.canEdit === true
                canQuote: !page.isList
                selectionMode: page.selecting
                selected: page.selected[model.id] === true
                onSelectToggled: page.toggleSelected(model.id)
                canDelete: page.contact.canDelete === true
                onReact: page.react(model.messageId, emoji, apply)
                onMoreReactionsRequested: page.pickReaction(messages.get(index))
                onEditRequested: page.startEdit(model.messageId, model.text, model.mentions)
                onQuoteRequested: page.startQuote(messages.get(index))
                quoted: model.quoteId ? page.quotedFor(model.quoteId, page.quoteCacheRevision) : null
                onQuotedRequested: page.showQuoted(model.quoteId)
                onSaveRequested: page.saveFile(model.id, false)
                onGifFavouriteRequested: backend.call("addGifFavourite", { message: model.id }, function(ok, result, kind, message) {
                    //% "Added to favourite GIFs"
                    Notices.show(ok ? qsTrId("threesea-la-gif-favourite-added") : Errors.text(kind, message),
                                 Notice.Short, Notice.Center)
                })
                onLocationRequested: page.openLocation(model.body)
                onStarToggled: backend.call("setStarred", { message: model.id, starred: !model.starred },
                                            function(ok, result) { if (ok && result) page.addMessage(result) })
                onDetailsRequested: pageStack.push(Qt.resolvedUrl("MessageDetailsPage.qml"),
                                                   { message: messages.get(index) })
                onForwardRequested: pageStack.push(Qt.resolvedUrl("SharePage.qml"),
                                                   { forwardMessage: model.id, text: Fmt.preview(messages.get(index)) })
                onPollRequested: {
                    if (model.body.state === 1) {
                        pageStack.push(Qt.resolvedUrl("PollResultsPage.qml"),
                                       { poll: model.body, names: page.pollNames(model.body) })
                        return
                    }
                    var pollPage = pageStack.push(Qt.resolvedUrl("PollPage.qml"),
                                                  { messageId: model.id, conversation: page.identity, poll: model.body })
                    var messageId = model.id
                    pollPage.changed.connect(function(message) { page.addMessage(message) })
                    pollPage.voteRequested.connect(function(choices) {
                        var ids = []
                        for (var i = 0; i < choices.length; ++i) ids.push(Number(choices[i]))
                        page.run("vote", { message: messageId, choices: ids })
                    })
                }
                onClosePollRequested: page.run("closePoll", { message: model.id })
                onFileRequested: {
                    var file = model.body
                    if (model.outgoing && file.uploadState === 3) {
                        backend.call("retryUpload", { message: model.id })
                    } else if (file.fileState === 2 && String(file.mediaType).indexOf("audio/") === 0
                               && file.rendering !== "file") {
                        if (voicePlayer.messageId === model.id && voicePlayer.playing) {
                            voicePlayer.pause()
                        } else {
                            page.playVoice(model.id)
                        }
                    } else if (file.fileState === 2 && Fmt.isPicture(file.mediaType)
                               && file.rendering !== "file") {
                        pageStack.push(Qt.resolvedUrl("ImageViewerPage.qml"), {
                            messageId: model.id,
                            animated: String(file.mediaType).toLowerCase() === "image/gif"
                        })
                    } else if (file.fileState === 2 && String(file.mediaType).indexOf("video/") === 0
                               && videoPlayer.available) {
                        pageStack.push(Qt.resolvedUrl("VideoPlayerPage.qml"), { messageId: model.id })
                    } else if (file.fileState === 2) {
                        // Other apps can only open what's in Downloads (our copies are encrypted).
                        page.saveFile(model.id, true)
                    } else if (file.fileState !== 1) {
                        backend.call("downloadFile", { message: model.id })
                    }
                }
                onDeleteRequested: {
                    var messageId = model.messageId
                    var outgoing = model.outgoing
                    var rowId = model.id
                    remorseAction(everyone
                                  //% "Deleting for everyone"
                                  ? qsTrId("threesea-la-deleting-everyone")
                                  //% "Deleting"
                                  : qsTrId("threesea-la-deleting-message"), function() {
                        page.deleteMessage(rowId, messageId, outgoing, everyone)
                    })
                }
            }

            VerticalScrollDecorator { }
        }
    }

    RemorsePopup { id: selectionRemorse }

    // Actions for the selected messages, like Gallery's selection panel.
    DockedPanel {
        id: selectionPanel
        width: parent.width
        height: Theme.itemSizeLarge
        dock: Dock.Bottom
        open: page.selecting

        Label {
            anchors {
                left: parent.left
                leftMargin: Theme.horizontalPageMargin
                verticalCenter: parent.verticalCenter
            }
            text: page.selectedCount > 0
                  //% "%n selected"
                  ? qsTrId("threesea-he-n-selected", page.selectedCount)
                  //% "Tap messages to select them"
                  : qsTrId("threesea-la-select-hint")
            color: Theme.highlightColor
            font.pixelSize: Theme.fontSizeSmall
        }
        Row {
            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                verticalCenter: parent.verticalCenter
            }
            spacing: Theme.paddingLarge
            IconButton {
                enabled: page.selectedCount > 0
                icon.source: "image://theme/icon-m-delete"
                onClicked: {
                    var list = page.selectedMessages()
                    page.endSelection()
                    //% "Deleting %n message(s)"
                    selectionRemorse.execute(qsTrId("threesea-la-deleting-messages", list.length), function() {
                        for (var i = 0; i < list.length; ++i) {
                            page.deleteMessage(list[i].id, list[i].messageId, list[i].outgoing, false)
                        }
                    })
                }
            }
            IconButton {
                icon.source: "image://theme/icon-m-forward"
                enabled: {
                    var list = page.selectedMessages()
                    for (var i = 0; i < list.length; ++i) if (!page.forwardable(list[i])) return false
                    return list.length > 0
                }
                onClicked: {
                    var ids = []
                    var list = page.selectedMessages()
                    for (var i = 0; i < list.length; ++i) ids.push(list[i].id)
                    page.endSelection()
                    pageStack.push(Qt.resolvedUrl("SharePage.qml"),
                                   //% "%n message(s)"
                                   { forwardMessages: ids, text: qsTrId("threesea-la-n-messages", ids.length) })
                }
            }
        }
    }

    Component {
        id: imagePicker
        MultiImagePickerDialog {
            onSelectedContentChanged: page.attachPicked(selectedContent, "image")
        }
    }
    Component {
        id: videoPicker
        MultiVideoPickerDialog {
            onSelectedContentChanged: page.attachPicked(selectedContent, "video")
        }
    }

    // A video's thumbnail (first frame, from the system thumbnailer) and length, then it is
    // sent as media; without a frame it goes out as a plain file.
    Item {
        id: videoPreparer
        // Off screen, not invisible: grabToImage only renders visible items.
        x: -2 * width
        width: 512
        height: 512
        property string path
        property string caption
        property real seconds: 0

        function start(path, caption) {
            videoPreparer.path = path
            videoPreparer.caption = caption
            seconds = 0
            frame.path = path
            durationProbe.source = path
            giveUp.restart()
        }
        function finish(image) {
            giveUp.stop()
            var path = videoPreparer.path
            if (path.length === 0) return
            videoPreparer.path = ""
            var done = function(ok, result, kind, message) {
                if (ok) {
                    page.addMessage(result)
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            }
            var sent = function(ok, result, kind, message) {
                done(ok, result, kind, message)
                page.sendNext()
            }
            if (image) backend.sendVideo(page.identity, path, image, seconds, caption, sent)
            else backend.sendFile(page.identity, path, false, caption, sent)
            frame.path = ""
            durationProbe.source = ""
        }

        // The thumbnailer's image provider, as lipstick uses it. It always gives a centred
        // square, so video thumbnails are square (the video keeps its proportions).
        Image {
            id: frame
            property string path
            width: implicitWidth
            height: implicitHeight
            sourceSize.width: 512
            sourceSize.height: 512
            asynchronous: true
            cache: false
            source: path.length > 0 ? "image://nemoThumbnail/" + path.replace(/^file:\/\//, "") : ""
            onStatusChanged: {
                if (videoPreparer.path.length === 0) return
                // A moment more for the length (the media service may not exist).
                if (status === Image.Ready) grabLater.restart()
                else if (status === Image.Error) videoPreparer.finish(null)
            }
        }
        MediaPlayer {
            id: durationProbe
            autoLoad: true
            onDurationChanged: if (duration > 0) videoPreparer.seconds = duration / 1000
        }
        Timer {
            id: grabLater
            interval: 800
            onTriggered: {
                if (!frame.grabToImage(function(result) { videoPreparer.finish(result.image) })) {
                    videoPreparer.finish(null)
                }
            }
        }
        Timer {
            id: giveUp
            interval: 10000
            onTriggered: videoPreparer.finish(null)
        }
    }

    Component {
        id: contentPicker
        MultiContentPickerDialog {
            //% "Send file"
            title: qsTrId("threesea-me-send-file")
            onSelectedContentChanged: page.attachPicked(selectedContent, "file")
        }
    }

    property Item inputField
}
