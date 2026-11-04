// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Nemo.Notifications 1.0
import Nemo.KeepAlive 1.2
import Sailfish.Share 1.0
import "pages"
import "components"
import "components/Format.js" as Fmt
import "components/Errors.js" as Errors

ApplicationWindow {
    id: window
    // Names, messages, captions… come from other people: never interpret them as rich text
    // (Jolla's apps do the same). Labels that want markup set textFormat themselves.
    _defaultLabelFormat: Text.PlainText

    MediaSession { }

    initialPage: Component { StartupPage { } }
    cover: Qt.resolvedUrl("cover/CoverPage.qml")
    allowedOrientations: defaultAllowedOrientations

    // Onboarding state machine: the backend's state decides the root page.
    property string shownState: "starting"
    // Identity of the chat currently open (no notification for it while the app is active).
    property string openChat: ""
    // The core's app settings (readReceipts, incognitoKeyboard, disableScreenshots…); settings
    // pages update it after changing one.
    property var appSettings: ({})
    // The chat of the voice message playing, for the system's media controls ("" for none).
    property string voiceFrom: ""
    // Privacy → Incognito keyboard: no learning or suggestions in fields where people write.
    readonly property int incognitoHints: appSettings.incognitoKeyboard === true
                                          ? Qt.ImhNoPredictiveText | Qt.ImhSensitiveData : Qt.ImhNone
    function loadAppSettings() {
        backend.call("settings", {}, function(ok, result) { if (ok) window.appSettings = result })
    }

    // App lock (ADR 0009): `lockInfo` from the core's lockStatus. `locked` is the state itself,
    // from the moment the app locks until the PIN was accepted; the lock page follows it
    // (settleStack), so neither a page transition nor a pop or replace from elsewhere can leave
    // the app open. Leaving the app for longer than the grace time locks again (and hides
    // private chats).
    property var lockInfo: null
    property bool locked: false
    property var lockPage: null
    // The lock page whose PIN was just accepted, until the stack could take it away.
    property var acceptedLock: null
    // This run's first look at the lock is done: passing later steps (a new license key) doesn't
    // ask for the PIN again.
    property bool lockChecked: false
    property double leftAt: 0
    // A call in progress: its page stays on top, and is the only way to see the video, the
    // camera switch and the running time.
    readonly property bool callRunning: calls.state !== "idle" && calls.state !== "ended"
    // Nothing from outside moves the stack while this holds: it waits (see the timer for the
    // pending things).
    readonly property bool stackHeld: locked || callRunning

    // While the stored data is still locked (after a restart) nothing arrives: say so.
    function lockedNotification() {
        if (backend.state === "locked" && Qt.application.state !== Qt.ApplicationActive) {
            // Not while the window is just being opened (a tapped notification, the launcher).
            lockedNotificationLater.restart()
        } else if (backend.state !== "locked") {
            clearNotification("locked")
        }
    }
    Timer {
        id: lockedNotificationLater
        interval: 3000
        onTriggered: {
            if (backend.state !== "locked" || Qt.application.state === Qt.ApplicationActive) return
            if (!notifications["locked"]) {
                var n = privateNotificationComponent.createObject(window)
                //% "Threesea is locked"
                n.summary = qsTrId("threesea-la-locked-notification")
                //% "Open it and enter your PIN or passphrase to receive messages."
                n.body = qsTrId("threesea-la-locked-notification-body")
                n.previewSummary = n.summary
                n.previewBody = n.body
                n.publish()
                notifications["locked"] = n
            }
        }
    }

    // Threema's directory refused the license and nothing arrives any more: in the background
    // there is no other way to find out.
    function licenseNotification() {
        if (Qt.application.state === Qt.ApplicationActive || notifications["license"]) return
        var n = privateNotificationComponent.createObject(window)
        //% "Threesea is offline"
        n.summary = qsTrId("threesea-la-license-notification")
        //% "Threema no longer accepts this license key. Open Threesea to check it."
        n.body = qsTrId("threesea-la-license-notification-body")
        n.previewSummary = n.summary
        n.previewBody = n.body
        n.publish()
        notifications["license"] = n
    }

    function lockChanged(status) {
        lockInfo = status
    }

    function refreshLock(lockNow) {
        backend.call("lockStatus", {}, function(ok, status) {
            if (!ok) return
            lockInfo = status
            lockChecked = true
            if (lockNow && status.lock && status.lock.appLock) showLock()
        })
    }

    // Close the private chats: their list and any private chat, wherever it was opened from
    // (the contacts, a link, a group's member list), with everything above them.
    property bool privateToClose: false
    function closePrivate() {
        privateToClose = true
        settleStack()
    }

    function showLock() {
        locked = true
        settleStack()
    }

    // The lock page on top; only a call's page may be above it (it shows only who calls).
    function lockPageUp() {
        var top = pageStack.currentPage
        if (top === lockPage) return true
        return !!top && top.objectName === "call" && pageStack.find(function(page) { return page === lockPage }) !== null
    }

    // A call without its page (it was taken away), or one coming in while the page of an earlier
    // call waits under the lock. Otherwise a running call's page stays where it is: below the
    // lock if the app locked during the call, whose cover has mute and hang up.
    function callPageMissing() {
        if (!callRunning) return false
        var top = pageStack.currentPage
        if (top && top.objectName === "call") return false
        return calls.state === "incoming"
               || pageStack.find(function(page) { return page.objectName === "call" }) === null
    }

    // Brings the stack in line with the state. The stack takes no change during a transition
    // (push and pop then do nothing and return null): that one is cut short, or it happens
    // right after it.
    function settleStack() {
        if (!acceptedLock && !privateToClose && !callPageMissing() && !(locked && !lockPageUp())) return
        if (pageStack.busy) pageStack.completeAnimation()
        if (pageStack.busy) {
            stackLater.restart()
            return
        }
        if (acceptedLock) {
            var accepted = acceptedLock
            acceptedLock = null
            locked = false
            lockPage = null
            if (pageStack.find(function(page) { return page === accepted })) {
                pageStack.pop(pageStack.previousPage(accepted), PageStackAction.Immediate)
            }
        }
        if (privateToClose) {
            privateToClose = false
            // The lowest one: private chats can lie on top of each other.
            var lowest = null
            pageStack.find(function(page) {
                if (page.objectName === "private" || (page.objectName === "chat" && page.isPrivate)) lowest = page
                return false
            })
            if (lowest) pageStack.pop(pageStack.previousPage(lowest), PageStackAction.Immediate)
        }
        // Something landed on the lock page (a reply that came late): back to it, so that one
        // PIN is all it takes. A call's page goes along and comes back with the next step.
        if (locked && !lockPageUp() && pageStack.find(function(page) { return page === lockPage })) {
            pageStack.pop(lockPage, PageStackAction.Immediate)
        }
        if (callPageMissing()) {
            if (!pageStack.push(Qt.resolvedUrl("pages/CallPage.qml"), {},
                                locked ? PageStackAction.Immediate : PageStackAction.Animated)) {
                stackLater.restart()
                return
            }
        }
        if (locked && !lockPageUp()) {
            var unlock = pageStack.push(Qt.resolvedUrl("pages/UnlockPage.qml"), { cancellable: false },
                                        PageStackAction.Immediate)
            if (!unlock) {
                stackLater.restart()
                return
            }
            lockPage = unlock
            unlock.unlocked.connect(function() {
                window.acceptedLock = unlock
                window.settleStack()
            })
        }
    }
    Timer {
        id: stackLater
        interval: 50
        onTriggered: window.settleStack()
    }
    // Whatever else moved the stack (a pop in a reply that came late): the lock and a running
    // call's page stay up. Later, because the stack takes no change from inside these signals.
    Connections {
        target: pageStack
        onCurrentPageChanged: if (window.stackHeld) stackLater.restart()
        onBusyChanged: if (window.stackHeld && !pageStack.busy) stackLater.restart()
    }

    // During a call what was asked for opens afterwards; the tap only shows the call otherwise.
    function sayAfterCall() {
        //% "Opens when the call is over"
        if (callRunning) Notices.show(qsTrId("threesea-la-opens-after-call"), Notice.Short, Notice.Center)
    }

    // The way back to the chat list for everything that comes from outside (notification, link,
    // share). False while that has to wait: nothing changed then.
    function showConversations() {
        if (stackHeld || pageStack.busy) return false
        var chats = pageStack.find(function(page) { return page.objectName === "conversations" })
        if (!chats) return false
        pageStack.pop(chats, PageStackAction.Immediate)
        return true
    }

    Connections {
        target: Qt.application
        onStateChanged: {
            window.lockedNotification()
            // The license page shows the stored key: behind the lock like the rest.
            if ((backend.state !== "ready" && backend.state !== "needsLicense")
                    || !window.lockInfo || !window.lockInfo.lock) return
            var grace = window.lockInfo.lock.graceSecs * 1000
            if (Qt.application.state !== Qt.ApplicationActive) {
                if (window.leftAt > 0) return
                window.leftAt = Date.now()
                if (grace === 0) {
                    // Immediately: hide everything before the switcher's snapshot.
                    window.lockNow()
                } else {
                    graceTimer.interval = grace
                    graceTimer.restart()
                }
            } else {
                graceTimer.stop()
                var away = window.leftAt > 0 ? Date.now() - window.leftAt : 0
                window.leftAt = 0
                if (away > grace) window.lockNow()
            }
        }
    }
    // The grace time is over while away: the cover, the media controls and the private chats
    // close now, not only when the app comes back. Timers sleep with the device, so coming back
    // still compares the clock, and so does every wake-up (wakeJob).
    Timer {
        id: graceTimer
        onTriggered: window.lockNow(true)
    }
    function checkGrace() {
        if (graceTimer.running && Date.now() - leftAt >= graceTimer.interval) {
            graceTimer.stop()
            lockNow(true)
        }
    }

    // Replace `chatPage` (a private chat while they are hidden) with the PIN, then the chat.
    function unlockChat(chatPage, identity) {
        var unlock = pageStack.replace(Qt.resolvedUrl("pages/UnlockPage.qml"), {
            privateChats: true,
            //% "Private chats"
            title: qsTrId("threesea-he-private-chats")
        }, PageStackAction.Immediate)
        unlock.unlocked.connect(function() {
            pageStack.replace(Qt.resolvedUrl("pages/ChatPage.qml"), { identity: identity })
        })
    }

    // `letVoiceFinish`: the grace time ran out while away. A voice message being listened to
    // (phone in the pocket, screen locked) plays to its end, without the name; nothing follows it.
    function lockNow(letVoiceFinish) {
        backend.call("lock")
        closePrivate()
        if (lockInfo && lockInfo.lock && lockInfo.lock.appLock) {
            // What's playing is behind the lock too. Only then: this also runs on coming back
            // after the grace time without any app lock (a private chat stops with its page).
            if (!letVoiceFinish || !voicePlayer.playing) voicePlayer.stop()
            videoPlayer.stop()
            showLock()
        }
    }
    // Paused on the lock screen while locked: gone, so that it can't be started again from there.
    Timer {
        interval: 1000
        running: window.locked && voicePlayer.active && !voicePlayer.playing
        onTriggered: voicePlayer.stop()
    }

    function showStatePage() {
        if (pageStack.busy) {
            navigateLater.restart()
            return
        }
        var state = backend.state
        if (state === shownState) {
            return
        }
        var page
        switch (state) {
        case "needsLicense": page = Qt.resolvedUrl("pages/LicensePage.qml"); break
        case "needsIdentity": page = Qt.resolvedUrl("pages/IdentitySetupPage.qml"); break
        case "ready": page = Qt.resolvedUrl("pages/ConversationsPage.qml"); break
        case "locked": page = Qt.resolvedUrl("pages/UnlockPage.qml"); break
        case "error":
            // After the passphrase or later: StartupPage again, which says what happened.
            if (shownState === "starting") return
            page = Qt.resolvedUrl("pages/StartupPage.qml")
            break
        default: return // starting is shown by StartupPage or the page before it
        }
        // Under a lock the new page never shows by itself.
        var immediate = shownState === "starting" || locked
        shownState = state
        // Starting over: nothing left that a lock could protect here.
        if (state !== "ready" && state !== "needsLicense") locked = false
        pageStack.replaceAbove(null, page, state === "locked" ? { database: true, cancellable: false } : {},
                               immediate ? PageStackAction.Immediate : PageStackAction.Animated)
        settleStack()
        if (state === "ready" || state === "needsLicense") {
            // Opened with the passphrase just now: that was the unlock.
            refreshLock(!lockChecked && !backend.unlockedAtStart)
            if (state === "ready") loadAppSettings()
        }
        lockedNotification()
    }

    function showChat(identity, text) {
        if (openChat !== identity && !showConversations()) {
            pendingChat = identity
            sayAfterCall()
            activate()
            return
        }
        if (openChat !== identity) {
            pageStack.push(Qt.resolvedUrl("pages/ChatPage.qml"), { identity: identity },
                           PageStackAction.Immediate)
        }
        if (text) {
            var chat = pageStack.find(function(page) { return page.identity === identity && page.inputField })
            if (chat) chat.inputField.text = text
        }
        activate()
    }

    // Threema links (threema://…, threema.id/…): from other apps via D-Bus, or tapped in a
    // message (`fromMessage`: anything else opens in its own app, a contact opens on top of the
    // chat). Kept until the app is ready.
    property string pendingLink
    property bool pendingLinkFromMessage: false
    // A chat to open (notification tapped) while still starting.
    property string pendingChat
    // What was shared from another app, until the share page can open.
    property var pendingShare: null
    function showShare() {
        if (pendingShare === null || !showConversations()) return
        var share = pendingShare
        pendingShare = null
        pageStack.push(Qt.resolvedUrl("pages/SharePage.qml"), share, PageStackAction.Immediate)
    }
    function openLink(url, fromMessage) {
        // Also after each reply: the app may have locked meanwhile. Running it again is safe,
        // adding a contact twice changes nothing.
        function mustWait() {
            if (!stackHeld && !pageStack.busy && backend.state !== "starting" && backend.state !== "locked") {
                return false
            }
            pendingLink = url
            pendingLinkFromMessage = fromMessage === true
            sayAfterCall()
            return true
        }
        if (mustWait()) return
        pendingLink = ""
        backend.call("parseLink", { url: url }, function(ok, link) {
            if (!ok) {
                if (fromMessage) Qt.openUrlExternally(url)
                return
            }
            if (mustWait()) return
            if (link.action === "license") {
                var licensePage = pageStack.find(function(page) { return page.objectName === "license" })
                if (licensePage) licensePage.useKey(link.key)
                activate()
                return
            }
            if (backend.state !== "ready") return
            if (link.action === "compose" && !link.identity) {
                showConversations()
                pageStack.push(Qt.resolvedUrl("pages/SharePage.qml"), { text: link.text || "" },
                               PageStackAction.Immediate)
                activate()
                return
            }
            backend.call("addContact", { identity: link.identity }, function(ok, contact, kind, message) {
                if (!ok) {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                    return
                }
                backend.refreshContacts()
                if (mustWait()) return
                if (link.action === "compose" || link.text) {
                    showChat(link.identity, link.text)
                } else if (fromMessage) {
                    pageStack.push(Qt.resolvedUrl("pages/ContactPage.qml"), { identity: link.identity })
                } else {
                    showConversations()
                    pageStack.push(Qt.resolvedUrl("pages/ContactPage.qml"), { identity: link.identity },
                                   PageStackAction.Immediate)
                    activate()
                }
            })
        })
    }
    Timer {
        // After start-up or a page transition.
        running: !window.stackHeld && (window.pendingLink.length > 0 || window.pendingChat.length > 0
                                       || window.pendingUpdate || window.pendingShare !== null)
        interval: 300
        repeat: true
        onTriggered: {
            // Also wait while the stored data is locked (passphrase).
            if (backend.state === "starting" || backend.state === "locked" || pageStack.busy
                    || shownState !== backend.state) return
            if (window.pendingLink.length > 0) window.openLink(window.pendingLink, window.pendingLinkFromMessage)
            // showChat() and showShare() ask again for later if they still can't.
            var chat = window.pendingChat
            window.pendingChat = ""
            if (chat.length > 0 && backend.state === "ready") window.showChat(chat)
            if (backend.state === "ready") window.showShare()
            else window.pendingShare = null
            // showUpdate() asks again for later if it still can't.
            if (window.pendingUpdate && backend.state === "ready") {
                window.pendingUpdate = false
                window.showUpdate()
            }
        }
    }

    // Wake up periodically while the device sleeps so a dead connection is noticed and
    // re-established (aligned wakeups, Harbour-allowed). Keep the CPU up briefly for it.
    BackgroundJob {
        id: wakeJob
        enabled: backend.state === "ready" && backend.keepRunning
        frequency: BackgroundJob.FiveMinutes
        onTriggered: {
            window.checkGrace()
            backend.call("reconnect")
            finishLater.restart()
        }
    }
    Timer {
        id: finishLater
        interval: 15000
        onTriggered: wakeJob.finished()
    }

    Timer {
        id: navigateLater
        interval: 100
        onTriggered: window.showStatePage()
    }

    // One notification per chat, updated as messages arrive (like Jolla Messages); actions:
    // tap opens the chat, Reply (quick reply in the notification), Mark as read.
    // Sound and vibration use Jolla Messages' categories: the chat's notification in
    // x-nemo.messaging.im plays the profile's instant message tone (silent: im.received), an
    // empty transient x-nemo.messaging.im.preview vibrates.
    property var notifications: ({})

    // `token`: Reply and Mark as read carry the chat's action token (bridge/dbusinterface.h).
    function dbusAction(name, displayName, method, identity, type, token) {
        var action = {
            "name": name,
            "displayName": displayName,
            "service": "org.threesea.harbour-threesea",
            "path": "/",
            "iface": "org.threesea.harbourthreesea",
            "method": method,
            "arguments": token ? [identity, token] : [identity]
        }
        if (type) action.type = type
        return action
    }

    Component {
        id: notificationComponent
        Notification {
            property string identity
            // A new token for every new notification; one taken over after a restart keeps its
            // chat's token, which lipstick still holds (bridge/dbusinterface.h).
            property bool renewToken: true
            property string token
            // Once both are set (a binding would run before them and renew too early).
            Component.onCompleted: token = dbus.actionToken(identity, renewToken)
            appName: "Threesea"
            appIcon: "harbour-threesea"
            // Lipstick delivers actions via D-Bus only (bridge/dbusinterface.cpp).
            remoteActions: [
                window.dbusAction("default", "", "openChat", identity),
                //% "Reply"
                window.dbusAction("reply", qsTrId("threesea-me-reply"), "reply", identity, "input", token),
                //% "Mark as read"
                window.dbusAction("read", qsTrId("threesea-me-mark-read"), "markRead", identity, "", token)
            ]
        }
    }

    // Private chats: no Reply or Mark as read (they would need the chat); tap opens the app.
    Component {
        id: privateNotificationComponent
        Notification {
            appName: "Threesea"
            appIcon: "harbour-threesea"
            remoteActions: [{
                "name": "default",
                "displayName": "",
                "service": "org.threesea.harbour-threesea",
                "path": "/",
                "iface": "org.threesea.harbourthreesea",
                "method": "activate",
                "arguments": []
            }]
        }
    }

    // The chat server saw this Threema ID log in with another device's cookie (Android's
    // rogue-device warning): someone else may be using the ID.
    Notification {
        id: deviceCookieNotification
        appName: "Threesea"
        appIcon: "harbour-threesea"
        //% "Threema ID used on another device"
        summary: qsTrId("threesea-la-device-cookie-title")
        //% "The Threema server noticed a connection from another device with your Threema ID. If that was not you or one of your linked devices, contact Threema support."
        body: qsTrId("threesea-la-device-cookie-body")
        previewSummary: summary
        previewBody: body
        remoteActions: [{
            "name": "default",
            "displayName": "",
            "service": "org.threesea.harbour-threesea",
            "path": "/",
            "iface": "org.threesea.harbourthreesea",
            "method": "activate",
            "arguments": []
        }]
    }

    // A newer release on GitHub (ADR 0014): tapping it opens Settings → Advanced, which links
    // to its page.
    Notification {
        id: updateNotification
        appName: "Threesea"
        appIcon: "harbour-threesea"
        remoteActions: [{
            "name": "default",
            "displayName": "",
            "service": "org.threesea.harbour-threesea",
            "path": "/",
            "iface": "org.threesea.harbourthreesea",
            "method": "showUpdate",
            "arguments": []
        }]
    }

    // Once, after setup, in builds for GitHub: may Threesea check for updates?
    function askUpdateCheck() {
        backend.call("updateSettings", {}, function(ok, result) {
            // Not asked yet: `enabled` neither true nor false (a JSON null arrives as missing).
            if (ok && result.available && typeof result.enabled !== "boolean" && !window.stackHeld && !pageStack.busy) {
                pageStack.push(Qt.resolvedUrl("pages/UpdateCheckDialog.qml"))
            }
        })
    }

    // Starting, locked or mid-transition: once that is over (the timer below).
    property bool pendingUpdate: false
    function showUpdate() {
        if (backend.state !== "ready" || shownState !== "ready" || window.stackHeld || pageStack.busy) {
            pendingUpdate = true
            return
        }
        pageStack.push(Qt.resolvedUrl("pages/settings/AdvancedSettingsPage.qml"))
    }

    Notification {
        id: previewNotification
        appName: "Threesea"
        appIcon: "harbour-threesea"
        category: "x-nemo.messaging.im.preview"
    }

    // Shows a new message in `n` (the chat's notification, which also shows the banner) with the
    // sound and vibration chosen in the settings.
    // `identity`: the chat's own tone, if it has one ("": none, e.g. private chats).
    function alert(n, identity, group, title, text) {
        var sound = group ? backend.soundGroups : backend.soundChats
        var vibrate = group ? backend.vibrateGroups : backend.vibrateChats
        n.category = sound ? "x-nemo.messaging.im" : "im.received"
        // A chosen tone replaces the profile's (lipstick plays it at the profile's volume).
        var tone = identity ? backend.chatTone(identity) : ""
        if (!tone) tone = group ? backend.toneGroups : backend.toneChats
        n.sound = sound ? tone : ""
        n.summary = title
        n.body = text
        n.previewSummary = title
        n.previewBody = text
        n.publish()
        if (vibrate) {
            // Nothing to show, only the vibration.
            previewNotification.replacesId = 0
            previewNotification.publish()
        }
    }

    // Android's in-app sounds and vibration: a message arriving in the chat the user is
    // reading gets the soft bell and the usual short vibration, nothing else.
    function inAppFeedback() {
        if (backend.inAppSounds) softSound.play()
        if (backend.inAppVibrate) {
            previewNotification.replacesId = 0
            previewNotification.publish()
        }
    }

    // Notifications still shown from before a restart: take them over (by their chat).
    Notification { id: notificationQuery }
    Component.onCompleted: {
        var existing = notificationQuery.notifications()
        for (var i = 0; i < existing.length; ++i) {
            var n = existing[i]
            var actions = n.remoteActions || []
            var identity = actions.length > 0 && actions[0].arguments ? actions[0].arguments[0] : ""
            // The private chats' notification opens the app only.
            if (!identity && actions.length > 0 && actions[0].method === "activate") identity = "private"
            if (identity && !notifications[identity]) {
                notifications[identity] = n
            } else {
                n.close()
            }
        }
    }

    function publish(identity, title, sender, message) {
        var n = notifications[identity]
        if (!n) {
            n = notificationComponent.createObject(window, { identity: identity })
            notifications[identity] = n
            n.itemCount = 0
        } else if (!n.identity) {
            // Taken over after a restart: it needs our actions again.
            n.remoteActions = notificationComponent.createObject(window, { identity: identity, renewToken: false }).remoteActions
        }
        n.itemCount = n.itemCount + 1
        var text = backend.notificationPreview
                   ? (sender ? sender + ": " : "") + Fmt.preview(message)
                   //% "New message"
                   : qsTrId("threesea-la-new-message")
        n.timestamp = new Date(message.createdAt)
        alert(n, identity, identity.indexOf("g:") === 0, title, text)
    }

    // Private chats: neither who nor what, and tapping only opens the app (ADR 0009).
    function notifyPrivate(message) {
        var n = notifications["private"]
        if (!n) {
            n = privateNotificationComponent.createObject(window)
            notifications["private"] = n
            n.itemCount = 0
        }
        n.itemCount = n.itemCount + 1
        n.timestamp = new Date(message.createdAt)
        alert(n, "", message.conversation.indexOf("g:") === 0,
              //% "Private chat"
              qsTrId("threesea-la-private-chat"), qsTrId("threesea-la-new-message"))
    }

    // `held`: the notification was kept back while another device led (multi-device), so the
    // message has been in the chat for a while — no in-app feedback for it any more.
    function notify(message, isPrivate, held) {
        // Of the status lines only missed calls ("call|missed|…", core/src/app_calls.rs).
        if (!backend.notificationsEnabled
                || (message.kind === "status" && String(message.body).indexOf("call|missed|") !== 0)) return
        if (Qt.application.state === Qt.ApplicationActive && openChat === message.conversation) {
            // The message is right there, so no notification — only Android's quiet in-app
            // feedback, and none for one that has been standing there.
            if (!held) window.inAppFeedback()
            return
        }
        if (isPrivate) {
            notifyPrivate(message)
            return
        }
        var identity = message.conversation
        if (identity.indexOf("g:") === 0) {
            backend.call("group", { key: identity }, function(ok, group) {
                if (!ok || group.left) return
                var sender = message.sender || ""
                for (var i = 0; i < group.members.length; ++i) {
                    if (group.members[i].identity === sender) sender = group.members[i].displayName
                }
                window.publish(identity, group.title, sender, message)
            })
        } else {
            backend.call("contact", { identity: identity }, function(ok, contact) {
                var name = ok ? contact.displayName : identity
                window.publish(identity, name, "", message)
            })
        }
    }

    function clearNotification(identity) {
        var n = notifications[identity]
        if (n) {
            n.close()
            delete notifications[identity]
        }
    }

    onOpenChatChanged: if (openChat.length > 0) clearNotification(openChat)

    // Shared from other apps ("Share" → Threesea): pick the chat, then it opens with the content
    // ready to send (one file as attachment, text in the input; several files are sent at once).
    ShareProvider {
        method: "threesea"
        // The D-Bus name is ours already (bridge/main.cpp).
        registerName: false
        capabilities: ["image/*", "video/*", "audio/*", "text/*", "application/*"]
        onTriggered: {
            var files = []
            var text = ""
            for (var i = 0; i < resources.length; ++i) {
                var resource = resources[i]
                var path = resource.filePath || ""
                if (path.length > 0) {
                    files.push({ path: path, name: path.split("/").pop(),
                                 photo: /\.(jpe?g|png|webp|bmp)$/i.test(path) })
                } else if (resource.data) {
                    text += (text.length > 0 ? "\n" : "") + resource.data
                }
            }
            if (files.length === 0 && text.length === 0) return
            window.pendingShare = { files: files, text: text }
            window.sayAfterCall()
            if (backend.state === "ready" && shownState === "ready") window.showShare()
            // Started for this share: the window isn't shown yet (bridge/main.cpp).
            dbus.activate()
        }
    }

    // Opt-in address book matching: when switched on, and daily while running. The reader (and
    // with it the system's contact cache) is loaded only for a sync and unloaded afterwards.
    function syncAddressBook(manual) {
        if (!backend.addressBookSync || backend.state !== "ready") return
        if (!manual && Date.now() - backend.addressBookSyncedAt < 24 * 3600 * 1000) return
        addressBook.manual = manual
        if (addressBook.status === Loader.Ready) addressBook.item.start()
        else addressBook.active = true
    }

    Loader {
        id: addressBook
        property bool manual
        active: false
        source: "components/AddressBookReader.qml"
        onLoaded: item.start()
        onStatusChanged: {
            if (status !== Loader.Error) return
            active = false
            //% "The address book is not available on this device."
            if (manual) Notices.show(qsTrId("threesea-la-address-book-unavailable"), Notice.Long, Notice.Center)
        }
    }
    Connections {
        target: addressBook.item
        onFinished: {
            var manual = addressBook.manual
            var checked = entries.length
            backend.call("matchAddressBook", { entries: entries }, function(ok, result, kind, message) {
                addressBook.active = false
                if (ok) {
                    backend.addressBookSyncedAt = Date.now()
                    backend.refreshContacts()
                    if (result.added > 0) {
                        //% "%n new contact(s) from your address book"
                        Notices.show(qsTrId("threesea-la-address-book-added", result.added), Notice.Short, Notice.Center)
                    } else if (manual && checked > 0) {
                        //% "%n address book entries checked, no new contacts."
                        Notices.show(qsTrId("threesea-la-address-book-none-new", checked), Notice.Long, Notice.Center)
                    } else if (manual) {
                        //% "No phone numbers or e-mail addresses found in the address book."
                        Notices.show(qsTrId("threesea-la-address-book-empty"), Notice.Long, Notice.Center)
                    }
                } else {
                    console.warn("address book matching failed:", kind)
                    if (manual) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        }
    }
    Timer {
        interval: 3600 * 1000
        repeat: true
        running: backend.addressBookSync
        onTriggered: window.syncAddressBook(false)
    }

    // A call starts or comes in: its page goes on top of whatever is shown (like Android's call
    // screen, also over the app lock: it shows only who calls).
    // The name of who is in a call (cover).
    property string callName
    Connections {
        target: calls
        onStateChanged: {
            if (calls.identity.length > 0 && calls.state !== "ended") {
                var identity = calls.identity
                backend.call("contact", { identity: identity }, function(ok, contact) {
                    if (calls.identity === identity) window.callName = ok ? contact.displayName : identity
                })
            }
            window.settleStack()
        }
    }

    Connections {
        target: dbus
        onOpenChatRequested: {
            if (backend.state === "ready" && shownState === "ready" && !pageStack.busy) window.showChat(identity)
            else if (backend.state === "needsLicense") window.activate()
            else {
                window.pendingChat = identity
                // Locked or starting: show the PIN/passphrase screen meanwhile.
                window.activate()
            }
        }
        onActivateRequested: window.activate()
        onShowUpdateRequested: window.showUpdate()
        onOpenUrlRequested: window.openLink(url, false)
        onMarkReadRequested: {
            backend.call("markRead", { identity: identity }, function() { backend.refreshUnread() })
            window.clearNotification(identity)
        }
        onReplyRequested: {
            if (text.trim().length === 0) return
            backend.call("sendText", { identity: identity, text: text }, function(ok, result, kind, message) {
                if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            })
            // Replying means it has been read.
            backend.call("markRead", { identity: identity }, function() { backend.refreshUnread() })
            window.clearNotification(identity)
        }
    }

    Connections {
        target: backend
        onStateChanged: {
            window.showStatePage()
            window.syncAddressBook(false)
            if (backend.state === "ready") window.clearNotification("license")
        }
        onCoreEvent: {
            // `notify`: the chat's own notification setting (muted, mentions only) allows it.
            if (name === "messageAdded" && !data.message.outgoing && data.notify !== false) {
                window.notify(data.message, data.private === true)
            } else if (name === "messageNotify" && data.notify !== false) {
                // A forwarded message still unread after its grace time (multi-device).
                window.notify(data.message, data.private === true, true)
            } else if (name === "conversationRead") {
                // Read in Threema Web, on another device, or from a notification.
                window.clearNotification(data.identity)
                backend.refreshUnread()
            } else if (name === "serverMessage") {
                Notices.show(data.message, Notice.Long, Notice.Center)
            } else if (name === "licenseChanged") {
                window.licenseNotification()
            } else if (name === "deviceCookieChanged") {
                deviceCookieNotification.publish()
            } else if (name === "updateAvailable") {
                //% "Threesea %1 is available"
                updateNotification.summary = qsTrId("threesea-la-update-notification").arg(data.version)
                //% "Tap for details and the download page."
                updateNotification.body = qsTrId("threesea-la-update-notification-body")
                updateNotification.previewSummary = updateNotification.summary
                updateNotification.previewBody = updateNotification.body
                updateNotification.publish()
            }
        }
    }
}
