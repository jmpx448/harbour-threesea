// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtMultimedia 5.6
import Sailfish.Silica 1.0
import "../components"

// Take a photo or record a video for a chat, like Jolla Camera: viewfinder, shutter at the
// bottom, photo/video and front/back camera beside it. The capture goes to the app's cache and
// is handed to `captured(path, video)`; the chat removes it once it is stored encrypted. The
// camera runs only while the page can be seen; a video being recorded then goes with its file.
Page {
    id: page
    allowedOrientations: Orientation.Portrait
    // Swiping away would throw the video away: the shutter stops it first.
    backNavigation: !recording

    // For a profile or group picture: no switching to video.
    property bool photoOnly
    property bool videoMode
    // A video the shutter stopped, on its way to the chat.
    property string pendingPath
    // The file being written, as the recorder reports it. Empty once it is handed on: from then on
    // it is the chat's, whatever the recorder still remembers or reports about it.
    property string capturePath
    // Said when the page is back.
    property bool videoDiscarded
    property bool handedOver
    readonly property bool recording: camera.videoRecorder.recorderState === CameraRecorder.RecordingState
    readonly property bool shown: Qt.application.state === Qt.ApplicationActive && status === PageStatus.Active

    signal captured(string path, bool video)

    function finish(path) {
        capturePath = ""
        page.captured(path, videoMode)
        // Not whatever came on top meanwhile (a call, the lock): then once that has gone.
        if (pageStack.currentPage === page) pageStack.pop()
        else handedOver = true
    }
    function removeCapture() {
        backend.removeCapture(capturePath)
        backend.removeCapture(pendingPath)
        capturePath = ""
        pendingPath = ""
    }
    function recorderWrites(location) {
        if (location.length === 0) return
        if (capturePath.length > 0) capturePath = location
        if (pendingPath.length > 0) pendingPath = location
    }

    onStatusChanged: if (status === PageStatus.Active && handedOver) pageStack.pop()
    onShownChanged: {
        if (shown) {
            if (videoDiscarded) {
                videoDiscarded = false
                //% "Recording stopped and discarded"
                Notices.show(qsTrId("threesea-la-recording-discarded"), Notice.Long, Notice.Center)
            }
        } else if (recording && pendingPath.length === 0) {
            camera.videoRecorder.stop()
        }
    }
    Component.onDestruction: {
        window.cameraRecording = false
        if (recording) camera.videoRecorder.stop()
        removeCapture()
    }

    onRecordingChanged: window.cameraRecording = recording
    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    Camera {
        id: camera
        cameraState: page.shown ? Camera.ActiveState : Camera.UnloadedState
        captureMode: page.videoMode ? Camera.CaptureVideo : Camera.CaptureStillImage
        focus.focusMode: Camera.FocusContinuous
        // The page is portrait only, so the sensor's mounting says how the picture lies.
        metaData.orientation: camera.orientation
        imageCapture.onImageSaved: page.finish(path)
        imageCapture.onCaptureFailed: {
            page.removeCapture()
            // Qt's words are in English and can name files: neither shown nor logged.
            console.warn("taking a photo failed")
            //% "The photo could not be taken."
            Notices.show(qsTrId("threesea-la-camera-photo-failed"), Notice.Long, Notice.Center)
        }
        videoRecorder {
            mediaContainer: "mp4"
            videoCodec: "video/x-h264"
            audioCodec: "audio/mpeg, mpegversion=(int)4"
            onActualLocationChanged: page.recorderWrites(location)
            onRecorderStateChanged: {
                if (camera.videoRecorder.recorderState !== CameraRecorder.StoppedState) return
                if (page.pendingPath.length > 0) {
                    var path = page.pendingPath
                    page.pendingPath = ""
                    page.finish(path)
                } else if (page.capturePath.length > 0) {
                    // Stopped without the shutter: the page was left or the camera was taken.
                    page.removeCapture()
                    if (page.shown) {
                        //% "Recording stopped and discarded"
                        Notices.show(qsTrId("threesea-la-recording-discarded"), Notice.Long, Notice.Center)
                    } else {
                        page.videoDiscarded = true
                    }
                }
            }
            onError: {
                page.removeCapture()
                console.warn("recording a video failed, code", camera.videoRecorder.errorCode)
                //% "The video could not be recorded."
                Notices.show(qsTrId("threesea-la-camera-video-failed"), Notice.Long, Notice.Center)
            }
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
            visible: !page.photoOnly
            enabled: !page.recording
            icon.source: page.videoMode ? "image://theme/icon-camera-camera-mode" : "image://theme/icon-camera-video"
            onClicked: page.videoMode = !page.videoMode
        }

        IconButton {
            // The microphone is a running call's.
            enabled: viewfinder.visible && (page.videoMode ? page.recording || !window.anyCallRunning
                                                           : camera.imageCapture.ready)
            icon.source: !page.videoMode ? "image://theme/icon-camera-shutter-release"
                       : page.recording ? "image://theme/icon-camera-video-shutter-off"
                                        : "image://theme/icon-camera-video-shutter-on"
            onClicked: {
                if (!page.videoMode) {
                    page.capturePath = backend.captureFile("jpg")
                    camera.imageCapture.captureToLocation(page.capturePath)
                } else if (page.recording) {
                    page.pendingPath = page.capturePath
                    camera.videoRecorder.stop()
                } else {
                    page.capturePath = backend.captureFile("mp4")
                    camera.videoRecorder.outputLocation = page.capturePath
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
