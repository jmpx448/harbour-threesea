// SPDX-License-Identifier: AGPL-3.0-only
.pragma library
.import "EmojiTable.js" as EmojiTable

// Clock time of a message (date sections carry the day), like Jolla Messages' lists.
function clock(ms, Format, Formatter) {
    return Format.formatDate(new Date(ms), Formatter.TimeValue)
}

// Time shown in a message bubble, like Jolla Messages: clock time today, "Tuesday 20:42"
// within the last week, otherwise "1 Sep 10:12", with the year if it is not the current one.
function time(ms, Format, Formatter) {
    var date = new Date(ms)
    var clockText = Format.formatDate(date, Formatter.TimeValue)
    var days = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86400000)
    if (days === 0) return clockText
    if (days > 0 && days < 7) return Qt.formatDate(date, "dddd") + " " + clockText
    var sameYear = date.getFullYear() === new Date().getFullYear()
    return Format.formatDate(date, sameYear ? Formatter.DateMediumWithoutYear : Formatter.DateMedium) + " " + clockText
}

function startOfDay(date) {
    return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
}

// Day of a message for section headers: "Today", "Yesterday", a weekday within the last week,
// otherwise the language's long date, "Wednesday, 14 October" (with the year if it is not the
// current one).
function day(ms) {
    var date = new Date(ms)
    var days = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86400000)
    //% "Today"
    if (days === 0) return qsTrId("threesea-la-today")
    //% "Yesterday"
    if (days === 1) return qsTrId("threesea-la-yesterday")
    if (days > 1 && days < 7) return Qt.formatDate(date, "dddd")
    var pattern = Qt.locale().dateFormat()
    if (date.getFullYear() === new Date().getFullYear()) pattern = withoutYear(pattern)
    return Qt.formatDate(date, pattern)
}

