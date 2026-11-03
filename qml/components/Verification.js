// SPDX-License-Identifier: AGPL-3.0-only
.pragma library

function text(level) {
    switch (level) {
    case 2:
        //% "Fully verified"
        return qsTrId("threesea-la-verification-full")
    case 1:
        //% "Verified by the server"
        return qsTrId("threesea-la-verification-server")
    default:
        //% "Not verified"
        return qsTrId("threesea-la-verification-none")
    }
}

// 64 hex digits as 4 lines of 16, like a fingerprint.
function formatKey(hex) {
    if (!hex) return ""
    var lines = []
    for (var i = 0; i < hex.length; i += 16) {
        lines.push(hex.substr(i, 16).replace(/(.{4})/g, "$1 ").trim())
    }
    return lines.join("\n")
}
