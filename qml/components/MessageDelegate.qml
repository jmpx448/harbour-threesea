// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Format.js" as Fmt

// One message, laid out like Jolla Messages:
// - bubbles keep a small gap to their screen edge;
// - the inner bottom corner is always rounded, the top corners only on the first bubble of a run
//   from the same side, the outer bottom corner is square;
// - bubbles of a run sit close together, runs are further apart;
// - every message shows its time (grey) and, if ours, the delivery state: after a short one-line
//   text, else on a row of their own under the content;
// - a centred day header precedes the first message of each day (the oldest one's day is in the
//   page's fixed day band, which shows the day of the topmost visible message).
// Reactions (Threema) sit as small chips under the bubble; the context menu starts with a row of
// quick reactions.
ListItem {
    id: item

    property bool outgoing: model.outgoing
    // This message, for the parts in Loaders below: there `item` is the Loader's own property.
    readonly property Item message: item
    // The neighbour in this newest-first model (index + 1): only what the layout asks of it, and
    // as plain values, so a new row that leaves the neighbour as it was changes nothing further.
    property bool hasOlder: false
    property real olderCreatedAt: 0
    property bool olderOutgoing: false
    property string olderSender: ""
    property string olderKind: ""
    // The list is moving, by a flick or the quick scroll: what can wait for it, waits.
    property bool listMoving: false
    // The contact understands any emoji; otherwise only thumbs up/down (legacy reactions).
    property bool anyReaction: true
    // Distribution lists have no reactions at all, not even the thumbs.
    property bool canReact: true

    // The contact understands edits / deletions for everyone.
    property bool canEdit: false
    // Notes (a notes group): edited and deleted at any age, as on Android.
    property bool noAgeLimit: false
    property bool canDelete: false
    // Distribution lists: the quoted message would not be in the receivers' chats.
    property bool canQuote: true
    // A blocked contact, a revoked ID or a group we left: nothing goes out, so what would send
    // something isn't offered. Reactions already there stay as they are.
    property bool readOnly: false

    // Selecting messages (chat pulley → Select messages): taps select, no menu, and a selected
    // bubble stays lit like a pressed one.
    property bool selectionMode: false
    property bool selected: false
    signal selectToggled()
    // A jump (a quote, a search hit) landed here just now.
    property bool landed: false
    highlighted: down || menuOpen || selected || landed

    // Apply or withdraw one of our reactions.
    signal react(string emoji, bool apply)
    // Any other emoji: the chat's emoji panel picks it.
    signal moreReactionsRequested()
    signal editRequested()
    signal quoteRequested()
    // The quoted message (looked up by the chat), or null; tapping the quote shows it.
    property var quoted: null
    signal quotedRequested()
    readonly property bool hasQuote: model.kind === "text" && !!model.quoteId
    signal deleteRequested(bool everyone)
    // Tapped a file message: show it, or fetch it first.
    signal fileRequested()
    signal saveRequested()
    signal drawRequested()
    signal gifFavouriteRequested()
    signal forwardRequested()
    signal starToggled()
    signal detailsRequested()
    signal resendRequested()
    signal cancelTransferRequested()
    signal shareRequested()
    // Held a reaction: who reacted.
    signal reactionsRequested()
    signal locationRequested()
    signal pollRequested()
    signal closePollRequested()

    // A poll, or its closing with the results (a message of its own, like Android's).
    readonly property bool isPoll: model.kind === "poll" || isPollResults
    readonly property bool isPollResults: model.kind === "pollClosed"
    readonly property bool myOpenPoll: isPoll && model.body.creator === backend.identity && model.body.state !== 1

    readonly property bool isFile: model.kind === "file"
    readonly property bool isLocation: model.kind === "location"
    // The core sends `mentions` only when the text (or caption) has @[ID] tags.
    readonly property bool hasMentions: !!model.mentions && (model.kind === "text" || isFile)
    readonly property Item chatView: ListView.view
    readonly property string ownText: (model.kind === "text" ? model.text : isFile ? model.body.caption : "") || ""
    // Shown as StyledText (mentionLabel): mentions, *formatting*, or Threema's emoji pictures
    // (Settings → Appearance → Emoji style); otherwise the plain LinkedLabel.
    readonly property bool richText: hasMentions || Fmt.mayHaveMarkup(ownText)
                                     || (backend.threemaEmoji && Fmt.mayHaveEmoji(ownText))

    readonly property bool isDeleted: model.kind === "deleted"
    readonly property bool isEdited: (model.kind === "text" || isFile) && !!model.editedAt
    // Ours, sent and young enough; messages nobody can have yet are simply changed or removed. Once
    // its send has begun someone may have it, even while one member keeps it pending.
    readonly property bool unsent: outgoing && ((model.state === 0 && !model.writtenOut) || model.state === 4)
    // It didn't go out, for good: a tap or the menu sends it again.
    readonly property bool failed: outgoing && model.state === 4 && !isDeleted
    readonly property bool recent: noAgeLimit || Date.now() - model.createdAt < Fmt.EDIT_DELETE_MAX_AGE_MS

    // Inner Repeaters have their own `model`.
    readonly property var reactions: model.reactions
    readonly property var reactionGroups: Fmt.reactionGroups(reactions)

    readonly property bool isStatus: model.kind === "status"
    // Where the new messages started when the chat was opened: a line in the highlight
    // colour above this message.
    property bool showUnread: false
    readonly property real unreadGap: showUnread ? unreadLine.height + 2 * Theme.paddingMedium : 0
    readonly property string dayText: Fmt.day(model.createdAt)
    // The oldest message's day is in the page's day band already.
    readonly property bool showDay: hasOlder && Fmt.day(olderCreatedAt) !== dayText
    // Group chats: who wrote it (shown above the first bubble of a run); empty in 1:1 chats.
    property string senderName: ""
    readonly property bool firstOfRun: showDay || !hasOlder || olderKind === "status"
                                       || olderOutgoing !== outgoing
                                       || olderSender !== (model.sender || "")
    readonly property real maxTextWidth: width - Theme.itemSizeLarge - 2 * Theme.paddingMedium
    readonly property color textColor: highlighted || outgoing ? Theme.highlightColor : Theme.primaryColor
    readonly property real topGap: firstOfRun ? Theme.paddingLarge : Theme.paddingSmall

    contentHeight: topGap
                   + (showDay ? dayLabel.height + Theme.paddingLarge : 0)
                   + unreadGap
                   + (showSender ? senderLabel.height : 0)
                   + (isStatus ? statusLabel.height : bubble.height)
                   + (reactionGroups.length > 0 ? reactionRow.height + Theme.paddingSmall : 0)
    enabled: !isStatus

    // Tapping a reaction: withdraw ours, else add it. Legacy peers can't withdraw: a legacy
    // reaction replaces the previous one instead.
    function toggleReaction(emoji) {
        var mine = Fmt.reactedWith(reactions, emoji)
        if (mine && !anyReaction) return
        react(emoji, !mine)
    }

    onClicked: {
        if (selectionMode) selectToggled()
        else if (failed && !readOnly) resendRequested()
        else if (isFile) fileRequested()
        else if (isPoll) pollRequested()
        else if (isLocation) locationRequested()
    }

    menu: selectionMode ? null : contextMenuComponent
    Component {
        id: contextMenuComponent
        ContextMenu {
            id: contextMenu
            readonly property string copyableText: Fmt.copyText(model.kind, model.text, model.body, model.mentions)
            // Its upload or file download is on the way.
            readonly property bool transferring: item.isFile && (model.body.fileState === 1
                                                                 || (item.outgoing && model.body.uploadState === 1))
            readonly property bool canDeleteEveryone: item.outgoing && !item.isDeleted && !item.isPoll
                                                      && !item.unsent && item.canDelete && item.recent
                                                      && !item.readOnly
            Row {
                id: reactionRow
                visible: item.canReact && !item.readOnly && !item.isStatus && !item.isDeleted
                width: parent.width
                // Threema's quick reactions, then any other emoji (Android's "+").
                readonly property var quick: item.anyReaction
                                             ? ["\ud83d\udc4d", "\ud83d\udc4e", "\u2764\ufe0f", "\ud83d\ude02", "\ud83d\ude2e", "\ud83d\ude22"]
                                             : ["\ud83d\udc4d", "\ud83d\udc4e"]
                readonly property real cell: width / (quick.length + (item.anyReaction ? 1 : 0))
                Repeater {
                    model: reactionRow.quick
                    BackgroundItem {
                        width: reactionRow.cell
                        height: Theme.itemSizeSmall
                        highlighted: down || Fmt.reactedWith(item.reactions, modelData)
                        Label {
                            anchors.centerIn: parent
                            // Threema's emoji pictures (Settings → Appearance), like the bubbles.
                            textFormat: backend.threemaEmoji ? Text.StyledText : Text.PlainText
                            text: backend.threemaEmoji ? Fmt.markEmoji(modelData, Math.round(font.pixelSize * 1.2))
                                                       : modelData
                            font.pixelSize: Theme.fontSizeLarge
                        }
                        onClicked: {
                            item.toggleReaction(modelData)
                            contextMenu.close()
                        }
                    }
                }
                BackgroundItem {
                    visible: item.anyReaction
                    width: reactionRow.cell
                    height: Theme.itemSizeSmall
                    onClicked: {
                        contextMenu.close()
                        item.moreReactionsRequested()
                    }
                    Icon {
                        anchors.centerIn: parent
                        source: "image://theme/icon-m-add"
                        highlighted: parent.highlighted
                    }
                }
            }
            // The user's layout: frequent actions as columns (icon with a small label), the one
            // used most at the right, nearest the thumb; the rest as ordinary menu items.
            ActionRow {
                actions: {
                    var list = []
                    if ((model.kind === "text" && !item.isDeleted) || item.isLocation
                            || (item.isFile && model.body.fileState === 2))
                        //% "Forward"
                        list.push({ id: "forward", icon: "icon-m-message-forward", text: qsTrId("threesea-me-forward") })
                    if (!item.isStatus && !item.isDeleted)
                        list.push({ id: "star", icon: model.starred ? "icon-m-favorite-selected" : "icon-m-favorite",
                                    text: model.starred
                                          //% "Unstar"
                                          ? qsTrId("threesea-me-unstar-short")
                                          //% "Star"
                                          : qsTrId("threesea-me-star") })
                    if (item.canQuote && !item.readOnly && !item.isStatus && !item.isDeleted && !item.isPoll)
                        //% "Quote"
                        list.push({ id: "quote", icon: "icon-m-message-reply", text: qsTrId("threesea-me-quote") })
                    return list
                }
                onTriggered: {
                    contextMenu.close()
                    if (id === "quote") item.quoteRequested()
                    else if (id === "forward") item.forwardRequested()
                    else item.starToggled()
                }
            }
            MenuItem {
                //% "Send again"
                text: qsTrId("threesea-me-send-again")
                visible: item.failed && !item.readOnly
                onClicked: item.resendRequested()
            }
            MenuItem {
                text: item.isFile && model.body.fileState === 1
                      //% "Stop download"
                      ? qsTrId("threesea-me-stop-download")
                      //% "Stop sending"
                      : qsTrId("threesea-me-stop-upload")
                visible: contextMenu.transferring
                onClicked: item.cancelTransferRequested()
            }
            MenuItem {
                //% "Copy"
                text: qsTrId("threesea-me-copy")
                visible: contextMenu.copyableText.length > 0
                onClicked: Clipboard.text = contextMenu.copyableText
            }
            MenuItem {
                //% "Share"
                text: qsTrId("threesea-me-share")
                visible: contextMenu.copyableText.length > 0 || (item.isFile && model.body.fileState === 2)
                onClicked: item.shareRequested()
            }
            MenuItem {
                //% "Edit"
                text: qsTrId("threesea-me-edit")
                // Nobody can have it yet: simply changed. Otherwise, like a sent one, it takes an
                // edit message, as someone may have it (the core's edit_message).
                visible: item.outgoing && !item.readOnly && model.kind === "text"
                         && ((model.state === 0 && !model.writtenOut)
                             || (model.state !== 4 && item.canEdit && item.recent))
                onClicked: item.editRequested()
            }
            MenuItem {
                //% "Save to Downloads"
                text: qsTrId("threesea-me-save-downloads")
                visible: item.isFile && model.body.fileState === 2
                onClicked: item.saveRequested()
            }
            MenuItem {
                //% "Draw on it"
                text: qsTrId("threesea-me-draw-on-picture")
                visible: item.isFile && model.body.fileState === 2 && !item.readOnly
                         && model.body.rendering !== "file" && Fmt.isPicture(model.body.mediaType)
                         && String(model.body.mediaType).toLowerCase() !== "image/gif"
                onClicked: item.drawRequested()
            }
            MenuItem {
                //% "Add to favourite GIFs"
                text: qsTrId("threesea-me-add-gif-favourite")
                visible: item.isFile && model.body.fileState === 2
                         && String(model.body.mediaType).toLowerCase() === "image/gif"
                onClicked: item.gifFavouriteRequested()
            }
            MenuItem {
                //% "Close poll"
                text: qsTrId("threesea-me-close-poll")
                visible: item.myOpenPoll && !item.readOnly
                onClicked: item.closePollRequested()
            }
            MenuItem {
                //% "Details"
                text: qsTrId("threesea-me-details")
                visible: !item.isStatus
                onClicked: item.detailsRequested()
            }
            // Delete: for me and for everyone side by side while deleting for everyone is
            // possible, else an ordinary Delete item (both through the remorse timer).
            ActionRow {
                actions: contextMenu.canDeleteEveryone
                         //% "Delete for me"
                         ? [{ id: "mine", icon: "icon-m-delete", text: qsTrId("threesea-me-delete-for-me") },
                            //% "Delete for everyone"
                            { id: "everyone", icon: "icon-m-delete", text: qsTrId("threesea-me-delete-for-everyone") }]
                         : []
                onTriggered: {
                    contextMenu.close()
                    item.deleteRequested(id === "everyone")
                }
            }
            MenuItem {
                visible: !contextMenu.canDeleteEveryone
                //% "Delete"
                text: qsTrId("threesea-me-delete")
                onClicked: item.deleteRequested(false)
            }
        }
    }

    Label {
        id: dayLabel
        visible: item.showDay
        y: item.topGap
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        text: item.dayText
        color: Theme.secondaryColor
    }

    Separator {
        id: unreadLine
        visible: item.showUnread
        x: Theme.horizontalPageMargin
        y: item.topGap + (item.showDay ? dayLabel.height + Theme.paddingLarge : 0)
           + Theme.paddingMedium
        width: parent.width - 2 * Theme.horizontalPageMargin
        color: Theme.highlightColor
        horizontalAlignment: Qt.AlignHCenter
    }

    readonly property bool showSender: senderName.length > 0 && firstOfRun && !isStatus
    readonly property real bodyY: item.topGap + (item.showDay ? dayLabel.height + Theme.paddingLarge : 0)
                                  + item.unreadGap
                                  + (showSender ? senderLabel.height : 0)

    Label {
        id: senderLabel
        visible: item.showSender
        x: Theme.paddingMedium + Theme.paddingMedium
        y: item.bodyY - height
        width: item.maxTextWidth
        text: item.senderName
        truncationMode: TruncationMode.Fade
        font.pixelSize: Theme.fontSizeExtraSmall
        color: item.highlighted ? Theme.highlightColor : Theme.secondaryHighlightColor
    }

    Label {
        id: statusLabel
        visible: item.isStatus
        x: Theme.horizontalPageMargin
        y: item.bodyY
        width: parent.width - 2 * Theme.horizontalPageMargin
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: item.isStatus ? Fmt.status(model.body) : ""
        font.pixelSize: Theme.fontSizeExtraSmall
        color: Theme.secondaryColor
    }

    Item {
        id: bubble
        visible: !item.isStatus
        y: item.bodyY
        // Whole pixels: BubbleShape's clipped corners snap to pixels, so at a fractional position
        // (text with emoji pictures has fractional widths) a seam shows next to them.
        x: Math.round(item.outgoing ? item.width - width - Theme.paddingMedium : Theme.paddingMedium)
        width: Math.ceil(content.width + 2 * Theme.paddingMedium)
        height: Math.ceil(content.height + 2 * Theme.paddingMedium)

        // A white overlay like Jolla Messages (measured on its screenshots: about 12 % for
        // incoming, 5–6 % for outgoing messages).
        BubbleShape {
            id: bubbleShape
            anchors.fill: parent
            radius: Theme.paddingLarge
            color: item.highlighted ? Theme.rgba(Theme.highlightBackgroundColor, Theme.highlightBackgroundOpacity)
                                    : Theme.rgba(Theme.primaryColor, Fmt.alpha(item.outgoing ? 0.06 : 0.12, backend.emulator))
            // Inner bottom always round, top corners only on the first bubble of a run.
            roundTopLeft: item.firstOfRun
            roundTopRight: item.firstOfRun
            roundBottomLeft: item.outgoing
            roundBottomRight: !item.outgoing
        }

        Item {
            id: content
            x: Theme.paddingMedium
            y: Theme.paddingMedium

            readonly property bool hasText: textLabel.plainText.length > 0
            readonly property real footerWidth: footer.width + Theme.paddingMedium
            // The parts only some messages have are loaded only for those (fewer objects per
            // bubble: scrolling creates them).
            readonly property Item quoteBlock: quoteLoader.item
            readonly property Item attachment: attachmentLoader.item
            readonly property Item mentionLabel: mentionLoader.item
            readonly property bool rich: item.richText && mentionLabel !== null
            // The mention text has a fixed width (no binding loop): its painted width counts.
            readonly property real textWidth: rich ? mentionLabel.contentWidth : textLabel.width
            readonly property real textNaturalWidth: rich ? mentionLabel.contentWidth : textLabel.implicitWidth
            // File messages: the file above the caption.
            // A quote above the text: a bar, the quoted sender and two lines of it.
            readonly property real quoteHeight: quoteBlock ? quoteBlock.height + Theme.paddingSmall : 0
            readonly property real fileHeight: quoteHeight + (!attachment ? 0
                                             : attachment.height + (!item.isFile || hasText ? Theme.paddingSmall : 0))
            // A short text-only message: time and delivery state on its line (user 2026-10-26);
            // everything else: on a row of their own.
            readonly property bool inline: hasText && fileHeight === 0
                                           && (rich ? mentionLabel.lineCount : textLabel.lineCount) === 1
                                           && textNaturalWidth + footerWidth <= item.maxTextWidth
            readonly property real textHeight: hasText ? (rich ? mentionLabel.height : textLabel.height) : 0
            width: Math.max(attachment ? attachment.width : 0,
                            quoteBlock ? Math.min(quoteBlock.implicitWidth, item.maxTextWidth) : 0,
                            inline ? textWidth + footerWidth : Math.max(hasText ? textWidth : 0, footer.width))
            height: fileHeight + (inline ? textHeight : textHeight + footer.height)

            Loader {
                id: quoteLoader
                active: item.hasQuote
                sourceComponent: quoteComponent
            }
            Component {
                id: quoteComponent
                Item {
                    width: Math.max(content.width, Math.min(implicitWidth, message.maxTextWidth))
                    height: quoteColumn.height
                    implicitWidth: Math.max(quoteSender.implicitWidth, quoteText.implicitWidth) + Theme.paddingMedium
                    Rectangle {
                        width: Math.round(Theme.paddingSmall / 2)
                        height: parent.height
                        radius: width / 2
                        color: message.highlighted ? Theme.highlightColor : Theme.secondaryHighlightColor
                    }
                    Column {
                        id: quoteColumn
                        x: Theme.paddingMedium
                        width: parent.width - x
                        Label {
                            id: quoteSender
                            width: parent.width
                            text: message.quoted ? message.quoted.senderName || "" : ""
                            visible: text.length > 0
                            truncationMode: TruncationMode.Fade
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: message.highlighted ? Theme.highlightColor : Theme.secondaryHighlightColor
                        }
                        Label {
                            id: quoteText
                            width: parent.width
                            text: message.quoted ? Fmt.preview(message.quoted)
                                              //% "Message not available"
                                              : qsTrId("threesea-la-quote-missing")
                            maximumLineCount: 2
                            wrapMode: Text.Wrap
                            elide: Text.ElideRight
                            font.pixelSize: Theme.fontSizeSmall
                            color: message.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                        }
                    }
                    MouseArea {
                        anchors.fill: parent
                        enabled: !message.selectionMode
                        onClicked: message.quotedRequested()
                        onPressAndHold: message.openMenu()
                    }
                }
            }

            // A message has at most one of these.
            Loader {
                id: attachmentLoader
                y: content.quoteHeight
                sourceComponent: item.isFile ? fileComponent : item.isPoll ? pollComponent
                                 : item.isLocation ? locationComponent : null
            }

            Component {
                id: locationComponent
                LocationContent {
                    location: model.body
                    maxWidth: message.maxTextWidth
                    highlighted: message.highlighted
                    textColor: message.textColor
                }
            }

            Component {
                id: pollComponent
                PollContent {
                    poll: model.body
                    results: message.isPollResults
                    myIdentity: backend.identity
                    maxWidth: message.maxTextWidth
                    highlighted: message.highlighted
                    textColor: message.textColor
                }
            }

            Component {
                id: fileComponent
                FileContent {
                    messageId: model.id
                    body: model.body
                    // In the view, not only built: a GIF scrolled past needn't be decoded frame
                    // by frame. Only a GIF asks, so other bubbles don't follow the scrolling.
                    onScreen: !isGif || (!!message.chatView
                                         && message.y + message.height > message.chatView.contentY
                                         && message.y < message.chatView.contentY + message.chatView.height)
                    listMoving: message.listMoving
                    maxWidth: message.maxTextWidth
                    highlighted: message.highlighted
                    textColor: message.textColor
                    newVoice: !message.outgoing && model.body.listened !== true
                    // Concentric with the bubble: its radius minus the padding around the picture.
                    radius: bubbleShape.radius - Theme.paddingMedium
                    roundTopLeft: bubbleShape.roundTopLeft
                    roundTopRight: bubbleShape.roundTopRight
                    roundBottomLeft: bubbleShape.roundBottomLeft
                    roundBottomRight: bubbleShape.roundBottomRight
                }
            }

            // Links, e-mail addresses and phone numbers are tappable, as in Jolla Messages;
            // Threema links open in the app.
            LinkedLabel {
                id: textLabel
                y: content.fileHeight
                // With mentions, formatting or emoji pictures this only finds the links;
                // mentionLabel shows the result.
                visible: content.hasText && !content.rich
                width: Math.min(implicitWidth, item.maxTextWidth)
                plainText: item.richText ? (model.kind === "text" ? model.text : model.body.caption || "")
                                            : model.kind === "text" ? model.text
                                            : item.isFile ? (model.body.caption || "")
                                            : item.isPoll || item.isLocation ? "" : Fmt.preview(model)
                font.italic: item.isDeleted
                // Up to three emoji alone: twice the size (Settings → Appearance).
                readonly property bool bigEmoji: backend.biggerEmojis && model.kind === "text" && !item.hasMentions
                                && Fmt.emojiOnlyCount(model.text) > 0 && Fmt.emojiOnlyCount(model.text) <= 3
                // Emoji pictures are a quarter taller than the font, which is about a line's
                // height: Qt hangs an inline picture from the bottom of its line, so a shorter
                // one leaves the rest of the line empty above it — and a message of nothing but
                // emoji has no glyphs to fill it. Hence 1.6 for the big font: × 1.25 is twice
                // the text size. System glyphs (Threema emoji off) fill their line themselves.
                readonly property real emojiSize: Math.round(font.pixelSize * 1.25)
                // Settings → Appearance → Message text (Android's font size).
                readonly property real baseSize: backend.chatFontSize === 0 ? Theme.fontSizeSmall
                                                 : backend.chatFontSize === 2 ? Theme.fontSizeLarge
                                                 : Theme.fontSizeMedium
                font.pixelSize: !bigEmoji ? baseSize
                                : Math.round((item.richText ? 1.6 : 2) * baseSize)
                color: item.isDeleted ? (item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor)
                                      : item.textColor
                linkColor: Theme.highlightColor
                defaultLinkActions: false
                onLinkActivated: item.selectionMode ? item.selectToggled() : window.openLink(link, true)
            }

            // Mentions in bold, in the ambience's highlight colour, to catch the eye; the sender's
            // formatting; emoji as pictures, a bit larger than the text like Threema's.
            Loader {
                id: mentionLoader
                y: content.fileHeight
                active: item.richText
                sourceComponent: mentionComponent
            }
            Component {
                id: mentionComponent
                Text {
                    visible: content.hasText
                    width: message.maxTextWidth
                    text: {
                        if (!message.richText) return ""
                        var html = Fmt.markMarkup(textLabel.text)
                        if (message.hasMentions) html = Fmt.markMentions(html, model.mentions, Theme.highlightColor)
                        if (Fmt.needsRichText(html)) html = Fmt.colourLinks(html, textLabel.linkColor)
                        return backend.threemaEmoji ? Fmt.markEmoji(html, textLabel.emojiSize) : html
                    }
                    textFormat: Fmt.needsRichText(text) ? Text.RichText : Text.StyledText
                    wrapMode: Text.Wrap
                    font: textLabel.font
                    color: textLabel.color
                    linkColor: textLabel.linkColor
                    onLinkActivated: message.selectionMode ? message.selectToggled() : window.openLink(link, true)
                }
            }

            Row {
                id: footer
                x: content.inline ? content.textNaturalWidth + Theme.paddingMedium : content.width - width
                y: content.fileHeight + (content.inline ? (content.rich ? content.mentionLabel.baselineOffset : textLabel.baselineOffset) - timeLabel.baselineOffset
                                                        : content.textHeight)
                spacing: Theme.paddingSmall

                // Starred (People's favourite star, small).
                Image {
                    visible: model.starred === true
                    anchors.verticalCenter: timeLabel.verticalCenter
                    width: Theme.iconSizeExtraSmall
                    height: width
                    sourceSize.width: width
                    sourceSize.height: height
                    source: "image://theme/icon-s-favorite?" + timeLabel.color
                }
                Label {
                    visible: item.isEdited
                    //% "Edited"
                    text: qsTrId("threesea-la-edited")
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: timeLabel.color
                }
                Label {
                    id: timeLabel
                    text: Fmt.time(model.createdAt, Format, Formatter)
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                }
                DeliveryState {
                    visible: item.outgoing
                    // A failed upload reads as a failed send (the warning), not as waiting:
                    // tapping the bubble tries again.
                    state: item.isFile && model.body.uploadState === 3 ? 4 : model.state
                    highlighted: item.highlighted
                    // Centred on the digits (not the label's box, which has room below them).
                    y: timeLabel.y + timeLabel.baselineOffset - Math.round(timeLabel.font.pixelSize * 0.36) - height / 2
                }
            }
        }
    }
    Flow {
        id: reactionRow
        visible: item.reactionGroups.length > 0
        y: bubble.y + bubble.height + Theme.paddingSmall
        x: item.outgoing ? item.width - width - Theme.paddingMedium : Theme.paddingMedium
        width: Math.min(implicitWidthHint, item.width - 2 * Theme.paddingMedium)
        spacing: Theme.paddingSmall
        layoutDirection: item.outgoing ? Qt.RightToLeft : Qt.LeftToRight

        // Flow has no implicit width: sum of the chips, so the row hugs the bubble's side.
        readonly property real implicitWidthHint: {
            var total = 0
            for (var i = 0; i < chips.count; ++i) {
                if (chips.itemAt(i)) total += chips.itemAt(i).width + spacing
            }
            return Math.max(0, total - spacing)
        }

        Repeater {
            id: chips
            model: item.reactionGroups
            // In the highlight colour, whoever reacted (user 2026-10-26); the top corner
            // under the bubble square (incoming left, outgoing right), the others round (user).
            Item {
                id: chip
                readonly property var ranges: backend.threemaEmoji ? Fmt.emojiRanges(modelData.emoji) : []
                width: Math.round(chipContent.width + 2 * Theme.paddingMedium)
                height: Math.round(Theme.iconSizeSmall + 2 * Theme.paddingSmall)
                BubbleShape {
                    anchors.fill: parent
                    radius: Theme.paddingLarge
                    color: Theme.rgba(Theme.highlightBackgroundColor, Fmt.alpha(0.3, backend.emulator))
                    roundTopLeft: item.outgoing
                    roundTopRight: !item.outgoing
                }
                Row {
                    id: chipContent
                    anchors.centerIn: parent
                    spacing: Theme.paddingSmall
                    // Threema's picture (Settings → Appearance), else the system's glyph.
                    Image {
                        visible: chip.ranges.length === 1
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.iconSizeSmall
                        height: width
                        sourceSize.width: width
                        sourceSize.height: height
                        source: visible ? "image://emoji/" + chip.ranges[0].cell : ""
                    }
                    Label {
                        visible: chip.ranges.length !== 1
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.emoji
                        font.pixelSize: Theme.fontSizeMedium
                    }
                    Label {
                        visible: modelData.count > 1
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.count
                        font.pixelSize: Theme.fontSizeSmall
                        color: modelData.mine ? Theme.highlightColor : Theme.primaryColor
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    enabled: !item.selectionMode
                    onClicked: if (!item.readOnly) item.toggleReaction(modelData.emoji)
                    onPressAndHold: item.reactionsRequested()
                }
            }
        }
    }
}
