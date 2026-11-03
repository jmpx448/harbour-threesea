// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"

// Settings → Chats.
Page {
    id: page
    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Chats"
                title: qsTrId("threesea-he-chats")
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.enterToSend
                //% "Enter sends"
                text: qsTrId("threesea-la-enter-to-send")
                //% "Enter sends the message, Shift+Enter starts a new line. Off: Enter starts a new line, Shift+Enter sends."
                description: qsTrId("threesea-la-enter-to-send-description")
                onClicked: backend.enterToSend = !checked
            }

            TextSwitch {
                // Android's "proximity sensor"; the sensor is Full-flavour only, so the switch
                // only shows where it can do something.
                visible: voicePlayer.earpieceAvailable
                automaticCheck: false
                checked: backend.earToPlay
                //% "Play voice messages at the ear"
                text: qsTrId("threesea-la-ear-to-play")
                //% "Hold the phone to your ear and the voice message plays on the earpiece, like a call. Take it away to pause."
                description: qsTrId("threesea-la-ear-to-play-description")
                onClicked: backend.earToPlay = !checked
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.autoPlayGifs
                //% "Play GIFs automatically"
                text: qsTrId("threesea-la-autoplay-gifs")
                //% "Animated images play in the chat. Off: they play when you open them."
                description: qsTrId("threesea-la-autoplay-gifs-description")
                onClicked: backend.autoPlayGifs = !checked
            }
        }

        VerticalScrollDecorator { }
    }
}
