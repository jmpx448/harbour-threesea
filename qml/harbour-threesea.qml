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
import "components/NotificationActions.js" as NotificationActions

ApplicationWindow {
    id: window

    readonly property alias messageOut: messageOutItem
    MessageOut { id: messageOutItem }
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
    // Where the reader stopped in each chat, for this session (chat -> the message at the top of
    // the view). Never a private chat's, and gone with the lock.
    property var readingPlaces: ({})
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
            var body = NotificationActions.replyWaiting()
                    //% "Open it and enter your PIN or passphrase to send your reply and receive messages."
                    ? qsTrId("threesea-la-locked-notification-reply-body")
                    //% "Open it and enter your PIN or passphrase to receive messages."
                    : qsTrId("threesea-la-locked-notification-body")
            var n = notifications["locked"]
            if (n && n.body === body) return
            // The last run's, taken over.
            clearNotification("appLocked")
            //% "Threesea is locked"
            plainNotification("locked", qsTrId("threesea-la-locked-notification"), body)
        }
    }

    // The start failed: in the background (the Full flavour's login start) nothing else says that
    // nothing will arrive.
    function startFailedNotification() {
        if (backend.state === "error" && !backend.dataRemovalStuck) {
            if (Qt.application.state !== Qt.ApplicationActive) startFailedLater.restart()
        } else if (backend.state !== "error") {
            startFailedLater.stop()
            clearNotification("startFailed")
        }
    }
    Timer {
        id: startFailedLater
        // Not while the window is just being opened, like the one above.
        interval: 3000
        onTriggered: {
            if (backend.state !== "error" || Qt.application.state === Qt.ApplicationActive) return
            plainNotification("startFailed", qsTrId("threesea-la-startup-failed"),
                              //% "Open it to see why. Messages arrive again once it runs."
                              qsTrId("threesea-la-startup-failed-notification-body"))
        }
    }

    // Threema's directory refused the license and nothing arrives any more: in the background
    // there is no other way to find out.
    function licenseNotification() {
        if (Qt.application.state === Qt.ApplicationActive || notifications["license"]) return
        //% "Threesea is offline"
        plainNotification("license", qsTrId("threesea-la-license-notification"),
                          //% "Threema no longer accepts this license key. Open Threesea to check it."
                          qsTrId("threesea-la-license-notification-body"))
    }

    // Threema Safe has kept failing for a week: the copy on the server is getting old, and expires,
    // while Settings says "On". Again when the count of days changes, without a banner; gone with
    // the first backup that works or when Safe goes off.
    property int safeWarningShown: 0
    function checkSafeWarning() {
        backend.call("safeStatus", {}, function(ok, status) {
            if (!ok) return
            if (!status.backupReminder) clearNotification("backupReminder")
            var days = status.warningDays || 0
            if (days === 0) {
                safeWarningShown = 0
                clearNotification("safeFailing")
            } else if (days !== safeWarningShown) {
                var quiet = safeWarningShown > 0 || !!notifications["safeFailing"]
                safeWarningShown = days
                //% "Threema Safe backup failing"
                plainNotification("safeFailing", qsTrId("threesea-la-safe-failing-notification"),
                                  //% "Threema Safe hasn't been able to back up for %n day(s). Tap to check."
                                  qsTrId("threesea-la-safe-failing-notification-body", days), quiet, "showSafe")
            }
        })
    }
    // A Data backup brought the ID (the setup's restore page): its import goes on over the chat
    // list, once the state has put that there. Its password stays no longer than that.
    property var pendingDataImport: null
    // Starting, locked or mid-transition: once that is over (the timer below).
    property bool pendingSafe: false
    function showSafe() {
        if (backend.state !== "ready" || shownState !== "ready" || stackHeld || pageStack.busy) {
            pendingSafe = true
            return
        }
        var top = pageStack.currentPage
        if (!top || !top.safePage) pushWithPin(Qt.resolvedUrl("pages/SafePage.qml"))
    }

    // What Threema's servers said last, or why they don't let this device in: on the chat list until
    // the next login, as from outside that looks like a bad network.
    property var offlineReason: null
    readonly property string offlineReasonText: reasonText(offlineReason)
    // What the background has been told since the window was last open: a server that goes on
    // refusing gets one notification, not one per attempt.
    property string offlineReasonNotified
    function reasonText(reason) {
        if (!reason) return ""
        if (reason.kind === "server") return reason.text
        //% "Threema's server keeps closing the connection during login."
        if (reason.kind === "loginRefused") return qsTrId("threesea-la-reason-login-refused")
        switch (reason.code) {
        case 4110:
            //% "Threema's server for linked devices no longer accepts this version of Threesea."
            return qsTrId("threesea-la-reason-mediator-version")
        case 4111:
            //% "Threema's server for linked devices turned this device away: this Threema ID has reached its limit of linked devices."
            return qsTrId("threesea-la-reason-mediator-device-limit")
        case 4112:
            //% "Another connection under this device's login took over on Threema's server for linked devices."
            return qsTrId("threesea-la-reason-mediator-duplicate")
        case 4114:
            //% "Too many messages are waiting for this device on Threema's server for linked devices, so it closed the connection."
            return qsTrId("threesea-la-reason-mediator-queue-full")
        default:
            //% "Threema's server for linked devices closed the connection (code %1)."
            return qsTrId("threesea-la-reason-mediator-other").arg(reason.code)
        }
    }
    function offlineReasonArrived(reason) {
        offlineReason = reason
        if (Qt.application.state === Qt.ApplicationActive) {
            Notices.show(offlineReasonText, Notice.Long, Notice.Center)
            return
        }
        var again = offlineReasonNotified === offlineReasonText
        if (again && notifications["serverMessage"]) return
        offlineReasonNotified = offlineReasonText
        plainNotification("serverMessage", reason.kind === "server"
                          //% "Message from Threema's server"
                          ? qsTrId("threesea-la-server-message-notification")
                          // The words are ours, about what the server did.
                          : qsTrId("threesea-la-license-notification"), offlineReasonText, again)
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
            // From the last run, and not true any more.
            if (!locked) clearNotification("appLocked")
            var waiting = notificationsWaiting
            notificationsWaiting = []
            waiting.forEach(function(item) { notify(item.message, item.isPrivate, item.held) })
            runNotificationActions()
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
        // Nothing to lock any more (the grace timer can outlive a failed start).
        if (backend.state !== "ready" && backend.state !== "needsLicense") return
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
        if (!acceptedLock && !privateToClose && !pinPageToClose && !callPageMissing()
                && !(locked && !lockPageUp())) return
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
        if (pinPageToClose) {
            pinPageToClose = false
            var pinPage = pageStack.find(function(page) { return page.objectName === "holdsPin" })
            if (pinPage) pageStack.pop(pageStack.previousPage(pinPage), PageStackAction.Immediate)
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
            if (Qt.application.state === Qt.ApplicationActive && window.captureDiscardedUnsaid) {
                window.sayCaptureDiscarded()
            }
            // Timers stood still while the device slept: "connected" may be an old story by now.
            if (Qt.application.state === Qt.ApplicationActive && backend.state === "ready") {
                backend.call("wake")
            }
            window.lockedNotification()
            window.startFailedNotification()
            // The chat list says it now.
            if (Qt.application.state === Qt.ApplicationActive) {
                window.clearNotification("serverMessage")
                window.offlineReasonNotified = ""
            }
            // The group may have become private during the call.
            if (Qt.application.state !== Qt.ApplicationActive && groupCalls.conversation.length > 0
                    && window.groupCallName.length > 0) {
                window.refreshGroupCallName()
            }
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
        onAboutToQuit: {
            window.appEnding()
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
    // Soon after the device wakes, also where nothing wakes it (wakeJob needs Keep running): the
    // notifications show the grace time's end at once, not after what was left of its timer.
    Timer {
        interval: 30000
        repeat: true
        running: graceTimer.running
        onTriggered: window.checkGrace()
    }
    // Whether the lock is on now, by the clock as well. What it hides must not rely on a timer.
    function lockActive() {
        checkGrace()
        return locked
    }
    onLockedChanged: {
        if (locked) {
            hideNotifications()
            // It named the caller a moment ago.
            if (callNotificationWanted) showCallNotification()
        } else {
            clearNotification("appLocked")
            appLockedNews = false
            runNotificationActions()
        }
    }

    // The whole chat as a ZIP in Downloads. The core writes the file; the words in it — "Me" and
    // the times' offset — come from here, like all texts. Here and not on the page that asks for
    // it, whose answer would be lost with the page in a long export.
    function exportChat(identity, media, password) {
        //% "Exporting chat"
        Notices.show(qsTrId("threesea-la-exporting-chat"), Notice.Short, Notice.Center)
        backend.call("exportChat", {
            identity: identity,
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

    // Opens a page whose commands need the PIN (ADR 0009; the core checks it each time): asks
    // first if a lock exists and hands the page its `pin`. `opened(page)`, if given, gets the
    // page. The X on the PIN page goes back without opening anything.
    function pushWithPin(url, properties, opened) {
        var args = properties || {}
        if (!(lockInfo && lockInfo.lock)) {
            var shown = pageStack.push(url, args)
            if (opened) opened(shown)
            return
        }
        var unlock = pageStack.push(Qt.resolvedUrl("pages/UnlockPage.qml"))
        unlock.unlocked.connect(function(pin) {
            args.pin = pin
            var replaced = pageStack.replace(url, args)
            if (opened) opened(replaced)
        })
    }

    // Replace `chatPage` (a private chat while they are hidden) with the PIN, then the chat, with
    // the text a link put into its input.
    function unlockChat(chatPage, identity) {
        var text = chatPage.inputField ? chatPage.inputField.text : ""
        var unlock = pageStack.replace(Qt.resolvedUrl("pages/UnlockPage.qml"), {
            privateChats: true,
            //% "Private chats"
            title: qsTrId("threesea-he-private-chats")
        }, PageStackAction.Immediate)
        unlock.unlocked.connect(function() {
            var chat = pageStack.replace(Qt.resolvedUrl("pages/ChatPage.qml"), { identity: identity })
            if (text.length > 0 && chat.inputField) chat.setInput(text)
        })
    }

    // A photo or video taken in the app went with its chat, unsent: said now, or on coming back.
    property bool captureDiscardedUnsaid: false
    function captureDiscarded() {
        if (Qt.application.state === Qt.ApplicationActive) sayCaptureDiscarded()
        else captureDiscardedUnsaid = true
    }
    function sayCaptureDiscarded() {
        captureDiscardedUnsaid = false
        //% "The photo or video waiting to be sent was discarded"
        Notices.show(qsTrId("threesea-la-capture-discarded"), Notice.Long, Notice.Center)
    }

    // The captions on their way with their files, by chat. A chat can be closed and opened again
    // meanwhile: the stored draft still holds such a caption, and it isn't a draft for now.
    property var captionsOnTheirWay: ({})
    function captionSent(identity, caption) {
        var list = captionsOnTheirWay[identity] || []
        list.push(caption)
        captionsOnTheirWay[identity] = list
    }
    function captionNotOnItsWay(identity, caption) {
        var list = captionsOnTheirWay[identity] || []
        var at = list.indexOf(caption)
        if (at >= 0) list.splice(at, 1)
    }
    // This caption, or without one any at all.
    function captionOnItsWay(identity, caption) {
        var list = captionsOnTheirWay[identity] || []
        return caption === undefined ? list.length > 0 : list.indexOf(caption) >= 0
    }
    // Forward messages, one command each, so that what the user does meanwhile gets its turn in
    // the core between them, a picture taking a while. Up here, as the page that asked is gone
    // before they are through: what couldn't be sent is said once at the end.
    function forwardInTurn(messages, identity) {
        var errors = []
        function next(at) {
            if (at >= messages.length) {
                for (var i = 0; i < errors.length; ++i) {
                    Notices.show(Errors.text(errors[i]), Notice.Long, Notice.Center)
                }
                return
            }
            backend.call("forwardMessages", { messages: [messages[at]], identity: identity },
                         function(ok, result, kind) {
                var kinds = ok ? result.errors : [kind]
                for (var k = 0; k < kinds.length; ++k) {
                    if (errors.indexOf(kinds[k]) < 0) errors.push(kinds[k])
                }
                next(at + 1)
            })
        }
        next(0)
    }
    // The answer to a file sent from a chat. Made here, not on the chat page: the answer may come
    // after that page has gone, when nothing of the page can be relied on to run, and a refused
    // file must still be said (`alive`: the page's own record of that). It returns the path of a
    // file the page took back, so that the bridge leaves it in place (removeTempSources).
    function fileAnswer(chat, alive, identity, file) {
        return function(ok, result, kind, message) {
            if (file.caption) captionNotOnItsWay(identity, file.caption)
            if (alive.yes) {
                var kept = chat.fileAnswered(file, ok, result, kind, message)
                chat.sendNext()
                return kept
            }
            if (ok) return undefined
            Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            // The chat, opened again meanwhile, left the caption alone until now.
            var open = pageStack.find(function(page) { return page.identity === identity && page.loadDraft })
            if (file.caption && open) open.loadDraft()
            return undefined
        }
    }

    // The same for a picture drawn on (sendHeldPicture): nothing comes back to the chat's input.
    function editedPictureAnswer(chat, alive) {
        return function(ok, result, kind, message) {
            if (ok) {
                if (alive.yes && result.id !== undefined) chat.messageSent(result)
            } else if (kind === "internal") {
                // From the bridge, in no language: what it was about is known here.
                Notices.show(qsTrId("threesea-la-edit-photo-failed"), Notice.Long, Notice.Center)
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        }
    }

    // Close the page holding a PIN (pushWithPin or the app lock settings; it and what it opened):
    // the next person gets the prompt again. Its PIN goes first, in case the stack can't pop right
    // now; settleStack takes the page away.
    property bool pinPageToClose: false
    function closePinPage() {
        var pinPage = pageStack.find(function(page) { return page.objectName === "holdsPin" })
        if (!pinPage) return
        pinPage.pin = ""
        pinPageToClose = true
        settleStack()
    }

    // `letVoiceFinish`: the grace time ran out while away. A voice message being listened to
    // (phone in the pocket, screen locked) plays to its end, without the name; nothing follows it.
    function lockNow(letVoiceFinish) {
        backend.call("lock")
        readingPlaces = {}
        closePrivate()
        closePinPage()
        if (lockInfo && lockInfo.lock && lockInfo.lock.appLock) {
            // What's playing is behind the lock too. Only then: this also runs on coming back
            // after the grace time without any app lock (a private chat stops with its page).
            if (!letVoiceFinish || !voicePlayer.playing) voicePlayer.stop()
            videoPlayer.stop()
            // So is what a recording would still pick up, and its sending.
            voiceRecorder.cancel()
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
        case "removing": page = Qt.resolvedUrl("pages/StartupPage.qml"); break
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
            pageStack.push(Qt.resolvedUrl("pages/ChatPage.qml"), { identity: identity, openAtNewest: !!text },
                           PageStackAction.Immediate)
        }
        if (text) {
            var chat = pageStack.find(function(page) { return page.identity === identity && page.inputField })
            if (chat) {
                chat.setInput(text)
                chat.showInput()
            }
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
            if (!stackHeld && !pageStack.busy && backend.state !== "starting" && backend.state !== "locked"
                    && backend.state !== "removing") {
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
                var licensePage = pageStack.find(function(page) {
                    return page.objectName === "license" || page.objectName === "identitySetup"
                })
                if (licensePage) licensePage.useKey(link.key)
                activate()
                return
            }
            if (backend.state !== "ready") return
            if (link.action === "verifyMobile") {
                // Threema's SMS, opened from outside. Someone's message in a chat doesn't get to
                // spend the attempts at a pending code.
                if (fromMessage) return
                activate()
                backend.call("verifyMobile", { code: link.code, fromLink: true }, function(ok, profile, kind, message) {
                    if (!ok) {
                        Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                        return
                    }
                    var top = pageStack.currentPage
                    if (top && top.objectName === "mobileCode" && !pageStack.busy) top.reject()
                    //% "Phone number confirmed"
                    Notices.show(qsTrId("threesea-la-mobile-verified"), Notice.Long, Notice.Center)
                })
                return
            }
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
                                       || window.pendingUpdate || window.pendingWebSessions
                                       || window.pendingSafe || window.pendingDataImport !== null
                                       || window.pendingShare !== null)
        interval: 300
        repeat: true
        onTriggered: {
            // The core won't come back, so nothing waiting here will ever go through. StartupPage
            // says what happened; shownState may never catch up with this state (showStatePage).
            if (backend.state === "error") {
                window.pendingLink = ""
                window.pendingChat = ""
                window.pendingShare = null
                window.pendingUpdate = false
                window.pendingWebSessions = false
                window.pendingSafe = false
                window.pendingDataImport = null
                return
            }
            // Also wait while the stored data is locked (passphrase).
            if (backend.state === "starting" || backend.state === "locked" || backend.state === "removing"
                    || pageStack.busy || shownState !== backend.state) return
            if (window.pendingDataImport !== null && backend.state === "ready") {
                var restored = window.pendingDataImport
                window.pendingDataImport = null
                pageStack.push(Qt.resolvedUrl("pages/DataBackupImportPage.qml"), restored)
                return
            }
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
            if (window.pendingWebSessions && backend.state === "ready") {
                window.pendingWebSessions = false
                window.showWebSessions()
            }
            if (window.pendingSafe && backend.state === "ready") {
                window.pendingSafe = false
                window.showSafe()
            }
        }
    }

    // Wake up periodically while the device sleeps so a dead connection is noticed and
    // re-established (aligned wakeups, Harbour-allowed). The CPU stays up until the core says how
    // it went (wakeSettled), usually right after the echo's answer.
    BackgroundJob {
        id: wakeJob
        enabled: backend.state === "ready" && backend.keepRunning
        frequency: BackgroundJob.FiveMinutes
        onTriggered: {
            window.checkGrace()
            lifecycle.checkInstallation()
            wakeLimit.restart()
            backend.call("wake", {}, function(ok, result) {
                if (!ok || !result.waiting) window.endWake()
            })
        }
    }
    // More than a dead connection takes to find out and come back: the echo's 15 s, then a new
    // connection.
    Timer {
        id: wakeLimit
        interval: 45000
        onTriggered: wakeJob.finished()
    }
    function endWake() {
        if (wakeLimit.running) {
            wakeLimit.stop()
            wakeJob.finished()
        }
    }
    // The screen may be off, and the login needs the device up. begin() triggers the job, if it is
    // enabled and not running already (checked against the SDK's plugin); without Keep running
    // there is no job to hold the device with.
    function wakeOnNetworkChange() {
        if (wakeJob.enabled && !wakeJob.running) {
            wakeJob.begin()
        } else {
            backend.call("wake")
        }
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
            "arguments": token ? [identity, token] : identity ? [identity] : []
        }
        if (type) action.type = type
        return action
    }

    Component {
        id: notificationComponent
        Notification {
            id: chatNotification
            // Made by us, unlike one taken over after a restart.
            readonly property bool own: true
            // When it last alerted, or would have (alert()).
            property real alertedAt: 0
            property string identity
            // The message it shows, and who in a group wrote it.
            property var messageId
            property string sender
            // A new token for every new notification; one taken over after a restart keeps its
            // chat's token, which lipstick still holds (bridge/dbusinterface.h).
            property bool renewToken: true
            property string token
            // Once both are set (a binding would run before them and renew too early).
            Component.onCompleted: token = dbus.actionToken(identity, renewToken)
            onClosed: {
                window.notificationClosed(identity, chatNotification)
                chatNotification.destroy()
            }
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

    // Private chats and what isn't about a chat: no Reply or Mark as read, tap opens the app.
    Component {
        id: plainNotificationComponent
        Notification {
            id: plain
            readonly property bool own: true
            property real alertedAt: 0
            // Under this it is in `notifications`.
            property string key
            // The chat tapping it opens; otherwise it opens the app.
            property string chat
            // Another of the D-Bus interface's methods to tap to, without an argument.
            property string method
            onClosed: {
                window.notificationClosed(key, plain)
                plain.destroy()
            }
            appName: "Threesea"
            appIcon: "harbour-threesea"
            remoteActions: [window.dbusAction("default", "", method || (chat ? "openChat" : "activate"), chat)]
        }
    }

    // The plain notification under this key, new or updated, on the screen; `quiet`: without a
    // banner, as this isn't news.
    function plainNotification(key, summary, body, quiet, method) {
        var n = notifications[key]
        if (!n) {
            n = plainNotificationComponent.createObject(window, { key: key, method: method || "" })
            notifications[key] = n
        }
        n.summary = summary
        n.body = body
        n.previewSummary = quiet ? "" : summary
        n.previewBody = quiet ? "" : body
        n.publish()
        return n
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

    // The store fails while receiving: from outside that looks like a bad network. The chat list's
    // header says so too, for as long as it lasts.
    property bool storageProblem: false
    Notification {
        id: storageNotification
        appName: "Threesea"
        appIcon: "harbour-threesea"
        //% "Saving messages failed"
        summary: qsTrId("threesea-la-storage-problem-title")
        //% "Threesea can't write to its data, so new messages wait on the server, for a limited time. Check that this device has free storage. The log in Settings, Advanced has the error code."
        body: qsTrId("threesea-la-storage-problem-body")
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

    // While a browser is connected to Threema Web: it reads and sends through this phone, perhaps
    // after a push woke its session, and nothing else here shows that. Silent, with one banner per
    // new connection, and it stays when tapped. It says nothing about any chat, so it also stays
    // under the app lock.
    Notification {
        id: webConnectedNotification
        // How many browsers it tells of.
        property int count: 0
        property bool quitting: false
        appName: "Threesea"
        appIcon: "harbour-threesea"
        urgency: Notification.Low
        resident: true
        expireTimeout: 0
        //% "Connected to Threema Web"
        summary: qsTrId("threesea-la-web-connected-notification")
        remoteActions: [window.dbusAction("default", "", "showWebSessions", "")]
        // Dismissed, also with Clear all: back at once while a browser is connected, as the system
        // has no notification that can't be dismissed for this type. Once per dismissal: publishing
        // doesn't close it, and with nothing connected updateWebConnected() closes nothing.
        onClosed: {
            replacesId = 0
            if (!quitting) window.updateWebConnected()
        }
    }
    // Here, not on the sessions page: leaving the page ends its remorse by carrying it out, and the
    // answer comes when the page is gone. A failure must still be told.
    signal webSessionsRemoved()
    function removeAllWebSessions() {
        backend.call("webDeleteAll", {}, function(ok, result, kind, message) {
            if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            window.webSessionsRemoved()
        })
    }
    function updateWebConnected() {
        backend.call("webSessions", {}, function(ok, result) {
            // Not known, which doesn't mean nothing is connected.
            if (!ok) return
            var n = webConnectedNotification
            var count = result.sessions.filter(function(session) { return session.state === "connected" }).length
            if (count === 0) {
                n.count = 0
                if (n.replacesId !== 0) {
                    n.close()
                    n.replacesId = 0
                }
                return
            }
            if (count === n.count && n.replacesId !== 0) return
            //% "%n browser(s) can read and send your messages through this phone. Tap to manage."
            n.body = qsTrId("threesea-la-web-connected-notification-body", count)
            // A banner for a new browser, not for one leaving or a dismissal undone.
            n.previewSummary = count > n.count ? n.summary : ""
            n.previewBody = count > n.count ? n.body : ""
            n.count = count
            n.publish()
        })
    }
    // The sessions end with the app; if it is killed, takeOver() closes this at the next start.
    Component.onDestruction: {
        webConnectedNotification.quitting = true
        webConnectedNotification.close()
    }

    // A self-hosted Threema Web server shows a new certificate, perhaps for a session a push woke
    // while Threema Web's page is closed: that page asks about it.
    Notification {
        id: webCertificateNotification
        appName: "Threesea"
        appIcon: "harbour-threesea"
        //% "Threema Web server has a new certificate"
        summary: qsTrId("threesea-la-web-certificate-notification")
        //% "Tap to decide whether to trust it."
        body: qsTrId("threesea-la-web-certificate-notification-body")
        previewSummary: summary
        previewBody: body
        remoteActions: [window.dbusAction("default", "", "showWebSessions", "")]
    }
    function webCertificateSeen() {
        webCertificateNotification.close()
    }

    // Starting, locked or mid-transition: once that is over (the timer below).
    property bool pendingWebSessions: false
    function showWebSessions() {
        if (backend.state !== "ready" || shownState !== "ready" || window.stackHeld || pageStack.busy) {
            pendingWebSessions = true
            return
        }
        var top = pageStack.currentPage
        if (!top || top.objectName !== "webSessions") {
            pageStack.push(Qt.resolvedUrl("pages/settings/WebSessionsPage.qml"))
        }
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

    // After a login the servers hand out what they kept, as a burst: one alert for all of it, at the
    // end, for the last message that may make itself heard. While it lasts, new messages only
    // change their chats' notifications.
    property bool receivingBacklog: false
    property var backlogAlert: null
    function backlogChanged(active) {
        if (active) {
            receivingBacklog = true
            backlogLimit.restart()
            return
        }
        backlogLimit.stop()
        // Notifications on their way wait for a contact's name, and answers come in the order asked.
        backend.call("lockStatus", {}, function() {
            // The next one has begun, and takes this along.
            if (backlogLimit.running) return
            receivingBacklog = false
            var last = backlogAlert
            backlogAlert = null
            // Its chat may have been opened meanwhile.
            if (!last || Object.keys(notifications).every(function(key) { return notifications[key] !== last.n })) return
            // What it shows by now: that message may have been deleted or edited since.
            alertNow(last.n, last.identity, last.group, last.n.summary, last.n.body)
        })
    }
    // Should the servers never say it is over: it is, this long after the last message.
    Timer {
        id: backlogLimit
        interval: 30000
        onTriggered: window.backlogChanged(false)
    }
    // More is coming in. Not once the servers have said it is over: what still arrives here then
    // was on its way, and is what backlogChanged() waits for.
    function backlogGoesOn() {
        if (backlogLimit.running) backlogLimit.restart()
    }

    // Shows a new message in `n` (the chat's notification, which also shows the banner) with the
    // sound and vibration chosen in the settings, unless it is one of a burst: like Android, a
    // notification alerts again after 30 seconds without news.
    // `identity`: the chat's own tone, if it has one ("": none, e.g. private chats).
    function alert(n, identity, group, title, text) {
        var now = Date.now()
        var recent = now - n.alertedAt < 30000
        n.alertedAt = now
        if (receivingBacklog) {
            var heard = group ? backend.soundGroups || backend.vibrateGroups
                              : backend.soundChats || backend.vibrateChats
            if (heard || !backlogAlert) {
                backlogAlert = { n: n, identity: identity, group: group }
            }
            backlogGoesOn()
            publishQuietly(n, title, text, false)
        } else if (recent) {
            publishQuietly(n, title, text, false)
        } else {
            alertNow(n, identity, group, title, text)
        }
    }

    function alertNow(n, identity, group, title, text) {
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

    // A call rings with the app out of sight, as on a locked phone: who calls, with Answer and
    // Decline, for as long as it rings. Its own category gives it priority and turns the display
    // on (notifications/, Full only; only there can a call ring). The ring itself is CallManager's.
    // Expires a little after the ring time in case this process dies with it on the screen.
    readonly property bool callNotificationWanted: calls.state === "incoming"
                                                   && Qt.application.state !== Qt.ApplicationActive
    onCallNotificationWantedChanged: {
        if (callNotificationWanted) {
            showCallNotification()
        } else {
            callNotification.close()
            // Only its grace starts: on a locked phone lipstick runs the action after the unlock,
            // when the window may be up already. The handlers look at the state.
            dbus.releaseToken("call")
        }
    }
    function showCallNotification() {
        var identity = calls.identity
        backend.call("contact", { identity: identity }, function(ok, contact) {
            if (!callNotificationWanted || calls.identity !== identity) return
            var kind = qsTrId("threesea-la-call-incoming")
            // As in the other notifications: neither who nor which chat behind a lock or for a
            // private chat.
            var named = ok && !calls.privateChat && !lockActive()
            var token = dbus.actionToken("call", true)
            callNotification.remoteActions = [
                dbusAction("default", "", "activate", ""),
                //% "Decline"
                callAction("decline", qsTrId("threesea-me-call-decline"), "declineCall", token),
                //% "Answer"
                callAction("answer", qsTrId("threesea-me-call-answer"), "answerCall", token)
            ]
            callNotification.summary = named ? contact.displayName : kind
            callNotification.body = named ? kind : ""
            callNotification.previewSummary = callNotification.summary
            callNotification.previewBody = callNotification.body
            callNotification.publish()
        })
    }
    function callAction(name, displayName, method, token) {
        var action = dbusAction(name, displayName, method, "")
        action.arguments = [token]
        return action
    }
    Notification {
        id: callNotification
        appName: "Threesea"
        appIcon: "harbour-threesea"
        category: "x-threesea.call"
        expireTimeout: 70000
    }

    // Notifications still shown from before a restart: take them over (by their chat).
    Notification { id: notificationQuery }
    // Called by main once the D-Bus name is ours, not on completion: a second process started from a
    // shell loads this too before it finds the name taken, and the notifications are the first's.
    function nameOwned() {
        var existing = notificationQuery.notifications()
        for (var i = 0; i < existing.length; ++i) takeOver(existing[i])
        dbus.retainTokens(Object.keys(notifications))
        if (backend.state === "locked") hideNotifications()
    }
    function takeOver(n) {
        // The last run promised to send a reply and ended first: that promise can't stand. The
        // app lock's reply waited in its chat's draft as well (keepWaitingReply).
        if (n.body === qsTrId("threesea-la-app-locked-reply-body")
                || n.body === qsTrId("threesea-la-locked-notification-reply-body")) {
            var kept = n.body === qsTrId("threesea-la-app-locked-reply-body")
            n.close()
            replyEnded(kept)
            return
        }
        // Said by the version before this one, about this one being there.
        if (n.summary === qsTrId("threesea-la-updated") || n.summary === qsTrId("threesea-la-removed")) {
            n.close()
            return
        }
        var actions = n.remoteActions || []
        // Of a call this process knows nothing about.
        if (actions.some(function(action) { return action.method === "answerCall" })) {
            n.close()
            return
        }
        var identity = actions.length > 0 && actions[0].arguments ? actions[0].arguments[0] : ""
        if (actions.length > 0 && actions[0].method === "showSafe") {
            // Still the news it was; checkSafeWarning() takes a failing Safe's away if it is over.
            identity = n.summary === qsTrId("threesea-la-safe-failing-notification") ? "safeFailing" : "backupReminder"
        } else if (n.summary === qsTrId("threesea-la-reply-not-sent")) {
            // It opens the chat, but isn't that chat's notification.
            identity = identity ? "replyNotSent:" + identity : "replyNotSent"
        } else if (!identity && actions.length > 0 && actions[0].method === "activate") {
            // The private chats' notification opens the app only, and so do the ones for the lock
            // and for a failed start.
            identity = n.summary === qsTrId("threesea-la-locked-notification") ? "appLocked"
                     : n.summary === qsTrId("threesea-la-startup-failed") ? "startFailed"
                     : n.summary === qsTrId("threesea-la-license-notification")
                       && n.body === qsTrId("threesea-la-license-notification-body") ? "license"
                     : n.summary === qsTrId("threesea-la-license-notification")
                       || n.summary === qsTrId("threesea-la-server-message-notification") ? "serverMessage" : "private"
        }
        if (identity && !notifications[identity]) {
            notifications[identity] = n
            if (identity === "appLocked") appLockedNews = true
            watchClosed(identity, n)
        } else {
            n.close()
        }
    }

    // Dismissed on the lock screen or the events view, or one of its actions tapped, which then
    // reaches us after this: its token stays for a little while. Not when a newer notification
    // has taken its place.
    function notificationClosed(identity, n) {
        if (notifications[identity] === n) {
            delete notifications[identity]
            dbus.releaseToken(identity)
        }
    }

    function watchClosed(identity, n) {
        n.closed.connect(function() { window.notificationClosed(identity, n) })
    }

    // Taken over after a restart: it needs our actions again, so one of ours takes its place on
    // the screen.
    function ownNotification(identity, taken) {
        var n = notificationComponent.createObject(window, { identity: identity, renewToken: false,
                                                             replacesId: taken.replacesId, itemCount: taken.itemCount,
                                                             summary: taken.summary, body: taken.body })
        notifications[identity] = n
        return n
    }

    function notificationText(sender, message) {
        if (backend.notificationPreview) return (sender ? sender + ": " : "") + Fmt.preview(message)
        // A status line is a call (notify()), which has no message to hide, only who started it.
        if (message.kind === "status" && String(message.body).split("|")[0] === "groupCallStarted") {
            return Fmt.status("groupCallStarted")
        }
        //% "New message"
        return message.kind === "status" ? Fmt.preview(message) : qsTrId("threesea-la-new-message")
    }

    function publish(identity, title, sender, message) {
        if (lockActive()) {
            notifyLocked(message, false)
            return
        }
        var n = notifications[identity]
        if (!n) {
            n = notificationComponent.createObject(window, { identity: identity })
            notifications[identity] = n
            n.itemCount = 0
        } else if (n.own !== true) {
            n = ownNotification(identity, n)
        }
        n.itemCount = n.itemCount + 1
        n.messageId = message.id
        n.sender = sender
        n.timestamp = new Date(message.createdAt)
        // A missed call isn't one more of a chat's messages.
        if (message.kind === "status") n.alertedAt = 0
        alert(n, identity, identity.indexOf("g:") === 0, title, notificationText(sender, message))
    }

    // Quietly, as this isn't news: no sound or vibration (this category only has the light), and
    // no banner. The next message's alert() sets them all again.
    function publishQuietly(n, summary, body, banner) {
        n.category = "im.received"
        n.sound = ""
        n.summary = summary
        n.body = body
        n.previewSummary = banner ? summary : ""
        n.previewBody = banner ? body : ""
        n.publish()
    }

    // Its sender deleted or edited a message: what the notification shows of it follows, or the
    // words stay readable on the lock screen.
    function messagesChanged(messages) {
        messages.forEach(function(message) {
            // 4 is failed: not out, and it won't go by itself. The chat shows that.
            if (message.outgoing && message.state !== 0) delete repliesOnTheirWay[message.id]
        })
        if (lockActive()) return
        messages.forEach(function(message) {
            var n = notifications[message.conversation]
            if (!n || message.outgoing) return
            var text
            if (n.own === true) {
                if (n.messageId !== message.id) return
                text = notificationText(n.sender, message)
            } else {
                // Taken over after a restart, it isn't known which one it shows, so any of them
                // may be it. Only for these two: reactions and the like change messages as well.
                if (message.kind !== "deleted" && !message.editedAt) return
                text = qsTrId("threesea-la-new-message")
                n = ownNotification(message.conversation, n)
            }
            if (n.body !== text) publishQuietly(n, n.summary, text, false)
        })
    }

    // Private chats: neither who nor what, and tapping only opens the app (ADR 0009).
    function notifyPrivate(message) {
        if (lockActive()) {
            notifyLocked(message, true)
            return
        }
        var n = notifications["private"]
        if (!n) {
            n = plainNotificationComponent.createObject(window, { key: "private" })
            notifications["private"] = n
            n.itemCount = 0
        }
        n.itemCount = n.itemCount + 1
        n.timestamp = new Date(message.createdAt)
        if (message.kind === "status") n.alertedAt = 0
        alert(n, "", message.conversation.indexOf("g:") === 0,
              //% "Private chat"
              qsTrId("threesea-la-private-chat"), qsTrId("threesea-la-new-message"))
    }

    // While the app is locked one notification stands for everything, like on Android: neither
    // who nor what, no Reply or Mark as read, and tapping it only opens the app. The core doesn't
    // redact these events, so this is the place that keeps the words off the screen.
    function appLockedNotification() {
        var n = notifications["appLocked"]
        if (!n) {
            n = plainNotificationComponent.createObject(window, { key: "appLocked" })
            notifications["appLocked"] = n
            n.itemCount = 0
        }
        return n
    }
    // It stands for more than a waiting reply.
    property bool appLockedNews: false
    function appLockedBody() {
        return NotificationActions.replyWaiting()
                //% "Open it and enter your PIN or passphrase to send your reply."
                ? qsTrId("threesea-la-app-locked-reply-body")
                //% "Something new has arrived. Open it and enter your PIN or passphrase to see it."
                : qsTrId("threesea-la-app-locked-body")
    }

    // A message or missed call while locked, with the chosen sound and vibration.
    function notifyLocked(message, isPrivate) {
        var n = appLockedNotification()
        appLockedNews = true
        n.itemCount = n.itemCount + 1
        n.timestamp = new Date(message.createdAt)
        if (message.kind === "status") n.alertedAt = 0
        alert(n, isPrivate ? "" : message.conversation, message.conversation.indexOf("g:") === 0,
              qsTrId("threesea-la-locked-notification"), appLockedBody())
    }

    // The package under this process has changed (bridge/applifecycle.h).
    function installationChanged(removed, windowOpen, windowAsked) {
        if (removed && !windowOpen) {
            // Nothing of a removed app stays on the screen, and none of its actions would work.
            repliesOnTheirWay = ({})
            forgetNotifications()
        } else if (removed) {
            //% "Threesea was removed from this device"
            plainNotification("installChanged", qsTrId("threesea-la-removed"),
                              //% "Close it to end it."
                              qsTrId("threesea-la-removed-body"))
        } else if (windowOpen) {
            //% "Threesea was updated"
            plainNotification("installChanged", qsTrId("threesea-la-updated"),
                              //% "Close it and open it again to use the new version."
                              qsTrId("threesea-la-updated-body"))
        } else if (windowAsked) {
            // A tap on the icon or a notification that this process ends on instead of answering.
            plainNotification("installChanged", qsTrId("threesea-la-updated"),
                              //% "Tap to open the new version."
                              qsTrId("threesea-la-updated-open-body"))
        } else if (backend.keepRunning) {
            // It was receiving messages without a window, and now it isn't.
            plainNotification("installChanged", qsTrId("threesea-la-updated"),
                              //% "Open it to receive messages again."
                              qsTrId("threesea-la-updated-closed-body"))
        } else {
            clearNotification("installChanged")
        }
    }

    // The app ends, and nobody keeps the grace time from here on: under a lock the words leave the
    // screen, as the next start may be far away. What waited in memory ends here, and a promise to
    // send it must not stay behind. All this reaches lipstick or doesn't before the process is
    // gone (takeOver() catches a promise that stayed).
    function appEnding() {
        lockedNotificationLater.stop()
        sayWhatIsOnItsWay()
        var kept = NotificationActions.allRepliesKept()
        var lost = NotificationActions.drop()
        if (lockInfo && lockInfo.lock && lockInfo.lock.appLock) hideNotifications()
        if (!lost) return
        clearNotification("locked")
        // It stands for something else as well, or only for the reply.
        if (appLockedNews) showAppLocked(0, false)
        else clearNotification("appLocked")
        replyEnded(kept)
    }

    function replyEnded(kept) {
        plainNotification("replyNotSent", qsTrId("threesea-la-reply-not-sent"), kept
                          //% "Threesea ended before sending it. Your text is kept as a draft in its chat."
                          ? qsTrId("threesea-la-reply-not-sent-ended-kept")
                          //% "Threesea ended before sending it. Write it again in the chat."
                          : qsTrId("threesea-la-reply-not-sent-ended"))
    }

    // Without a sound; `banner`: it is an answer to something just done.
    function showAppLocked(added, banner) {
        var body = appLockedBody()
        if (!added && notifications["appLocked"] && notifications["appLocked"].body === body) return
        var n = appLockedNotification()
        n.itemCount = Math.max(n.itemCount + added, 1)
        publishQuietly(n, qsTrId("threesea-la-locked-notification"), body, banner)
    }

    // The lock came on: what is on the screen about a chat goes, with its Reply and Mark as read.
    // Their tokens only start their grace: lipstick closes a notification first and calls us after
    // the device is unlocked, and a newer notification shares that token, so a tap may be on its
    // way. It waits for the PIN (runNotificationActions).
    function hideNotifications() {
        var added = 0
        Object.keys(notifications).forEach(function(key) {
            // Who wrote what isn't in these.
            if (["appLocked", "locked", "license", "serverMessage", "replyNotSent", "installChanged", "safeFailing", "backupReminder"].indexOf(key) >= 0
                    || key.indexOf("replyNotSent:") === 0) return
            added += Math.max(notifications[key].itemCount, 1)
            if (closeNotification(key)) dbus.releaseToken(key)
        })
        if (added > 0) appLockedNews = true
        var replyWaiting = NotificationActions.replyWaiting()
        // For the stored data's lock lockedNotification() tells about the reply.
        if (added > 0 || (replyWaiting && backend.state !== "locked")) showAppLocked(added, replyWaiting)
    }

    // The messages that came before the first look at the lock; until then it is not known what
    // their notifications may show.
    property var notificationsWaiting: []

    // `held`: the notification was kept back while another device led (multi-device), so the
    // message has been in the chat for a while — no in-app feedback for it any more.
    function notify(message, isPrivate, held) {
        if (!lockChecked) {
            notificationsWaiting.push({ message: message, isPrivate: isPrivate, held: held })
            return
        }
        // Of the status lines only missed calls ("call|missed|…", core/src/app_calls.rs) and group
        // calls others start ("groupCallStarted|…", core/src/app_group_calls.rs; without the name
        // in a locked private chat).
        var statusLine = String(message.body)
        if (!backend.notificationsEnabled
                || (message.kind === "status" && statusLine.indexOf("call|missed|") !== 0
                    && statusLine.split("|")[0] !== "groupCallStarted")) return
        if (!lockActive() && Qt.application.state === Qt.ApplicationActive && openChat === message.conversation) {
            // The message is right there, so no notification — only Android's quiet in-app
            // feedback, and none for one that has been standing there or a whole backlog.
            if (!held && !receivingBacklog) window.inAppFeedback()
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

    // Says whether there was one.
    function closeNotification(identity) {
        var n = notifications[identity]
        if (!n) return false
        n.close()
        delete notifications[identity]
        // A taken-over one belongs to the JavaScript engine, which collects it from here.
        if (n.own === true) n.destroy()
        return true
    }

    function clearNotification(identity) {
        if (closeNotification(identity)) dbus.forgetToken(identity)
    }

    // The ID was removed from this device.
    function forgetNotifications() {
        Object.keys(notifications).forEach(clearNotification)
        // And the ones outside that list: an update, the device cookie.
        var shown = notificationQuery.notifications()
        for (var i = 0; i < shown.length; ++i) shown[i].close()
        dbus.forgetAllTokens()
    }

    // Everything the window keeps of the ID (the bridge has done its part).
    function dataRemoved(linkedDevicesRemain) {
        forgetNotifications()
        NotificationActions.drop()
        repliesOnTheirWay = ({})
        actionsCame = false
        actionsRunning = 0
        actionsSendLimit.stop()
        actionsWaitLimit.stop()
        addressBook.active = false
        graceTimer.stop()
        lockedNotificationLater.stop()
        pendingLink = ""
        pendingChat = ""
        pendingShare = null
        notificationsWaiting = []
        offlineReason = null
        offlineReasonNotified = ""
        backlogLimit.stop()
        receivingBacklog = false
        backlogAlert = null
        appSettings = ({})
        lockInfo = null
        lockChecked = false
        leftAt = 0
        callName = ""
        groupCallName = ""
        voiceFrom = ""
        //% "This phone could not reach the server, so your linked devices stay linked to this ID."
        if (linkedDevicesRemain) Notices.show(qsTrId("threesea-la-delete-data-linked-remain"), Notice.Long, Notice.Center)
    }

    onOpenChatChanged: {
        if (openChat.length === 0) return
        clearNotification(openChat)
        clearNotification("replyNotSent:" + openChat)
    }

    // Reply and Mark as read from a notification. Such a tap can be what starts the app, so these
    // wait until the core is there (components/NotificationActions.js), and one made just before
    // the app locked waits for the PIN: nothing goes out in the user's name without it.
    function notificationAction(action) {
        NotificationActions.add(action)
        actionsCame = true
        runNotificationActions()
        lockedNotification()
        if (locked && action.text !== undefined) showAppLocked(0, true)
    }

    function runNotificationActions() {
        // Only these two states have an app lock (showLock); not known yet counts as locked.
        if ((backend.state === "ready" || backend.state === "needsLicense") && (!lockChecked || lockActive())) {
            NotificationActions.repliesToKeep().forEach(window.keepWaitingReply)
            leaveAfterActions()
            return
        }
        NotificationActions.release(backend.state).forEach(function(action) {
            window.actionsRunning = window.actionsRunning + 1
            if (action.text === undefined) {
                window.markNotifiedRead(action.identity, window.actionEnded)
                return
            }
            backend.call("sendText", { identity: action.identity, text: action.text },
                         function(ok, result, kind, message) {
                // Replying means it has been read; one that didn't go out hasn't been answered.
                if (ok) {
                    if (result && result.state === 0) window.replyOnItsWay(result.id, action.identity)
                    window.markNotifiedRead(action.identity, window.actionEnded)
                    if (action.kept || action.keeping) window.dropKeptReply(action.identity, action.text)
                } else {
                    window.replyRefused(action, Errors.text(kind, message), window.actionEnded)
                }
            })
        })
        if (backend.state === "error" && NotificationActions.drop()) {
            //% "Your reply was not sent"
            plainNotification("replyNotSent", qsTrId("threesea-la-reply-not-sent"),
                              //% "Threesea could not start. Open it to see why."
                              qsTrId("threesea-la-reply-not-sent-body"))
        }
        leaveAfterActions()
    }

    // With Keep running off or the package changed under it, and no window, a run ends with a
    // notification's action (lifecycle.leaveIfUnwanted, which knows about the window). The core
    // answering isn't enough: a reply it has stored would stay unsent until the app is next opened,
    // and that may be days. A wake-up asks the server for an answer after what is on its way, and
    // says how it went.
    property bool actionsCame: false
    property int actionsRunning: 0
    // Replies from notifications the core has stored and the server has not taken yet, by message:
    // their chats. If the app ends first they go out when it is next opened, which it says.
    property var repliesOnTheirWay: ({})
    function replyOnItsWay(id, identity) {
        repliesOnTheirWay[id] = identity
    }
    function sayWhatIsOnItsWay() {
        Object.keys(repliesOnTheirWay).forEach(function(id) {
            var identity = repliesOnTheirWay[id]
            var key = "replyNotSent:" + identity
            var n = notifications[key]
            if (!n) {
                n = plainNotificationComponent.createObject(window, { key: key, chat: identity })
                notifications[key] = n
            }
            n.summary = qsTrId("threesea-la-reply-not-sent")
            //% "It is sent when you next open Threesea."
            n.body = qsTrId("threesea-la-reply-not-sent-yet")
            n.previewSummary = n.summary
            n.previewBody = n.body
            n.publish()
        })
        repliesOnTheirWay = ({})
    }
    function actionEnded() {
        actionsRunning = actionsRunning - 1
        leaveAfterActions()
    }
    function runUnwanted() {
        return !backend.keepRunning || lifecycle.packageChanged()
    }
    function leaveAfterActions() {
        if (!actionsCame || !runUnwanted() || actionsRunning > 0) return
        if (NotificationActions.anyWaiting()) {
            actionsWaitLimit.restart()
            return
        }
        actionsCame = false
        actionsWaitLimit.stop()
        actionsSendLimit.restart()
        backend.call("wake", {}, function(ok, result) {
            if (!ok || !result.waiting) window.actionsSent()
        })
    }
    function actionsSent() {
        if (!actionsSendLimit.running) return
        actionsSendLimit.stop()
        leaveIfActionsDone()
    }
    // One that came just now is on its way, and ends the run when it has gone.
    function leaveIfActionsDone() {
        if (actionsRunning === 0 && !NotificationActions.anyWaiting()) lifecycle.leaveIfUnwanted()
    }
    // As long as a dead connection takes to find out and come back (wakeLimit).
    Timer {
        id: actionsSendLimit
        interval: 45000
        onTriggered: window.leaveIfActionsDone()
    }
    // Waiting behind a lock. As it ends the app says what became of the reply (appEnding): in the
    // chat's draft, or to be written again if the core refused that. So nobody has to come back for
    // it to end. Behind the passphrase no core runs and there is no draft: the process holds the
    // only copy and stays for it, with nothing connected.
    Timer {
        id: actionsWaitLimit
        interval: 600000
        onTriggered: window.limitActionsWait()
    }

    function limitActionsWait() {
        if (runUnwanted() && backend.state !== "locked") lifecycle.leaveIfUnwanted()
    }

    function markNotifiedRead(identity, done) {
        backend.call("markRead", { identity: identity }, function() {
            backend.refreshUnread()
            if (done) done()
        })
        clearNotification(identity)
    }

    // A reply from a notification that the core refused. A notice would be drawn in a window that
    // isn't on screen, and the notification it was typed in has gone: this one says so, without
    // the words, which go back into the chat and wait there.
    function replyRefused(action, reason, done) {
        keepReply(action, function(kept) {
            var key = "replyNotSent:" + action.identity
            var n = notifications[key]
            if (!n) {
                // Tapping it opens the chat, where the words are.
                n = plainNotificationComponent.createObject(window, { key: key, chat: action.identity })
                notifications[key] = n
            }
            n.summary = qsTrId("threesea-la-reply-not-sent")
            //% "%1 Your text is kept as a draft in the chat."
            n.body = kept ? qsTrId("threesea-la-reply-not-sent-kept").arg(reason) : reason
            n.previewSummary = n.summary
            n.previewBody = n.body
            n.publish()
            done()
        })
    }

    // A reply without a valid token, after its grace or with its notification gone. Which chat is
    // not known: anything on the session bus can make such a call, so it isn't taken from there,
    // and that is also why this stays one notification.
    function replyLost() {
        if (notifications["replyNotSent"]) return
        plainNotification("replyNotSent", qsTrId("threesea-la-reply-not-sent"),
                          //% "It reached Threesea too late. Write it again in the chat."
                          qsTrId("threesea-la-reply-not-sent-late"))
    }

    // After what the chat's input holds already, there if the chat is open, otherwise in its draft.
    function keepReply(action, done) {
        var chat = pageStack.find(function(page) { return page.identity === action.identity && page.inputField })
        if (chat) {
            chat.keepText(action.text)
            chat.saveDraft()
            done(true)
            return
        }
        if (action.kept || action.keeping) done(true)
        else addToDraft(action.identity, action.text, done)
    }

    function addToDraft(identity, text, done) {
        backend.call("draft", { identity: identity }, function(ok, draft) {
            if (!ok) {
                done(false)
                return
            }
            var args = { identity: identity, text: draft.text ? draft.text + "\n" + text : text }
            if (draft.quote) args.quote = draft.quote.messageId
            backend.call("setDraft", args, function(saved) { done(saved) })
        })
    }

    // A reply waiting for the PIN: the core runs, so its words go into the chat's draft in the
    // encrypted store meanwhile, or they would be gone if the app ended first. Nothing is sent.
    function keepWaitingReply(action) {
        action.keeping = true
        addToDraft(action.identity, action.text, function(kept) {
            action.keeping = false
            action.kept = kept
        })
    }

    // It went out after all: its words leave the draft, whatever else is there by now. A draft
    // that held only them has gone already (the core's forget_sent_draft).
    function dropKeptReply(identity, text) {
        backend.call("draft", { identity: identity }, function(ok, draft) {
            if (!ok || !draft.text) return
            var at = draft.text.lastIndexOf(text)
            if (at < 0) return
            var args = { identity: identity,
                         text: draft.text.substring(0, at).replace(/\n$/, "") + draft.text.substring(at + text.length) }
            if (draft.quote) args.quote = draft.quote.messageId
            backend.call("setDraft", args)
        })
    }

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
                                 photo: /\.(jpe?g|png|webp|bmp)$/i.test(path),
                                 video: /\.(mp4|m4v|mov|3gp|webm|mkv|avi|mpe?g)$/i.test(path) })
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
            backend.call("matchAddressBook", { entries: entries, automatic: !manual }, function(ok, result, kind, message) {
                addressBook.active = false
                // Also after an error: who was found before it is in the list.
                backend.refreshContacts()
                if (ok) {
                    backend.addressBookSyncedAt = Date.now()
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

    // The call page on top can be held to the ear: only then does the sensor run for it.
    Binding {
        target: proximity
        property: "forCall"
        value: Qt.application.state === Qt.ApplicationActive && !!pageStack.currentPage
               && pageStack.currentPage.atEar === true
    }

    Connections {
        target: callRoute
        //% "The sound could not be switched"
        onFailed: Notices.show(qsTrId("threesea-la-call-route-failed"), Notice.Short, Notice.Center)
    }

    // The camera page is filming.
    property bool cameraRecording: false
    readonly property bool videoCallShowing: {
        if ((calls.camera || calls.remoteCamera) && (calls.state === "connecting" || calls.state === "active")) return true
        if (groupCalls.state !== "connected") return false
        if (groupCalls.camera) return true
        var others = groupCalls.participants
        for (var i = 0; i < others.length; ++i) {
            if (others[i].camera === true) return true
        }
        return false
    }
    // What is watched or goes on without a touch keeps the screen on while the app is in view: a
    // video, a call's pictures, filming, and talking into a voice message, also while it waits
    // for an answer at the limit (blanking leaves the app, which stops or throws these away).
    // Once the app has been left, the phone blanks and locks as usual.
    DisplayBlanking {
        preventBlanking: Qt.application.state === Qt.ApplicationActive
                         && (videoPlayer.playing || window.cameraRecording || window.videoCallShowing
                             || voiceRecorder.recording || voiceRecorder.held)
    }
    // A call goes on with the screen off, and a ringing one has to be answered in time.
    KeepAlive {
        enabled: window.anyCallRunning
    }

    // A call takes the sound and the microphone, also a group call whose page isn't on top. What
    // is playing pauses and stays paused, the call may be long. A recording has nobody to ask
    // during the call, and would pick up its sound.
    readonly property bool anyCallRunning: callRunning || (groupCalls.state !== "idle" && groupCalls.state !== "ended")
    onAnyCallRunningChanged: {
        if (!anyCallRunning) return
        if (voiceRecorder.recording || voiceRecorder.held) {
            voiceRecorder.cancel()
            //% "Recording stopped and discarded"
            Notices.show(qsTrId("threesea-la-recording-discarded"), Notice.Long, Notice.Center)
        }
        // The chime leads to the next message: that must not start in the call either.
        // Active: also while it is still being read, which would start it during the call.
        if (voicePlayer.active) voicePlayer.pause()
        if (videoPlayer.playing || videoPlayer.loading) videoPlayer.pause()
    }
    // Wherever it was started from: the file turned out unreadable, or the player gave up on it.
    Connections {
        target: voicePlayer
        //% "This voice message could not be played"
        onFailed: Notices.show(qsTrId("threesea-la-cannot-play"), Notice.Long, Notice.Center)
    }

    // The group of the group call we are in (cover); empty for a private group, as in the
    // notifications: the cover then shows only the state and the time.
    property string groupCallName
    function refreshGroupCallName() {
        var conversation = groupCalls.conversation
        backend.call("group", { key: conversation }, function(ok, group) {
            if (groupCalls.conversation !== conversation) return
            window.groupCallName = ok && group.private !== true ? group.title : ""
        })
    }
    Connections {
        target: groupCalls
        onStateChanged: {
            if (groupCalls.state !== "joining") return
            window.groupCallName = ""
            window.refreshGroupCallName()
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
        onShowWebSessionsRequested: window.showWebSessions()
        onShowSafeRequested: window.showSafe()
        onOpenUrlRequested: window.openLink(url, false)
        onMarkReadRequested: window.notificationAction({ identity: identity })
        onReplyRequested: window.notificationAction({ identity: identity, text: text })
        onAnswerCallRequested: {
            if (calls.state !== "incoming") return
            calls.accept()
            window.activate()
        }
        onDeclineCallRequested: if (calls.state === "incoming") calls.reject()
        onReplyLost: window.replyLost()
        onBlankReply: {
            window.actionsCame = true
            window.leaveAfterActions()
        }
    }

    Connections {
        target: lifecycle
        onInstallationChanged: window.installationChanged(removed, windowOpen, windowAsked)
    }

    Connections {
        target: backend
        // Logged in: whatever a server said before that has passed. What it says from here on
        // comes after this.
        onConnectionStateChanged: {
            if (backend.connectionState !== "connected") return
            // What the server itself said remains something it said. Our words about it being
            // refused or closed aren't true any more.
            if (window.offlineReason && window.offlineReason.kind !== "server") {
                window.clearNotification("serverMessage")
            }
            window.offlineReason = null
        }
        onStateChanged: {
            // A last run's notifications, after a crash or a restart: their names and words stay
            // behind the passphrase too. They would otherwise last until it is entered.
            if (backend.state === "locked") window.hideNotifications()
            window.showStatePage()
            window.startFailedNotification()
            window.syncAddressBook(false)
            if (backend.state === "ready") {
                window.clearNotification("license")
                window.checkSafeWarning()
            } else {
                window.pendingDataImport = null
            }
            window.runNotificationActions()
        }
        onDataRemoved: window.dataRemoved(linkedDevicesRemain)
        onDataRemovalFailed: {
            //% "The ID was not removed: %1"
            Notices.show(qsTrId("threesea-la-delete-data-failed").arg(Errors.text(kind, message)), Notice.Long, Notice.Center)
        }
        onNetworkChangedSettled: window.wakeOnNetworkChange()
        onCoreEvent: {
            // Held back for now (notify: false) or not, the backlog is still coming.
            if (name === "messageAdded") window.backlogGoesOn()
            // `notify`: the chat's own notification setting (muted, mentions only) allows it.
            if (name === "messageAdded" && !data.message.outgoing && data.notify !== false) {
                window.notify(data.message, data.private === true)
            } else if (name === "messageNotify" && data.notify !== false) {
                // A forwarded message still unread after its grace time (multi-device).
                window.notify(data.message, data.private === true, true)
            } else if (name === "messagesChanged") {
                window.messagesChanged(data.messages)
            } else if (name === "settingsChanged") {
                window.loadAppSettings()
            } else if (name === "conversationRead") {
                // Read in Threema Web, on another device, or from a notification.
                window.clearNotification(data.identity)
                backend.refreshUnread()
            } else if (name === "chatDeleted") {
                // There is no place left to come back to.
                delete window.readingPlaces[data.identity]
            } else if (name === "privateChanged") {
                // Marked here, on another device or by a restore: its text leaves Events too.
                if (data.private) {
                    window.clearNotification(data.identity)
                    delete window.readingPlaces[data.identity]
                    window.clearNotification("replyNotSent:" + data.identity)
                }
            } else if (name === "offlineReason") {
                window.offlineReasonArrived(data.reason)
            } else if (name === "backlog") {
                window.backlogChanged(data.active)
            } else if (name === "wakeSettled") {
                window.endWake()
                window.actionsSent()
            } else if (name === "safeChanged" || name === "androidExportDone") {
                // A Data backup written ends the reminder as well.
                window.checkSafeWarning()
            } else if (name === "backupReminder") {
                //% "Back up your Threema ID"
                window.plainNotification("backupReminder", qsTrId("threesea-la-backup-reminder"),
                                         //% "A backup brings your ID, contacts and groups back on a new phone. Tap to set up Threema Safe."
                                         qsTrId("threesea-la-backup-reminder-body"), false, "showSafe")
            } else if (name === "safeServerCopyLeft") {
                // From here: the user may have left the Safe page during the wait.
                //% "Threema Safe is off. The backup could not be deleted from Threema's server, so it stays there until it expires."
                Notices.show(qsTrId("threesea-la-safe-server-copy-left"), Notice.Long, Notice.Center)
            } else if (name === "deviceRemovalFailed") {
                // On Linked devices the page itself tells. Elsewhere this is the only word: the
                // user may have left it during the wait.
                var top = pageStack.currentPage
                if (!top || top.objectName !== "linkedDevices") {
                    //% "Removing linked devices did not finish. Linked devices shows what is still linked, and you can try again there."
                    Notices.show(qsTrId("threesea-la-device-removal-failed"), Notice.Long, Notice.Center)
                }
            } else if (name === "licenseChanged") {
                window.licenseNotification()
            } else if (name === "deviceCookieChanged") {
                deviceCookieNotification.publish()
            } else if (name === "webSession") {
                window.updateWebConnected()
            } else if (name === "webCertificateChanged") {
                // Not when the page is there to ask at once.
                var page = pageStack.currentPage
                if (Qt.application.state !== Qt.ApplicationActive || !page || page.objectName !== "webSessions") {
                    webCertificateNotification.publish()
                }
            } else if (name === "storageProblem") {
                window.storageProblem = data.active
                if (data.active) storageNotification.publish()
                else storageNotification.close()
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
