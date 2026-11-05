// SPDX-License-Identifier: AGPL-3.0-only
.pragma library

// What a page really needs to be opened with, by its path under qml/. Pages not named here open
// with no properties.
var table = {
    "pages/ChatPage.qml": { identity: "ABCD1234" },
    "pages/ContactPage.qml": { identity: "ABCD1234" },
    "pages/GroupPage.qml": { identity: "g:ABCD1234:0102030405060708" },
    "pages/DistributionListPage.qml": { identity: "d:1" },
    "pages/ImageViewerPage.qml": { identity: "ABCD1234", messageId: 5 },
    "pages/SafePasswordDialog.qml": { firstRun: true },
    "pages/MemberPickerDialog.qml": { askName: true, title: "New group", acceptText: "Create" },
    "pages/PlacePickerPage.qml": { positioning: { center: { latitude: 47.2, longitude: 8.8, recent: true },
                                                  place: { name: "Cafe" }, waitingText: "",
                                                  choosePlace: function() {} } },
    "pages/PollsPage.qml": { identity: "ABCD1234", chatName: "Sam" },
    "pages/PollPage.qml": { messageId: 12, conversation: "ABCD1234",
                            poll: { id: 1, creator: "AAAAAAAA", description: "When?", multiple: false, announce: true,
                                    state: 0, choices: [{ id: 0, name: "Now" }, { id: 1, name: "Later" }],
                                    votes: { ABCD1234: [0], AAAAAAAA: [1] } } },
}

function properties(path) {
    return table[path] || {}
}

// Anything to set on the fakes first.
function prepare(path, fakes) {
}
