// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components/Errors.js" as Errors
import "../../components/Format.js" as Fmt

// Linking Threema for Desktop, step 2 (Android's wizard): connect to it, let the user pick the
// emojis the computer shows (one of three rows is right), send the data, show the result.
Page {
    id: page
    allowedOrientations: Orientation.All

    property string code
    // connecting, verify, linking, done, failed
    property string step: "connecting"
    property var choices: []
    property int correct: -1
    property string errorText
    // Not while the data goes over: leaving would cut it off.
    backNavigation: step !== "linking"

    function fail(text) {
        errorText = text
        step = "failed"
    }

    // The emoji rows take taps only after a moment: a tap meant for the page before (the picture
    // picker) must not confirm the check by landing on them.
    property bool choicesArmed: false
    Timer {
        id: armChoices
        interval: 800
        onTriggered: page.choicesArmed = true
    }

    function pick(index) {
        if (!choicesArmed) return
        if (index !== correct) {
            backend.call("cancelLinking", {})
            //% "The emojis don't match. Close Threema for Desktop, open it again and scan its new code."
            fail(qsTrId("threesea-la-link-emoji-mismatch"))
            return
        }
        step = "linking"
        backend.call("confirmLinking", {}, function(ok, result, kind, message) {
            if (ok) page.step = "done"
            else page.fail(Errors.text(kind, message))
        })
    }

    // Once shown, not when created: a dialog creates its accept destination in advance.
    property bool started: false
    onStatusChanged: {
        if (status !== PageStatus.Active || started) return
        started = true
        backend.call("startLinking", { code: code }, function(ok, result, kind, message) {
            if (!ok) {
                page.fail(Errors.text(kind, message))
                return
            }
            page.choices = result.choices
            page.correct = result.correct
            page.step = "verify"
            armChoices.restart()
        })
    }
    Component.onDestruction: if (step === "verify") backend.call("cancelLinking", {})

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                //% "Link device"
                title: qsTrId("threesea-he-link-device")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: page.step === "connecting"
                      //% "Connecting to the computer…"
                      ? qsTrId("threesea-la-link-connecting")
                      : page.step === "verify"
                        //% "Which emojis does the computer show?"
                        ? qsTrId("threesea-la-link-verify")
                        : page.step === "linking"
                          //% "Linking. Keep Threesea open until the computer is ready."
                          ? qsTrId("threesea-la-link-linking")
                          : page.step === "done"
                            //% "Threema for Desktop is linked."
                            ? qsTrId("threesea-la-link-done")
                            : page.errorText
            }

            // The emoji check: three rows, like Android.
            Repeater {
                model: page.step === "verify" ? page.choices : []
                BackgroundItem {
                    width: column.width
                    height: Theme.itemSizeExtraLarge
                    enabled: page.choicesArmed
                    onClicked: page.pick(index)
                    Row {
                        anchors.centerIn: parent
                        spacing: Theme.paddingLarge * 2
                        Repeater {
                            model: modelData
                            Label {
                                textFormat: Text.StyledText
                                text: backend.threemaEmoji ? Fmt.markEmoji(modelData, Theme.iconSizeLarge) : modelData
                                font.pixelSize: Theme.fontSizeHuge
                            }
                        }
                    }
                }
            }

            Item {
                width: parent.width
                height: Theme.itemSizeLarge
                visible: page.step === "connecting" || page.step === "linking"
                BusyIndicator {
                    anchors.centerIn: parent
                    size: BusyIndicatorSize.Large
                    running: parent.visible
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.step === "verify"
                //% "None of them"
                text: qsTrId("threesea-me-link-no-match")
                onClicked: page.pick(-1)
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.step === "done" || page.step === "failed"
                //% "Done"
                text: qsTrId("threesea-me-done")
                onClicked: pageStack.pop()
            }
        }
    }
}
