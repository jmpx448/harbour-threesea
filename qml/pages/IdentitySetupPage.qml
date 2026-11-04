// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

Page {
    id: page
    objectName: "identitySetup"
    allowedOrientations: Orientation.All

    property bool working: false

    function createIdentity() {
        working = true
        errorLabel.text = ""
        backend.call("createIdentity", {}, function(ok, result, kind, message) {
            working = false
            if (ok) {
                backend.refresh()
            } else {
                errorLabel.text = Errors.text(kind, message)
            }
        })
    }

    // From a threema://license?key=… link: the directory may have turned the last key down here.
    function useKey(key) {
        if (working) return
        working = true
        errorLabel.text = ""
        backend.call("checkLicense", { licenseKey: key }, function(ok, result, kind, message) {
            working = false
            if (ok) {
                //% "License key changed"
                Notices.show(qsTrId("threesea-la-license-key-changed"), Notice.Long, Notice.Center)
            } else {
                errorLabel.text = Errors.text(kind, message)
            }
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            enabled: !page.working
            MenuItem {
                //% "Change license key"
                text: qsTrId("threesea-me-change-license-key")
                onClicked: backend.call("forgetLicense", {}, function(ok, result, kind, message) {
                    if (ok) backend.refresh()
                    else errorLabel.text = Errors.text(kind, message)
                })
            }
        }

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Your Threema ID"
                title: qsTrId("threesea-he-identity-setup")
            }

            InfoLabel {
                //% "Your Threema ID identifies you to your contacts. Create a new one, or restore the ID you already use from Threema Safe or an ID backup."
                text: qsTrId("threesea-la-identity-intro")
            }

            ErrorLabel { id: errorLabel }

            ButtonLayout {
                enabled: !page.working

                Button {
                    //% "Create new ID"
                    text: qsTrId("threesea-bt-create-identity")
                    onClicked: page.createIdentity()
                }
                Button {
                    //% "Restore from Threema Safe"
                    text: qsTrId("threesea-bt-restore-safe")
                    onClicked: pageStack.push(Qt.resolvedUrl("SafeRestorePage.qml"))
                }
                Button {
                    //% "Restore ID backup"
                    text: qsTrId("threesea-bt-restore-backup")
                    onClicked: pageStack.push(Qt.resolvedUrl("RestoreBackupPage.qml"))
                }
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.working
                visible: running
            }
        }

        VerticalScrollDecorator { }
    }
}
