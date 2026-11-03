// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"

// Settings → Privacy: receipts, typing, unknown senders, blocked contacts, address book.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var settings: ({})

    function setSetting(name, value) {
        backend.call("setSetting", { name: name, value: value }, function(ok, result) {
            if (!ok) return
            page.settings = result
            window.appSettings = result
        })
    }

    property int blockedCount: 0

    Component.onCompleted: backend.call("settings", {}, function(ok, result) {
        if (ok) page.settings = result
    })
    // Back from the block list: its number may have changed.
    onStatusChanged: if (status === PageStatus.Active) {
        backend.call("blockedContacts", {}, function(ok, result) {
            if (ok) page.blockedCount = result.blocked.length
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Privacy"
                title: qsTrId("threesea-se-privacy")
            }

            SectionHeader {
                //% "Contacts"
                text: qsTrId("threesea-se-contacts")
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.addressBookSync
                //% "Find contacts from the address book"
                text: qsTrId("threesea-la-address-book-sync")
                //% "Daily, only hashed e-mail addresses and phone numbers are sent to Threema to find who uses it. Found people become contacts with their address book names."
                description: qsTrId("threesea-la-address-book-sync-description")
                onClicked: {
                    backend.addressBookSync = !checked
                    if (backend.addressBookSync) window.syncAddressBook(true)
                }
            }

            SettingsButton {
                enabled: backend.addressBookSync
                //% "Exclusion list"
                text: qsTrId("threesea-he-exclusion-list")
                onClicked: pageStack.push(Qt.resolvedUrl("../BlockedContactsPage.qml"), { exclusion: true })
            }

            SectionHeader {
                //% "Blocking"
                text: qsTrId("threesea-se-blocking")
            }

            // Its own page, not tied to the switch below.
            ValueButton {
                //% "Blocked contacts"
                label: qsTrId("threesea-he-blocked-contacts")
                value: page.blockedCount > 0 ? page.blockedCount : ""
                onClicked: pageStack.push(Qt.resolvedUrl("../BlockedContactsPage.qml"))
            }

            TextSwitch {
                automaticCheck: false
                checked: page.settings.blockUnknown === true
                //% "Block unknown senders"
                text: qsTrId("threesea-la-block-unknown")
                //% "Ignore messages from people who are not in your contacts."
                description: qsTrId("threesea-la-block-unknown-description")
                onClicked: page.setSetting("blockUnknown", !checked)
            }

            SectionHeader {
                //% "Receipts"
                text: qsTrId("threesea-se-receipts")
            }

            TextSwitch {
                automaticCheck: false
                checked: page.settings.readReceipts !== false
                //% "Send read receipts"
                text: qsTrId("threesea-la-send-read-receipts")
                //% "Let people know when you have read their messages."
                description: qsTrId("threesea-la-send-read-receipts-description")
                onClicked: page.setSetting("readReceipts", !checked)
            }

            TextSwitch {
                automaticCheck: false
                checked: page.settings.sendTyping !== false
                //% "Send typing indicator"
                text: qsTrId("threesea-la-send-typing")
                //% "Let people see when you are writing to them."
                description: qsTrId("threesea-la-send-typing-description")
                onClicked: page.setSetting("sendTyping", !checked)
            }

            // Android's "Clear individual settings": contacts' own choices (contact page) go.
            SettingsButton {
                //% "Clear individual settings"
                text: qsTrId("threesea-me-clear-contact-policies")
                //% "Clearing individual settings"
                onClicked: Remorse.popupAction(page, qsTrId("threesea-la-clearing-contact-policies"), function() {
                    backend.call("clearContactPolicies", {}, function(ok, result) {
                        if (ok && result.cleared === 0) {
                            //% "No contact has its own setting."
                            Notices.show(qsTrId("threesea-la-no-contact-policies"), Notice.Short, Notice.Center)
                        }
                    })
                })
            }

            SectionHeader {
                //% "Other"
                text: qsTrId("threesea-se-other")
            }

            TextSwitch {
                automaticCheck: false
                checked: page.settings.incognitoKeyboard === true
                //% "Incognito keyboard"
                text: qsTrId("threesea-la-incognito-keyboard")
                //% "Ask the keyboard not to learn or suggest from what you type in chats."
                description: qsTrId("threesea-la-incognito-keyboard-description")
                onClicked: page.setSetting("incognitoKeyboard", !checked)
            }

            TextSwitch {
                automaticCheck: false
                checked: page.settings.disableScreenshots === true
                //% "Hide content on the cover"
                text: qsTrId("threesea-la-hide-cover")
                //% "The cover only shows the number of new messages, not who wrote."
                description: qsTrId("threesea-la-hide-cover-description")
                onClicked: page.setSetting("disableScreenshots", !checked)
            }

            SectionHeader {
                //% "Encryption"
                text: qsTrId("threesea-se-encryption")
            }

            TextSwitch {
                id: fsSwitch
                automaticCheck: false
                // Multi-device needs it off (the protocol's rule): the switch rests then.
                //% "Forward secrecy"
                text: qsTrId("threesea-la-forward-secrecy")
                //% "Protects chats with keys that change continuously. Messages are end-to-end encrypted either way; contacts see a security notice when the protection starts or resets."
                description: qsTrId("threesea-la-forward-secrecy-description")
                Component.onCompleted: refresh()
                function refresh() {
                    backend.call("forwardSecurity", {}, function(ok, result) {
                        if (!ok) return
                        fsSwitch.checked = result.on
                        fsSwitch.enabled = result.multiDevice !== true
                    })
                }
                onClicked: {
                    busy = true
                    backend.call("setForwardSecurity", { on: !checked }, function(ok, result) {
                        fsSwitch.busy = false
                        if (ok) fsSwitch.checked = result.on
                    })
                }
                Connections {
                    target: backend
                    onCoreEvent: if (name === "multiDeviceChanged") fsSwitch.refresh()
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
