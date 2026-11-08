// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// What can be done with a Threema ID that is held down: copy it, share it.
ContextMenu {
    id: menu

    property string identity
    property var _share: IdentityShare {}

    MenuItem {
        //% "Copy ID"
        text: qsTrId("threesea-me-copy-id")
        onClicked: {
            Clipboard.text = menu.identity
            Notices.show(qsTrId("threesea-la-copied"), Notice.Short, Notice.Center)
        }
    }
    MenuItem {
        //% "Share ID"
        text: qsTrId("threesea-me-share-id")
        onClicked: menu._share.shareIdentity(menu.identity)
    }
}
