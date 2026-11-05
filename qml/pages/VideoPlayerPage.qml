// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtMultimedia 5.6
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// A video message, full screen on black like Jolla Videos: tap to show/hide the controls,
// play/pause in the middle, a slider to seek. Streamed from memory (VideoPlayer).
Page {
    id: page
    allowedOrientations: Orientation.All
    // Full screen on black also beside a landscape cutout, like Gallery's video player; the
    // controls sit in the middle and at the bottom, away from the cutout.
    cutoutMode: CutoutMode.FullScreen
    showNavigationIndicator: controls.visible

    property int messageId
    property bool controlsShown: true

    Component.onCompleted: {
        voicePlayer.stop()
        if (!videoPlayer.play(messageId)) {
            //% "Can't play videos on this device"
            Notices.show(qsTrId("threesea-la-cannot-play-video"), Notice.Long, Notice.Center)
        }
    }
    Component.onDestruction: videoPlayer.stop()

    Connections {
        target: videoPlayer
        // Ended from outside (the window closed, the system's media controls): nothing to show.
        onMessageIdChanged: {
            if (videoPlayer.messageId === -1 && page.status === PageStatus.Active) {
                pageStack.pop(undefined, PageStackAction.Immediate)
            }
        }
        onFailed: Notices.show(qsTrId("threesea-la-cannot-play-video"), Notice.Long, Notice.Center)
    }
    // A video is for watching: pause when the app goes away.
    Connections {
        target: Qt.application
        onStateChanged: {
            if (Qt.application.state !== Qt.ApplicationActive && videoPlayer.playing) videoPlayer.pause()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    VideoOutput {
        anchors.fill: parent
        source: videoPlayer
        fillMode: VideoOutput.PreserveAspectFit
    }

    MouseArea {
        anchors.fill: parent
        onClicked: page.controlsShown = !page.controlsShown
    }

    Item {
        id: controls
        anchors.fill: parent
        visible: opacity > 0
        opacity: page.controlsShown || !videoPlayer.playing ? 1.0 : 0.0
        Behavior on opacity { FadeAnimation { } }

        IconButton {
            anchors.centerIn: parent
            icon.source: "image://theme/" + (videoPlayer.playing ? "icon-l-pause" : "icon-l-play")
            onClicked: videoPlayer.playing ? videoPlayer.pause() : videoPlayer.resume()
        }

        Slider {
            id: positionSlider
            anchors {
                left: parent.left
                right: parent.right
                bottom: parent.bottom
                bottomMargin: Theme.paddingLarge
            }
            minimumValue: 0
            maximumValue: Math.max(1, videoPlayer.duration)
            valueText: Fmt.duration(videoPlayer.position / 1000) + " / " + Fmt.duration(videoPlayer.duration / 1000)
            onReleased: videoPlayer.seek(value)
        }
        // Not a plain `value:` binding: the slider writes its value when let go, which would end it.
        Binding {
            target: positionSlider
            property: "value"
            value: videoPlayer.position
            when: !positionSlider.pressed
        }
    }
}