// A language's long date pattern without its year: "dddd, d. MMMM yyyy" gives "dddd, d. MMMM". Silica's
// Formatter.DateFullWithoutYear doesn't follow the language ("Montag, 14 Oktober").
function withoutYear(pattern) {
    return pattern.replace(/^y+(?:[^dM']|'[^']*')*/, "")
                  .replace(/(?:[\s,.\/-]|'[^']*')*y+(?:[\s.]|'[^']*')*/, "")
}

// What the app lock's row says. A PIN without "Lock the app" only guards the private chats: the app
// itself opens without asking then (lock: lockStatus's, or null).
function lockValue(lock) {
    //% "Off"
    if (!lock) return qsTrId("threesea-me-lock-off")
    if (lock.appLock) {
        return lock.keyboard
                //% "Passphrase"
                ? qsTrId("threesea-la-lock-passphrase")
                //% "PIN"
                : qsTrId("threesea-la-lock-pin")
    }
    return lock.keyboard
            //% "Passphrase for private chats"
            ? qsTrId("threesea-la-lock-private-passphrase")
            //% "PIN for private chats"
            : qsTrId("threesea-la-lock-private-pin")
}

// A call in the chat: "call|result|outgoing|seconds" (core/src/app_calls.rs).
function callStatus(result, outgoing, seconds) {
    switch (result) {
    case "finished":
        var length = Math.floor(seconds / 60) + ":" + ("0" + (seconds % 60)).slice(-2)
        return outgoing
                //% "Outgoing call, %1"
                ? qsTrId("threesea-la-status-call-outgoing").arg(length)
                //% "Incoming call, %1"
                : qsTrId("threesea-la-status-call-incoming").arg(length)
    case "missed":
        //% "Missed call"
        return qsTrId("threesea-la-status-call-missed")
    case "rejected":
        return outgoing
                //% "Call declined"
                ? qsTrId("threesea-la-status-call-declined")
                //% "You declined the call"
                : qsTrId("threesea-la-status-call-you-declined")
    case "busy":
        //% "Call not answered: busy"
        return qsTrId("threesea-la-status-call-busy")
    case "unanswered":
        //% "Call not answered"
        return qsTrId("threesea-la-status-call-unanswered")
    case "disabled":
        //% "Call not answered: calls are off"
        return qsTrId("threesea-la-status-call-disabled")
    case "aborted":
        //% "Call cancelled"
        return qsTrId("threesea-la-status-call-cancelled")
    default:
        //% "Call failed"
        return qsTrId("threesea-la-status-call-failed")
    }
}

// Text of a status line (core/src/connection.rs status codes).
function status(code) {
    //% "You created this distribution list."
    if (code === "distributionListCreated") return qsTrId("threesea-la-status-list-created")
    // Group status lines carry an argument: "groupMemberLeft|Name".
    var parts = String(code).split("|")
    var arg = parts.slice(1).join("|")
    if (parts[0] === "call") return callStatus(parts[1], parts[2] === "1", parseInt(parts[3], 10) || 0)
    switch (parts[0]) {
    case "groupCreated":
        //% "You created this group."
        return qsTrId("threesea-la-status-group-created")
    case "groupJoined":
        //% "You were added to this group."
        return qsTrId("threesea-la-status-group-joined")
    case "groupRemoved":
        //% "You were removed from this group."
        return qsTrId("threesea-la-status-group-removed")
    case "groupNoLongerMember":
        //% "You are no longer a member of this group."
        return qsTrId("threesea-la-status-group-no-longer-member")
    case "groupDisbanded":
        //% "This group was dissolved."
        return qsTrId("threesea-la-status-group-disbanded")
    case "groupLeft":
        //% "You left this group."
        return qsTrId("threesea-la-status-group-left")
    case "groupRenamed":
        //% "Group renamed to %1."
        return qsTrId("threesea-la-status-group-renamed").arg(arg)
    case "groupMemberAdded":
        //% "%1 was added."
        return qsTrId("threesea-la-status-group-member-added").arg(arg)
    case "groupMemberRemoved":
        //% "%1 was removed."
        return qsTrId("threesea-la-status-group-member-removed").arg(arg)
    case "groupMemberLeft":
        //% "%1 left the group."
        return qsTrId("threesea-la-status-group-member-left").arg(arg)
    case "groupCallStarted":
        // A locked private chat's line has lost the name, and notifications without preview ask
        // for it without.
        //% "A group call started."
        if (arg.length === 0) return qsTrId("threesea-la-status-group-call-started-unnamed")
        //% "%1 started a group call."
        return qsTrId("threesea-la-status-group-call-started").arg(arg)
    case "groupCallYouStarted":
        //% "You started a group call."
        return qsTrId("threesea-la-status-group-call-you-started")
    case "groupCallEnded":
        //% "The group call ended."
        return qsTrId("threesea-la-status-group-call-ended")
    case "fsEstablished":
        //% "Perfect Forward Secrecy is active for this chat."
        return qsTrId("threesea-la-status-fs-established")
    case "fsReset":
        //% "Perfect Forward Secrecy was reset. This can happen when the contact changes devices or reinstalls the app."
        return qsTrId("threesea-la-status-fs-session-reset")
    case "fsMessageRejected":
        //% "Perfect Forward Secrecy was reset. Affected messages are sent again."
        return qsTrId("threesea-la-status-fs-message-rejected")
    case "fsMessageWithoutFs":
        //% "A message without Perfect Forward Secrecy was received. Check with the contact that they turned it off on purpose."
        return qsTrId("threesea-la-status-fs-message-without")
    case "fsUnavailable":
        //% "Perfect Forward Secrecy is off in this chat because this contact's app is not using it at the moment."
        return qsTrId("threesea-la-status-fs-unavailable")
    case "fsTerminated":
        // Only in older chats: new ones say why (fsReset, fsUnavailable).
        //% "The Perfect Forward Secrecy session with this contact ended."
        return qsTrId("threesea-la-status-fs-session-ended")
    default:
        return code
    }
}

// One-line preview of a message for lists and notifications.
// "@[ID]" in group texts as "@Name" (`mentions` from the core: ID → name, "" for us).
function withMentions(text, mentions) {
    if (!text || text.indexOf("@[") < 0) return text || ""
    return text.replace(/@\[([0-9A-Z*@]{8})\]/g, function(tag, identity) {
        //% "All"
        if (identity === "@@@@@@@@") return "@" + qsTrId("threesea-la-mention-all")
        var name = mentions ? mentions[identity] : undefined
        //% "Me"
        if (name === "") return "@" + qsTrId("threesea-la-mention-me")
        return "@" + (name || identity)
    })
}

// The text a message offers for copying or sharing, "" if it has none: its text, a file's caption
// or a place's coordinates.
function copyText(kind, text, body, mentions) {
    if (kind === "text") return withMentions(text, mentions)
    if (kind === "location" && body) {
        return Number(body.latitude).toFixed(6) + ", " + Number(body.longitude).toFixed(6)
    }
    if (kind === "file" && body && body.caption) return withMentions(body.caption, mentions)
    return ""
}

// What a selection of messages can do as one, by Android's rules: each action only if every message
// qualifies, and only one message's text is shared.
function selectionActions(messages) {
    var copy = messages.length > 0
    var save = messages.length > 0
    for (var i = 0; i < messages.length; ++i) {
        var m = messages[i]
        if (copyText(m.kind, m.text, m.body, m.mentions).length === 0) copy = false
        if (!(m.kind === "file" && !!m.body && m.body.fileState === 2)) save = false
    }
    return { copy: copy, save: save, shareText: messages.length === 1 && copy }
}

// How a tapped download stands, by the file's state in a change of its message. The thumbnail starts
// first, so a change can still show the file's old state: nothing counts until it has been seen
// running. `running`: it has been. Gives "waiting", "running", "failed" or "ended".
function downloadStep(running, fileState) {
    if (fileState === 1) return "running"
    if (fileState === 2) return "ended"
    if (!running) return "waiting"
    return fileState === 3 ? "failed" : "ended"
}

function escapeHtml(text) {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;")
}

// The @[ID] tags in already linkified (escaped) text as bold "@Name" in `color`.
function markMentions(html, mentions, color) {
    return html.replace(/@\[([0-9A-Z*@]{8})\]/g, function(tag) {
        return "<b><font color=\"" + color + "\">" + escapeHtml(withMentions(tag, mentions)) + "</font></b>"
    })
}

// "49.61123, 6.13291" (and "± 12 m" when known), like a maps app's coordinates.
function coordinates(location) {
    if (!location || location.latitude === undefined) return ""
    var text = Number(location.latitude).toFixed(5) + ", " + Number(location.longitude).toFixed(5)
    if (location.accuracy) text += " \u00b1 " + Math.round(location.accuracy) + " m"
    return text
}

// Whether a position's time (a Date) is recent enough to say where the device is now. Without a
// valid time it isn't: Qt's own QGeoPositionInfo counts as invalid then.
function isRecentFix(timestamp, now) {
    if (!timestamp) return false
    var taken = new Date(timestamp).getTime()
    return !isNaN(taken) && now - taken <= 60000
}

// A distance in metres, as a maps app gives it: "120 m", "1.2 km".
function distance(metres, decimalPoint) {
    if (metres < 1000) return Math.round(metres) + " m"
    var km = metres / 1000
    return (km < 100 ? km.toFixed(1).replace(".", decimalPoint) : String(Math.round(km))) + " km"
}

// A local path as a file: URL. Names come from senders and from the device's files: #, ? and % in
// them are part of the name, and only encoded do they stay that (as a query or a fragment the rest
// of the name would be lost, and %2F would lead to another folder). Some paths come as URLs already.
function fileUrl(path) {
    path = String(path)
    if (path.indexOf("file://") === 0) return path
    return "file://" + path.split("/").map(encodeURIComponent).join("/")
}

// A geo: link (RFC 5870) for maps apps.
function geoUri(location) {
    return "geo:" + location.latitude + "," + location.longitude
}

// What of a long one-line `text` to show for a match of `query`, in a space of `limit` characters:
// from a little before the match if it would be too far along. Cut before anything is drawn, as
// Qt can't elide text with pictures in it (`previewLine`), and so that the cut keeps the match.
function matchWindow(text, query, limit) {
    if (!query) return text
    var first = text.toLowerCase().indexOf(query.toLowerCase())
    if (first <= 60 && first + query.length + 8 <= limit) return text
    // With a long query less of what is before it, so that all of the query fits.
    var before = Math.min(40, Math.floor(limit / 3), Math.max(0, limit - query.length - 9))
    var start = first - before
    if (first < 0 || start <= 0) return text
    // Not inside an emoji: a joined one, a skin tone, a flag's pair, or a pair of units. Enough to
    // see the longest sequence that could reach over the cut.
    var offset = 0
    var boundary = 0
    var step = function(units) {
        if (offset <= start) boundary = offset
        offset += units.length
    }
    scanEmoji(text.substr(0, start + 32), step, step)
    if (offset <= start) boundary = offset
    return "\u2026" + text.substring(boundary)
}

// Plain `text` as StyledText with each (case-insensitive) occurrence of `query` in `color`,
// everything HTML-escaped.
function highlightMatch(text, query, color) {
    text = text || ""
    if (!query) return escapeHtml(text)
    var lower = text.toLowerCase()
    var needle = query.toLowerCase()
    var html = ""
    var position = 0
    var index
    while ((index = lower.indexOf(needle, position)) >= 0) {
        html += escapeHtml(text.substring(position, index))
              + "<font color=\"" + color + "\">" + escapeHtml(text.substring(index, index + needle.length)) + "</font>"
        position = index + needle.length
    }
    return html + escapeHtml(text.substring(position))
}

// ---- Threema's text formatting: *bold*, _italic_, ~strikethrough~ (Android's MarkupParser, whose
// test cases are in tests/format). The stored text keeps its marks, whatever shows it. ----

// Cheap check before the real work: the same mark twice on a line.
function mayHaveMarkup(text) {
    return /([*_~])[^\n]*\1/.test(text || "")
}

var markupBoundary = /[ \t\n\x0B\f\r.,!?¡¿‽⸮;:&(){}\[\]⟨⟩‹›«»'"‘’“”*~\-_…⋯᠁]/
var markupUrlChar = /[a-zA-Z0-9\-._~:\/?#\[\]@!$&'()*+,;=%]/
var markupLetter = /[a-zA-Z]/
// Stands for what isn't text of its own: a link with everything in it, a mention. A word's part,
// for the marks around it.
var markupObject = "\uFFFC"
var markupEntities = { "&lt;": "<", "&gt;": ">", "&amp;": "&", "&quot;": "\"", "&#39;": "'", "&apos;": "'" }
var markupTags = { "*": "b", "_": "i", "~": "s" }

// Is a URL starting at `i`? Letters, then "://". Only the first of the letters needs to look.
function markupUrlStart(chars, i) {
    if (!markupLetter.test(chars[i]) || (i > 0 && markupLetter.test(chars[i - 1]))) return false
    var end = i
    while (end < chars.length && markupLetter.test(chars[end])) ++end
    return chars[end] === ":" && chars[end + 1] === "/" && chars[end + 2] === "/"
}

// The marks that format something in `chars` (one string per unit: the character it shows as):
// [{open, close, mark}] as indices. Spans don't overlap: each lies within another or apart from it.
function markupSpans(chars) {
    var tokens = []
    var inUrl = false
    var i
    function text() {
        if (tokens.length === 0 || tokens[tokens.length - 1].mark) tokens.push({ mark: "" })
    }
    function boundary(at) {
        return at < 0 || at >= chars.length || markupBoundary.test(chars[at])
    }
    for (i = 0; i < chars.length; ++i) {
        var c = chars[i]
        if (!inUrl) inUrl = markupUrlStart(chars, i)
        if (inUrl) {
            if (i + 1 >= chars.length || !markupUrlChar.test(chars[i + 1])) inUrl = false
            text()
        } else if (markupTags[c] && (boundary(i - 1) || boundary(i + 1))) {
            tokens.push({ mark: c, at: i })
        } else if (c === "\n") {
            tokens.push({ mark: "\n" })
        } else {
            text()
        }
    }

    // A mark looks for its partner on the stack; what lies between counts as text, also marks of
    // other kinds. After a line's end nothing looks further back than the line.
    var spans = []
    var stack = []
    var present = {}
    for (i = 0; i < tokens.length; ++i) {
        var token = tokens[i]
        if (token.mark === "") {
            stack.push(token)
        } else if (token.mark === "\n") {
            present = {}
        } else if (!present[token.mark]) {
            stack.push(token)
            present[token.mark] = true
        } else {
            var inner = 0
            while (true) {
                var top = stack.pop()
                if (!top) return []
                if (top.mark === token.mark) {
                    if (inner > 0) spans.push({ open: top.at, close: token.at, mark: token.mark })
                    stack.push({ mark: "" })
                    present[token.mark] = false
                    break
                }
                ++inner
                if (top.mark) present[top.mark] = false
            }
        }
    }
    return spans
}

// The units' sources, with `tag(mark, closing)` in place of the marks that format.
function markupRender(units, chars, tag) {
    var spans = markupSpans(chars)
    if (spans.length === 0) return units.join("")
    var replaced = {}
    for (var i = 0; i < spans.length; ++i) {
        replaced[spans[i].open] = tag(spans[i].mark, false)
        replaced[spans[i].close] = tag(spans[i].mark, true)
    }
    var out = ""
    for (var u = 0; u < units.length; ++u) out += replaced[u] !== undefined ? replaced[u] : units[u]
    return out
}

// The marks in LinkedLabel's linkified, escaped text as StyledText. Links and mention tags stay
// whole; an entity counts as the character it is.
function markMarkup(html) {
    if (!mayHaveMarkup(html)) return html
    var units = html.match(/<a\b[^>]*>[\s\S]*?<\/a>|<[^>]*>|&[a-zA-Z0-9#]+;|@\[[0-9A-Z*@]{8}\]|[\s\S]/g) || []
    var chars = units.map(function(unit) {
        if (unit.length === 1) return unit
        if (unit.charAt(0) === "&") return markupEntities[unit] || markupObject
        return /^<br\b/.test(unit) ? "\n" : markupObject
    })
    return markupRender(units, chars, function(mark, closing) {
        return "<" + (closing ? "/" : "") + markupTags[mark] + ">"
    })
}

// StyledText has no strikethrough in Qt 5.6 (tests/format), so a text with it is rich text.
function needsRichText(html) {
    return html.indexOf("<s>") >= 0
}

// Rich text takes no linkColor: links say it themselves.
function colourLinks(html, color) {
    return html.replace(/<a /g, "<a style=\"color:" + color + "\" ")
}

// Plain `text` as it reads without its formatting marks: previews, notifications.
function stripMarkup(text) {
    if (!mayHaveMarkup(text)) return text || ""
    var units = text.match(/@\[[0-9A-Z*@]{8}\]|[\s\S]/g) || []
    var chars = units.map(function(unit) { return unit.length === 1 ? unit : markupObject })
    return markupRender(units, chars, function() { return "" })
}

// A search hit's matching text: a message's own text without its marks; file and place names are
// names, whatever they look like.
function snippet(message) {
    var own = message.kind === "text" || (!!message.body && message.snippet === message.body.caption)
    return withMentions(own ? stripMarkup(message.snippet) : message.snippet, message.mentions)
}

function preview(message) {
    if (!message) return ""
    if (message.kind === "text") return withMentions(stripMarkup(message.text), message.mentions).replace(/\s+/g, " ")
    if (message.kind === "status") return status(message.body)
    //% "Location"
    if (message.kind === "location") return (message.body && message.body.name) || qsTrId("threesea-la-location")
    if (message.kind === "file") {
        var file = message.body || {}
        if (file.caption) return withMentions(stripMarkup(file.caption), message.mentions).replace(/\s+/g, " ")
        //% "Voice message"
        if (String(file.mediaType).indexOf("audio/") === 0 && file.rendering !== "file") return qsTrId("threesea-la-voice-message")
        if (String(file.mediaType).toLowerCase() === "image/gif" && file.rendering !== "file") return qsTrId("threesea-la-gif")
        //% "Photo"
        if (isPicture(file.mediaType) && file.rendering !== "file") return qsTrId("threesea-la-photo")
        //% "Video"
        if (String(file.mediaType).indexOf("video/") === 0 && file.rendering !== "file") return qsTrId("threesea-la-video")
        //% "File"
        return file.name || qsTrId("threesea-la-file")
    }
    //% "Poll: %1"
    if (message.kind === "poll") return qsTrId("threesea-la-poll-preview").arg((message.body || {}).description || "")
    //% "Poll closed: %1"
    if (message.kind === "pollClosed") return qsTrId("threesea-la-poll-closed-preview").arg((message.body || {}).description || "")
    //% "Message deleted"
    if (message.kind === "deleted") return qsTrId("threesea-la-message-deleted")
    return ""
}

// Translucency as measured on a device; the emulator squares alpha, so compensate there.
function alpha(value, emulator) {
    return emulator ? Math.sqrt(value) : value
}

// Edit and delete for everyone are offered for 6 hours (Threema spec, sender side).
var EDIT_DELETE_MAX_AGE_MS = 6 * 60 * 60 * 1000

// Reactions grouped by emoji in order of first use: [{emoji, count, mine}].
function reactionGroups(reactions) {
    var groups = []
    var byEmoji = {}
    for (var i = 0; reactions && i < reactions.length; ++i) {
        var reaction = reactions[i]
        var group = byEmoji[reaction.emoji]
        if (!group) {
            group = { emoji: reaction.emoji, count: 0, mine: false }
            byEmoji[reaction.emoji] = group
            groups.push(group)
        }
        group.count++
        if (reaction.sender === "") group.mine = true
    }
    return groups
}

// Who reacted with what: [{emoji, senders}], `sender` as in the reactions ("" for us, first).
function reactionSenders(reactions) {
    var groups = []
    var byEmoji = {}
    for (var i = 0; reactions && i < reactions.length; ++i) {
        var reaction = reactions[i]
        var group = byEmoji[reaction.emoji]
        if (!group) {
            group = { emoji: reaction.emoji, senders: [] }
            byEmoji[reaction.emoji] = group
            groups.push(group)
        }
        if (reaction.sender === "") group.senders.unshift("")
        else group.senders.push(reaction.sender)
    }
    return groups
}

// Did we react with this emoji?
function reactedWith(reactions, emoji) {
    for (var i = 0; reactions && i < reactions.length; ++i) {
        if (reactions[i].sender === "" && reactions[i].emoji === emoji) return true
    }
    return false
}

// Whether a picture of this media type shows in the app (bridge/peerimage.h decodes these and no
// others); anything else is a file like any other, as on Android.
function isPicture(mediaType) {
    return /^image\/(jpeg|jpg|png|gif|webp)\s*(;|$)/i.test(String(mediaType))
}

// Theme icon for a file by the core's checked `fileType` (a download icon while it is not on the
// device).
function fileIcon(fileType, state) {
    if (state === 0) return "icon-m-cloud-download"
    if (state === 3) return "icon-m-refresh"
    if (fileType === "image") return "icon-m-file-image"
    if (fileType === "audio") return "icon-m-file-audio"
    if (fileType === "video") return "icon-m-file-video"
    if (fileType === "pdf") return "icon-m-file-pdf"
    if (fileType === "archive") return "icon-m-file-archive-folder"
    if (fileType === "document") return "icon-m-file-document"
    if (fileType === "contact") return "icon-m-file-vcard"
    return "icon-m-file-other"
}

// "0:07", "12:34" for a duration in seconds.
function duration(seconds) {
    var total = Math.max(0, Math.round(seconds || 0))
    var s = total % 60
    return Math.floor(total / 60) + ":" + (s < 10 ? "0" : "") + s
}


// How many emoji `text` consists of, or 0 if it has anything else (Android's "bigger single
// emojis": up to 3 are shown large). Skin tones, variation selectors, joiners (family, flags of
// regions) and keycap marks belong to the emoji before them; a regional indicator pair is one
// flag.
function emojiOnlyCount(text) {
    if (!text) return 0
    var count = 0
    var joined = false
    var pendingRegional = false
    for (var i = 0; i < text.length; ++i) {
        var c = text.charCodeAt(i)
        if (c >= 0xD800 && c <= 0xDBFF && i + 1 < text.length) {
            c = (c - 0xD800) * 0x400 + (text.charCodeAt(i + 1) - 0xDC00) + 0x10000
            ++i
        }
        if (c === 0x20 || c === 0x0A) { joined = false; pendingRegional = false; continue }
        if (c === 0x200D) { joined = true; continue }
        if (c === 0xFE0F || c === 0xFE0E || c === 0x20E3 || (c >= 0x1F3FB && c <= 0x1F3FF)
                || (c >= 0xE0020 && c <= 0xE007F)) continue
        if (c >= 0x1F1E6 && c <= 0x1F1FF) {
            if (pendingRegional) { pendingRegional = false; continue }
            pendingRegional = true
            ++count
            continue
        }
        var emoji = (c >= 0x1F000 && c <= 0x1FAFF) || (c >= 0x2600 && c <= 0x27BF)
                || (c >= 0x2300 && c <= 0x23FF) || (c >= 0x2B00 && c <= 0x2BFF)
                || (c >= 0x2190 && c <= 0x21FF) || (c >= 0x25A0 && c <= 0x25FF)
                || c === 0x00A9 || c === 0x00AE || c === 0x203C || c === 0x2049 || c === 0x2122
                || c === 0x2139 || c === 0x24C2 || c === 0x2934 || c === 0x2935 || c === 0x3030
                || c === 0x303D || c === 0x3297 || c === 0x3299
        if (!emoji) return 0
        if (!joined) ++count
        joined = false
    }
    return count
}

// Whether `text` may contain an emoji (cheap pre-check before markEmoji).
function mayHaveEmoji(text) {
    return /[©®‼-㊙]|[\ud83c-\ud83e][\udc00-\udfff]|⃣/.test(text || "")
}

// The emoji in already escaped/linkified HTML as Threema's (Twemoji) pictures, `size` pixels high,
// via image://emoji (Settings → Appearance → Emoji style). Tags are left alone.
function markEmoji(html, size) {
    if (!mayHaveEmoji(html)) return html
    var parts = html.split(/(<[^>]*>)/)
    for (var p = 0; p < parts.length; ++p) {
        if (p % 2 === 1 || !mayHaveEmoji(parts[p])) continue
        parts[p] = replaceEmoji(parts[p], size)
    }
    return parts.join("")
}

// A chat list's preview line: the message as one flowing line (newlines and runs of spaces
// become single spaces, as Jolla's lists show them), cut with an ellipsis when it is longer
// than `limit` characters. We cut it ourselves because Qt cannot elide text that has pictures
// in it — with `elide` the emoji end up a line away from where they belong (user 2026-10-30).
// `limit` comes from the label's width and font, so it follows the device. With `keep` (a search's
// query) the cut doesn't go back to a space before the end of its first match.
function previewLine(text, limit, keep) {
    var one = (text || "").replace(/\s+/g, " ").trim()
    if (one.length <= limit) return one
    // Measured in character widths, not in code units: an emoji is about two characters wide
    // however long its sequence is (a waving man with a skin tone and a gender is seven units).
    var out = ""
    var width = 0
    var lastSpace = -1
    var full = true
    scanEmoji(one.substr(0, limit * 8), function(unit) {
        if (width >= limit) { full = false; return }
        if (unit === " ") lastSpace = out.length
        out += unit
        width += 1
    }, function(units) {
        if (width >= limit) { full = false; return }
        out += units
        width += 2
    })
    if (full && out.length >= one.length) return one
    // Back to the last word, unless that would throw most of the line away.
    var kept = keep ? one.toLowerCase().indexOf(keep.toLowerCase()) : -1
    var keepUntil = kept >= 0 ? kept + keep.length : 0
    if (lastSpace > limit / 3 && lastSpace >= keepUntil) out = out.substr(0, lastSpace)
    return out + "…"
}

// The emoji in plain `text`: `[{start, length, cell}]` in UTF-16 units (`cell`: the
// image://emoji cell), for drawing Threema's pictures over an editor's text.
function emojiRanges(text) {
    var ranges = []
    if (!mayHaveEmoji(text)) return ranges
    var offset = 0
    scanEmoji(text, function(unit) { offset += unit.length }, function(units, cell) {
        ranges.push({ start: offset, length: units.length, cell: cell })
        offset += units.length
    })
    return ranges
}

// Walks `text`, calling `plain(unit)` for each other UTF-16 unit or pair, `emoji(units, cell)`
// for each emoji.
function scanEmoji(text, plain, emoji) {
    // Most texts: nothing to look up, and no need for the table at all.
    var candidates = mayHaveEmoji(text)
    var codepoints = []
    var units = []
    for (var i = 0; i < text.length; ++i) {
        var c = text.charCodeAt(i)
        if (c >= 0xD800 && c <= 0xDBFF && i + 1 < text.length) {
            codepoints.push((c - 0xD800) * 0x400 + (text.charCodeAt(i + 1) - 0xDC00) + 0x10000)
            units.push(text.substr(i, 2))
            ++i
        } else {
            codepoints.push(c)
            units.push(text.charAt(i))
        }
    }
    var n = 0
    while (n < codepoints.length) {
        var first = codepoints[n]
        var found = null
        var length = 0
        if (candidates && EmojiTable.mayStart(first)) {
            var table = EmojiTable.table()
            var key = ""
            // The longest sequence that is an emoji. No longer one can start with what isn't in
            // the table at all.
            for (var k = 0; n + k < codepoints.length; ++k) {
                key += (k > 0 ? "-" : "") + codepoints[n + k].toString(16)
                var cell = table[key]
                if (cell === undefined) break
                if (cell) { found = cell; length = k + 1 }
            }
        }
        if (found) {
            emoji(units.slice(n, n + length).join(""), found)
            n += length
        } else {
            plain(units[n])
            ++n
        }
    }
}

function replaceEmoji(text, size) {
    var out = ""
    scanEmoji(text, function(unit) { out += unit }, function(units, cell) {
        out += "<img src=\"image://emoji/" + cell + "\" width=\"" + size + "\" height=\"" + size + "\">"
    })
    return out
}

