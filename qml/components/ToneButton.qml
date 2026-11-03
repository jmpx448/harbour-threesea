// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A notification tone setting (ToneDialog): `tone` is the chosen file, "" the default
// (`defaultText`). `chosen(tone)` once the dialog is accepted.
ValueButton {
    id: root

    property string tone
    property string defaultText
    signal chosen(string tone)

    //% "Notification tone"
    label: qsTrId("threesea-la-notification-tone")
    value: tone.length > 0 ? backend.toneTitle(tone) : defaultText
    onClicked: {
        var dialog = pageStack.push(Qt.resolvedUrl("../pages/ToneDialog.qml"),
                                    { tone: root.tone, defaultText: root.defaultText })
        dialog.accepted.connect(function() { root.chosen(dialog.tone) })
    }
}
