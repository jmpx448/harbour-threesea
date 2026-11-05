// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Format.js" as Fmt

// The settings, laid out like Sailfish's Settings app: section headers, below them a row with
// an icon per category, each opening its page in pages/settings/.
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
                //% "Settings"
                title: qsTrId("threesea-he-settings")
            }

            SectionHeader {
                //% "Account"
                text: qsTrId("threesea-se-account")
            }
            SettingsEntry {
                icon: "icon-m-contact"
                //% "My profile"
                text: qsTrId("threesea-se-profile")
                description: backend.identity
                page: "ProfileSettingsPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-backup"
                text: qsTrId("threesea-se-backup")
                page: "BackupSettingsPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-computer"
                //% "Linked devices"
                text: qsTrId("threesea-se-linked-devices")
                page: "LinkedDevicesPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-website"
                text: qsTrId("threesea-he-threema-web")
                page: "WebSessionsPage.qml"
            }

            SectionHeader {
                //% "Privacy and security"
                text: qsTrId("threesea-se-privacy-security")
            }
            SettingsEntry {
                icon: "icon-m-incognito"
                text: qsTrId("threesea-se-privacy")
                page: "PrivacySettingsPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-device-lock"
                text: qsTrId("threesea-se-security")
                description: !(window.lockInfo && window.lockInfo.lock)
                             //% "App lock off"
                             ? qsTrId("threesea-la-app-lock-off")
                             : window.lockInfo.lock.appLock
                               //% "App lock on"
                               ? qsTrId("threesea-la-app-lock-on")
                               : Fmt.lockValue(window.lockInfo.lock)
                page: "SecuritySettingsPage.qml"
            }

            SectionHeader {
                //% "App"
                text: qsTrId("threesea-se-app")
            }
            SettingsEntry {
                icon: "icon-m-chat"
                text: qsTrId("threesea-he-chats")
                page: "ChatSettingsPage.qml"
            }
            SettingsEntry {
                visible: calls.supported
                icon: "icon-m-call"
                text: qsTrId("threesea-se-calls")
                page: "CallsSettingsPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-image"
                text: qsTrId("threesea-se-media-storage")
                page: "MediaSettingsPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-ambience"
                text: qsTrId("threesea-se-appearance")
                page: "AppearanceSettingsPage.qml"
            }
            SettingsEntry {
                // Ours: the theme's icon-m-notifications has no full-opacity element and looks
                // washed out beside the others (scripts/make-ui-icons.py).
                icon: "notifications.png"
                text: qsTrId("threesea-se-notifications")
                page: "NotificationSettingsPage.qml"
            }
            SettingsEntry {
                icon: "icon-m-developer-mode"
                text: qsTrId("threesea-se-advanced")
                page: "AdvancedSettingsPage.qml"
            }

            SectionHeader {
                //% "Info"
                text: qsTrId("threesea-se-info")
            }
            SettingsEntry {
                icon: "icon-m-about"
                //% "About"
                text: qsTrId("threesea-me-about")
                page: "../AboutPage.qml"
            }
        }

        VerticalScrollDecorator { }
    }
}
