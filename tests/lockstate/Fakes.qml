// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6

// What the app's QML sees of the C++ side, as far as the window needs it. backend.call replies
// after a moment; `hold(cmd)` keeps a command's replies back until `release(cmd)`, for the
// moments in between; `refused[cmd] = kind` makes it answer with that error.
QtObject {
    property QtObject backend: QtObject {
        property string state: "starting"
        property string errorText
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
                    else now[i].callback(true, backend.results[now[i].cmd] || {})
                }
            }
        }
        function call(cmd, args, callback) {
            sent.push(cmd)
            lastArgs[cmd] = args
            waiting.push({ cmd: cmd, callback: callback })
        }
        function hold(cmd) { held[cmd] = true }
        function release(cmd) { held[cmd] = false }
        function refreshContacts() { }
        function refreshUnread() { }
        function chatTone() { return "" }
    }
    property QtObject dbus: QtObject {
        signal openChatRequested(string identity)
        signal activateRequested()
        signal showUpdateRequested()
        signal openUrlRequested(string url)
        signal markReadRequested(string identity)
        signal replyRequested(string identity, string text)
        function actionToken() { return "" }
        function retainTokens(identities) { }
        function releaseToken(identity) { }
        function forgetToken(identity) { }
        function forgetAllTokens() { }
        function activate() { }
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
        property bool muted
        property bool camera
        property bool frontCamera
        property double activeSince
        property var participants: []
        function call(conversation) { return ({}) }
        function leave() { }
    }
    property QtObject voicePlayer: QtObject {
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
        function pause() { playing = false }
        function resume() { playing = true }
    }
    property QtObject videoPlayer: QtObject {
        property int messageId: -1
        property bool playing
        property int position
        property int duration
        function stop() { }
    }
    property QtObject softSound: QtObject {
        function play() { }
    }
}
