// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components"
import "../components/Format.js" as Fmt

// A 1:1 call (ADR 0012), laid out like Jolla's phone app: the picture and name in the upper
// half, the state or the running time under them, the buttons at the bottom; in video calls the
// videos on top, camera actions in the pull-down menu. Follows `calls` and closes itself once
// the call is over.
Page {
    id: page
    objectName: "call"

    property var contact: ({})
    // Kept: `calls.identity` is cleared once the call is over, the page may still show.
    property string identity: calls.identity
    // Our own picture hidden (the camera keeps sending).
    property bool hideSelf: false
    // Seconds since the call connected, for the running time.
    property int seconds: 0

    readonly property string state: calls.state

    backNavigation: state === "ended"
    allowedOrientations: Orientation.Portrait

    function load() {
        if (page.identity.length === 0) return
        backend.call("contact", { identity: page.identity }, function(ok, result) {
            if (ok) page.contact = result
        })
    }

    function stateText() {
        switch (state) {
        case "starting":
        case "calling":
            //% "Calling…"
            return qsTrId("threesea-la-call-calling")
        case "ringing":
            //% "Ringing…"
            return qsTrId("threesea-la-call-ringing")
        case "incoming":
            return calls.video
                    //% "Incoming video call"
                    ? qsTrId("threesea-la-call-incoming-video")
                    //% "Incoming call"
                    : qsTrId("threesea-la-call-incoming")
        case "connecting":
            //% "Connecting…"
            return qsTrId("threesea-la-call-connecting")
        case "active":
            return Math.floor(seconds / 60) + ":" + ("0" + (seconds % 60)).slice(-2)
        case "ended":
            return calls.result === "finished" || calls.result === "aborted"
                    //% "Call ended"
                    ? qsTrId("threesea-la-call-ended")
                    : Fmt.callStatus(calls.result, calls.outgoing, 0)
        }
        return ""
    }

    Component.onCompleted: {
        identity = identity // break the binding: keep who it was
        load()
    }

    Connections {
        target: calls
        onStateChanged: {
            if (calls.state === "idle" && page.status === PageStatus.Active) pageStack.pop()
        }
    }

    Timer {
        running: page.state === "active"
        interval: 1000
        repeat: true
        triggeredOnStart: true
        onTriggered: page.seconds = Math.max(0, Math.floor((Date.now() - calls.activeSince) / 1000))
    }

    // The other side's video while its camera is on, and our camera's preview: like a chat's
    // bubbles (radius, square outer bottom corner), on the ambience; the controls under them.
    readonly property bool remoteVideo: calls.video && calls.remoteCamera && remote.hasFrame
    readonly property bool inCall: state !== "incoming" && state !== "ended"

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: height

        // Occasional camera actions, out of the way of the video.
        PullDownMenu {
            visible: calls.video && calls.camera && page.inCall
            MenuItem {
                text: page.hideSelf
                      //% "Show own picture"
                      ? qsTrId("threesea-me-show-self")
                      //% "Hide own picture"
                      : qsTrId("threesea-me-hide-self")
                onClicked: page.hideSelf = !page.hideSelf
            }
            MenuItem {
                //% "Switch camera"
                text: qsTrId("threesea-me-switch-camera")
                onClicked: calls.frontCamera = !calls.frontCamera
            }
        }

        CutoutInset { id: cutout }

        CallVideo {
            id: remote
            x: Theme.horizontalPageMargin
            y: cutout.inset + Theme.horizontalPageMargin
            width: parent.width - 2 * x
            // Down to the name and time, which sit right above the buttons.
            height: Math.max(0, header.y - Theme.paddingLarge - y)
            manager: calls
            remote: true
            visible: page.remoteVideo
            radius: Theme.paddingLarge
            roundBottomLeft: false
        }

        // Our camera: top right, over the other side's video when it shows.
        CallVideo {
            id: preview
            width: Math.round(page.width / 4)
            height: Math.round(width * 4 / 3)
            x: page.remoteVideo ? remote.x + remote.width - width - Theme.paddingLarge
                                : page.width - width - Theme.horizontalPageMargin
            y: page.remoteVideo ? remote.y + Theme.paddingLarge
                                : cutout.inset + Theme.horizontalPageMargin
            manager: calls
            remote: false
            mirror: calls.frontCamera
            visible: calls.camera && hasFrame && !page.hideSelf
            radius: Theme.paddingMedium
            roundBottomRight: false
        }

        Column {
            id: header
            y: page.remoteVideo ? controls.y - Theme.paddingSmall - height : Theme.itemSizeHuge
            width: parent.width
            spacing: page.remoteVideo ? 0 : Theme.paddingLarge

            Thumbnail {
                visible: !page.remoteVideo
                anchors.horizontalCenter: parent.horizontalCenter
                height: Math.round(Math.min(Screen.width, Screen.height) / 2.5)
                source: page.contact.pictureVersion > 0
                        ? "image://threesea/contact/" + page.identity + "?" + page.contact.pictureVersion : ""
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                font.pixelSize: page.remoteVideo ? Theme.fontSizeLarge : Theme.fontSizeExtraLarge
                color: Theme.highlightColor
                text: page.contact.displayName || page.identity
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: Theme.paddingSmall

                Label {
                    font.pixelSize: page.remoteVideo ? Theme.fontSizeMedium : Theme.fontSizeLarge
                    color: Theme.secondaryHighlightColor
                    text: page.stateText()
                }
                // The other side muted its microphone.
                Image {
                    visible: calls.remoteMuted && page.state === "active"
                    anchors.verticalCenter: parent.verticalCenter
                    source: "image://theme/icon-m-mic-mute?" + Theme.secondaryHighlightColor
                }
            }
        }

        // Incoming: answer and decline side by side.
        Row {
            visible: page.state === "incoming"
            anchors {
                horizontalCenter: parent.horizontalCenter
                bottom: parent.bottom
                bottomMargin: Theme.itemSizeHuge
            }
            spacing: Theme.itemSizeHuge

            IconButton {
                icon.source: "image://theme/icon-l-reject"
                onClicked: calls.reject()
            }
            IconButton {
                icon.source: "image://theme/icon-l-answer"
                onClicked: calls.accept()
            }
        }

        // During the call: mute, camera (video calls, in the middle), hang up (one tap each, like
        // Jolla's phone app).
        Row {
            id: controls
            visible: page.inCall
            anchors {
                horizontalCenter: parent.horizontalCenter
                bottom: parent.bottom
                bottomMargin: page.remoteVideo ? Theme.paddingLarge : Theme.itemSizeHuge
            }
            spacing: calls.video ? Theme.itemSizeMedium : Theme.itemSizeLarge

            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                icon.source: "image://theme/icon-m-mic-mute"
                highlighted: down || calls.muted
                onClicked: calls.muted = !calls.muted
            }
            IconButton {
                visible: calls.video
                anchors.verticalCenter: parent.verticalCenter
                icon.source: "image://theme/icon-m-video"
                highlighted: down || calls.camera
                onClicked: calls.camera = !calls.camera
            }
            IconButton {
                icon.source: "image://theme/icon-l-reject"
                onClicked: calls.hangUp()
            }
        }
    }
}
