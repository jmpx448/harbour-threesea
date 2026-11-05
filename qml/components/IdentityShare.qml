// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Share 1.0

// Passes on a Threema ID as the link core/src/links.rs reads, which any Threema app opens. The
// ID only: a name, if any, is for the user to add.
ShareAction {
    //% "Share ID"
    title: qsTrId("threesea-he-share-id")
    mimeType: "text/x-url"

    function shareIdentity(identity) {
        resources = [{ type: "text/x-url", status: "https://threema.id/" + identity }]
        trigger()
    }
}
