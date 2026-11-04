// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

CoverBackground {
    id: cover

    // A call: who and its state or running time, with answer/decline or hang up (Jolla's phone
    // app's cover). A 1:1 call that rings or runs comes first: it takes the microphone from a
    // group call when answered.
    readonly property bool inGroupCall: calls.state === "idle" && groupCalls.state !== "idle"
                                        && groupCalls.state !== "ended"
    readonly property bool inCall: calls.state !== "idle" || inGroupCall
    property int seconds: 0

    function callStart() {
        if (!inGroupCall) return calls.activeSince
        // Since the SFU started it, else since we joined, like the group call page.
        var startedAt = groupCalls.call(groupCalls.conversation).startedAt
        return startedAt > 0 ? Math.min(startedAt, groupCalls.activeSince) : groupCalls.activeSince
    }

    Timer {
        running: (calls.state === "active" || (cover.inGroupCall && groupCalls.state === "connected"))
                 && cover.status === Cover.Active
        interval: 1000
        repeat: true
        triggeredOnStart: true
        onTriggered: cover.seconds = Math.max(0, Math.floor((Date.now() - cover.callStart()) / 1000))
    }

    Column {
        visible: cover.inCall
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.paddingLarge
        spacing: Theme.paddingSmall

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            maximumLineCount: 2
            // "Hide content on the cover" (Privacy): only the call's state.
            visible: window.appSettings.disableScreenshots !== true
                     && !(cover.inGroupCall && window.locked)
            text: cover.inGroupCall ? window.groupCallName : window.callName
            color: Theme.highlightColor
        }
        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.secondaryColor
            text: cover.inGroupCall && groupCalls.state !== "connected"
                  ? qsTrId("threesea-la-group-call-joining")
                  : cover.inGroupCall && groupCalls.participants.length === 0
                  ? qsTrId("threesea-la-group-call-waiting")
                  : calls.state === "active" || cover.inGroupCall
                  ? Math.floor(cover.seconds / 60) + ":" + ("0" + (cover.seconds % 60)).slice(-2)
                  : calls.state === "incoming"
                    ? qsTrId("threesea-la-call-incoming")
                    : calls.state === "ringing"
                      ? qsTrId("threesea-la-call-ringing")
                      : calls.state === "connecting"
                        ? qsTrId("threesea-la-call-connecting")
                        : calls.state === "ended"
                          ? qsTrId("threesea-la-call-ended")
                          : qsTrId("threesea-la-call-calling")
        }
    }

    CoverActionList {
        enabled: calls.state === "incoming"
        CoverAction {
            iconSource: "image://theme/icon-cover-reject"
            onTriggered: calls.reject()
        }
        CoverAction {
            iconSource: "image://theme/icon-cover-answer"
            onTriggered: {
                calls.accept()
                window.activate()
            }
        }
    }

    CoverActionList {
        enabled: calls.state !== "idle" && calls.state !== "incoming" && calls.state !== "ended"
        CoverAction {
            iconSource: calls.muted ? "image://theme/icon-cover-unmute" : "image://theme/icon-cover-mute"
            onTriggered: calls.muted = !calls.muted
        }
        CoverAction {
            iconSource: "image://theme/icon-cover-hangup"
            onTriggered: calls.hangUp()
        }
    }

    CoverActionList {
        enabled: cover.inGroupCall
        CoverAction {
            iconSource: groupCalls.muted ? "image://theme/icon-cover-unmute" : "image://theme/icon-cover-mute"
            onTriggered: groupCalls.muted = !groupCalls.muted
        }
        CoverAction {
            iconSource: "image://theme/icon-cover-hangup"
            onTriggered: groupCalls.leave()
        }
    }

    Column {
        visible: !cover.inCall
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.paddingLarge
        spacing: Theme.paddingSmall

        Label {
            visible: backend.unreadCount > 0
            anchors.horizontalCenter: parent.horizontalCenter
            text: backend.unreadCount
            font.pixelSize: Theme.fontSizeHuge
            color: Theme.primaryColor
        }

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            color: Theme.secondaryColor
            text: backend.unreadCount === 1
                  //% "unread message"
                  ? qsTrId("threesea-la-cover-unread-one")
                  : backend.unreadCount > 1
                    //% "unread messages"
                    ? qsTrId("threesea-la-cover-unread-many")
                    //% "No new messages"
                    : qsTrId("threesea-la-cover-no-new")
        }

        Label {
            // Who is waiting: the newest chat that still has something unread. "Hide content on
            // the cover" (Privacy) and the app lock show only the count.
            visible: backend.unreadCount > 0 && backend.unreadFrom.length > 0 && !window.locked
                     && window.appSettings.disableScreenshots !== true
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            truncationMode: TruncationMode.Fade
            text: backend.unreadFrom
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.highlightColor
        }
    }

    CoverActionList {
        enabled: backend.state === "ready" && !cover.inCall && !window.locked
        CoverAction {
            iconSource: "image://theme/icon-cover-new"
            // New message: the contacts, attached to the chat list (swipe left there).
            onTriggered: {
                window.activate()
                var chats = pageStack.find(function(page) { return page.objectName === "conversations" })
                if (!chats) return
                pageStack.pop(chats, PageStackAction.Immediate)
                if (pageStack.nextPage(chats) === null) {
                    pageStack.pushAttached(Qt.resolvedUrl("../pages/ContactsPage.qml"))
                }
                pageStack.navigateForward(PageStackAction.Immediate)
            }
        }
    }
}
