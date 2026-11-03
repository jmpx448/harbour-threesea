// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A new poll: the question, the choices (a new empty field appears below the last one), single
// or multiple choice, and whether everyone sees the votes before the poll is closed.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property alias question: questionField.text
    property alias multiple: multipleSwitch.checked
    property alias announce: announceSwitch.checked
    property var choices: []

    readonly property int filledChoices: {
        var n = 0
        for (var i = 0; i < choiceModel.count; ++i) if (choiceModel.get(i).text.trim().length > 0) n++
        return n
    }
    canAccept: questionField.text.trim().length > 0 && filledChoices >= 2

    onAccepted: {
        var list = []
        for (var i = 0; i < choiceModel.count; ++i) {
            var text = choiceModel.get(i).text.trim()
            if (text.length > 0) list.push(text)
        }
        choices = list
    }

    ListModel {
        id: choiceModel
        ListElement { text: "" }
        ListElement { text: "" }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            DialogHeader {
                //% "Send"
                acceptText: qsTrId("threesea-he-send")
                //% "New poll"
                title: qsTrId("threesea-he-new-poll")
            }

            TextField {

                inputMethodHints: window.incognitoHints
                id: questionField
                width: parent.width
                focus: true
                //% "Question"
                label: qsTrId("threesea-la-poll-question")
                placeholderText: label
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: choiceFields.itemAt(0).forceActiveFocus()
            }

            SectionHeader {
                //% "Choices"
                text: qsTrId("threesea-he-poll-choices")
            }

            Repeater {
                id: choiceFields
                model: choiceModel
                TextField {
                    inputMethodHints: window.incognitoHints
                    width: column.width
                    text: model.text
                    //% "Choice %1"
                    label: qsTrId("threesea-la-poll-choice").arg(index + 1)
                    placeholderText: label
                    onTextChanged: {
                        choiceModel.setProperty(index, "text", text)
                        // Always one empty field at the end.
                        if (index === choiceModel.count - 1 && text.length > 0) choiceModel.append({ text: "" })
                    }
                    function forceActiveFocus() { focus = true }
                    EnterKey.iconSource: "image://theme/icon-m-enter-next"
                    EnterKey.onClicked: if (index + 1 < choiceModel.count) choiceFields.itemAt(index + 1).focus = true
                }
            }

            TextSwitch {
                id: multipleSwitch
                //% "Multiple choice"
                text: qsTrId("threesea-la-poll-multiple")
            }
            TextSwitch {
                id: announceSwitch
                //% "Show votes before closing"
                text: qsTrId("threesea-la-poll-announce")
                //% "Otherwise only you see the votes until you close the poll."
                description: qsTrId("threesea-la-poll-announce-description")
            }
        }

        VerticalScrollDecorator { }
    }
}
