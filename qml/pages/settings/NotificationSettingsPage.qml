// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"

// Settings → Notifications.
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
                //% "Notifications"
                title: qsTrId("threesea-se-notifications")
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.notificationsEnabled
                // Calls are Full only.
                text: backend.full
                      //% "Notify about new messages and missed calls"
                      ? qsTrId("threesea-la-notifications-calls")
                      //% "Notify about new messages"
                      : qsTrId("threesea-la-notifications")
                onClicked: backend.notificationsEnabled = !checked
            }

            TextSwitch {
                automaticCheck: false
                enabled: backend.notificationsEnabled
                checked: backend.notificationPreview
                //% "Show message content"
                text: qsTrId("threesea-la-notification-preview")
                //% "Otherwise notifications show who wrote, but not what, e.g. on the lock screen."
                description: qsTrId("threesea-la-notification-preview-who-description")
                onClicked: backend.notificationPreview = !checked
            }

            // Android's sound and vibration, for chats and for groups.
            SectionHeader {
                //% "Chats"
                text: qsTrId("threesea-se-notifications-chats")
            }

            TextSwitch {
                automaticCheck: false
                enabled: backend.notificationsEnabled
                checked: backend.soundChats
                //% "Sound"
                text: qsTrId("threesea-la-notification-sound")
                //% "Played at the volume of the current sound profile."
                description: qsTrId("threesea-la-notification-sound-description")
                onClicked: backend.soundChats = !checked
            }

            ToneButton {
                enabled: backend.notificationsEnabled && backend.soundChats
                tone: backend.toneChats
                //% "Instant message tone"
                defaultText: qsTrId("threesea-la-tone-profile")
                onChosen: backend.toneChats = tone
            }

            TextSwitch {
                automaticCheck: false
                enabled: backend.notificationsEnabled
                checked: backend.vibrateChats
                //% "Vibrate"
                text: qsTrId("threesea-la-notification-vibrate")
                onClicked: backend.vibrateChats = !checked
            }

            SectionHeader {
                //% "Groups"
                text: qsTrId("threesea-se-notifications-groups")
            }

            TextSwitch {
                automaticCheck: false
                enabled: backend.notificationsEnabled
                checked: backend.soundGroups
                text: qsTrId("threesea-la-notification-sound")
                description: qsTrId("threesea-la-notification-sound-description")
                onClicked: backend.soundGroups = !checked
            }

            ToneButton {
                enabled: backend.notificationsEnabled && backend.soundGroups
                tone: backend.toneGroups
                defaultText: qsTrId("threesea-la-tone-profile")
                onChosen: backend.toneGroups = tone
            }

            TextSwitch {
                automaticCheck: false
                enabled: backend.notificationsEnabled
                checked: backend.vibrateGroups
                text: qsTrId("threesea-la-notification-vibrate")
                onClicked: backend.vibrateGroups = !checked
            }

            // The tone and vibration of an incoming call; they ring with notifications off too.
            SectionHeader {
                visible: calls.supported
                //% "Calls"
                text: qsTrId("threesea-se-notifications-calls")
            }

            ToneButton {
                visible: calls.supported
                tone: backend.toneCalls
                //% "Ringtone"
                label: qsTrId("threesea-la-call-tone")
                //% "System call tone"
                defaultText: qsTrId("threesea-la-call-tone-system")
                onChosen: backend.toneCalls = tone
            }

            TextSwitch {
                visible: calls.supported
                automaticCheck: false
                checked: backend.vibrateCalls
                text: qsTrId("threesea-la-notification-vibrate")
                onClicked: backend.vibrateCalls = !checked
            }

            // Android's in-app sounds and vibration: a message arriving in the chat the user
            // is reading needs no notification, only a hint.
            SectionHeader {
                //% "In the app"
                text: qsTrId("threesea-se-notifications-inapp")
            }

            // Between voice messages it plays with notifications off as well.
            TextSwitch {
                automaticCheck: false
                checked: backend.inAppSounds
                text: qsTrId("threesea-la-notification-sound")
                description: backend.notificationsEnabled
                             //% "A soft sound for a message arriving in the chat you are reading, and between two voice messages playing one after the other."
                             ? qsTrId("threesea-la-inapp-sound-description")
                             //% "A soft sound between two voice messages playing one after the other."
                             : qsTrId("threesea-la-inapp-sound-voice-description")
                onClicked: backend.inAppSounds = !checked
            }

            TextSwitch {
                automaticCheck: false
                enabled: backend.notificationsEnabled
                checked: backend.inAppVibrate
                text: qsTrId("threesea-la-notification-vibrate")
                //% "A short vibration for a message arriving in the chat you are reading."
                description: qsTrId("threesea-la-inapp-vibrate-description")
                onClicked: backend.inAppVibrate = !checked
            }
        }

        VerticalScrollDecorator { }
    }
}
