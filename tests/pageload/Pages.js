// SPDX-License-Identifier: AGPL-3.0-only
.pragma library

// What a page really needs to be opened with, by its path under qml/. Pages not named here open
// with no properties.
var table = {
    "pages/ChatPage.qml": { identity: "ABCD1234" },
    "pages/ContactPage.qml": { identity: "ABCD1234" },
    "pages/GroupPage.qml": { identity: "g:ABCD1234:0102030405060708" },
    "pages/DistributionListPage.qml": { identity: "d:1" },
}

function properties(path) {
    return table[path] || {}
}

// Anything to set on the fakes first.
function prepare(path, fakes) {
}
