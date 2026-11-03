// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtMultimedia 5.6
import Sailfish.Silica 1.0
import "../components"

// Take a photo or record a video for a chat, like Jolla Camera: viewfinder, shutter at the
// bottom, photo/video and front/back camera beside it. The capture goes to the app's cache and
// is handed to `captured(path, video)`; the chat removes it once it is stored encrypted.
Page {
    id: page
    allowedOrientations: Orientation.Portrait

    property bool videoMode
    property string pendingPath
    readonly property bool recording: camera.videoRecorder.recorderState === CameraRecorder.RecordingState

    signal captured(string path, bool video)

    function finish(path) {
        page.captured(path, videoMode)
        pageStack.pop()
    }

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    Camera {
        id: camera
        captureMode: page.videoMode ? Camera.CaptureVideo : Camera.CaptureStillImage
        focus.focusMode: Camera.FocusContinuous
        imageCapture.onImageSaved: page.finish(path)
        imageCapture.onCaptureFailed: Notices.show(message, Notice.Short, Notice.Center)
        videoRecorder {
            mediaContainer: "mp4"
            videoCodec: "video/x-h264"
            audioCodec: "audio/mpeg, mpegversion=(int)4"
            onRecorderStateChanged: {
                if (camera.videoRecorder.recorderState === CameraRecorder.StoppedState
                        && page.pendingPath.length > 0) {
                    var path = page.pendingPath
                    page.pendingPath = ""
                    page.finish(path)
                }
            }
            onError: Notices.show(camera.videoRecorder.errorString, Notice.Short, Notice.Center)
        }
    }

    VideoOutput {
        id: viewfinder
        anchors.fill: parent
        source: camera
        fillMode: VideoOutput.PreserveAspectCrop
        visible: camera.availability === Camera.Available
    }

    Label {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.horizontalPageMargin
        visible: !viewfinder.visible
        wrapMode: Text.Wrap
        horizontalAlignment: Text.AlignHCenter
        //% "The camera is not available."
        text: qsTrId("threesea-la-camera-unavailable")
        color: Theme.secondaryHighlightColor
    }

    CutoutInset { id: cutout }

    // Recording time, like Jolla Camera (below a display cutout, which sits exactly there).
    Label {
        visible: page.recording
        anchors {
            top: parent.top
            topMargin: cutout.inset + Theme.paddingLarge
            horizontalCenter: parent.horizontalCenter
        }
        text: Format.formatDuration(camera.videoRecorder.duration / 1000, Formatter.DurationShort)
        color: "white"
        font.pixelSize: Theme.fontSizeLarge
    }

    Row {
        anchors {
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge * 2
            horizontalCenter: parent.horizontalCenter
        }
        spacing: Theme.paddingLarge * 3

        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            enabled: !page.recording
            icon.source: page.videoMode ? "image://theme/icon-camera-camera-mode" : "image://theme/icon-camera-video"
            onClicked: page.videoMode = !page.videoMode
        }

        IconButton {
            enabled: viewfinder.visible && (page.videoMode || camera.imageCapture.ready)
            icon.source: !page.videoMode ? "image://theme/icon-camera-shutter-release"
                       : page.recording ? "image://theme/icon-camera-video-shutter-off"
                                        : "image://theme/icon-camera-video-shutter-on"
            onClicked: {
                if (!page.videoMode) {
                    camera.imageCapture.captureToLocation(backend.captureFile("jpg"))
                } else if (page.recording) {
                    page.pendingPath = camera.videoRecorder.actualLocation.toString()
                    camera.videoRecorder.stop()
                } else {
                    camera.videoRecorder.outputLocation = backend.captureFile("mp4")
                    camera.videoRecorder.record()
                }
            }
        }

        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            enabled: !page.recording
            icon.source: camera.position === Camera.FrontFace
                         ? "image://theme/icon-camera-backcamera" : "image://theme/icon-camera-front-camera"
            onClicked: camera.position = camera.position === Camera.FrontFace ? Camera.BackFace : Camera.FrontFace
        }
    }
}
