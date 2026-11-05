// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica.private 1.0
import "../../qml/components/EmojiSearch.js" as EmojiSearch
import "../../qml/components/Format.js" as Fmt

QtObject {
    id: root
    property var failed: []
    property int count: 0
    property bool done: false

    function check(what, got, expected) {
        ++count
        if (got !== expected) {
            var list = failed
            list.push(what + ": got [" + got + "] expected [" + expected + "]")
            failed = list
        }
    }

    // As the bubble gets a plain text from LinkedLabel when it holds no link.
    function html(plain) {
        return Fmt.escapeHtml(plain).replace(/\n/g, "<br>")
    }

    // [input, the same with b, i, s around what is formatted]. Android's MarkupParserTest,
    // case by case, then what only happens here.
    readonly property var markupCases: [
        ["hello *there*", "hello <b>there</b>"],
        ["hello _there_", "hello <i>there</i>"],
        ["hello ~there~", "hello <s>there</s>"],
        ["two *bold* *parts*", "two <b>bold</b> <b>parts</b>"],
        ["two _italic_ _bits_", "two <i>italic</i> <i>bits</i>"],
        ["two ~striked~ ~through~", "two <s>striked</s> <s>through</s>"],
        ["*bold* and _italic_", "<b>bold</b> and <i>italic</i>"],
        ["*bold with _italic_*", "<b>bold with <i>italic</i></b>"],
        ["(*bold*)", "(<b>bold</b>)"],
        ["¡*Threema* es fantástico!", "¡<b>Threema</b> es fantástico!"],
        ["«_great_ service»", "«<i>great</i> service»"],
        ["\"_great_ service\"", "&quot;<i>great</i> service&quot;"],
        ["*bold*…", "<b>bold</b>…"],
        ["_<a href=\"https://threema.com\">Threema</a>_",
         "<i>&lt;a href=&quot;https://threema.com&quot;&gt;Threema&lt;/a&gt;</i>"],
        ["so not_really_italic", "so not_really_italic"],
        ["invalid*bold*stuff", "invalid*bold*stuff"],
        ["no~strike~through", "no~strike~through"],
        ["<_< >_>", "&lt;_&lt; &gt;_&gt;"],
        ["<a href=\"https://threema.com\">_Threema_</a>",
         "&lt;a href=&quot;https://threema.com&quot;&gt;_Threema_&lt;/a&gt;"],
        ["*bold_but_no~strike~through*", "<b>bold_but_no~strike~through</b>"],
        ["https://example.com/_output_/", "https://example.com/_output_/"],
        ["https://example.com/*output*/", "https://example.com/*output*/"],
        ["https://example.com?__twitter_impression=true", "https://example.com?__twitter_impression=true"],
        ["https://example.com?_twitter_impression=true", "https://example.com?_twitter_impression=true"],
        ["https://en.wikipedia.org/wiki/Java_class_file *nice*", "https://en.wikipedia.org/wiki/Java_class_file <b>nice</b>"],
        ["https://example.com/image_-_1.jpg", "https://example.com/image_-_1.jpg"],
        ["*invalid markup (do not parse)_", "*invalid markup (do not parse)_"],
        ["random *asterisk", "random *asterisk"],
        ["*First line\n and a new one. (do not parse)*", "*First line<br> and a new one. (do not parse)*"],
        ["*\nbegins with linebreak. (do not parse)*", "*<br>begins with linebreak. (do not parse)*"],
        ["*Just some text. But it ends with newline (do not parse)\n*", "*Just some text. But it ends with newline (do not parse)<br>*"],
        ["_*first line*\n*second* line_", "_<b>first line</b><br><b>second</b> line_"],
        // The finding's own example.
        ["*Important*: _tomorrow_ ~10~ 11 o'clock", "<b>Important</b>: <i>tomorrow</i> <s>10</s> 11 o'clock"],
        // Escaped: an entity's ";" is no boundary, what it stands for decides.
        ["x<*a*", "x&lt;*a*"],
        ["a & *b*", "a &amp; <b>b</b>"],
        ["&*b*", "&amp;<b>b</b>"],
        // Nothing between the marks, nothing formatted.
        ["**", "**"],
        ["~~ a", "~~ a"],
        // A URL ends where its line ends, whatever comes on the next one.
        ["https://example.com/_a_\n*b*", "https://example.com/_a_<br><b>b</b>"],
        // Emoji are characters like others.
        ["*😀*", "<b>😀</b>"],
    ]

    // Links and mentions as the bubble has them: whole, and a word's part for the marks around them.
    readonly property var linkedCases: [
        ["*see <a href=\"https://x.ch/a_b_c\">https://x.ch/a_b_c</a> now*",
         "<b>see <a href=\"https://x.ch/a_b_c\">https://x.ch/a_b_c</a> now</b>"],
        ["<a href=\"https://x.ch/_a_\">https://x.ch/_a_</a>", "<a href=\"https://x.ch/_a_\">https://x.ch/_a_</a>"],
        ["_<a href=\"https://x.ch\">x</a>_", "<i><a href=\"https://x.ch\">x</a></i>"],
        ["*@[ABCD1234]*", "<b>@[ABCD1234]</b>"],
        ["@[*ABCDEFG] *hi* @[*ABCDEFH]", "@[*ABCDEFG] <b>hi</b> @[*ABCDEFH]"],
        ["a<br>b_c_d", "a<br>b_c_d"],
    ]

    readonly property var stripCases: [
        ["*Important*: _tomorrow_ ~10~ 11", "Important: tomorrow 10 11"],
        ["a_b_c", "a_b_c"],
        ["x<*a* b*", "x<*a b"],
        ["*one\ntwo*", "*one\ntwo*"],
        ["_*first*\n*second* line_", "_first\nsecond line_"],
        ["https://example.com/_a_ *b*", "https://example.com/_a_ b"],
        ["@[*ABCDEFG] *hi* @[*ABCDEFH]", "@[*ABCDEFG] hi @[*ABCDEFH]"],
        ["", ""],
    ]

    function actionsOf(list) {
        var a = Fmt.selectionActions(list)
        return [a.copy, a.save, a.shareText].join()
    }

    function checkMessageActions() {
        check("copy, text", Fmt.copyText("text", "hi", null, null), "hi")
        check("copy, a mention", Fmt.copyText("text", "hi @[ABCD1234]", null, { ABCD1234: "Ann" }), "hi @Ann")
        check("copy, a caption", Fmt.copyText("file", "", { caption: "see @[ABCD1234]" }, { ABCD1234: "Ann" }), "see @Ann")
        check("copy, no caption", Fmt.copyText("file", "", { caption: "" }, null), "")
        check("copy, a place", Fmt.copyText("location", "", { latitude: 1.5, longitude: -2 }, null), "1.500000, -2.000000")
        check("copy, a poll", Fmt.copyText("poll", "", {}, null), "")
        check("copy, deleted", Fmt.copyText("deleted", "", null, null), "")

        var text = { kind: "text", text: "hi" }
        var photo = { kind: "file", body: { fileState: 2 } }
        var captioned = { kind: "file", body: { fileState: 2, caption: "hi" } }
        var away = { kind: "file", body: { fileState: 0, caption: "hi" } }
        var deleted = { kind: "deleted" }
        // copy, save, share text
        check("selection, nothing", actionsOf([]), "false,false,false")
        check("selection, a text", actionsOf([text]), "true,false,true")
        check("selection, texts", actionsOf([text, text]), "true,false,false")
        check("selection, a photo", actionsOf([photo]), "false,true,false")
        check("selection, photos", actionsOf([photo, captioned]), "false,true,false")
        check("selection, a captioned photo", actionsOf([captioned]), "true,true,true")
        check("selection, captioned photos", actionsOf([captioned, captioned]), "true,true,false")
        check("selection, a file not here", actionsOf([away]), "true,false,true")
        check("selection, one of them not here", actionsOf([photo, away]), "false,false,false")
        check("selection, text and photo", actionsOf([text, captioned]), "true,false,false")
        check("selection, deleted among them", actionsOf([text, deleted]), "false,false,false")

        // A failed file with its thumbnail gone: thumbnail first, the file's old state seen.
        check("download, thumbnail first, file still away", Fmt.downloadStep(false, 0), "waiting")
        check("download, thumbnail first, file failed before", Fmt.downloadStep(false, 3), "waiting")
        check("download, seen running", Fmt.downloadStep(false, 1), "running")
        check("download, still running", Fmt.downloadStep(true, 1), "running")
        check("download, failed", Fmt.downloadStep(true, 3), "failed")
        check("download, here", Fmt.downloadStep(true, 2), "ended")
        check("download, here without being seen", Fmt.downloadStep(false, 2), "ended")
        check("download, stopped", Fmt.downloadStep(true, 0), "ended")

        check("long date without year, German", Fmt.withoutYear("dddd, d. MMMM yyyy"), "dddd, d. MMMM")
        check("long date without year, English", Fmt.withoutYear("dddd, d MMMM yyyy"), "dddd, d MMMM")
        check("long date without year, American", Fmt.withoutYear("dddd, MMMM d, yyyy"), "dddd, MMMM d")
        check("long date without year, French", Fmt.withoutYear("dddd d MMMM yyyy"), "dddd d MMMM")
        check("long date without year, Spanish", Fmt.withoutYear("dddd, d 'de' MMMM 'de' yyyy"), "dddd, d 'de' MMMM")
        check("long date without year, Hungarian", Fmt.withoutYear("yyyy. MMMM d., dddd"), "MMMM d., dddd")
        check("long date without year, Japanese", Fmt.withoutYear("yyyy\u5e74M\u6708d\u65e5dddd"), "M\u6708d\u65e5dddd")
        check("long date without year, a word after it", Fmt.withoutYear("dddd, d MMMM yyyy '\u0433'."), "dddd, d MMMM")
        check("long date without year, year in the middle", Fmt.withoutYear("d MMMM yyyy, dddd"), "d MMMM, dddd")

        check("a day of an earlier year has its year", Fmt.day(new Date(2001, 0, 15, 12).getTime()).indexOf("2001") >= 0, true)

        check("preview, a GIF", Fmt.preview({ kind: "file", body: { mediaType: "image/gif", rendering: "media" } }), "threesea-la-gif")
        check("preview, a GIF as a file", Fmt.preview({ kind: "file", body: { mediaType: "image/gif", rendering: "file", name: "a.gif" } }) === "threesea-la-gif", false)
        check("preview, a photo", Fmt.preview({ kind: "file", body: { mediaType: "image/jpeg", rendering: "media" } }), "threesea-la-photo")

        var senders = Fmt.reactionSenders([{ emoji: "a", sender: "ABCD1234" }, { emoji: "b", sender: "" },
                                           { emoji: "a", sender: "" }, { emoji: "a", sender: "EFGH5678" }])
        check("who reacted, by emoji", senders.length, 2)
        check("who reacted, we first", senders[0].senders.join(), ",ABCD1234,EFGH5678")
        check("who reacted, nobody", Fmt.reactionSenders(null).length, 0)
    }

    Component.onCompleted: {
        checkMessageActions()
        var i
        var now = 1700000000000
        check("metres", Fmt.distance(120.4, "."), "120 m")
        check("kilometres", Fmt.distance(1234, ","), "1,2 km")
        check("far away", Fmt.distance(123456, "."), "123 km")
        check("fix now", Fmt.isRecentFix(new Date(now), now), true)
        check("fix a minute old", Fmt.isRecentFix(new Date(now - 60000), now), true)
        check("fix too old", Fmt.isRecentFix(new Date(now - 60001), now), false)
        check("fix hours old", Fmt.isRecentFix(new Date(now - 7200000), now), false)
        check("fix slightly ahead of the clock", Fmt.isRecentFix(new Date(now + 2000), now), true)
        check("fix without time", Fmt.isRecentFix(new Date(NaN), now), false)
        check("fix with nothing", Fmt.isRecentFix(undefined, now), false)
        for (i = 0; i < markupCases.length; ++i) {
            check("mark " + markupCases[i][0], Fmt.markMarkup(html(markupCases[i][0])), markupCases[i][1])
        }
        for (i = 0; i < linkedCases.length; ++i) {
            check("linked " + linkedCases[i][0], Fmt.markMarkup(linkedCases[i][0]), linkedCases[i][1])
        }
        for (i = 0; i < stripCases.length; ++i) {
            check("strip " + stripCases[i][0], Fmt.stripMarkup(stripCases[i][0]), stripCases[i][1])
        }
        check("gate", Fmt.mayHaveMarkup("a_b"), false)
        check("gate", Fmt.mayHaveMarkup("a *b\nc* d"), false)
        check("gate", Fmt.mayHaveMarkup("a *b* d"), true)
        check("gate", Fmt.mayHaveMarkup(undefined), false)
        check("language", EmojiSearch.languageOf("de_CH"), "de")
        check("language", EmojiSearch.languageOf("nb_NO"), "no")
        check("language", EmojiSearch.languageOf("xx"), "en")

        // Previews lose the marks, and a name with marks in it stays as it is.
        check("preview", Fmt.preview({ kind: "text", text: "*Hi*  _you_\nthere" }), "Hi you there")
        check("preview", Fmt.preview({ kind: "text", text: "@[ABCD1234] *x*", mentions: { ABCD1234: "a_b_ *c*" } }),
              "@a_b_ *c* x")
        check("preview", Fmt.preview({ kind: "file", body: { caption: "~old~ new" } }), "old new")
        check("preview stays", Fmt.preview({ kind: "file", body: { name: "_a_.pdf", mediaType: "application/pdf" } }), "_a_.pdf")

        // Struck through goes through rich text, whose links need their colour.
        check("rich", Fmt.needsRichText("<b>a</b>"), false)
        check("rich", Fmt.needsRichText("<s>a</s>"), true)
        check("rich links", Fmt.colourLinks("x <a href=\"y\">y</a>", "#102030"), "x <a style=\"color:#102030\" href=\"y\">y</a>")

        // Search hits.
        check("snippet", Fmt.snippet({ kind: "text", snippet: "*a* @[ABCD1234]", mentions: { ABCD1234: "b_c_" } }), "a @b_c_")
        check("snippet", Fmt.snippet({ kind: "file", snippet: "~a~ b", body: { caption: "~a~ b", name: "c" } }), "a b")
        check("snippet", Fmt.snippet({ kind: "file", snippet: "_a_.pdf", body: { caption: "", name: "_a_.pdf" } }), "_a_.pdf")
        check("snippet", Fmt.snippet({ kind: "location", snippet: "_a_", body: { name: "_a_" } }), "_a_")

        // What the real link finder gives is what markMarkup expects. It takes a moment: until
        // then a bubble has the text without its links.
        for (i = 0; i < linkerCases.length; ++i) {
            var parser = linkParser.createObject(root, { text: linkerCases[i][0] })
            parsers.push(parser)
            // The text a bubble has until then counts as well.
            if (linkerCases[i][1] === null) {
                check("before the link finder " + linkerCases[i][0],
                      Fmt.markMarkup(parser.linkedText.replace(/\n/g, "<br>")).indexOf(linkerCases[i][2]), -1)
            }
        }
        linkerDone.start()
    }

    // [text, result with links as LINK, or (a third entry) what must not be in the result].
    readonly property var linkerCases: [
        // A mark or a bracket in front keeps the finder from it: the URL rule of the parser holds.
        ["*go* <b> & https://example.com/a_b_c *now*", "<b>go</b> &lt;b&gt; &amp; LINK <b>now</b>"],
        ["_https://example.com/a_b_c_ *now*", "_https://example.com/a_b_c_ <b>now</b>"],
        ["*https://example.com*", "*https://example.com*"],
        ["(https://example.com/_x_) ~a~", "(https://example.com/_x_) <s>a</s>"],
        ["*a* mail a_b_c@example.com *b*", "<b>a</b> mail LINK <b>b</b>"],
        // What comes in stays text.
        ["~a~ <img src=\"x\"> https://example.com/\"><img src=x> <s>", null, "<img"],
        // Written as an entity: stays what it reads as, also when something around it is formatted.
        ["~a~ &lt;img src=x&gt; *b*", null, "<img"],
        ["~a~ &amp;lt;img src=x&amp;gt; _c_", null, "<img"],
        ["*&lt;a href=x&gt;y&lt;/a&gt;*", null, "<a "],
        ["~a~ <a href=\"x\">y</a> <font color=\"red\">z</font>", null, "<font"],
        ["*a <br> b* https://example.com/?a=<img>", null, "<img"],
    ]
    function searchIn(locale, then) {
        EmojiSearch.load(Qt.resolvedUrl("../../emojis/search/"), locale, function(ok) {
            root.check("search terms for " + locale, ok, true)
            then()
        })
    }
    function hasEmoji(items, emoji) {
        return items.some(function(item) { return item[0] === emoji })
    }
    property var parsers: []
    property Component linkParser: Component { LinkParser { } }
    property Timer linkerDone: Timer {
        interval: 20
        repeat: true
        onTriggered: {
            var i
            for (i = 0; i < root.parsers.length; ++i) if (root.parsers[i].parsing) return
            stop()
            for (i = 0; i < root.parsers.length; ++i) {
                var want = root.linkerCases[i]
                var got = Fmt.markMarkup(root.parsers[i].linkedText.replace(/\n/g, "<br>"))
                if (want[1] === null) root.check("link finder " + want[0], got.indexOf(want[2]), -1)
                else root.check("link finder " + want[0], got.replace(/<a [^>]*>[^<]*<\/a>/g, "LINK"), want[1])
            }
            root.searchIn("de_CH", function() {
                var heart = "\u2764\uFE0F"
                root.check("search, a word's beginning", root.hasEmoji(EmojiSearch.search("HERZ"), heart), true)
                root.check("search, not in English here", root.hasEmoji(EmojiSearch.search("heart"), heart), false)
                root.check("search, not a word's middle", EmojiSearch.search("xqzerz").length, 0)
                root.check("search, nothing asked", EmojiSearch.search("  ").length, 0)
                root.searchIn("xx_XX", function() {
                    var found = EmojiSearch.search("heart")
                    root.check("search, English by default", root.hasEmoji(found, heart), true)
                    root.check("search, cells of the picker", /^[a-z]+-[0-9]+,/.test(found[0][1]), true)
                    var thumbs = EmojiSearch.search("thumbs")
                    root.check("search, skin tones come along", thumbs.length > 0 && thumbs[0].length, 3)
                    root.done = true
                })
            })
        }
    }
}
