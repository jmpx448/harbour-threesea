// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// "Media" with the number of photos, videos, voice messages and files on a contact, group or list
// page; opens the chat's media grid. Hidden while the chat has none.
ValueButton {
    id: root

    property string identity
    // Shown under the grid's title.
    property string name
    property int count

    function load() {
        backend.call("chatMedia", { identity: identity }, function(ok, result) {
            if (!ok) return
            var total = 0
            for (var kind in result.counts) total += result.counts[kind]
            root.count = total
        })
    }

    visible: count > 0
    //% "Media"
    label: qsTrId("threesea-he-media")
    value: count
    onIdentityChanged: if (identity.length > 0) load()
    onClicked: pageStack.push(Qt.resolvedUrl("../pages/MediaPage.qml"), { identity: identity, name: name })

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "messageAdded" && data.message.conversation === root.identity) root.load()
            else if (name === "messagesChanged") {
                for (var i = 0; i < data.messages.length; ++i) {
                    if (data.messages[i].conversation === root.identity) return root.load()
                }
            }
        }
    }
}
