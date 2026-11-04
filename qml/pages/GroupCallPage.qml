// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// A group call (ADR 0012): the group's name and the running time on top, everyone in the call
// as square tiles (us first; their picture or camera, name and a crossed-out microphone when it
// is off), mute, camera and hang up at the bottom like the 1:1 call page. Back leaves the call running; the
// group chat's banner returns to it.
Page {
    id: page
    objectName: "groupCall"

    property string conversation
    property var group: ({})
    property var profile: ({})
    // Seconds the call runs (since the SFU started it, else since we joined).
    property int seconds: 0
    property real joinedAt: 0

    readonly property bool connected: groupCalls.state === "connected"
    readonly property int tileSize: Math.floor((width - 2 * Theme.horizontalPageMargin - Theme.paddingMedium) / 2)

    allowedOrientations: Orientation.Portrait

    function duration() {
        var h = Math.floor(seconds / 3600)
        var m = Math.floor(seconds / 60) % 60
        var s = ("0" + (seconds % 60)).slice(-2)
        return h > 0 ? h + ":" + ("0" + m).slice(-2) + ":" + s : m + ":" + s
    }

    Component.onCompleted: {
        backend.call("group", { key: conversation }, function(ok, result) {
            if (ok) page.group = result
        })
        backend.call("profile", {}, function(ok, result) {
            if (ok) page.profile = result
        })
        if (connected) joinedAt = Date.now()
    }

    // The call ended while another page was on top (a 1:1 call takes the microphone over).
    onStatusChanged: {
        if (status === PageStatus.Active && groupCalls.state === "idle") pageStack.pop()
    }

    Connections {
        target: groupCalls
        onStateChanged: {
            if (groupCalls.state === "connected") page.joinedAt = Date.now()
            if (groupCalls.state === "idle" && page.status === PageStatus.Active) {
                if (groupCalls.error.length > 0) Notices.show(groupCalls.error, Notice.Long, Notice.Center)
                pageStack.pop()
            }
        }
    }

    Timer {
        running: page.connected && Qt.application.active
        interval: 1000
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            var call = groupCalls.call(page.conversation)
            var since = call.startedAt > 0 ? Math.min(call.startedAt, page.joinedAt) : page.joinedAt
            page.seconds = Math.max(0, Math.floor((Date.now() - since) / 1000))
        }
    }

    SilicaFlickable {
        anchors {
            top: parent.top
            left: parent.left
            right: parent.right
            bottom: controls.top
            bottomMargin: Theme.paddingLarge
        }
        clip: true
        contentHeight: column.height + Theme.paddingLarge

        // Occasional camera actions, like the 1:1 call page.
        PullDownMenu {
            visible: groupCalls.camera
            MenuItem {
                //% "Switch camera"
                text: qsTrId("threesea-me-switch-camera")
                onClicked: groupCalls.frontCamera = !groupCalls.frontCamera
            }
        }

        Column {
            id: column
            width: parent.width

            PageHeader {
                title: page.group.title || ""
                description: !page.connected
                             //% "Joining…"
                             ? qsTrId("threesea-la-group-call-joining")
                             : groupCalls.participants.length === 0
                               //% "Waiting for others"
                               ? qsTrId("threesea-la-group-call-waiting")
                               : page.duration()
            }

            Grid {
                anchors.horizontalCenter: parent.horizontalCenter
                columns: 2
                spacing: Theme.paddingMedium

                // Us.
                GroupCallTile {
                    size: page.tileSize
                    source: page.profile.pictureVersion > 0
                            ? "image://threesea/contact/me?" + page.profile.pictureVersion : ""
                    //% "Me"
                    name: page.profile.nickname ? "~" + page.profile.nickname : qsTrId("threesea-la-me")
                    microphone: !groupCalls.muted
                    own: true
                    camera: groupCalls.camera
                    mirror: groupCalls.frontCamera
                }

                Repeater {
                    model: groupCalls.participants
                    GroupCallTile {
                        size: page.tileSize
                        source: modelData.authenticated && modelData.pictureVersion > 0
                                ? "image://threesea/contact/" + modelData.identity + "?" + modelData.pictureVersion : ""
                        name: modelData.authenticated
                              ? modelData.name
                              //% "Connecting…"
                              : qsTrId("threesea-la-call-connecting")
                        microphone: modelData.microphone === true
                        connecting: !modelData.authenticated
                        camera: modelData.camera === true
                        participant: modelData.id
                    }
                }
            }
        }

        VerticalScrollDecorator { }
    }

    // Mute, camera (in the middle) and hang up, like the 1:1 call page.
    Row {
        id: controls
        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge * 2
        }
        spacing: Theme.itemSizeMedium

        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            icon.source: "image://theme/icon-m-mic-mute"
            highlighted: down || groupCalls.muted
            onClicked: groupCalls.muted = !groupCalls.muted
        }
        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            enabled: page.connected
            icon.source: "image://theme/icon-m-video"
            highlighted: down || groupCalls.camera
            onClicked: groupCalls.camera = !groupCalls.camera
        }
        IconButton {
            icon.source: "image://theme/icon-l-reject"
            onClicked: groupCalls.leave()
        }
    }
}
