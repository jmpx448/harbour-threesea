// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

// A chat's media as a grid, newest first, like Jolla Gallery's (its ImageGridView is not
// Harbour-allowed, so the same layout with Silica). Like Android's gallery it filters by kind
// (photos, GIFs, videos, voice messages, audio, files; pull-down "Show", a dialog accepted by
// swipe, saved for all chats). Tap: view, play or open (downloading first if needed); long press:
// show in chat, save to Downloads, share, stop a download, delete (on this device, with a remorse
// timer). Pull-down "Select": taps select tiles, a panel at the bottom saves, shares or deletes them.
Page {
    id: page
    allowedOrientations: Orientation.All

    property string identity
    property string name
    property var kinds: []
    property var counts: ({})

    readonly property var allKinds: ["image", "gif", "video", "voice", "audio", "file"]
    readonly property var kindNames: [
        //% "Photos"
        qsTrId("threesea-la-kind-photos"),
        //% "GIFs"
        qsTrId("threesea-la-kind-gifs"),
        //% "Videos"
        qsTrId("threesea-la-kind-videos"),
        //% "Voice messages"
        qsTrId("threesea-la-kind-voice"),
        //% "Audio"
        qsTrId("threesea-la-kind-audio"),
        //% "Files"
        qsTrId("threesea-la-kind-files")
    ]
    // The kinds this chat has (filters for the others would show nothing).
    readonly property var presentKinds: allKinds.filter(function(k) { return counts[k] > 0 })

    // The gallery comes page by page while scrolling. `load` gets what is shown so far anew, when
    // what is in it changes; `generation` tells whose answer is still wanted.
    readonly property int pageSize: 60
    property string nextPage
    property bool loading
    property bool loadingMore
    property int generation
    // The row of a message by its id, and the history position of the last row.
    property var rowOf: ({})
    property var oldestKey

    property bool selecting: false
    property var selected: ({})
    // Selecting everything takes all pages, as they come.
    property bool selectingAll: false
    readonly property int selectedCount: {
        var n = 0
        for (var id in selected) if (selected[id]) n++
        return n
    }
    function toggleSelected(id) {
        var map = selected
        map[id] = !map[id]
        selected = map
    }
    function selectAll() {
        selectFrom(0)
    }
    // From this row on: what arrives later is selected too, what was taken out since stays out.
    function selectFrom(row) {
        var map = selected
        for (var i = row; i < media.count; ++i) map[media.get(i).id] = true
        selected = map
        selectingAll = nextPage.length > 0
        loadMore()
    }
    function endSelection() {
        selecting = false
        selectingAll = false
        selected = {}
    }
    // What is selected and still among the rows, as they are now.
    function selectedRows() {
        var rows = []
        for (var i = 0; i < media.count; ++i) {
            var row = media.get(i)
            if (selected[row.id]) rows.push({ id: row.id, messageId: row.messageId, outgoing: row.outgoing,
                                              body: JSON.parse(row.body) })
        }
        return rows
    }
    function selectedIds() {
        return selectedRows().map(function(row) { return row.id })
    }
    readonly property bool selectionHere: {
        if (selectedCount === 0) return false
        var rows = selectedRows()
        for (var i = 0; i < rows.length; ++i) if (rows[i].body.fileState !== 2) return false
        return rows.length > 0
    }

    function load() {
        var asked = ++generation
        loading = true
        loadingMore = false
        backend.call("chatMedia", { identity: identity, limit: Math.max(pageSize, media.count) },
                     function(ok, result) {
            if (asked !== page.generation) return
            page.loading = false
            if (!ok) return
            page.kinds = result.kinds
            page.counts = result.counts
            media.update(result.messages)
            page.nextPage = result.next || ""
            grid.loadMoreIfNear()
            // A reload took the place of a page on its way.
            if (page.selectingAll) page.loadMore()
        })
    }

    function loadMore() {
        if (nextPage.length === 0 || loading || loadingMore) return
        var asked = generation
        loadingMore = true
        backend.call("chatMedia", { identity: identity, limit: pageSize, before: nextPage },
                     function(ok, result) {
            if (asked !== page.generation) return
            page.loadingMore = false
            if (!ok) return
            var first = media.count
            media.add(result.messages)
            page.nextPage = result.next || ""
            if (page.selectingAll) page.selectFrom(first)
        })
    }

    // One message changed: its row takes that over. Whether anything else has to be got anew: a
    // message that went, one that belongs among the rows and isn't there yet.
    function applyChange(message) {
        var row = rowOf[message.id]
        if (row === undefined) {
            return message.kind === "deleted"
                    || (message.kind === "file" && (nextPage.length === 0 || message.sortKey > oldestKey))
        }
        if (message.kind !== "file") return true
        var body = JSON.stringify(message.body)
        if (media.get(row).body !== body) media.setProperty(row, "body", body)
        return false
    }

    function chooseKinds() {
        var dialog = pageStack.push(Qt.resolvedUrl("MediaFilterDialog.qml"), {
            kinds: presentKinds,
            names: presentKinds.map(function(k) { return kindNames[allKinds.indexOf(k)] }),
            // Kinds the chat doesn't have keep their saved state.
            selected: kinds.slice()
        })
        dialog.accepted.connect(function() {
            backend.call("setMediaKinds", { kinds: dialog.selected }, function() { page.load() })
        })
    }

    Component.onCompleted: load()

    // A file arriving is several events.
    Timer {
        id: reload
        interval: 200
        onTriggered: page.load()
    }

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "messageAdded") {
                if (data.message.conversation === page.identity && data.message.kind === "file") reload.restart()
            } else if (name === "messagesChanged") {
                for (var i = 0; i < data.messages.length; ++i) {
                    if (data.messages[i].conversation === page.identity && page.applyChange(data.messages[i])) {
                        reload.restart()
                    }
                }
            }
        }
    }

    // Row-wise updates keep the grid's position (a reset would jump to the top). The body is kept
    // as JSON text: ListModel would turn an object into a nested model.
    ListModel {
        id: media

        function row(message) {
            return { id: message.id, kind: message.mediaKind, body: JSON.stringify(message.body),
                     messageId: message.messageId, outgoing: message.outgoing }
        }

        function update(messages) {
            var rows = {}
            for (var i = 0; i < messages.length; ++i) {
                var message = row(messages[i])
                rows[message.id] = i
                if (i < count) {
                    var shown = get(i)
                    if (shown.id !== message.id || shown.body !== message.body || shown.kind !== message.kind) {
                        set(i, message)
                    }
                } else {
                    append(message)
                }
            }
            if (count > messages.length) remove(messages.length, count - messages.length)
            page.rowOf = rows
            if (messages.length > 0) page.oldestKey = messages[messages.length - 1].sortKey
        }

        function add(messages) {
            for (var i = 0; i < messages.length; ++i) {
                page.rowOf[messages[i].id] = count
                append(row(messages[i]))
            }
            if (messages.length > 0) page.oldestKey = messages[messages.length - 1].sortKey
        }
    }

    property int voiceToHear: -1
    function voiceHeard() {
        if (voiceToHear >= 0) backend.call("setVoiceListened", { message: voiceToHear })
        voiceToHear = -1
    }
    Connections {
        target: voicePlayer
        onMessageIdChanged: if (voicePlayer.messageId >= 0 && voicePlayer.messageId === voiceToHear) voiceHeard()
    }

    function open(id, kind, body, outgoing) {
        if (body.fileState !== 2) {
            if (body.fileState !== 1) window.messageOut.download(id)
        } else if ((kind === "image" || kind === "gif") && Fmt.isPicture(body.mediaType)) {
            pageStack.push(Qt.resolvedUrl("ImageViewerPage.qml"), { identity: identity, messageId: id, newestFirst: true,
                                                                   animated: kind === "gif" })
        } else if (kind === "voice") {
            if (voicePlayer.messageId === id && voicePlayer.playing) {
                voicePlayer.pause()
            } else {
                voicePlayer.play(id)
                window.voiceFrom = ""
                // Heard is heard, wherever it was: its chat stops showing it as new. Once it could
                // be read.
                voiceToHear = !outgoing && body.listened !== true ? id : -1
                if (voicePlayer.messageId === id) voiceHeard()
            }
        } else if (kind === "video" && videoPlayer.available) {
            pageStack.push(Qt.resolvedUrl("VideoPlayerPage.qml"), { messageId: id })
        } else {
            save(id, true)
        }
    }

    function save(id, open) {
        backend.call("saveFile", { message: id, directory: StandardPaths.download },
                     function(ok, result, kind, message) {
            if (!ok) {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            } else {
                //% "Saved to Downloads"
                Notices.show(qsTrId("threesea-la-saved-downloads"), Notice.Short, Notice.Center)
                // Documents and media only; the core decides by the name the file has now.
                if (open && result.openable) Qt.openUrlExternally(Fmt.fileUrl(result.path))
            }
        })
    }

    // Like the chat's delete for media: on this device only.
    function remove(messageId, outgoing) {
        backend.call("deleteMessage", { identity: identity, messageId: messageId, outgoing: outgoing,
                                         everyone: false }, function(ok, result, kind, message) {
            if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            page.load()
        })
    }

    // Several: one look at what is left, after the last.
    function removeAll(rows) {
        var left = rows.length
        var problem = ""
        function answered(ok, result, kind, message) {
            if (!ok) problem = Errors.text(kind, message)
            if (--left > 0) return
            if (problem.length > 0) Notices.show(problem, Notice.Long, Notice.Center)
            page.load()
        }
        for (var i = 0; i < rows.length; ++i) {
            backend.call("deleteMessage", { identity: identity, messageId: rows[i].messageId, outgoing: rows[i].outgoing,
                                             everyone: false }, answered)
        }
    }

    function showInChat(id) {
        var chat = pageStack.find(function(p) { return p.objectName === "chat" && p.identity === page.identity })
        if (chat) {
            chat.showMessage(id)
            pageStack.pop(chat)
        } else {
            pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: page.identity, focusMessage: id })
        }
    }

    SilicaGridView {
        id: grid
        anchors.fill: parent
        anchors.bottomMargin: selectionPanel.visibleSize
        clip: true
        model: media
        // Jolla Gallery's grid: as many columns as fit Theme.itemSizeHuge.
        readonly property real cellSize: Math.floor(width / Math.max(1, Math.floor(width / Theme.itemSizeHuge)))
        cellWidth: cellSize
        cellHeight: cellSize
        cacheBuffer: 1000

        function loadMoreIfNear() {
            if (originY + contentHeight - contentY - height < 2 * height) page.loadMore()
        }
        onContentYChanged: loadMoreIfNear()
        onContentHeightChanged: loadMoreIfNear()

        PullDownMenu {
            visible: grid.count > 0 || page.presentKinds.length > 0
            MenuItem {
                // Only worth it with more than one kind, or when the saved choice (it is for all
                // chats) leaves this chat's only kind out: the way back.
                visible: page.presentKinds.length > 1 || (grid.count === 0 && page.presentKinds.length > 0)
                //% "Show"
                text: qsTrId("threesea-me-media-show")
                onClicked: page.chooseKinds()
            }
            MenuItem {
                visible: page.selecting
                text: qsTrId("threesea-me-cancel-selection")
                onClicked: page.endSelection()
            }
            MenuItem {
                visible: page.selecting && grid.count > 0
                //% "Select all"
                text: qsTrId("threesea-me-select-all")
                onClicked: page.selectAll()
            }
            MenuItem {
                visible: !page.selecting && grid.count > 0
                //% "Select"
                text: qsTrId("threesea-me-select")
                onClicked: page.selecting = true
            }
        }

        header: PageHeader {
            //% "Media"
            title: qsTrId("threesea-he-media")
            description: page.name
        }

        delegate: GridItem {
            id: tile
            readonly property var body: JSON.parse(model.body)
            readonly property bool isVideo: model.kind === "video"
            readonly property bool isPicture: (model.kind === "image" || model.kind === "gif")
                                              && Fmt.isPicture(body.mediaType)
            readonly property bool hasThumbnail: body.thumbnailState === 2
            readonly property bool playing: voicePlayer.messageId === model.id && voicePlayer.playing
            width: grid.cellWidth
            contentHeight: grid.cellHeight
            highlighted: down || menuOpen || (page.selecting && page.selected[model.id] === true)
            onClicked: {
                if (page.selecting) page.toggleSelected(model.id)
                else page.open(model.id, model.kind, body, model.outgoing)
            }

            Rectangle {
                anchors.fill: parent
                color: Theme.rgba(Theme.primaryColor, Fmt.alpha(0.05, backend.emulator))
            }
            Image {
                id: picture
                anchors.fill: parent
                fillMode: Image.PreserveAspectCrop
                clip: true
                asynchronous: true
                cache: false
                sourceSize.width: grid.cellSize
                sourceSize.height: grid.cellSize
                // The thumbnail; a photo without one once it is here.
                source: tile.hasThumbnail ? "image://threesea/" + model.id + "/thumb"
                        : tile.isPicture && tile.body.fileState === 2 ? "image://threesea/" + model.id + "/file" : ""
                opacity: tile.highlighted ? Theme.opacityHigh : 1
            }
            // No picture: the file's icon and name (voice messages: play/pause and the length).
            Column {
                visible: picture.status !== Image.Ready
                anchors.centerIn: parent
                width: parent.width - 2 * Theme.paddingMedium
                spacing: Theme.paddingSmall
                Image {
                    anchors.horizontalCenter: parent.horizontalCenter
                    source: "image://theme/" + (model.kind === "voice" && tile.body.fileState === 2
                                                ? (tile.playing ? "icon-m-pause" : "icon-m-play")
                                                : Fmt.fileIcon(tile.body.fileType, tile.body.fileState))
                            + (tile.highlighted ? "?" + Theme.highlightColor : "")
                }
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    visible: !tile.isPicture && !tile.isVideo
                    text: model.kind === "voice"
                          ? Fmt.duration(tile.playing ? voicePlayer.position / 1000 : tile.body.duration || 0)
                          : tile.body.name || ""
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: tile.highlighted ? Theme.highlightColor : Theme.secondaryColor
                    maximumLineCount: 2
                    wrapMode: Text.WrapAnywhere
                    elide: Text.ElideRight
                }
            }
            BusyIndicator {
                anchors.centerIn: parent
                size: BusyIndicatorSize.Medium
                running: tile.body.fileState === 1
            }

            // Like Gallery's video thumbnails: a shade at the bottom with the length.
            Rectangle {
                visible: tile.isVideo
                anchors.bottom: parent.bottom
                width: parent.width
                height: parent.height / 2
                opacity: Theme.opacityOverlay
                gradient: Gradient {
                    GradientStop { position: 0.0; color: "transparent" }
                    GradientStop { position: 1.0; color: Theme.highlightDimmerColor }
                }
            }
            // The camera icon too: without a media service the length is unknown.
            Row {
                visible: tile.isVideo
                anchors {
                    left: parent.left
                    bottom: parent.bottom
                    margins: Theme.paddingSmall
                }
                spacing: Theme.paddingSmall
                Image {
                    anchors.verticalCenter: parent.verticalCenter
                    source: "image://theme/icon-m-video"
                }
                Label {
                    anchors.verticalCenter: parent.verticalCenter
                    text: tile.body.duration > 0 ? Fmt.duration(tile.body.duration) : ""
                    font.pixelSize: Theme.fontSizeSmall
                }
            }

            menu: page.selecting ? null : tileMenu
            Component {
                id: tileMenu
                ContextMenu {
                    MenuItem {
                        //% "Show in chat"
                        text: qsTrId("threesea-me-show-in-chat")
                        onClicked: page.showInChat(model.id)
                    }
                    MenuItem {
                        //% "Save to Downloads"
                        text: qsTrId("threesea-me-save-downloads")
                        visible: tile.body.fileState === 2
                        onClicked: page.save(model.id, false)
                    }
                    MenuItem {
                        text: qsTrId("threesea-me-share")
                        visible: tile.body.fileState === 2
                        onClicked: window.messageOut.shareFiles([model.id])
                    }
                    MenuItem {
                        text: qsTrId("threesea-me-stop-download")
                        visible: tile.body.fileState === 1
                        onClicked: backend.call("cancelTransfer", { message: model.id })
                    }
                    MenuItem {
                        text: qsTrId("threesea-me-delete")
                        onClicked: {
                            // Kept: the callback runs after the menu (and its context) is gone.
                            var mediaPage = page
                            var messageId = model.messageId
                            var outgoing = model.outgoing
                            tile.remorseDelete(function() { mediaPage.remove(messageId, outgoing) })
                        }
                    }
                }
            }
        }

        ViewPlaceholder {
            enabled: grid.count === 0
            text: page.presentKinds.length > 0
                  //% "Nothing of the kinds shown"
                  ? qsTrId("threesea-la-no-media-filtered")
                  //% "No media"
                  : qsTrId("threesea-la-no-media")
            //% "Pull down to choose what to show"
            hintText: page.presentKinds.length > 0 ? qsTrId("threesea-la-no-media-filtered-hint") : ""
        }

        VerticalScrollDecorator {}
    }

    RemorsePopup { id: selectionRemorse }

    // Like the chat's selection panel.
    DockedPanel {
        id: selectionPanel
        width: parent.width
        height: Theme.itemSizeLarge
        dock: Dock.Bottom
        open: page.selecting

        Label {
            anchors {
                left: parent.left
                leftMargin: Theme.horizontalPageMargin
                right: selectionButtons.left
                rightMargin: Theme.paddingMedium
                verticalCenter: parent.verticalCenter
            }
            truncationMode: TruncationMode.Fade
            text: page.selectedCount > 0
                  ? qsTrId("threesea-he-n-selected", page.selectedCount)
                  //% "Tap to select"
                  : qsTrId("threesea-la-select-media-hint")
            color: Theme.highlightColor
            font.pixelSize: Theme.fontSizeSmall
        }
        Row {
            id: selectionButtons
            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                verticalCenter: parent.verticalCenter
            }
            spacing: Theme.paddingLarge
            IconButton {
                enabled: page.selectionHere
                icon.source: "image://theme/icon-m-downloads"
                onClicked: {
                    var ids = page.selectedIds()
                    page.endSelection()
                    window.messageOut.saveFiles(ids)
                }
            }
            IconButton {
                enabled: page.selectionHere
                icon.source: "image://theme/icon-m-share"
                onClicked: {
                    var ids = page.selectedIds()
                    page.endSelection()
                    window.messageOut.shareFiles(ids)
                }
            }
            IconButton {
                enabled: page.selectedCount > 0
                icon.source: "image://theme/icon-m-delete"
                onClicked: {
                    var rows = page.selectedRows()
                    page.endSelection()
                    selectionRemorse.execute(qsTrId("threesea-la-deleting-messages", rows.length), function() {
                        page.removeAll(rows)
                    })
                }
            }
        }
    }
}
