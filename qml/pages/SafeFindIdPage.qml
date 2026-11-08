// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// Restoring from Threema Safe without knowing the ID: the directory finds it from a linked number
// or address. One find goes straight back; several are for the user to choose from.
Page {
    id: page
    allowedOrientations: Orientation.All

    signal chosen(string identity)

    property bool working: false
    // Filled once a search was answered with more than one ID.
    property var identities: []

    readonly property bool canSearch: !working && (phoneField.text.trim().length > 0
                                                   || emailField.text.trim().length > 0)

    function take(identity) {
        chosen(identity)
        pageStack.pop()
    }

    function search() {
        if (!canSearch) return
        working = true
        errorLabel.text = ""
        identities = []
        backend.call("safeSearchId", { phone: phoneField.text, email: emailField.text },
                     function(ok, result, kind, message) {
            page.working = false
            if (!ok) {
                errorLabel.text = Errors.text(kind, message)
            } else if (result.identities.length === 1) {
                page.take(result.identities[0])
            } else if (result.identities.length === 0) {
                //% "No Threema ID is linked to this."
                errorLabel.text = qsTrId("threesea-la-safe-find-id-none")
            } else {
                page.identities = result.identities
            }
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Find my ID"
                title: qsTrId("threesea-he-safe-find-id")
            }

            InfoLabel {
                //% "Enter the phone number or e-mail address linked to your Threema ID. It is hashed on this device, and Threema's directory is asked with the hash."
                text: qsTrId("threesea-la-safe-find-id-intro")
            }

            TextField {
                id: phoneField
                width: parent.width
                enabled: !page.working
                //% "Phone number"
                label: qsTrId("threesea-la-safe-find-id-phone")
                placeholderText: "+41 79 000 00 00"
                inputMethodHints: Qt.ImhDialableCharactersOnly | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: emailField.focus = true
            }

            TextField {
                id: emailField
                width: parent.width
                enabled: !page.working
                //% "E-mail address"
                label: qsTrId("threesea-la-safe-find-id-email")
                placeholderText: label
                inputMethodHints: Qt.ImhEmailCharactersOnly | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                EnterKey.enabled: page.canSearch
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.search()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                enabled: page.canSearch
                //% "Find"
                text: qsTrId("threesea-bt-safe-find-id")
                onClicked: page.search()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.working
                visible: running
            }

            Column {
                width: parent.width
                visible: page.identities.length > 0

                SectionHeader {
                    //% "Linked IDs"
                    text: qsTrId("threesea-se-safe-find-id-found")
                }

                Repeater {
                    model: page.identities
                    BackgroundItem {
                        width: parent.width
                        onClicked: page.take(modelData)

                        Label {
                            x: Theme.horizontalPageMargin
                            width: parent.width - 2 * x
                            anchors.verticalCenter: parent.verticalCenter
                            text: modelData
                            color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                        }
                    }
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
