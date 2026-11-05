// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// One contact's own read receipt or typing indicator choice (Android's contact page): like the
// app setting, always send, or never send.
ChoiceBox {
    id: root

    property string identity
    // "readReceipts" or "typing".
    property string field
    // `choice`: 0 like the app setting, 1 send, 2 don't send.
    // The app setting, for the first option's text.
    property bool appDefault: true

    signal changed(var contact)

    options: [
        appDefault
            //% "Same as in Settings (on)"
            ? qsTrId("threesea-me-policy-default-on")
            //% "Same as in Settings (off)"
            : qsTrId("threesea-me-policy-default-off"),
        //% "Always"
        qsTrId("threesea-me-policy-send"),
        //% "Never"
        qsTrId("threesea-me-policy-dont-send")
    ]

    onChosen: {
        var args = { identity: root.identity }
        args[root.field] = index
        backend.call("setContactPolicies", args, function(ok, result) { if (ok) root.changed(result) })
    }
}
