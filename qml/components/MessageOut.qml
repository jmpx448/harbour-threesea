// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Share 1.0
import "Format.js" as Fmt

// What leaves a chat on the user's word: text to another app over the system's share dialog, files
// to Downloads. Also watches the downloads the user asked for, as only those are worth a word when
// they fail. It stays with the window, so an answer finds it after its page has gone.
Item {
    id: out
    visible: false

    // The messages whose downloads were tapped, until they end: true once the file's is running.
    property var askedFor: ({})

    function download(id) {
        var asked = askedFor
        asked[id] = false
        askedFor = asked
        backend.call("downloadFile", { message: id })
    }

    function shareText(text) {
        action.mimeType = "text/plain"
        action.resources = [{ name: "message.txt", data: text, type: "text/plain" }]
        action.trigger()
    }

    ShareAction { id: action }

    Connections {
        target: backend
        onCoreEvent: {
            if (name !== "messagesChanged") return
            var asked = out.askedFor
            for (var i = 0; i < data.messages.length; ++i) {
                var m = data.messages[i]
                if (asked[m.id] === undefined || !m.body) continue
                var step = Fmt.downloadStep(asked[m.id], m.body.fileState)
                if (step === "running") asked[m.id] = true
                else if (step !== "waiting") delete asked[m.id]
                if (step === "failed") {
                    //% "The file could not be downloaded. It may no longer be on the server."
                    Notices.show(qsTrId("threesea-la-download-failed"), Notice.Long, Notice.Center)
                }
            }
        }
    }
}
