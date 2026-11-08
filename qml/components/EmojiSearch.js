// SPDX-License-Identifier: AGPL-3.0-only
// Finding an emoji by a word, like Android's EmojiSearchIndex: a word's beginning against the
// search terms in the user's language (emojis/search/<language>.csv, one emoji per line, "|"
// between it and its terms; from Threema Android, see scripts/make-emoji-table.py). Results come in
// the picker's order, as the picker's items. The file is read at the first search.
.pragma library
.import "EmojiPickerData.js" as EmojiData

var languages = ["be", "bg", "ca", "cs", "de", "en", "es", "fr", "gsw", "hu", "it", "nl", "no", "pl", "pt",
                 "ru", "sk", "tr", "uk"]
var loaded = ""
var entries = []

// The file for a locale's name ("de_CH"), English if there is none.
function languageOf(locale) {
    var language = String(locale).split(/[_-]/)[0].toLowerCase()
    if (language === "nb" || language === "nn") language = "no"
    return languages.indexOf(language) >= 0 ? language : "en"
}

// A variation selector here or there is the same emoji.
function plainEmoji(text) {
    return text.replace(/️/g, "")
}

function build(csv) {
    var terms = {}
    var lines = csv.split("\n")
    var i
    for (i = 0; i < lines.length; ++i) {
        var bar = lines[i].indexOf("|")
        if (bar > 0) terms[plainEmoji(lines[i].substring(0, bar))] = lines[i].substring(bar).toLowerCase()
    }
    entries = []
    for (var category = 0; category < EmojiData.names.length; ++category) {
        var items = EmojiData.items(category)
        for (i = 0; i < items.length; ++i) {
            var found = terms[plainEmoji(items[i][0])]
            if (found) entries.push({ item: items[i], terms: found })
        }
    }
}

// Reads the terms for `locale` from `directory` (a URL ending in "/"); done(true) when they are
// there, done(false) if they can't be read.
function load(directory, locale, done) {
    var language = languageOf(locale)
    if (loaded === language) {
        done(true)
        return
    }
    var request = new XMLHttpRequest()
    request.onreadystatechange = function() {
        if (request.readyState !== XMLHttpRequest.DONE) return
        var ok = request.responseText.length > 0
        if (ok) {
            build(request.responseText)
            loaded = language
        }
        done(ok)
    }
    request.open("GET", directory + language + ".csv")
    request.send()
}

// The picker's items whose terms have a word or phrase beginning with `query`.
function search(query) {
    var needle = String(query).trim().toLowerCase()
    if (needle.length === 0) return []
    needle = "|" + needle
    var found = []
    for (var i = 0; i < entries.length; ++i) {
        if (entries[i].terms.indexOf(needle) >= 0) found.push(entries[i].item)
    }
    return found
}
