// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Page {
    objectName: "chat"
    property string identity
    property bool isPrivate
    property QtObject inputField: QtObject { property string text }
    property var attached: []
    function attachAll(files) { attached = files }
    function setInput(text) { inputField.text = text }
    Component.onCompleted: window.openChat = identity
    Component.onDestruction: if (window.openChat === identity) window.openChat = ""
}
