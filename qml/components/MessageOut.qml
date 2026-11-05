// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Share 1.0

// What leaves a chat on the user's word: text to another app over the system's share dialog. It
// stays with the window, so an answer finds it after its page has gone.
Item {
    visible: false

    function shareText(text) {
        action.mimeType = "text/plain"
        action.resources = [{ name: "message.txt", data: text, type: "text/plain" }]
        action.trigger()
    }

    ShareAction { id: action }
}
