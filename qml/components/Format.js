// SPDX-License-Identifier: AGPL-3.0-only
.pragma library
.import "EmojiTable.js" as EmojiTable

// Clock time of a message (date sections carry the day), like Jolla Messages' lists.
function clock(ms, Format, Formatter) {
    return Format.formatDate(new Date(ms), Formatter.TimeValue)
}

// Time shown in a message bubble, like Jolla Messages: clock time today, "Tuesday 20:42"
// within the last week, otherwise "1 Sep 10:12".
function time(ms, Format, Formatter) {
    var date = new Date(ms)
    var clockText = Format.formatDate(date, Formatter.TimeValue)
    var days = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86400000)
    if (days === 0) return clockText
    if (days > 0 && days < 7) return Qt.formatDate(date, "dddd") + " " + clockText
    return Format.formatDate(date, Formatter.DateMediumWithoutYear) + " " + clockText
}

function startOfDay(date) {
    return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
}

// Day of a message for section headers: "Today", "Yesterday", a weekday within the last week,
// otherwise "Wednesday, 14 October" (with the year if it is not the current one).
function day(ms) {
    var date = new Date(ms)
    var days = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86400000)
    //% "Today"
    if (days === 0) return qsTrId("threesea-la-today")
    //% "Yesterday"
    if (days === 1) return qsTrId("threesea-la-yesterday")
    if (days > 1 && days < 7) return Qt.formatDate(date, "dddd")
    if (date.getFullYear() === new Date().getFullYear()) return Qt.formatDate(date, "dddd, d MMMM")
    return Qt.formatDate(date, "dddd, d MMMM yyyy")
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
    case "fsEstablished":
        //% "Forward secrecy is active for this chat."
        return qsTrId("threesea-la-status-fs-established")
    case "fsReset":
        //% "Forward secrecy was reset. This can happen when the contact changes devices or reinstalls the app."
        return qsTrId("threesea-la-status-fs-session-reset")
    case "fsMessageRejected":
        //% "Forward secrecy was reset. Affected messages are sent again."
        return qsTrId("threesea-la-status-fs-message-rejected")
    case "fsUnavailable":
        //% "Forward secrecy is off in this chat because this contact's app is not using it at the moment."
        return qsTrId("threesea-la-status-fs-unavailable")
    case "fsTerminated":
        // Only in older chats: new ones say why (fsReset, fsUnavailable).
        //% "The forward secrecy session with this contact ended."
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

// A geo: link (RFC 5870) for maps apps.
function geoUri(location) {
    return "geo:" + location.latitude + "," + location.longitude
}

// Plain `text` as StyledText with each (case-insensitive) occurrence of `query` in `color`,
// everything HTML-escaped. Around the first match only a window is kept for long texts.
function highlightMatch(text, query, color) {
    text = text || ""
    if (!query) return escapeHtml(text)
    var lower = text.toLowerCase()
    var needle = query.toLowerCase()
    var first = lower.indexOf(needle)
    if (first > 60) {
        text = "\u2026" + text.substring(first - 40)
        lower = text.toLowerCase()
    }
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

function preview(message) {
    if (!message) return ""
    if (message.kind === "text") return withMentions(message.text, message.mentions).replace(/\s+/g, " ")
    if (message.kind === "status") return status(message.body)
    //% "Location"
    if (message.kind === "location") return (message.body && message.body.name) || qsTrId("threesea-la-location")
    if (message.kind === "file") {
        var file = message.body || {}
        if (file.caption) return withMentions(file.caption, message.mentions).replace(/\s+/g, " ")
        //% "Voice message"
        if (String(file.mediaType).indexOf("audio/") === 0 && file.rendering !== "file") return qsTrId("threesea-la-voice-message")
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
// `limit` comes from the label's width and font, so it follows the device.
function previewLine(text, limit) {
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
    if (lastSpace > limit / 3) out = out.substr(0, lastSpace)
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
        // Emoji start at these: pictographs, symbols, and the digits/#/* of keycaps.
        if (first >= 0xA9 || first === 0x23 || first === 0x2A || (first >= 0x30 && first <= 0x39)) {
            var key = ""
            for (var k = 0; k < EmojiTable.longest && n + k < codepoints.length; ++k) {
                key += (k > 0 ? "-" : "") + codepoints[n + k].toString(16)
                var cell = EmojiTable.table[key]
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

