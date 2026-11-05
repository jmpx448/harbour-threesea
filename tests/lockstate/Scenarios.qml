// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// One scenario per process: steps with the time to wait after each.
QtObject {
    id: root

    readonly property var stack: window.pageStack
    property int failures: 0
    property int step: 0
    property string handedOver

    readonly property var backend: fakes.backend
    readonly property var calls: fakes.calls
    readonly property var voice: fakes.voicePlayer
    readonly property var video: fakes.videoPlayer
    readonly property var recorder: fakes.voiceRecorder

    function names() {
        var list = []
        stack.find(function(page) {
            list.unshift(page.objectName)
            return false
        })
        return list.join(" ")
    }
    function page(name) {
        return stack.find(function(p) { return p.objectName === name })
    }
    function check(what, actual, expected) {
        if (String(actual) === String(expected)) return
        console.warn("FAILED", scenario, what, "got:", actual, "expected:", expected)
        failures = failures + 1
    }
    function checkStack(expected) {
        check("stack", names(), expected)
    }
    function push(url, properties, animated) {
        return stack.push(Qt.resolvedUrl(url), properties || {},
                          animated ? PageStackAction.Animated : PageStackAction.Immediate)
    }
    function unlock() {
        window.lockPage.unlocked("1234")
    }
    function findItem(item, test) {
        if (test(item)) return item
        var children = item.children || []
        for (var i = 0; i < children.length; ++i) {
            var found = findItem(children[i], test)
            if (found) return found
        }
        return null
    }
    // The code entry on one of the real pages.
    function entryOf(page) {
        return findItem(page, function(item) { return item.errorText !== undefined && item.title !== undefined })
    }
    // Its way out, wherever that is right now.
    function cancelButton(page) {
        return findItem(page, function(item) {
            return item.visible && item.icon !== undefined && String(item.icon.source).indexOf("icon-m-cancel") >= 0
        })
    }
    function countSent(command) {
        return backend.sent.filter(function(sent) { return sent === command }).length
    }

    // A message arriving in a chat that isn't open.
    readonly property string chat: "ABCD1234"
    property var kept: null
    function incoming(text, more) {
        var message = { id: "m" + text, conversation: chat, outgoing: false, kind: "text", text: text,
                        createdAt: Date.now() }
        for (var key in more) message[key] = more[key]
        backend.results["contact"] = { displayName: "A Contact" }
        backend.coreEvent("messageAdded", { message: message, notify: true })
    }

    // A message of that chat changed.
    function changed(message) {
        message.conversation = chat
        if (message.outgoing === undefined) message.outgoing = false
        backend.coreEvent("messagesChanged", { messages: [message] })
    }

    // The one notification while locked: neither who nor what, tapping it only opens the app.
    function checkAppLocked(count) {
        var n = window.notifications["appLocked"]
        check("the lock's notification", !!n, true)
        check("its title", n.summary, "threesea-la-locked-notification")
        check("its text", n.body, "threesea-la-app-locked-body")
        check("count", n.itemCount, count)
        check("only opens the app", n.remoteActions.length === 1 && n.remoteActions[0].method, "activate")
        check("no arguments", n.remoteActions[0].arguments.length, 0)
    }

    // The usual start: ready, with a lock (or without) as the core reports it.
    function ready(appLock, graceSecs) {
        harness.setActive(true)
        backend.results["lockStatus"] = { lock: { appLock: appLock, graceSecs: graceSecs || 0, keyboard: false } }
        backend.unlockedAtStart = true
        backend.state = "ready"
    }

    readonly property var dbus: fakes.dbus
    readonly property var lifecycle: fakes.lifecycle

    readonly property var scenarios: ({
        // The package changing under the process: a word about it, and the old one's goes.
        updateWithAWindowUpSaysSo: [
            [function() { ready(false) }, 100],
            [function() {
                lifecycle.installationChanged(false, true, false)
                var n = window.notifications["installChanged"]
                check("told", n.summary, "threesea-la-updated")
                check("what to do", n.body, "threesea-la-updated-body")
                check("tapping it opens the app", n.remoteActions[0].method, "activate")
                lifecycle.installationChanged(false, false, false)
                check("Keep running off: nothing is missed", window.notifications["installChanged"], undefined)
                backend.keepRunning = true
                lifecycle.installationChanged(false, false, false)
                check("ended, which is news", window.notifications["installChanged"].body, "threesea-la-updated-closed-body")
            }, 0],
        ],
        // Asked for the window after an update: it can't start the new version itself.
        updateAskedForTheWindowSaysToTap: [
            [function() { ready(false) }, 100],
            [function() {
                lifecycle.installationChanged(false, false, true)
                var n = window.notifications["installChanged"]
                check("told", n.summary, "threesea-la-updated")
                check("what to do", n.body, "threesea-la-updated-open-body")
                check("tapping it opens the app", n.remoteActions[0].method, "activate")
            }, 0],
        ],
        // A reply after an update goes out from here, and then this process ends, Keep running or not.
        replyAfterAnUpdateEndsTheRun: [
            [function() {
                backend.keepRunning = true
                lifecycle.changed = true
                ready(false)
            }, 100],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                check("sent", countSent("sendText"), 1)
                check("a wake-up asks how it went", countSent("wake"), 1)
                check("gone", lifecycle.left, 1)
            }, 0],
        ],
        // The send limit running out while another reply is on its way: that one ends the run.
        replyOnItsWayHoldsTheEnd: [
            [function() {
                ready(false)
                backend.hold("sendText")
            }, 100],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                window.leaveIfActionsDone()
                check("stays for it", lifecycle.left, 0)
                backend.release("sendText")
            }, 100],
            [function() { check("gone", lifecycle.left, 1) }, 0],
        ],
        updateNoticeGoesWithTheNewVersion: [
            [function() {
                var n = { summary: "threesea-la-updated", closed: false, close: function() { n.closed = true } }
                window.takeOver(n)
                check("closed", n.closed, true)
                check("not kept", window.notifications["installChanged"], undefined)
                check("nor as something else", window.notifications["private"], undefined)
            }, 0],
        ],
        removalLeavesNoNotifications: [
            [function() { ready(false) }, 100],
            [function() { incoming("words") }, 100],
            [function() {
                check("one", !!window.notifications[chat], true)
                lifecycle.installationChanged(true, false, false)
                check("gone", window.notifications[chat], undefined)
            }, 0],
        ],
        // Keep running off and started by a notification's action: it stays until the reply has
        // gone out, and no longer.
        replyEndsTheRunWhenItHasGone: [
            [function() {
                ready(false)
                backend.results["wake"] = { waiting: true }
            }, 100],
            [function() { check("nothing happened yet", lifecycle.left, 0) }, 0],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                check("sent", countSent("sendText"), 1)
                check("read", countSent("markRead"), 1)
                check("a wake-up asks how it went", countSent("wake"), 1)
                check("still on its way", lifecycle.left, 0)
                backend.coreEvent("wakeSettled", {})
                check("gone", lifecycle.left, 1)
            }, 100],
        ],
        // The core not connected: nothing to wait for.
        markReadEndsTheRunAtOnceWithoutAConnection: [
            [function() { ready(false) }, 100],
            [function() { dbus.markReadRequested(chat) }, 100],
            [function() {
                check("read", countSent("markRead"), 1)
                check("gone", lifecycle.left, 1)
            }, 0],
        ],
        blankReplyEndsTheRun: [
            [function() { ready(false) }, 100],
            [function() { dbus.blankReply() }, 100],
            [function() { check("gone", lifecycle.left, 1) }, 0],
        ],
        // A second process loads the window too, and the notifications it finds are the first's.
        notificationsWaitForTheName: [
            [function() {
                check("not before the name is ours", dbus.retained, 0)
                window.nameOwned()
                check("then", dbus.retained, 1)
            }, 0],
        ],
        // The core runs and took no copy: the run ends all the same, and says the reply is lost.
        waitingReplyWithoutACopyEndsTheRunAtTheLimit: [
            [function() {
                ready(true)
                backend.refused["setDraft"] = "internal"
            }, 100],
            [function() { window.lockNow() }, 50],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                check("stays for now", lifecycle.left, 0)
                window.limitActionsWait()
                check("gone", lifecycle.left, 1)
            }, 0],
        ],
        // No core: the process holds the only copy.
        waitingReplyStaysBehindThePassphrase: [
            [function() { backend.state = "locked" }, 100],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                window.limitActionsWait()
                check("stays", lifecycle.left, 0)
            }, 0],
        ],
        // A refused reply goes to its draft first.
        refusedReplyEndsTheRunWhenKept: [
            [function() {
                ready(false)
                backend.refused["sendText"] = "messageTooLong"
            }, 100],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                check("kept", countSent("setDraft"), 1)
                check("no read", countSent("markRead"), 0)
                check("gone", lifecycle.left, 1)
            }, 0],
        ],
        // Keep running on: the run goes on.
        replyLeavesARunThatKeepsRunning: [
            [function() {
                backend.keepRunning = true
                ready(false)
            }, 100],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                check("sent", countSent("sendText"), 1)
                check("no wake-up for it", countSent("wake"), 0)
                check("stays", lifecycle.left, 0)
            }, 0],
        ],
        // Behind the app lock the reply waits for the PIN, and the run with it.
        replyBehindTheLockEndsTheRunAfterThePin: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() { dbus.replyRequested(chat, "words") }, 100],
            [function() {
                check("not sent", countSent("sendText"), 0)
                check("stays", lifecycle.left, 0)
                unlock()
            }, 100],
            [function() {
                check("sent", countSent("sendText"), 1)
                check("gone", lifecycle.left, 1)
            }, 0],
        ],
        // Behind the lock nothing shows that the microphone is on.
        lockEndsRecording: [
            [function() { ready(true) }, 100],
            [function() {
                recorder.recording = true
                window.lockNow()
                check("recording gone", recorder.recording, false)
            }, 100],
        ],
        // The grace time ending lets a voice message finish, but not a recording.
        graceEndsRecording: [
            [function() { ready(true, 1) }, 100],
            [function() {
                recorder.recording = true
                window.lockNow(true)
                check("recording gone", recorder.recording, false)
            }, 100],
        ],
        // Without an app lock, coming back after the grace time leaves it alone.
        noLockKeepsRecording: [
            [function() { ready(false) }, 100],
            [function() {
                recorder.recording = true
                window.lockNow()
                check("still recording", recorder.recording, true)
            }, 100],
        ],
        // A call, ringing already: no recording goes on, and nothing plays over it.
        callStopsMedia: [
            [function() { ready(false) }, 100],
            [function() {
                recorder.recording = true
                voice.active = true
                voice.playing = true
                video.playing = true
                calls.state = "incoming"
                check("recording gone", recorder.recording, false)
                check("voice paused", voice.playing, false)
                check("voice not stopped", voice.stops, 0)
                check("video paused", video.playing, false)
            }, 100],
        ],
        // A recording waiting for its answer goes as well, and the end of a call starts nothing.
        callEndDoesNotResume: [
            [function() { ready(false) }, 100],
            [function() {
                recorder.held = true
                calls.state = "calling"
                check("held recording gone", recorder.held, false)
                calls.state = "active"
                check("one cancel for one call", recorder.cancels, 1)
                calls.state = "ended"
                calls.state = "idle"
                check("voice stays paused", voice.playing, false)
            }, 100],
        ],
        // Left in the middle of a capture: its plain file goes with the page.
        cameraPageRemovesCapture: [
            [function() { ready(false) }, 100],
            [function() {
                var camera = push("pages/CameraPage.qml")
                check("camera page loads", !!camera, true)
                camera.capturePath = "/cache/capture/left.mp4"
                camera.pendingPath = "/cache/capture/stopped.mp4"
                stack.pop(page("conversations"), PageStackAction.Immediate)
            }, 200],
            [function() {
                check("capture removed", backend.removedCaptures.indexOf("/cache/capture/left.mp4") >= 0, true)
                check("stopped one removed", backend.removedCaptures.indexOf("/cache/capture/stopped.mp4") >= 0, true)
            }, 0],
        ],
        // The file the recorder reports is the one that goes.
        cameraPageRemovesWhatIsWritten: [
            [function() { ready(false) }, 100],
            [function() {
                var camera = push("pages/CameraPage.qml")
                camera.capturePath = "/cache/capture/asked.mp4"
                camera.recorderWrites("file:///cache/capture/written.mp4")
                camera.recorderWrites("")
                check("follows the recorder", camera.capturePath, "file:///cache/capture/written.mp4")
                // An error between the shutter and the stop: nothing is left to hand over.
                camera.pendingPath = camera.capturePath
                camera.removeCapture()
                check("removed", backend.removedCaptures.indexOf("file:///cache/capture/written.mp4") >= 0, true)
                check("nothing to hand over", camera.pendingPath, "")
            }, 0],
        ],
        // A video handed to the chat is the chat's: the page going, a late error or a late report
        // from the recorder leave it alone.
        cameraKeepsItsHandsOffHandedOverVideo: [
            [function() { ready(false) }, 100],
            [function() {
                var camera = push("pages/CameraPage.qml", { objectName: "camera" })
                camera.captured.connect(function(path, video) { root.handedOver = path })
            }, 100],
            [function() {
                var camera = page("camera")
                camera.capturePath = "/cache/capture/video.mp4"
                camera.recorderWrites("file:///cache/capture/video.mp4")
                camera.finish("file:///cache/capture/video.mp4")
                check("handed over", root.handedOver, "file:///cache/capture/video.mp4")
                camera.recorderWrites("file:///cache/capture/video.mp4")
                check("a late report doesn't take it back", camera.capturePath, "")
                camera.removeCapture()
            }, 1000],
            [function() {
                checkStack("conversations")
                check("nothing removed", backend.removedCaptures.length, 0)
            }, 0],
        ],
        // It is only seen while the app is in front and it is on top.
        cameraPagesFollowTheApp: [
            [function() { ready(false) }, 100],
            [function() { push("pages/CameraPage.qml", { objectName: "camera" }) }, 100],
            [function() {
                var camera = page("camera")
                check("camera shown", camera.shown, true)
                harness.setActive(false)
                check("camera not shown after leaving", camera.shown, false)
                harness.setActive(true)
                check("camera shown again", camera.shown, true)
                push("pages/ScanQrPage.qml", { objectName: "scanner" })
            }, 100],
            [function() {
                var scanner = page("scanner")
                check("camera under the scanner", page("camera").shown, false)
                check("scanner shown", scanner.shown, true)
                harness.setActive(false)
                check("scanner not shown after leaving", scanner.shown, false)
                harness.setActive(true)
            }, 0],
        ],
        // A capture finishing under the call's page or the lock: not those that are popped.
        cameraHandsOverUnderAPage: [
            [function() { ready(false) }, 100],
            [function() {
                var camera = push("pages/CameraPage.qml", { objectName: "camera" })
                camera.captured.connect(function(path, video) { root.handedOver = path })
                push("pages/PlainPage.qml")
            }, 100],
            [function() {
                page("camera").finish("/cache/capture/done.jpg")
                check("handed over", root.handedOver, "/cache/capture/done.jpg")
            }, 100],
            [function() {
                checkStack("conversations camera plain")
                stack.pop(undefined, PageStackAction.Immediate)
            }, 200],
            [function() { checkStack("conversations") }, 0],
        ],
        // Between two voice messages: the next one doesn't follow into the call.
        callEndsChimeQueue: [
            [function() { ready(false) }, 100],
            [function() {
                voice.active = true
                voice.chiming = true
                calls.state = "incoming"
                check("queue ended", voice.stops + voice.pauses, 1)
            }, 100],
        ],
        // A group call likewise, whose page need not be on top.
        groupCallStopsMedia: [
            [function() { ready(false) }, 100],
            [function() {
                recorder.recording = true
                voice.active = true
                voice.playing = true
                fakes.groupCalls.state = "joining"
                check("recording gone", recorder.recording, false)
                check("voice paused", voice.playing, false)
            }, 100],
        ],
        // Leaving during a page transition: the lock comes anyway.
        lockDuringTransition: [
            [function() { ready(true) }, 100],
            [function() {
                push("pages/PlainPage.qml", {}, true)
                check("transition running", stack.busy, true)
                window.lockNow()
                check("locked", window.locked, true)
            }, 200],
            [function() { checkStack("conversations plain unlock") }, 0],
        ],
        // What the cover's new message action, or any late reply, does to the stack.
        lockPagePopped: [
            [function() {
                ready(true)
            }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                checkStack("conversations unlock")
                stack.pop(page("conversations"), PageStackAction.Immediate)
            }, 200],
            [function() {
                checkStack("conversations unlock")
                check("still locked", window.locked, true)
                unlock()
            }, 100],
            [function() {
                checkStack("conversations")
                check("unlocked", window.locked, false)
            }, 0],
        ],
        // A plain push from a late reply: gone again, and one PIN is enough.
        pagePushedOverLock: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() { push("pages/PlainPage.qml") }, 200],
            [function() {
                checkStack("conversations unlock")
                unlock()
            }, 100],
            [function() {
                checkStack("conversations")
                check("unlocked", window.locked, false)
            }, 0],
        ],
        // The same with a call's page over the lock: that one stays.
        pagePushedOverCallOverLock: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                calls.identity = "ABCD1234"
                calls.state = "incoming"
            }, 700],
            [function() { push("pages/PlainPage.qml") }, 800],
            [function() {
                checkStack("conversations unlock call")
                check("locked", window.locked, true)
            }, 0],
        ],
        // The directory's reply to a link comes after the lock: nothing moves, it waits for the PIN.
        linkWhileLocking: [
            [function() {
                backend.results["parseLink"] = { action: "add", identity: "ABCD1234" }
                ready(true)
            }, 100],
            [function() {
                backend.hold("addContact")
                window.openLink("https://threema.id/ABCD1234", false)
            }, 50],
            [function() {
                window.lockNow()
                backend.release("addContact")
            }, 200],
            [function() {
                checkStack("conversations unlock")
                check("kept", window.pendingLink, "https://threema.id/ABCD1234")
                unlock()
            }, 500],
            [function() {
                checkStack("conversations contact")
                check("done", window.pendingLink, "")
            }, 0],
        ],
        // Tapped in a message: after the PIN the contact opens over the chat being read, which
        // stays open.
        linkFromMessageWhileLocking: [
            [function() {
                backend.results["parseLink"] = { action: "add", identity: "ABCD1234" }
                ready(true)
            }, 100],
            [function() { push("pages/ChatPage.qml", { identity: "EFGH5678" }) }, 50],
            [function() {
                backend.hold("addContact")
                window.openLink("https://threema.id/ABCD1234", true)
            }, 50],
            [function() {
                window.lockNow()
                backend.release("addContact")
            }, 200],
            [function() {
                checkStack("conversations chat unlock")
                unlock()
            }, 900],
            [function() {
                checkStack("conversations chat contact")
                check("done", window.pendingLink, "")
            }, 0],
        ],
        chatTappedWhileLocked: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                fakes.dbus.openChatRequested("ABCD1234")
                checkStack("conversations unlock")
                unlock()
            }, 500],
            [function() { checkStack("conversations chat") }, 0],
        ],
        shareWhileLocked: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                window.pendingShare = { files: [], text: "hello" }
                window.showShare()
                checkStack("conversations unlock")
                unlock()
            }, 500],
            [function() {
                checkStack("conversations share")
                check("text", page("share").text, "hello")
            }, 0],
        ],
        // On the keyboard the keypad is hidden, and its X with it: there is another one.
        passphrasePromptCanBeLeft: [
            [function() { ready(false) }, 800],
            [function() {
                var keyboard = push("pages/RealUnlockPage.qml", { passphraseCheck: true })
                check("keyboard", entryOf(keyboard).keyboard, true)
                cancelButton(keyboard).clicked(null)
            }, 800],
            [function() {
                checkStack("conversations")
                var keypad = push("pages/RealUnlockPage.qml")
                check("keypad", entryOf(keypad).keyboard, false)
                check("the keypad's X", !!cancelButton(keypad), true)
                cancelButton(keypad).clicked(null)
            }, 800],
            [function() {
                checkStack("conversations")
                var repeat = push("pages/PinSetupPage.qml", { minimumLength: 8, startKeyboard: true })
                entryOf(repeat).entered("12345678", true)
                check("repeat step", repeat.first, "12345678")
                check("no keyboard button", entryOf(repeat).allowKeyboard, false)
                cancelButton(repeat).clicked(null)
            }, 800],
            [function() { checkStack("conversations") }, 0],
        ],
        // The app lock stays up, on the keypad and on the keyboard.
        appLockCannotBeLeft: [
            [function() { ready(false) }, 100],
            [function() {
                var keypad = push("pages/RealUnlockPage.qml", { cancellable: false })
                check("no X on the keypad", !!cancelButton(keypad), false)
                entryOf(keypad).keyboard = true
                check("no X on the keyboard", !!cancelButton(keypad), false)
            }, 0],
        ],
        // The second tap comes before the core's reply: one attempt, not two.
        pinCheckIsOneAttempt: [
            [function() { ready(false) }, 100],
            [function() {
                backend.hold("unlock")
                backend.refused["unlock"] = "wrongPin"
                var unlockPage = push("pages/RealUnlockPage.qml", { cancellable: false })
                var entry = entryOf(unlockPage)
                entry._pin = "1234"
                entry.accept()
                entry.accept()
                check("busy", entry.busy, true)
                check("attempts", countSent("unlock"), 1)
            }, 50],
            [function() {
                check("attempts after a while", countSent("unlock"), 1)
                backend.release("unlock")
            }, 50],
            [function() {
                var entry = entryOf(stack.currentPage)
                check("busy afterwards", entry.busy, false)
                check("entry cleared", entry.pin, "")
                check("says so", entry.errorText, qsTrId("threesea-la-wrong-pin"))
            }, 0],
        ],
        // The mismatch stays with the step it belongs to, and the hints keep working.
        pinMismatchHint: [
            [function() { ready(false) }, 100],
            [function() {
                var setup = push("pages/PinSetupPage.qml")
                var entry = entryOf(setup)
                var hint = qsTrId("threesea-la-pin-hint")
                var mismatch = qsTrId("threesea-la-pin-mismatch")
                check("hint", entry.errorText, hint)
                entry.entered("1234", false)
                check("nothing under the repeat", entry.errorText, "")
                entry.entered("9999", false)
                check("first step again", setup.first, "")
                check("mismatch", entry.errorText, mismatch)
                entry.entered("1234", false)
                check("repeat after a mismatch", entry.errorText, "")
                entry.entered("9999", false)
                check("mismatch again", entry.errorText, mismatch)
                var chosen = ""
                setup.chosen.connect(function(pin) { chosen = pin })
                entry.entered("1234", false)
                entry.entered("1234", false)
                check("chosen", chosen, "1234")
            }, 0],
        ],
        // A hidden private chat: the PIN before anything is handed over; the X comes back to
        // the choice with the content still there.
        shareToPrivateChat: [
            [function() { ready(false) }, 100],
            [function() {
                backend.results["conversations"] = { conversations: [] }
                backend.refused["draft"] = "locked"
                var share = push("pages/RealSharePage.qml", { files: [{ path: "/made/up.jpg", name: "up.jpg" }], text: "hello" })
                share.open("ABCD1234")
                share.open("ABCD1234")
            }, 800],
            [function() {
                checkStack("conversations share unlock")
                check("private chats", page("unlock").privateChats, true)
                stack.pop(undefined, PageStackAction.Immediate)
                checkStack("conversations share")
                page("share").open("ABCD1234")
            }, 800],
            [function() {
                checkStack("conversations share unlock")
                page("unlock").unlocked("1234")
                checkStack("conversations chat")
                var chat = page("chat")
                check("identity", chat.identity, "ABCD1234")
                check("files", chat.attached.length, 1)
                check("text", chat.inputField.text, "hello")
            }, 0],
        ],
        // A blocked contact or a revoked ID takes nothing: the content stays here for another chat.
        shareToBlockedChat: [
            [function() { ready(false) }, 100],
            [function() {
                backend.results["conversations"] = { conversations: [] }
                backend.results["contact"] = { blocked: true }
                push("pages/RealSharePage.qml", { files: [{ path: "/made/up.jpg", name: "up.jpg" }] }).open("ABCD1234")
            }, 200],
            [function() {
                checkStack("conversations share")
                check("nothing asked of the chat", countSent("draft"), 0)
                backend.results["contact"] = { invalid: true }
                page("share").open("ABCD1234")
            }, 200],
            [function() {
                checkStack("conversations share")
                check("still nothing", countSent("draft"), 0)
                // Another chat takes it.
                backend.results["contact"] = {}
                page("share").open("BCDE2345")
            }, 200],
            [function() {
                checkStack("conversations chat")
                check("files", page("chat").attached.length, 1)
            }, 0],
        ],
        forwardToPrivateChat: [
            [function() { ready(false) }, 100],
            [function() {
                backend.results["conversations"] = { conversations: [] }
                backend.refused["draft"] = "locked"
                push("pages/RealSharePage.qml", { forwardMessage: 7 }).open("ABCD1234")
            }, 800],
            [function() {
                checkStack("conversations share unlock")
                check("nothing sent yet", countSent("forwardMessages"), 0)
                page("unlock").unlocked("1234")
                checkStack("conversations chat")
                check("sent", countSent("forwardMessages"), 1)
                check("to", backend.lastArgs["forwardMessages"].identity, "ABCD1234")
            }, 0],
        ],
        // A chat that can be read needs no PIN; other errors aren't the PIN's business either.
        shareToOpenChat: [
            [function() { ready(false) }, 100],
            [function() {
                backend.results["conversations"] = { conversations: [] }
                push("pages/RealSharePage.qml", { files: [{ path: "/made/up.jpg", name: "up.jpg" }] }).open("ABCD1234")
            }, 800],
            [function() {
                checkStack("conversations chat")
                check("files", page("chat").attached.length, 1)
            }, 0],
        ],
        // The app locks while the core answers: the lock stays on top, the choice stays open.
        lockWhileChoosingShareTarget: [
            [function() { ready(true) }, 100],
            [function() {
                backend.results["conversations"] = { conversations: [] }
                backend.hold("draft")
                push("pages/RealSharePage.qml", { files: [{ path: "/made/up.jpg", name: "up.jpg" }] }).open("ABCD1234")
                window.lockNow()
                backend.release("draft")
            }, 800],
            [function() {
                checkStack("conversations share unlock")
                check("the window's lock", stack.currentPage, window.lockPage)
                unlock()
            }, 100],
            [function() { checkStack("conversations share") }, 0],
        ],
        // A file's answer from the window: to the chat while it is there, and if it has gone
        // nothing is kept (so the bridge removes the file), and its caption is off the list.
        fileAnswerOutlivesChat: [
            [function() { ready(false) }, 100],
            [function() {
                var asked = []
                var chat = {
                    fileAnswered: function(file, ok) { asked.push(ok); return ok ? "" : file.path },
                    sendNext: function() { asked.push("next") }
                }
                var alive = { yes: true }
                var file = { path: "/cache/capture/a.jpg", caption: "a caption" }
                window.captionSent("ABCD1234", "a caption")
                check("on its way", window.captionOnItsWay("ABCD1234", "a caption"), true)
                check("another", window.captionOnItsWay("ABCD1234", "another"), false)
                check("any", window.captionOnItsWay("ABCD1234"), true)
                check("other chat", window.captionOnItsWay("EFGH5678"), false)
                var kept = window.fileAnswer(chat, alive, "ABCD1234", file)(false, {}, "messageTooLong", "")
                check("taken back", kept, file.path)
                check("told", asked.join(), "false,next")
                check("answered", window.captionOnItsWay("ABCD1234"), false)
                alive.yes = false
                window.captionSent("ABCD1234", "a caption")
                kept = window.fileAnswer(chat, alive, "ABCD1234", file)(false, {}, "messageTooLong", "")
                check("nothing kept", kept, undefined)
                check("chat not asked", asked.length, 2)
                check("answered too", window.captionOnItsWay("ABCD1234"), false)
            }, 0],
        ],
        // A link's text for a private chat, set before the chat found out: it survives the PIN.
        linkTextToPrivateChat: [
            [function() { ready(false) }, 100],
            [function() {
                var chat = push("pages/ChatPage.qml", { identity: "ABCD1234" })
                chat.inputField.text = "hello"
                window.unlockChat(chat, "ABCD1234")
                checkStack("conversations unlock")
                page("unlock").unlocked("1234")
                checkStack("conversations chat")
                check("text", page("chat").inputField.text, "hello")
            }, 0],
        ],
        // The license refused under the lock: the lock stays over the license page, and a new key
        // doesn't ask twice.
        licenseRefused: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() { backend.state = "needsLicense" }, 300],
            [function() {
                checkStack("license unlock")
                check("locked", window.locked, true)
                unlock()
            }, 100],
            [function() {
                checkStack("license")
                backend.state = "ready"
            }, 800],
            [function() {
                checkStack("conversations")
                check("not locked again", window.locked, false)
            }, 0],
        ],
        // Started with the license refused: the stored key is behind the PIN too.
        licenseRefusedAtStart: [
            [function() {
                backend.results["lockStatus"] = { lock: { appLock: true, graceSecs: 0, keyboard: false } }
                backend.state = "needsLicense"
            }, 300],
            [function() {
                checkStack("license unlock")
                unlock()
            }, 100],
            [function() {
                checkStack("license")
                backend.state = "ready"
            }, 800],
            [function() { checkStack("conversations") }, 0],
        ],
        // An import ending behind the lock: the result waits for the PIN.
        importBehindLock: [
            [function() { ready(true) }, 100],
            [function() { push("pages/DataBackupImportPage.qml", { filePath: "/x", password: "x" }) }, 100],
            [function() {
                window.lockNow()
                page("").finish({}, "")
            }, 400],
            [function() {
                checkStack("conversations  unlock")
                unlock()
            }, 400],
            [function() { checkStack("conversations backupResult") }, 0],
        ],
        // A call ending behind the lock leaves no page behind.
        callEndsBehindLock: [
            [function() { ready(true) }, 100],
            [function() {
                calls.identity = "ABCD1234"
                calls.state = "active"
            }, 700],
            [function() {
                checkStack("conversations call")
                window.lockNow()
            }, 50],
            [function() {
                checkStack("conversations call unlock")
                calls.state = "ended"
                calls.state = "idle"
            }, 200],
            [function() {
                checkStack("conversations call unlock")
                unlock()
            }, 500],
            [function() { checkStack("conversations") }, 0],
        ],
        // Under the lock the running call's state changes push no second page.
        callChangesBehindLock: [
            [function() { ready(true) }, 100],
            [function() {
                calls.identity = "ABCD1234"
                calls.state = "connecting"
            }, 700],
            [function() { window.lockNow() }, 50],
            [function() { calls.state = "active" }, 300],
            [function() {
                checkStack("conversations call unlock")
                unlock()
            }, 100],
            [function() { checkStack("conversations call") }, 0],
        ],
        // A call coming in while locked goes over the lock; afterwards the PIN is still asked.
        callOverLock: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                calls.identity = "ABCD1234"
                calls.state = "incoming"
            }, 700],
            [function() {
                checkStack("conversations unlock call")
                calls.state = "ended"
                calls.state = "idle"
            }, 500],
            [function() {
                checkStack("conversations unlock")
                check("locked", window.locked, true)
            }, 0],
        ],
        // A tap during a call: the call stays, the chat opens afterwards.
        tapDuringCall: [
            [function() { ready(false) }, 100],
            [function() {
                calls.identity = "ABCD1234"
                calls.state = "active"
            }, 700],
            [function() {
                fakes.dbus.openChatRequested("EFGH5678")
                window.openLink("https://threema.id/ABCD1234", false)
            }, 500],
            [function() {
                checkStack("conversations call")
                check("chat waits", window.pendingChat, "EFGH5678")
                calls.state = "ended"
            }, 800],
            [function() {
                check("opened", names().indexOf("call"), -1)
                check("nothing waits", window.pendingChat + window.pendingLink, "")
            }, 0],
        ],
        // Answering takes the microphone from the group call whose page lies beneath: when the
        // 1:1 call is over, neither page stays.
        groupCallEndsUnderCall: [
            [function() { ready(false) }, 100],
            [function() {
                fakes.groupCalls.conversation = "GROUP"
                fakes.groupCalls.state = "connected"
                push("pages/GroupCallPage.qml", { conversation: "GROUP" })
                calls.identity = "ABCD1234"
                calls.state = "incoming"
            }, 700],
            [function() {
                checkStack("conversations groupCall call")
                fakes.groupCalls.conversation = ""
                fakes.groupCalls.state = "idle"
                calls.state = "active"
            }, 200],
            [function() {
                checkStack("conversations groupCall call")
                calls.state = "ended"
                calls.state = "idle"
            }, 1500],
            [function() { checkStack("conversations") }, 0],
        ],
        // Anything else popping a running call's page: it comes back.
        callPagePopped: [
            [function() { ready(false) }, 100],
            [function() {
                calls.identity = "ABCD1234"
                calls.state = "active"
            }, 700],
            [function() { stack.pop(page("conversations"), PageStackAction.Immediate) }, 800],
            [function() { checkStack("conversations call") }, 0],
        ],
        // A private chat from the contacts, without any app lock.
        privateChatClosed: [
            [function() { ready(false) }, 100],
            [function() {
                push("pages/ContactsPage.qml")
                push("pages/ChatPage.qml", { identity: "ABCD1234", isPrivate: true })
                push("pages/PlainPage.qml")
                push("pages/ChatPage.qml", { identity: "EFGH5678", isPrivate: true })
                window.lockNow()
                checkStack("conversations contacts")
            }, 0],
        ],
        privateChatClosedDuringTransition: [
            [function() { ready(false) }, 100],
            [function() {
                push("pages/ChatPage.qml", { identity: "ABCD1234", isPrivate: true })
                push("pages/PlainPage.qml", {}, true)
                window.lockNow()
            }, 300],
            [function() { checkStack("conversations") }, 0],
        ],
        // A page holding the PIN goes when a lock takes effect, a lock for private chats alone too.
        pinPageClosed: [
            [function() { ready(false) }, 100],
            [function() {
                var holder = push("pages/PinPage.qml", { pin: "1234" })
                push("pages/PlainPage.qml")
                window.lockNow()
                check("PIN gone", holder.pin, "")
                checkStack("conversations")
            }, 0],
        ],
        // The stack takes no change during a transition: the page still goes, with its PIN first.
        pinPageClosedDuringTransition: [
            [function() { ready(false) }, 100],
            [function() {
                var holder = push("pages/PinPage.qml", { pin: "1234" })
                push("pages/PlainPage.qml", {}, true)
                window.lockNow()
                check("PIN gone", holder.pin, "")
            }, 300],
            [function() { checkStack("conversations") }, 0],
        ],
        // With the app lock on, the same page is not there behind the PIN either.
        pinPageClosedUnderLock: [
            [function() { ready(true) }, 100],
            [function() {
                push("pages/PinPage.qml", { pin: "1234" })
                window.lockNow()
                checkStack("conversations unlock")
                unlock()
            }, 100],
            [function() { checkStack("conversations") }, 0],
        ],
        // With a lock as well: the chat doesn't come back after the PIN.
        privateChatUnderLock: [
            [function() { ready(true) }, 100],
            [function() {
                push("pages/ChatPage.qml", { identity: "ABCD1234", isPrivate: true })
                window.lockNow()
                checkStack("conversations unlock")
                unlock()
            }, 100],
            [function() { checkStack("conversations") }, 0],
        ],
        // A private chat's PIN page (unlockChat) left open shows nothing of the chat, and stays:
        // it still takes the PIN to get on.
        privateChatPinLeftOpen: [
            [function() { ready(false) }, 100],
            [function() {
                window.unlockChat(push("pages/ChatPage.qml", { identity: "ABCD1234" }), "ABCD1234")
                window.lockNow()
            }, 100],
            [function() {
                checkStack("conversations unlock")
                page("unlock").unlocked("1234")
                checkStack("conversations chat")
            }, 0],
        ],
        // With a lock as well: the lock's PIN only leads back to that page.
        privateChatPinLeftOpenUnderLock: [
            [function() { ready(true) }, 100],
            [function() {
                window.unlockChat(push("pages/ChatPage.qml", { identity: "ABCD1234" }), "ABCD1234")
                stack.currentPage.objectName = "chatPin"
                window.lockNow()
            }, 100],
            [function() {
                checkStack("conversations chatPin unlock")
                unlock()
            }, 100],
            [function() {
                checkStack("conversations chatPin")
                check("not locked", window.locked, false)
                stack.currentPage.unlocked("1234")
                checkStack("conversations chat")
            }, 0],
        ],
        // That page's answer comes when the lock is up: nothing follows from it, and it asks again.
        privateChatPinAnsweredUnderLock: [
            [function() { ready(true) }, 100],
            [function() {
                backend.hold("unlock")
                var pinPage = push("pages/RealUnlockPage.qml", { privateChats: true })
                pinPage.objectName = "chatPin"
                pinPage.unlocked.connect(function() { handedOver = "chat" })
                var entry = entryOf(pinPage)
                entry._pin = "1234"
                entry.accept()
                window.lockNow()
            }, 100],
            [function() {
                checkStack("conversations chatPin unlock")
                backend.release("unlock")
            }, 100],
            [function() {
                check("nothing follows", handedOver, "")
                checkStack("conversations chatPin unlock")
                unlock()
            }, 100],
            [function() {
                checkStack("conversations chatPin")
                check("still nothing", handedOver, "")
                check("entry cleared", entryOf(stack.currentPage).pin, "")
            }, 0],
        ],
        // A running call's page survives closing the private chat beneath it.
        privateChatUnderCall: [
            [function() { ready(false) }, 100],
            [function() {
                push("pages/ChatPage.qml", { identity: "ABCD1234", isPrivate: true })
                calls.identity = "ABCD1234"
                calls.state = "active"
            }, 700],
            [function() { window.lockNow() }, 700],
            [function() { checkStack("conversations call") }, 0],
        ],
        // The grace time runs out while away.
        graceRunsOut: [
            [function() { ready(true, 1) }, 100],
            [function() {
                voice.active = true
                voice.messageId = 7
                voice.playing = true
                harness.setActive(false)
                check("not yet", window.locked, false)
                backend.sent = []
            }, 1300],
            [function() {
                check("locked while away", window.locked, true)
                checkStack("conversations unlock")
                check("the core locked", backend.sent.indexOf("lock") >= 0, true)
                check("voice plays on", voice.playing, true)
                check("not stopped", voice.stops, 0)
                voice.playing = false
            }, 1300],
            [function() {
                check("paused while locked: gone", voice.active, false)
                harness.setActive(true)
            }, 100],
            [function() { checkStack("conversations unlock") }, 0],
        ],
        // Coming back in time.
        backWithinGrace: [
            [function() { ready(true, 1) }, 100],
            [function() {
                voice.active = true
                voice.messageId = 7
                voice.playing = true
                harness.setActive(false)
            }, 300],
            [function() { harness.setActive(true) }, 1200],
            [function() {
                check("not locked", window.locked, false)
                checkStack("conversations")
                check("voice untouched", voice.stops, 0)
            }, 0],
        ],
        // Immediately: locked on leaving, and what plays stops.
        graceImmediately: [
            [function() { ready(true, 0) }, 100],
            [function() {
                voice.active = true
                voice.messageId = 7
                voice.playing = true
                harness.setActive(false)
                check("locked at once", window.locked, true)
                checkStack("conversations unlock")
                check("voice stopped", voice.playing, false)
            }, 0],
        ],
        // The first start fails, with a share and more waiting: StartupPage stays, nothing waits on.
        failsWithContentWaiting: [
            [function() {
                window.pendingShare = { files: [], text: "hello" }
                window.pendingChat = "ABCD1234"
                window.pendingLink = "threema://add?id=ABCD1234"
                window.pendingUpdate = true
                backend.errorText = "store"
                backend.state = "error"
            }, 800],
            [function() {
                check("share", window.pendingShare, null)
                check("chat", window.pendingChat, "")
                check("link", window.pendingLink, "")
                check("update", window.pendingUpdate, false)
            }, 0],
        ],
        // A long upgrade says what is going on; a short one doesn't flash a text.
        upgradingExplains: [
            [function() { backend.upgrading = true }, 200],
            [function() {
                check("not yet", stack.currentPage.upgradeTakesLong, false)
            }, 1500],
            [function() {
                check("now", stack.currentPage.upgradeTakesLong, true)
                backend.upgrading = false
            }, 0],
        ],
        // A failed start says what happened and how to go on, also when nobody is looking.
        failedStartExplains: [
            [function() {
                harness.setActive(false)
                backend.errorKind = "secrets"
                backend.errorText = "secrets read 22"
                backend.state = "error"
            }, 3300],
            [function() {
                var n = window.notifications["startFailed"]
                check("its notification", !!n, true)
                check("its title", n.summary, "threesea-la-startup-failed")
                var page = stack.currentPage
                check("can try again", page.canRetry, true)
                var hint = page.failureHint()
                check("the text for the kind", hint.indexOf("threesea-la-error-secrets") >= 0, true)
                check("how to go on", hint.indexOf("threesea-la-startup-try-again") >= 0, true)
                check("the code apart from the text", hint.indexOf("threesea-la-startup-code") >= 0, true)
                backend.errorKind = "storeTooNew"
                check("a retry can't help a downgrade", page.canRetry, false)
                check("nor does the text offer it",
                      page.failureHint().indexOf("threesea-la-startup-try-again") >= 0, false)
                backend.state = "starting"
            }, 50],
            [function() {
                check("gone when trying again", window.notifications["startFailed"], undefined)
                checkStack("")
            }, 0],
        ],
        // A failure the window is looking at needs no notification.
        failedStartInFrontStaysQuiet: [
            [function() {
                harness.setActive(true)
                backend.errorKind = "diskFull"
                backend.errorText = "diskFull sqlite 13"
                backend.state = "error"
            }, 3300],
            [function() { check("no notification", window.notifications["startFailed"], undefined) }, 0],
        ],
        // The store fails after the passphrase: the failure shows.
        failsAfterPassphrase: [
            [function() { backend.state = "locked" }, 100],
            [function() {
                checkStack("unlock")
                backend.state = "starting"
            }, 100],
            [function() {
                checkStack("unlock")
                backend.errorText = "store"
                backend.state = "error"
            }, 800],
            [function() { checkStack("") }, 0],
        ],
        // A chat's notification goes when it is closed, either way, with what it held.
        notificationObjectsGo: [
            [function() { ready(false) }, 100],
            [function() { incoming("first") }, 50],
            [function() {
                root.kept = window.notifications[chat]
                check("its object", root.kept.own, true)
                root.kept.closed(1)
                check("out of the list", window.notifications[chat], undefined)
            }, 50],
            [function() {
                check("destroyed when dismissed", root.kept, null)
                incoming("second")
            }, 50],
            [function() {
                root.kept = window.notifications[chat]
                window.clearNotification(chat)
                check("out of the list too", window.notifications[chat], undefined)
            }, 50],
            [function() { check("destroyed when cleared", root.kept, null) }, 0],
        ],
        // One taken over after a restart: replaced once by one of ours, at the same place.
        notificationTakenOver: [
            [function() { ready(false) }, 100],
            [function() {
                root.kept = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { replacesId: 42; itemCount: 3 }", root)
                window.notifications[chat] = root.kept
                incoming("first")
            }, 50],
            [function() {
                var n = window.notifications[chat]
                check("one of ours", n.own, true)
                check("the chat", n.identity, chat)
                check("counted on", n.itemCount, 4)
                root.kept = n
                incoming("second")
            }, 50],
            [function() {
                var n = window.notifications[chat]
                check("still that one", n === root.kept, true)
                check("counted on", n.itemCount, 5)
            }, 0],
        ],
        // Locked: one notification without who or what, without Reply, for messages and calls.
        lockedMessageShowsNothing: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() { incoming("the words") }, 50],
            [function() {
                check("no chat notification", window.notifications[chat], undefined)
                checkAppLocked(1)
                incoming("more", { kind: "status", text: undefined, body: "call|missed|0" })
            }, 50],
            [function() {
                check("no chat notification", window.notifications[chat], undefined)
                checkAppLocked(2)
                backend.coreEvent("messageAdded", { private: true, notify: true,
                    message: { conversation: chat, outgoing: false, kind: "text", createdAt: Date.now() } })
            }, 50],
            [function() {
                check("no notification for the private chat", window.notifications["private"], undefined)
                checkAppLocked(3)
                unlock()
            }, 100],
            [function() {
                check("gone with the lock", window.notifications["appLocked"], undefined)
                incoming("after")
            }, 50],
            [function() { check("shown again", window.notifications[chat].body.indexOf("after") >= 0, true) }, 0],
        ],
        // Past the grace time by the clock, while its timer sleeps with the device.
        lockedByClockShowsNothing: [
            [function() { ready(true, 60) }, 100],
            [function() {
                harness.setActive(false)
                check("still open", window.locked, false)
            }, 20],
            [function() {
                window.leftAt = Date.now() - 61000
                incoming("the words")
            }, 50],
            [function() {
                check("locked by now", window.locked, true)
                check("no chat notification", window.notifications[chat], undefined)
                checkAppLocked(1)
            }, 0],
        ],
        // Within the grace time it is still the chat's own notification.
        graceShowsTheMessage: [
            [function() { ready(true, 60) }, 100],
            [function() { harness.setActive(false) }, 20],
            [function() { incoming("the words") }, 50],
            [function() {
                var n = window.notifications[chat]
                check("the text", n.body.indexOf("the words") >= 0, true)
                check("Reply and Mark as read", n.remoteActions.length, 3)
            }, 0],
        ],
        // What was on the screen goes when the lock comes on, tokens included.
        lockHidesWhatIsShown: [
            [function() { ready(true, 60) }, 100],
            [function() { harness.setActive(false) }, 20],
            [function() { incoming("the words") }, 50],
            [function() {
                fakes.dbus.forgotten = []
                window.lockNow(true)
                check("no chat notification", window.notifications[chat], undefined)
                // A tap may be on its way: the PIN stops what it asks for, not the token.
                check("its token's grace", fakes.dbus.released.indexOf(chat) >= 0, true)
                check("its token still counts", fakes.dbus.forgotten.indexOf(chat), -1)
                checkAppLocked(1)
                check("no sound", window.notifications["appLocked"].sound, "")
                check("no banner", window.notifications["appLocked"].previewBody, "")
            }, 0],
        ],
        // The app ending under a lock: nobody keeps the grace time from here on.
        quitHidesWhatIsShown: [
            [function() { ready(true, 60) }, 100],
            [function() { incoming("the words") }, 50],
            [function() {
                Qt.application.aboutToQuit()
                check("no chat notification", window.notifications[chat], undefined)
                checkAppLocked(1)
            }, 0],
        ],
        // Without a lock it stays.
        quitLeavesWithoutLock: [
            [function() { ready(false) }, 100],
            [function() { incoming("the words") }, 50],
            [function() {
                Qt.application.aboutToQuit()
                check("still there", window.notifications[chat].body, "the words")
            }, 0],
        ],
        // A last run's notifications don't wait for the passphrase to go.
        passphraseHidesTakenOver: [
            [function() {
                root.kept = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { summary: \"A Contact\"; body: \"the words\"; itemCount: 2 }", root)
                root.kept.remoteActions = [{ name: "default", displayName: "", service: "s", path: "/", iface: "i",
                                             method: "openChat", arguments: [chat] }]
                window.takeOver(root.kept)
                check("taken over", window.notifications[chat] === root.kept, true)
                backend.state = "locked"
            }, 100],
            [function() {
                check("its text is gone", window.notifications[chat], undefined)
                checkAppLocked(2)
                check("its token's grace", fakes.dbus.released.indexOf(chat) >= 0, true)
                backend.unlockedAtStart = true
                backend.results["lockStatus"] = { lock: null }
                backend.state = "ready"
            }, 200],
            [function() { check("over with the passphrase", window.notifications["appLocked"], undefined) }, 0],
        ],
        // A reply waiting for the PIN is in the chat's draft meanwhile, and leaves it once sent.
        waitingReplyKeptInDraft: [
            [function() {
                ready(true)
                backend.results["draft"] = { text: "started earlier", quote: { messageId: "00ff" } }
            }, 100],
            [function() { window.lockNow() }, 50],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("nothing sent", countSent("sendText"), 0)
                check("kept", backend.lastArgs["setDraft"].text, "started earlier\na reply")
                check("once", countSent("setDraft"), 1)
                check("with its quote", backend.lastArgs["setDraft"].quote, "00ff")
                backend.results["draft"] = { text: "started earlier\na reply and more", quote: { messageId: "00ff" } }
                unlock()
            }, 100],
            [function() {
                check("sent", backend.lastArgs["sendText"].text, "a reply")
                check("out of the draft again", backend.lastArgs["setDraft"].text, "started earlier and more")
                check("with its quote", backend.lastArgs["setDraft"].quote, "00ff")
                check("no word of a failure", window.notifications["replyNotSent"], undefined)
            }, 0],
        ],
        // Refused after the PIN: it stays where it is, not in there twice.
        waitingReplyRefused: [
            [function() {
                ready(true)
                backend.refused["sendText"] = "contactBlocked"
            }, 100],
            [function() { window.lockNow() }, 50],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("kept", countSent("setDraft"), 1)
                unlock()
            }, 100],
            [function() {
                check("not twice", countSent("setDraft"), 1)
                check("says so", window.notifications["replyNotSent:" + chat].summary, "threesea-la-reply-not-sent")
            }, 0],
        ],
        // The app ends before the PIN: no promise stays, only what is true.
        endingLosesWaitingReply: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("promised", window.notifications["appLocked"].body, "threesea-la-app-locked-reply-body")
                Qt.application.aboutToQuit()
                check("no promise left", window.notifications["appLocked"], undefined)
                var n = window.notifications["replyNotSent"]
                check("says so", n.summary, "threesea-la-reply-not-sent")
                check("kept", n.body, "threesea-la-reply-not-sent-ended-kept")
                check("no words", (n.body + n.previewBody).indexOf("a reply"), -1)
            }, 0],
        ],
        // News as well: the notification stays for that, without the promise.
        endingKeepsTheNews: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() { incoming("the words") }, 50],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                Qt.application.aboutToQuit()
                checkAppLocked(1)
                check("says so", !!window.notifications["replyNotSent"], true)
            }, 0],
        ],
        // Waiting for the passphrase there is no core and no copy.
        endingBeforeThePassphrase: [
            [function() { backend.state = "locked" }, 100],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("no draft", countSent("setDraft"), 0)
                Qt.application.aboutToQuit()
                check("not kept", window.notifications["replyNotSent"].body, "threesea-la-reply-not-sent-ended")
            }, 0],
        ],
        // The promise from a last run that ended without taking it back.
        promiseTakenOver: [
            [function() { ready(true) }, 100],
            [function() {
                root.kept = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { summary: \"threesea-la-locked-notification\"; body: \"threesea-la-app-locked-reply-body\" }", root)
                root.kept.remoteActions = [{ name: "default", displayName: "", service: "s", path: "/", iface: "i",
                                             method: "activate", arguments: [] }]
                window.takeOver(root.kept)
                check("no promise", window.notifications["appLocked"], undefined)
                check("says so", window.notifications["replyNotSent"].body, "threesea-la-reply-not-sent-ended-kept")
                delete window.notifications["replyNotSent"]
                var other = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { summary: \"threesea-la-locked-notification\"; body: \"threesea-la-locked-notification-reply-body\" }", root)
                other.remoteActions = root.kept.remoteActions
                window.takeOver(other)
                check("no copy for the passphrase's", window.notifications["replyNotSent"].body, "threesea-la-reply-not-sent-ended")
            }, 0],
        ],
        // Before the first look at the lock nothing is known: these wait for the answer.
        messagesWaitForTheLockAnswer: [
            [function() {
                backend.hold("lockStatus")
                backend.results["lockStatus"] = { lock: { appLock: true, graceSecs: 0, keyboard: false } }
                backend.state = "ready"
            }, 100],
            [function() {
                incoming("the words")
                fakes.dbus.replyRequested(chat, "a reply")
            }, 50],
            [function() {
                check("nothing shown", window.notifications[chat], undefined)
                check("nothing sent", countSent("sendText"), 0)
                backend.release("lockStatus")
            }, 100],
            [function() {
                check("locked", window.locked, true)
                check("no chat notification", window.notifications[chat], undefined)
                check("nothing sent", countSent("sendText"), 0)
                check("the words stay out", window.notifications["appLocked"].body.indexOf("the words"), -1)
                check("says the reply waits", window.notifications["appLocked"].body, "threesea-la-app-locked-reply-body")
            }, 0],
        ],
        // Without a lock they go through as soon as that is known.
        messagesPassWithoutLock: [
            [function() {
                backend.hold("lockStatus")
                backend.results["lockStatus"] = { lock: null }
                backend.state = "ready"
            }, 100],
            [function() {
                incoming("the words")
                check("nothing yet", window.notifications[chat], undefined)
                backend.release("lockStatus")
            }, 100],
            [function() {
                check("the text", window.notifications[chat].body.indexOf("the words") >= 0, true)
                check("nothing about a lock", window.notifications["appLocked"], undefined)
            }, 0],
        ],
        // A Reply and a Mark as read on their way when the lock came on: only with the PIN.
        actionsWaitForThePin: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                fakes.dbus.replyRequested(chat, "a reply")
                fakes.dbus.markReadRequested("EFGH5678")
            }, 50],
            [function() {
                check("nothing sent", countSent("sendText"), 0)
                check("nothing read", countSent("markRead"), 0)
                check("says so", window.notifications["appLocked"].body, "threesea-la-app-locked-reply-body")
                check("with a banner", window.notifications["appLocked"].previewBody, "threesea-la-app-locked-reply-body")
                unlock()
            }, 100],
            [function() {
                check("sent", countSent("sendText"), 1)
                check("the words", backend.lastArgs["sendText"].text, "a reply")
                check("both read", countSent("markRead"), 2)
                check("gone with the lock", window.notifications["appLocked"], undefined)
            }, 0],
        ],
        // The action came past the grace time, before its timer ran out.
        actionWaitsPastTheGrace: [
            [function() { ready(true, 60) }, 100],
            [function() { harness.setActive(false) }, 20],
            [function() {
                window.leftAt = Date.now() - 61000
                fakes.dbus.replyRequested(chat, "a reply")
            }, 50],
            [function() {
                check("locked by now", window.locked, true)
                check("nothing sent", countSent("sendText"), 0)
            }, 0],
        ],
        // A reply that went out: read, and the notification gone.
        replySent: [
            [function() { ready(false) }, 100],
            [function() { incoming("the words") }, 50],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("sent", backend.lastArgs["sendText"].text, "a reply")
                check("read", countSent("markRead"), 1)
                check("gone", window.notifications[chat], undefined)
                check("nothing about a refusal", window.notifications["replyNotSent:" + chat], undefined)
            }, 0],
        ],
        // A refused reply: unread, the words kept, and a word about it without them.
        replyRefused: [
            [function() {
                ready(true, 60)
                backend.refused["sendText"] = "contactBlocked"
                backend.results["draft"] = { text: "started earlier", quote: { messageId: "00ff" } }
            }, 100],
            [function() { incoming("the words") }, 50],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("not read", countSent("markRead"), 0)
                check("the chat's notification stays", !!window.notifications[chat], true)
                check("after the draft", backend.lastArgs["setDraft"].text, "started earlier\na reply")
                check("for this chat", backend.lastArgs["setDraft"].identity, chat)
                check("with its quote", backend.lastArgs["setDraft"].quote, "00ff")
                var n = window.notifications["replyNotSent:" + chat]
                check("says so", n.summary, "threesea-la-reply-not-sent")
                check("not the words", (n.body + n.previewBody).indexOf("a reply"), -1)
                check("kept", n.body.indexOf("threesea-la-reply-not-sent-kept"), 0)
                check("opens the chat", n.remoteActions[0].method, "openChat")
                check("that chat", n.remoteActions[0].arguments[0], chat)
                // Still there behind the lock: nothing of who or what in it.
                window.lockNow()
                check("stays under the lock", !!window.notifications["replyNotSent:" + chat], true)
                check("the chat's goes", window.notifications[chat], undefined)
            }, 0],
        ],
        // One from the last run is about a chat, but isn't that chat's notification.
        replyNotSentTakenOver: [
            [function() { ready(false) }, 100],
            [function() {
                var n = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { summary: \"threesea-la-reply-not-sent\"; body: \"the reason\" }", root)
                n.remoteActions = [{ name: "default", displayName: "", service: "s", path: "/", iface: "i",
                                     method: "openChat", arguments: [chat] }]
                window.takeOver(n)
                check("not the chat's", window.notifications[chat], undefined)
                check("filed as what it is", window.notifications["replyNotSent:" + chat] === n, true)
                changed({ id: "unknown", kind: "deleted" })
                check("left alone by a deletion", n.body, "the reason")
                incoming("the words")
            }, 50],
            [function() {
                check("the chat's own", window.notifications[chat].body, "the words")
                check("still there", !!window.notifications["replyNotSent:" + chat], true)
                var plain = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { summary: \"threesea-la-reply-not-sent\" }", root)
                plain.remoteActions = [{ name: "default", displayName: "", service: "s", path: "/", iface: "i",
                                         method: "activate", arguments: [] }]
                window.takeOver(plain)
                check("nothing about a chat", window.notifications["replyNotSent"] === plain, true)
                check("not a private chat's", window.notifications["private"], undefined)
            }, 0],
        ],
        // Too late for its token: it says so, once, about no chat in particular.
        replyLostSaysSo: [
            [function() { ready(false) }, 100],
            [function() {
                fakes.dbus.replyLost()
                var n = window.notifications["replyNotSent"]
                check("says so", n.summary, "threesea-la-reply-not-sent")
                check("why", n.body, "threesea-la-reply-not-sent-late")
                check("opens the app", n.remoteActions[0].method, "activate")
                root.kept = n
                fakes.dbus.replyLost()
                check("the same one", window.notifications["replyNotSent"] === root.kept, true)
                check("nothing sent", countSent("sendText"), 0)
            }, 0],
        ],
        // Nowhere to keep it: it still says so.
        replyRefusedNotKept: [
            [function() {
                ready(false)
                backend.refused["sendText"] = "contactBlocked"
                backend.refused["draft"] = "notFound"
            }, 100],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("nothing saved", countSent("setDraft"), 0)
                var n = window.notifications["replyNotSent:" + chat]
                check("says so", n.summary, "threesea-la-reply-not-sent")
                check("no promise", n.body.indexOf("threesea-la-reply-not-sent-kept"), -1)
                check("not the words", n.body.indexOf("a reply"), -1)
            }, 0],
        ],
        // The chat is open: into its input, after what is there, and from there into its draft.
        replyRefusedChatOpen: [
            [function() {
                ready(false)
                backend.refused["sendText"] = "contactBlocked"
            }, 100],
            [function() {
                root.kept = push("pages/ChatPage.qml", { identity: chat })
            }, 100],
            [function() { fakes.dbus.replyRequested(chat, "a reply") }, 50],
            [function() {
                check("handed to the chat", root.kept.kept, "a reply")
                check("saved", root.kept.draftsSaved, 1)
                check("not over its draft", countSent("setDraft"), 0)
                check("says so", window.notifications["replyNotSent:" + chat].body.indexOf("threesea-la-reply-not-sent-kept"), 0)
                // Opening the chat ends it; here the chat is opened again after leaving.
                window.openChat = ""
                window.openChat = chat
                check("gone with the chat opened", window.notifications["replyNotSent:" + chat], undefined)
            }, 0],
        ],
        // Deleted by its sender: the words leave the notification, without another alert.
        deletedLeavesNotification: [
            [function() { ready(false) }, 100],
            [function() { incoming("the words") }, 50],
            [function() {
                var n = window.notifications[chat]
                check("the text", n.body, "the words")
                check("the banner", n.previewBody, "the words")
                changed({ id: "mthe words", kind: "deleted" })
                check("the words are gone", n.body, "threesea-la-message-deleted")
                check("no banner", n.previewBody, "")
                check("no sound", n.sound, "")
                check("only the light", n.category, "im.received")
                check("not counted", n.itemCount, 1)
                check("the same title", n.summary, "A Contact")
            }, 0],
        ],
        // Edited: the new words, with who wrote them in a group.
        editedChangesNotification: [
            [function() {
                ready(false)
                backend.results["group"] = { title: "A Group", left: false,
                                             members: [{ identity: "EFGH5678", displayName: "A Member" }] }
            }, 100],
            [function() {
                backend.coreEvent("messageAdded", { notify: true, message: {
                    id: "m1", conversation: "g:KEY", outgoing: false, kind: "text", text: "old",
                    sender: "EFGH5678", createdAt: Date.now() } })
            }, 50],
            [function() {
                check("the text", window.notifications["g:KEY"].body, "A Member: old")
                backend.coreEvent("messagesChanged", { messages: [{ id: "m1", conversation: "g:KEY", outgoing: false,
                                                                    kind: "text", text: "new", editedAt: 1 }] })
                check("the new text", window.notifications["g:KEY"].body, "A Member: new")
            }, 0],
        ],
        // Past the grace time by the clock, an edit brings no new words onto the screen.
        editedPastTheGrace: [
            [function() { ready(true, 60) }, 100],
            [function() { harness.setActive(false) }, 20],
            [function() { incoming("the words") }, 50],
            [function() {
                window.leftAt = Date.now() - 61000
                changed({ id: "mthe words", kind: "text", text: "new words", editedAt: 1 })
                check("locked by now", window.locked, true)
                check("no chat notification", window.notifications[chat], undefined)
                checkAppLocked(1)
            }, 0],
        ],
        // What it doesn't show, or what hasn't changed, leaves it alone.
        otherChangesLeaveNotification: [
            [function() { ready(false) }, 100],
            [function() { incoming("the words") }, 50],
            [function() {
                var n = window.notifications[chat]
                changed({ id: "another", kind: "deleted" })
                check("another message", n.body, "the words")
                changed({ id: "mthe words", kind: "text", text: "own", outgoing: true })
                check("an outgoing one", n.body, "the words")
                changed({ id: "mthe words", kind: "text", text: "the words" })
                check("the banner stays: not published", n.previewBody, "the words")
            }, 0],
        ],
        // Previews off, a missed call is still called that.
        missedCallWithoutPreview: [
            [function() {
                ready(false)
                backend.notificationPreview = false
                incoming("", { kind: "status", text: undefined, body: "call|missed|0" })
            }, 50],
            [function() { check("what it is", window.notifications[chat].body, "threesea-la-status-call-missed") }, 0],
        ],
        // Previews off: it stays generic.
        deletedWithoutPreview: [
            [function() {
                ready(false)
                backend.notificationPreview = false
            }, 100],
            [function() { incoming("the words") }, 50],
            [function() {
                var n = window.notifications[chat]
                check("generic", n.body, "threesea-la-new-message")
                changed({ id: "mthe words", kind: "deleted" })
                check("still generic", n.body, "threesea-la-new-message")
                check("not published", n.previewBody, "threesea-la-new-message")
            }, 0],
        ],
        // Taken over after a restart: which message it shows isn't known, so any deletion counts.
        deletedLeavesTakenOver: [
            [function() { ready(false) }, 100],
            [function() {
                // Publishing without a session bus sets its id to 0, so seen here, before that.
                var taken = Qt.createQmlObject("import Nemo.Notifications 1.0; Notification { replacesId: 42 }", root)
                check("the same place", window.ownNotification(chat, taken).replacesId, 42)
                window.notifications[chat] = Qt.createQmlObject(
                    "import Nemo.Notifications 1.0; Notification { replacesId: 42; summary: \"A Contact\"; body: \"the words\" }", root)
                changed({ id: "unknown", kind: "text", text: "a reaction changed" })
                check("only for a deletion or edit", window.notifications[chat].own, undefined)
                changed({ id: "unknown", kind: "deleted" })
                var n = window.notifications[chat]
                check("one of ours now", n.own, true)
                check("the same title", n.summary, "A Contact")
                check("the words are gone", n.body, "threesea-la-new-message")
                check("no sound", n.sound, "")
            }, 0],
        ],
        // A failure of a running core with the lock up: no PIN page for a dead core.
        failsWhileLocked: [
            [function() { ready(true) }, 100],
            [function() { window.lockNow() }, 50],
            [function() {
                backend.errorText = "store"
                backend.state = "error"
            }, 800],
            [function() {
                checkStack("")
                check("no lock", window.locked, false)
            }, 0],
        ],
    })

    readonly property var runner: Timer {
        id: runner
        running: true
        interval: 300
        onTriggered: {
            // For the test script: which scenarios there are.
            if (scenario === "--list") {
                console.warn("SCENARIOS", Object.keys(root.scenarios).join(" ")) // log-ok: the names of this file's scenarios
                harness.finish(0)
                return
            }
            var steps = root.scenarios[scenario]
            if (!steps) {
                console.warn("no such scenario:", scenario)
                harness.finish(2)
                return
            }
            if (root.step >= steps.length) {
                harness.finish(root.failures > 0 ? 1 : 0)
                return
            }
            var current = steps[root.step]
            root.step = root.step + 1
            current[0]()
            runner.interval = Math.max(current[1], 1)
            runner.restart()
        }
    }
}
