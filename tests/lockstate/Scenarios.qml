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

    // The usual start: ready, with a lock (or without) as the core reports it.
    function ready(appLock, graceSecs) {
        harness.setActive(true)
        backend.results["lockStatus"] = { lock: { appLock: appLock, graceSecs: graceSecs || 0, keyboard: false } }
        backend.unlockedAtStart = true
        backend.state = "ready"
    }

    readonly property var scenarios: ({
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
