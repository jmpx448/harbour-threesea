// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

Page {
    id: page
    objectName: "license"
    allowedOrientations: Orientation.All

    property bool checking: false
    // Threema's directory refused the stored key: its own words, shown with the key.
    property bool refused: false

    Component.onCompleted: {
        backend.call("licenseStatus", {}, function(ok, status) {
            if (!ok || !status.refused) return
            refused = true
            licenseField.text = status.key
            errorLabel.text = status.refused
        })
    }

    // From a threema://license?key=… link.
    function useKey(key) {
        licenseField.text = key
        check()
    }

    function check() {
        if (checking || !licenseField.acceptableInput) {
            return
        }
        checking = true
        errorLabel.text = ""
        backend.call("checkLicense", { licenseKey: licenseField.text }, function(ok, result, kind, message) {
            checking = false
            if (ok) {
                backend.refresh()
            } else {
                errorLabel.text = Errors.text(kind, message)
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
                title: page.refused
                       //% "License key"
                       ? qsTrId("threesea-he-license-key")
                       //% "Welcome to Threesea"
                       : qsTrId("threesea-he-welcome")
            }

            InfoLabel {
                text: page.refused
                      //% "Threesea stays offline until a license key is accepted. Your chats stay on this device."
                      ? qsTrId("threesea-la-license-refused")
                      //% "Threesea is an unofficial Threema client for Sailfish OS. It needs a Threema Libre license key, which you can buy in the Threema Shop (shop.threema.ch)."
                      : qsTrId("threesea-la-license-intro")
            }

            TextField {
                id: licenseField
                width: parent.width
                enabled: !page.checking
                //% "License key"
                label: qsTrId("threesea-la-license-key")
                placeholderText: "ABCDE-12345"
                inputMethodHints: Qt.ImhUppercaseOnly | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                validator: RegExpValidator { regExp: /^\s*[A-Za-z0-9]{5}-[A-Za-z0-9]{5}\s*$/ }
                EnterKey.enabled: acceptableInput
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.check()
            }

            ErrorLabel { id: errorLabel }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                enabled: licenseField.acceptableInput && !page.checking
                //% "Check license"
                text: qsTrId("threesea-bt-check-license")
                onClicked: page.check()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                running: page.checking
                visible: running
            }
        }

        VerticalScrollDecorator { }
    }
}
