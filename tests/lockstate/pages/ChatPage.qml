// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Page {
    objectName: "chat"
    property string identity
    property bool isPrivate
    readonly property var inputField: null
    Component.onCompleted: window.openChat = identity
    Component.onDestruction: if (window.openChat === identity) window.openChat = ""
}
