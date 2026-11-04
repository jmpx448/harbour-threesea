// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

// A chat's media as a grid, newest first, like Jolla Gallery's (its ImageGridView is not
// Harbour-allowed, so the same layout with Silica). Like Android's gallery it filters by kind
// (photos, GIFs, videos, voice messages, audio, files; pull-down "Show", a dialog accepted by
// swipe, saved for all chats). Tap: view, play or open (downloading first if needed); long press:
// show in chat, save to Downloads, delete (on this device, with a remorse timer).
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

    function load() {
        backend.call("chatMedia", { identity: identity }, function(ok, result) {
            if (!ok) return
            page.kinds = result.kinds
            page.counts = result.counts
            media.update(result.messages)
        })
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

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "messageAdded" && data.message.conversation === page.identity) {
                page.load()
            } else if (name === "messagesChanged") {
                for (var i = 0; i < data.messages.length; ++i) {
                    if (data.messages[i].conversation === page.identity) return page.load()
                }
            }
        }
    }

    // Row-wise updates keep the grid's position (a reset would jump to the top). The body is kept
    // as JSON text: ListModel would turn an object into a nested model.
    ListModel {
        id: media

        function update(messages) {
            for (var i = 0; i < messages.length; ++i) {
                var message = { id: messages[i].id, kind: messages[i].mediaKind,
                                body: JSON.stringify(messages[i].body),
                                messageId: messages[i].messageId, outgoing: messages[i].outgoing }
                if (i < count) {
                    if (get(i).id !== message.id || get(i).body !== message.body || get(i).kind !== message.kind) {
                        set(i, message)
                    }
                } else {
                    append(message)
                }
            }
            if (count > messages.length) remove(messages.length, count - messages.length)
        }
    }

    function open(id, kind, body) {
        if (body.fileState !== 2) {
            if (body.fileState !== 1) backend.call("downloadFile", { message: id })
        } else if (kind === "image" || kind === "gif") {
            pageStack.push(Qt.resolvedUrl("ImageViewerPage.qml"), { messageId: id, animated: kind === "gif" })
        } else if (kind === "voice") {
            if (voicePlayer.messageId === id && voicePlayer.playing) {
                voicePlayer.pause()
            } else if (voicePlayer.play(id)) {
                window.voiceFrom = ""
            } else {
                Notices.show(qsTrId("threesea-la-cannot-play"), Notice.Long, Notice.Center)
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
                if (open && result.openable) Qt.openUrlExternally("file://" + result.path)
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
        model: media
        // Jolla Gallery's grid: as many columns as fit Theme.itemSizeHuge.
        readonly property real cellSize: Math.floor(width / Math.max(1, Math.floor(width / Theme.itemSizeHuge)))
        cellWidth: cellSize
        cellHeight: cellSize
        cacheBuffer: 1000

        // Only worth it with more than one kind.
        PullDownMenu {
            visible: page.presentKinds.length > 1
            MenuItem {
                //% "Show"
                text: qsTrId("threesea-me-media-show")
                onClicked: page.chooseKinds()
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
            readonly property bool isPicture: model.kind === "image" || model.kind === "gif"
            readonly property bool hasThumbnail: body.thumbnailState === 2
            readonly property bool playing: voicePlayer.messageId === model.id && voicePlayer.playing
            width: grid.cellWidth
            contentHeight: grid.cellHeight
            onClicked: page.open(model.id, model.kind, body)

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

            menu: Component {
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
}
