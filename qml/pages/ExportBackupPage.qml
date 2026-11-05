// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Share 1.0
import "../components"
import "../components/Errors.js" as Errors

Page {
    id: page
    objectName: "holdsPin"
    allowedOrientations: Orientation.All

    property bool working: false
    property string backup: ""
    property var qrCode: ({ size: 0, bits: "" })
    // Copied or shared: it can't be lost by a swipe any more. Until then leaving asks first, as
    // what is shown goes with the page, like Android's export.
    property bool handedOver: false
    backNavigation: backup.length === 0 || handedOver
    // The app lock's PIN, while this page lives (window.pushWithPin).
    property string pin

    function exportBackup() {
        working = true
        errorLabel.text = ""
        backend.call("exportIdentityBackup", { password: passwordField.text, pin: page.pin }, function(ok, result, kind, message) {
            working = false
            if (ok) {
                backup = result.backup
                qrCode = result.qr
                passwordField.text = ""
                repeatField.text = ""
                Qt.inputMethod.hide()
            } else {
                errorLabel.text = Errors.text(kind, message)
            }
        })
    }

    function close() {
        if (pageStack.currentPage === page) pageStack.pop()
    }

    RemorsePopup { id: remorse }

    ShareAction {
        id: shareAction
        //% "Share ID backup"
        title: qsTrId("threesea-he-share-id-backup")
        mimeType: "text/plain"
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Export ID backup"
                title: qsTrId("threesea-he-export-backup")
            }

            InfoLabel {
                //% "An ID backup lets you restore your Threema ID on another device. Keep it and its password safe: anyone with both can use your ID."
                text: qsTrId("threesea-la-export-intro")
            }

            PasswordField {
                id: passwordField
                width: parent.width
                visible: page.backup.length === 0
                enabled: !page.working
                //% "Password (at least 8 characters)"
                label: qsTrId("threesea-la-new-backup-password")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: repeatField.focus = true
            }

            PasswordField {
                id: repeatField
                width: parent.width
                visible: page.backup.length === 0
                enabled: !page.working
                //% "Repeat password"
                label: qsTrId("threesea-la-repeat-password")
                errorHighlight: text.length > 0 && text !== passwordField.text
                EnterKey.enabled: passwordField.text.length >= 8 && text === passwordField.text
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.exportBackup()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.backup.length === 0
                enabled: !page.working && passwordField.text.length >= 8 && repeatField.text === passwordField.text
                //% "Create backup"
                text: qsTrId("threesea-bt-create-backup")
                onClicked: page.exportBackup()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.working
                visible: running
            }

            QrCodeView {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.backup.length > 0
                // All of it on screen at once in landscape too.
                width: Math.min(page.width - 2 * Theme.horizontalPageMargin,
                                page.height - 2 * Theme.paddingLarge)
                code: page.qrCode
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.backup.length > 0
                // Zero-width spaces after the dashes let lines break between groups only.
                text: page.backup.replace(/-/g, "-\u200b")
                wrapMode: Text.Wrap
                font.family: "monospace"
                color: Theme.highlightColor
            }

            ButtonLayout {
                visible: page.backup.length > 0

                Button {
                    //% "Copy to clipboard"
                    text: qsTrId("threesea-bt-copy")
                    onClicked: {
                        Clipboard.text = page.backup
                        page.handedOver = true
                        //% "Copied"
                        Notices.show(qsTrId("threesea-la-copied"), Notice.Short, Notice.Center)
                    }
                }
                Button {
                    //% "Share"
                    text: qsTrId("threesea-bt-share")
                    onClicked: {
                        // As text, not as data: a share method could write that to a file of its own.
                        shareAction.resources = [{ type: "text/plain", status: page.backup }]
                        shareAction.trigger()
                        page.handedOver = true
                    }
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.backup.length > 0
                //% "Done"
                text: qsTrId("threesea-bt-done")
                onClicked: {
                    if (page.handedOver) page.close()
                    //% "Closing ID backup"
                    else remorse.execute(qsTrId("threesea-la-closing-id-backup"), page.close)
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
