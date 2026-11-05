// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"

// Settings → Calls (Full build, ADR 0012); Android's "Calls" settings. Both are app settings
// (Threema Safe, linked devices).
Page {
    id: page
    allowedOrientations: Orientation.All

    function setSetting(name, value) {
        backend.call("setSetting", { name: name, value: value }, function(ok, result) {
            if (ok) window.appSettings = result
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Calls"
                title: qsTrId("threesea-se-calls")
            }

            TextSwitch {
                automaticCheck: false
                checked: window.appSettings.threemaCalls !== false
                //% "Threema calls"
                text: qsTrId("threesea-la-threema-calls")
                //% "Contacts can call you, and you can call them from the chat's pull-down menu."
                description: qsTrId("threesea-la-threema-calls-description")
                onClicked: page.setSetting("threemaCalls", !checked)
            }

            TextSwitch {
                enabled: window.appSettings.threemaCalls !== false
                automaticCheck: false
                checked: window.appSettings.relayThreemaCalls === true
                //% "Always relay calls"
                text: qsTrId("threesea-la-relay-calls")
                //% "Calls go through a Threema server, which keeps your IP address hidden from the other side. Call quality may be lower."
                description: qsTrId("threesea-la-relay-calls-description")
                onClicked: page.setSetting("relayThreemaCalls", !checked)
            }

            TextSwitch {
                // Android's group call policy (linked devices know it too). The feature mask
                // keeps the bit, as on Android: off only means this device stays out. Its own
                // setting, also as on Android: Threema calls are the 1:1 calls.
                visible: groupCalls.supported
                automaticCheck: false
                checked: window.appSettings.groupCalls !== false
                //% "Group calls"
                text: qsTrId("threesea-la-group-calls")
                //% "You can start group calls and join the ones running in your groups."
                description: qsTrId("threesea-la-group-calls-description")
                onClicked: page.setSetting("groupCalls", !checked)
            }

            SectionHeader {
                //% "Video"
                text: qsTrId("threesea-se-video")
            }

            TextSwitch {
                enabled: window.appSettings.threemaCalls !== false
                automaticCheck: false
                checked: window.appSettings.videoCalls !== false
                //% "Video in one-to-one calls"
                text: qsTrId("threesea-la-video-calls")
                //% "One-to-one calls can show the camera of both sides; each side turns its camera on when it wants."
                description: qsTrId("threesea-la-video-calls-description")
                onClicked: page.setSetting("videoCalls", !checked)
            }

            ChoiceBox {
                enabled: window.appSettings.threemaCalls !== false && window.appSettings.videoCalls !== false
                //% "Video quality"
                label: qsTrId("threesea-la-video-quality")
                //% "Standard"
                options: [qsTrId("threesea-la-video-quality-auto"),
                          //% "Low data use"
                          qsTrId("threesea-la-video-quality-low"),
                          //% "Highest quality"
                          qsTrId("threesea-la-video-quality-max")]
                choice: Math.max(0, ["auto", "low", "max"].indexOf(window.appSettings.videoQuality))
                //% "Both sides use the lower of their two settings. Calls that go only through a relay (Always relay calls, or with an unverified contact) use Standard at most."
                description: qsTrId("threesea-la-video-quality-description")
                onChosen: backend.call("setVideoQuality", { quality: ["auto", "low", "max"][index] },
                                       function(ok, result) { if (ok) window.appSettings = result })
            }
        }

        VerticalScrollDecorator { }
    }
}
