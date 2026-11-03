// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

CoverBackground {
    id: cover

    // A call: who and its state or running time, with answer/decline or hang up (Jolla's phone
    // app's cover).
    readonly property bool inCall: calls.state !== "idle"
    property int seconds: 0

    Timer {
        running: calls.state === "active" && cover.status === Cover.Active
        interval: 1000
        repeat: true
        triggeredOnStart: true
        onTriggered: cover.seconds = Math.max(0, Math.floor((Date.now() - calls.activeSince) / 1000))
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
            text: window.callName
            color: Theme.highlightColor
        }
        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.secondaryColor
            text: calls.state === "active"
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
        enabled: cover.inCall && calls.state !== "incoming" && calls.state !== "ended"
        CoverAction {
            iconSource: calls.muted ? "image://theme/icon-cover-unmute" : "image://theme/icon-cover-mute"
            onTriggered: calls.muted = !calls.muted
        }
        CoverAction {
            iconSource: "image://theme/icon-cover-hangup"
            onTriggered: calls.hangUp()
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
        enabled: backend.state === "ready" && !cover.inCall
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
