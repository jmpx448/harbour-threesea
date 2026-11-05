// SPDX-License-Identifier: AGPL-3.0-only
.pragma library

// What the core answers to the commands the pages send (docs/ffi.md), in the shape it really has:
// made-up people, a bit of everything for the lists, so that the rows' bindings run as well.
// Anything else gets an empty object.

function message(id, kind, more) {
    var row = { id: id, conversation: "ABCD1234", messageId: "000000000000" + (1000 + id), outgoing: id % 2 === 0,
                kind: kind, text: null, quoteId: null, body: null, createdAt: 1700000000000 + id * 3600000,
                arrivedAt: 1700000000000 + id * 3600000, updatedAt: 1700000000000 + id * 3600000, state: 2,
                editedAt: null, sender: id % 2 === 0 ? null : "ABCD1234", reactions: [], starred: false,
                sentAt: null, deliveredAt: null, readAt: null, receivedAt: null, sortKey: id, writtenOut: true }
    for (var key in more) row[key] = more[key]
    return row
}

function file(mediaType, rendering, more) {
    var body = { name: "file", caption: "", mediaType: mediaType, rendering: rendering, size: 1234, fileState: 2,
                 thumbnailState: 2, uploadState: 2, listened: false, duration: 3 }
    for (var key in more) body[key] = more[key]
    return body
}

var poll = { id: 1, creator: "AAAAAAAA", description: "When?", multiple: false, announce: true, state: 0,
             choices: [{ id: 0, name: "Now" }, { id: 1, name: "Later" }],
             votes: { ABCD1234: [0], AAAAAAAA: [1] } }

function messages() {
    return [
        message(1, "status", { body: "groupCreated" }),
        message(2, "text", { text: "a short one" }),
        message(3, "text", { text: "with a reaction, starred and edited", starred: true, editedAt: 1700000000000,
                             reactions: [{ emoji: "👍", identity: "AAAAAAAA", mine: true }] }),
        message(4, "text", { text: "a quote", quoteId: "0000000000001002" }),
        message(5, "file", { body: file("image/jpeg", "image") }),
        message(6, "file", { body: file("image/gif", "image") }),
        message(7, "file", { body: file("video/mp4", "media") }),
        message(8, "file", { body: file("audio/aac", "media") }),
        message(9, "file", { body: file("audio/aac", "media", { fileState: 0 }) }),
        message(10, "file", { body: file("application/pdf", "file", { name: "Invoice #12.pdf", uploadState: 3 }) }),
        message(11, "location", { body: { latitude: 47.3, longitude: 8.5, accuracy: 10, name: "A place" } }),
        message(12, "poll", { body: poll }),
        message(13, "pollClosed", { body: { id: 1, creator: "AAAAAAAA", description: "When?", multiple: false,
                                            announce: true, state: 1, choices: poll.choices, votes: poll.votes } }),
        message(14, "deleted"),
        message(15, "status", { body: "call|missed|0|0" }),
        message(16, "text", { text: "mention @[ABCD1234]", mentions: { ABCD1234: "Ada" } }),
    ]
}

function contact() {
    return { identity: "ABCD1234", displayName: "Ada", firstName: "Ada", lastName: "", nickname: "ada",
             verification: 1, work: false, workVerified: false, inactive: false, invalid: false, hidden: false,
             saved: true, reactions: true, canEdit: true, canDelete: true, canPoll: true, canCall: true,
             canVideoCall: true, publicKey: "0000000000000000000000000000000000000000000000000000000000000000",
             readReceipts: 0, typing: 0, pictureVersion: 0, pictureRecipient: null, notifications: {},
             pinned: null, blocked: false, neededByGroup: false }
}

var member = { identity: "ABCD1234", displayName: "Ada", creator: true, pictureVersion: 0 }

function group() {
    return { key: "g:ABCD1234:0102030405060708", name: "A group", title: "A group", creator: "ABCD1234",
             isCreator: false, left: false, isNotes: false, members: [member], createdAt: 1700000000000,
             pictureVersion: 0, notifications: {}, pinned: null, reactions: true, canDelete: true, canPoll: true }
}

function list() {
    return { key: "d:1", name: "A list", displayName: "A list", members: [member], pinned: null, pictureVersion: 0 }
}

function row(identity, name, more) {
    var item = { identity: identity, displayName: name, verification: 1, unread: 0, pictureVersion: 0, pinned: null,
                 markedUnread: false, lastMessage: message(2, "text", { text: "the last one" }), muted: false }
    for (var key in more) item[key] = more[key]
    return item
}

