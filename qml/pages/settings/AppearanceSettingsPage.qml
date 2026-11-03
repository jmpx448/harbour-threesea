// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "../../components"

// Settings → Appearance (Android's): colours and fonts follow the Ambience and the system.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var display: ({})

    function set(args) {
        backend.call("setDisplaySettings", args, function(ok, result) {
            if (!ok) return
            page.display = result
            backend.refreshContacts()
        })
    }

    Component.onCompleted: backend.call("displaySettings", {}, function(ok, result) {
        if (ok) page.display = result
    })

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            MenuItem {
                visible: backend.wallpaper !== ""
                //% "Use the Ambience's background"
                text: qsTrId("threesea-me-wallpaper-clear")
                onClicked: backend.wallpaper = ""
            }
        }

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Appearance"
                title: qsTrId("threesea-se-appearance")
            }

            SectionHeader {
                text: qsTrId("threesea-he-chats")
            }

            ChoiceBox {
                options: [
                    //% "Threema"
                    qsTrId("threesea-me-emoji-threema"),
                    //% "System"
                    qsTrId("threesea-me-emoji-system")
                ]
                //% "Emoji style"
                label: qsTrId("threesea-la-emoji-style")
                choice: backend.threemaEmoji ? 0 : 1
                onChosen: backend.threemaEmoji = index === 0
            }

            ChoiceBox {
                options: [
                    //% "Small"
                    qsTrId("threesea-me-font-size-small"),
                    //% "Normal"
                    qsTrId("threesea-me-font-size-normal"),
                    //% "Large"
                    qsTrId("threesea-me-font-size-large")
                ]
                //% "Message text"
                label: qsTrId("threesea-la-chat-font-size")
                choice: backend.chatFontSize
                onChosen: backend.chatFontSize = index
            }

            TextSwitch {
                automaticCheck: false
                checked: backend.biggerEmojis
                //% "Bigger single emojis"
                text: qsTrId("threesea-la-bigger-emojis")
                //% "Messages of up to three emojis are shown larger."
                description: qsTrId("threesea-la-bigger-emojis-description")
                onClicked: backend.biggerEmojis = !checked
            }

            // Android's chat wallpaper: one picture behind the messages in every chat.
            ValueButton {
                //% "Chat background"
                label: qsTrId("threesea-la-wallpaper")
                value: backend.wallpaper
                       ? backend.wallpaper.split("/").pop()
                         //% "The Ambience's background"
                       : qsTrId("threesea-la-wallpaper-ambience")
                onClicked: pageStack.push(wallpaperPicker)
            }

            SectionHeader {
                //% "Contacts"
                text: qsTrId("threesea-se-contacts")
            }

            TextSwitch {
                automaticCheck: false
                checked: window.appSettings.receiveProfilePictures !== false
                //% "Accept profile pictures"
                text: qsTrId("threesea-la-receive-pictures")
                //% "Contacts who share their profile picture are shown with it."
                description: qsTrId("threesea-la-receive-pictures-description")
                onClicked: backend.call("setSetting", { name: "receiveProfilePictures", value: !checked },
                                        function(ok, result) { if (ok) window.appSettings = result })
            }

            ChoiceBox {
                options: [
                    //% "First name first"
                    qsTrId("threesea-me-first-name-first"),
                    //% "Last name first"
                    qsTrId("threesea-me-last-name-first")
                ]
                //% "Names"
                label: qsTrId("threesea-la-name-order")
                choice: page.display.lastNameFirst ? 1 : 0
                onChosen: page.set({ lastNameFirst: index === 1 })
            }

            ChoiceBox {
                options: [
                    //% "By first name"
                    qsTrId("threesea-me-sort-first-name"),
                    //% "By last name"
                    qsTrId("threesea-me-sort-last-name")
                ]
                //% "Sorting"
                label: qsTrId("threesea-la-sorting")
                choice: page.display.sortByLastName ? 1 : 0
                onChosen: page.set({ sortByLastName: index === 1 })
            }

            TextSwitch {
                automaticCheck: false
                checked: page.display.showInactive !== false
                //% "Show inactive IDs"
                text: qsTrId("threesea-la-show-inactive")
                //% "Contacts whose Threema ID has not been used for a long time stay in the list."
                description: qsTrId("threesea-la-show-inactive-description")
                onClicked: page.set({ showInactive: !checked })
            }
        }

        VerticalScrollDecorator { }
    }

    Component {
        id: wallpaperPicker
        ImagePickerPage {
            onSelectedContentPropertiesChanged: backend.wallpaper = selectedContentProperties.filePath
        }
    }
}
