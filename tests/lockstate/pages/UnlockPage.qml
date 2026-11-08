// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Page {
    objectName: "unlock"
    property bool cancellable: true
    property bool privateChats
    property bool database
    property string title
    signal unlocked(string pin)
    backNavigation: false
}