var table = {
    contacts: { contacts: [{ identity: "ABCD1234", displayName: "Ada", section: "A", pictureVersion: 0 },
                           { identity: "BCDE2345", displayName: "Bob", section: "B", pictureVersion: 0 }] },
    conversations: function() {
        return { conversations: [
            row("ABCD1234", "Ada", { unread: 2 }),
            row("g:ABCD1234:0102030405060708", "A group", { group: true, left: false, pinned: 1, senderName: "Ada" }),
            row("d:1", "A list", { distributionList: true }),
        ] }
    },
    messages: function(args) {
        // The window asked for, once: the pages go back for more while it fills the view.
        var all = args && (args.before !== undefined || args.after !== undefined) ? [] : messages()
        return { identity: args ? args.identity : "", messages: all, private: false, firstUnread: null, reachesNewest: true }
    },
    contact: contact,
    group: group,
    distributionList: list,
    draft: { text: "", quote: null },
    spamPotential: { potential: true },
    lockStatus: { lock: null, unlocked: true, graceTimes: [] },
    profile: { identity: "AAAAAAAA", nickname: "me", publicKey: "00", pictureVersion: 0, pictureShare: "everyone",
               pictureRecipients: [], linkedEmail: "", linkedEmailPending: false, linkedMobile: "",
               linkedMobilePending: "" },
    settings: { readReceipts: true, sendTyping: true, blockUnknown: false, threemaCalls: true, groupCalls: true,
                videoCalls: true, relayThreemaCalls: false, videoQuality: "auto", blockedContacts: [] },
    chatMedia: function() {
        return { messages: [message(5, "file", { mediaKind: "image", body: file("image/jpeg", "image") })],
                 kinds: ["image"], counts: { image: 1, gif: 0, video: 0, voice: 0, audio: 0, file: 0 } }
    },
    blockedContacts: { blocked: [{ identity: "ABCD1234", displayName: "Ada" }] },
    forwardSecurity: { on: true, multiDevice: false },
    logText: { text: "a line\n" },
    multiDevice: { active: true },
    linkedDevices: { devices: [{ id: "01", current: true, label: "This phone", platform: "unknown", platformDetails: "",
                                 appVersion: "1", connectedSince: 1700000000000, lastDisconnectAt: null },
                               { id: "02", current: false, label: "", alias: "A computer", platform: "desktop",
                                 platformDetails: "", appVersion: "1", connectedSince: null,
                                 lastDisconnectAt: 1700000000000 }],
                     label: "This phone", active: true },
    safeStatus: { enabled: true, server: "", lastBackup: 1700000000000, lastError: "", size: 1, maxBytes: 2,
                  retentionDays: 180, restore: { pendingContacts: 0, pendingGroups: 0, pendingLists: 0,
                                                 importError: "", summary: null } },
    startLinking: { choices: [], correct: 0 },
    webSessions: { enabled: true,
                   sessions: [{ id: "s1", state: "stopped", client: "firefox", host: "web.example.org", selfHosted: true,
                                persistent: true, createdAt: 1700000000000, lastConnection: 1700000000000 },
                              { id: "s2", state: "connected", client: "", host: "web.threema.ch", selfHosted: false,
                                persistent: false, createdAt: 1700000000000, lastConnection: null }],
                   servers: ["web.example.org"], certificates: [] },
    addressBookCountry: { chosen: "", automaticCode: "41", countries: [{ region: "CH", code: "41" }] },
    androidBackupImport: { contacts: { added: 1, existing: 0, updated: 0, keyConflicts: 0 },
                           groups: { added: 1, existing: 0, skippedDeleted: 0, skippedUnknownCreator: 0 },
                           lists: { added: 0, existing: 0 },
                           pictures: { own: 0, contacts: 0, groups: 0, kept: 0, localAvatarsNotImported: 0, skipped: 0 },
                           nonces: { csp: 0, d2d: 0, existing: 0, skipped: 0, missing: 0 }, invalidRows: 0 },
    gifFavourites: { favourites: [] },
    recentEmoji: { emoji: [] },
    searchMessages: { messages: [] },
    starredMessages: { messages: [] },
}

function answer(cmd, args) {
    var known = table[cmd]
    if (known === undefined) return {}
    return typeof known === "function" ? known(args) : known
}
