// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Asked once after creating an ID or restoring it from an ID backup. Until there is an answer,
// messages leave the name contacts have stored alone. Accept: this name; back or cancel: not now.
// Settings → My profile can change it either way.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    canAccept: field.text.trim().length > 0

    onAccepted: backend.call("setNickname", { nickname: field.text })
    onRejected: backend.call("declineNickname", {})

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: qsTrId("threesea-he-confirm")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraLarge
                font.family: Theme.fontFamilyHeading
                color: Theme.highlightColor
                //% "Choose a nickname"
                text: qsTrId("threesea-la-nickname-ask-title")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                //% "This name goes with your messages. Contacts who haven't given you a name of their own see it. You can change it in Settings, My profile."
                text: qsTrId("threesea-la-nickname-ask-text")
            }

            TextField {
                id: field
                width: parent.width
                focus: true
                label: qsTrId("threesea-la-nickname")
                placeholderText: label
                EnterKey.enabled: dialog.canAccept
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }
        }

        VerticalScrollDecorator { }
    }
}
