// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6

// What the app's QML sees of the C++ side, as far as the window needs it. backend.call replies
// after a moment; `hold(cmd)` keeps a command's replies back until `release(cmd)`, for the
// moments in between; `refused[cmd] = kind` makes it answer with that error. A result may be a
// function of the command's arguments.
QtObject {
    property QtObject backend: QtObject {
        property string state: "starting"
        property bool upgrading: false
        property string errorKind
        property string errorText
        property int startRetries: 0
        function retryStart() { startRetries++ }
        property bool unlockedAtStart: false
        property bool keepRunning: false
        property bool addressBookSync: false
        property double addressBookSyncedAt: 0
        property bool notificationsEnabled: true
        property bool notificationPreview: true
        property bool passphraseKeyboard: true
        signal coreEvent(string name, var data)
        // Commands so far, for the checks.
        property var sent: []
        // Answers by command; anything else gets an empty result.
        property var results: ({})
        property var refused: ({})
        // The arguments of each command's last call.
        property var lastArgs: ({})
        property var contacts: null
        property var held: ({})
        property var waiting: []
        property var replyTimer: Timer {
            interval: 1
            repeat: true
            running: true
            onTriggered: {
                var now = backend.waiting
                backend.waiting = []
                for (var i = 0; i < now.length; ++i) {
                    if (backend.held[now[i].cmd]) backend.waiting.push(now[i])
                    else if (!now[i].callback) continue
                    else if (backend.refused[now[i].cmd]) now[i].callback(false, {}, backend.refused[now[i].cmd], "")
                    else {
                        var result = backend.results[now[i].cmd] || {}
                        now[i].callback(true, typeof result === "function" ? result(now[i].args) : result)
                    }
                }
            }
        }
        function call(cmd, args, callback) {
            sent.push(cmd)
            lastArgs[cmd] = args
            waiting.push({ cmd: cmd, args: args, callback: callback })
        }
        function lockWithFilesOnTheirWay() { call("lock") }
        // What SendInTurn handed over, and an answer for each.
        property var filesSent: []
        function sendFile(identity, path, photo, caption, callback) {
            filesSent.push(identity)
            call("sendFile", { identity: identity, path: path, caption: caption }, callback)
        }
        function sendVideo(identity, path, frame, seconds, caption, callback) {
            sendFile(identity, path, false, caption, callback)
        }
        property bool sendingInTurn: false
        function setSendingInTurn(sending) { sendingInTurn = sending }
        function sharedFileKind(path) { return path.indexOf("/own/") === 0 ? "own" : "picture" }
        function hold(cmd) { held[cmd] = true }
        function release(cmd) { held[cmd] = false }
        function refreshContacts() { }
        function refreshUnread() { }
        function chatTone() { return "" }
        // What the camera pages were told to remove.
        property var removedCaptures: []
        function captureFile(extension) { return "/cache/capture/test." + extension }
        function removeCapture(path) {
            if (path) removedCaptures.push(path)
            return !!path
        }
        function decodeQr(image, callback) { callback("") }
        // What the photo editor asked for last.
        property var edit
        function editPhoto(path, crop, rotation, marks, callback) {
            edit = { path: path, crop: crop, rotation: rotation, marks: marks }
            callback("/cache/capture/edited.jpg")
        }
    }
    property QtObject dbus: QtObject {
        signal openChatRequested(string identity)
        signal activateRequested()
        signal launched()
        // What the window guard last dropped.
        property var refused: ({})
        function refusedCall() { return refused }
        signal showUpdateRequested()
        signal openUrlRequested(string url)
        signal markReadRequested(string identity)
        signal replyRequested(string identity, string text)
        signal replyLost()
        signal blankReply()
        function actionToken() { return "" }
        property int retained: 0
        function retainTokens(identities) { retained = retained + 1 }
        // Those whose grace started.
        property var released: []
        function releaseToken(identity) { released.push(identity) }
        // The tokens ended by us, for the checks.
        property var forgotten: []
        function forgetToken(identity) { forgotten.push(identity) }
        function forgetAllTokens() { }
        function activate() { }
    }
    property QtObject lifecycle: QtObject {
        // How often the window said that this run has done what it was for.
        property int left: 0
        function leaveIfUnwanted() { left = left + 1 }
        function checkInstallation() { }
        signal installationChanged(bool removed, bool windowOpen, bool windowAsked)
        property bool changed: false
        function packageChanged() { return changed }
    }
    property QtObject calls: QtObject {
        property string state: "idle"
        property string identity
        property string result
        property bool outgoing
        property bool video
        property bool camera
        property bool muted
        property bool frontCamera
        property bool remoteCamera
        property bool remoteMuted
        property double activeSince
        function accept() { }
        function reject() { }
        function hangUp() { }
    }
    property QtObject groupCalls: QtObject {
        property bool supported: false
        property string state: "idle"
        property string conversation
        property string error
        property string errorMessage
        property bool muted
        property bool camera
        property bool frontCamera
        property double activeSince
        property var participants: []
        function call(conversation) { return ({}) }
        function leave() { }
    }
    property QtObject voicePlayer: QtObject {
        signal failed(string error)
        property int messageId: -1
        property bool active
        property bool chiming
        property bool playing
        property int position
        property int duration
        property real speed: 1
        property int stops: 0
        function stop() {
            stops = stops + 1
            messageId = -1
            active = false
            playing = false
        }
        property int pauses: 0
        function pause() {
            pauses = pauses + 1
            playing = false
        }
        function resume() { playing = true }
    }
    property QtObject videoPlayer: QtObject {
        property int messageId: -1
        property bool loading
        property bool playing
        property int position
        property int duration
        function stop() { }
        function pause() { playing = false }
    }
    property QtObject voiceRecorder: QtObject {
        property bool available: true
        property bool recording
        property bool held
        property QtObject owner
        property int duration
        property int cancels: 0
        property int left: 0
        function appLeft() { left = left + 1 }
        function cancel() {
            cancels = cancels + 1
            recording = false
            held = false
            owner = null
        }
    }
    property QtObject softSound: QtObject {
        function play() { }
    }
}
