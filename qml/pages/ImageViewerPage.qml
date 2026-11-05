// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../components" as Components
import "../components/Errors.js" as Errors
import "../components/Format.js" as Fmt

// A chat's photos, GIFs and videos, full screen on black like Jolla Gallery: swipe sideways to the
// next one (past the last comes the first again, like Gallery), down to go back, pinch or double
// tap to zoom, tap to show or hide the actions. A video plays on its own page. Whatever the
// gallery's saved filter says, this shows all three kinds.
// Without `identity`, one picture of ours with no message behind it (`source`).
FullscreenContentPage {
    id: page

    // The message opened and its chat.
    property int messageId
    property string identity
    // It is a GIF. For when the chat's list can't be had; otherwise the list says.
    property bool animated
    // Opened from the gallery, whose grid runs that way; the chat's order otherwise.
    property bool newestFirst
    // Or any picture of ours (a profile or group picture: image://threesea/contact/…).
    property string source

    readonly property bool single: source.length > 0
    readonly property var viewedKinds: ["image", "gif", "video"]
    property bool listReady
    property int startIndex
    property bool overlayShown: true
    property var rowOf: ({})
    readonly property var currentItem: slideshow.item ? slideshow.item.currentItem : null
    readonly property bool zoomed: currentItem !== null && currentItem.zoomed

    allowedOrientations: Orientation.All
    backNavigation: !zoomed
    showNavigationIndicator: !zoomed && overlayShown

    function row(message) {
        return { id: message.id, kind: message.mediaKind, body: JSON.stringify(message.body) }
    }

    // The list comes whole, before the view is made: the view goes round in a circle, so with only
    // part of the chat in it, the end of that part would lead to the wrong picture.
    function load(before, found) {
        var args = { identity: identity, kinds: viewedKinds, limit: 500 }
        if (before) args.before = before
        backend.call("chatMedia", args, function(ok, result) {
            if (!ok) {
                show(found)
                return
            }
            for (var i = 0; i < result.messages.length; ++i) {
                var message = result.messages[i]
                if (message.mediaKind === "video" || Fmt.isPicture(message.body.mediaType)) found.push(row(message))
            }
            if (result.next) page.load(result.next, found)
            else page.show(found)
        })
    }

    // `found`: newest first. The message opened is shown in any case.
    function show(found) {
        var rows = newestFirst ? found : found.reverse()
        var start = -1
        for (var i = 0; i < rows.length && start < 0; ++i) {
            if (rows[i].id === messageId) start = i
        }
        if (start < 0) {
            rows = [{ id: messageId, kind: animated ? "gif" : "image", body: JSON.stringify({ fileState: 2 }) }]
            start = 0
        }
        for (var j = 0; j < rows.length; ++j) media.append(rows[j])
        indexRows()
        startIndex = start
        listReady = true
    }

    function indexRows() {
        var rows = {}
        for (var i = 0; i < media.count; ++i) rows[media.get(i).id] = i
        rowOf = rows
    }

    // One message changed: its row takes that over, or goes with it.
    function applyChange(message) {
        var row = rowOf[message.id]
        if (row === undefined) return
        if (message.kind !== "file") {
            media.remove(row)
            indexRows()
            if (media.count === 0) pageStack.pop(pageStack.previousPage(page), PageStackAction.Immediate)
            return
        }
        var body = JSON.stringify(message.body)
        if (media.get(row).body !== body) media.setProperty(row, "body", body)
    }

    // New ones are the newest ones: those above the newest row known.
    function addNew() {
        backend.call("chatMedia", { identity: identity, kinds: viewedKinds, limit: 30 }, function(ok, result) {
            if (!ok) return
            var added = []
            for (var i = 0; i < result.messages.length; ++i) {
                var message = result.messages[i]
                if (page.rowOf[message.id] !== undefined) break
                if (message.mediaKind === "video" || Fmt.isPicture(message.body.mediaType)) added.push(page.row(message))
            }
            for (var j = added.length - 1; j >= 0; --j) {
                if (page.newestFirst) media.insert(0, added[j])
                else media.append(added[j])
            }
            page.indexRows()
        })
    }

    function act(what) {
        if (!currentItem) return
        if (what === "chat") showInChat(currentItem.message)
        else save(currentItem.message, false)
    }

    function save(id, open) {
        backend.call("saveFile", { message: id, directory: StandardPaths.download },
                     function(ok, result, kind, message) {
            if (!ok) {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            } else {
                Notices.show(qsTrId("threesea-la-saved-downloads"), Notice.Short, Notice.Center)
                if (open && result.openable) Qt.openUrlExternally(Fmt.fileUrl(result.path))
            }
        })
    }

    // The chat this was opened from if it is still there: back to it, at that message.
    function showInChat(id) {
        var chat = pageStack.find(function(p) { return p.objectName === "chat" && p.identity === page.identity })
        if (chat) {
            chat.showMessage(id)
            pageStack.pop(chat)
        } else {
            pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: page.identity, focusMessage: id })
        }
    }

    // Only on being asked for, and only the one showing: the player is shared by the whole app.
    function playVideo(id) {
        if (videoPlayer.available) pageStack.push(Qt.resolvedUrl("VideoPlayerPage.qml"), { messageId: id })
        else save(id, true)
    }

    Component.onCompleted: {
        if (single) {
            media.append({ id: -1, kind: "image", body: JSON.stringify({ fileState: 2 }) })
            listReady = true
        } else {
            load("", [])
        }
    }

    ListModel {
        id: media
    }

    // A file arriving is several events.
    Timer {
        id: newArrived
        interval: 500
        onTriggered: page.addNew()
    }

    Connections {
        target: page.single ? null : backend
        onCoreEvent: {
            if (name === "messageAdded") {
                if (data.message.conversation === page.identity && data.message.kind === "file") newArrived.restart()
            } else if (name === "messagesChanged") {
                for (var i = 0; i < data.messages.length; ++i) {
                    if (data.messages[i].conversation === page.identity) page.applyChange(data.messages[i])
                }
            }
        }
    }

    // Made once the list is there, so that it starts at the message opened without passing others.
    Loader {
        id: slideshow
        anchors.fill: parent
        active: page.listReady
        sourceComponent: SlideshowView {
            id: view
            model: media
            currentIndex: page.startIndex
            interactive: count > 1 && !page.zoomed

            delegate: Item {
                id: item

                readonly property int message: model.id
                readonly property bool current: PathView.isCurrentItem
                readonly property var body: JSON.parse(model.body)
                readonly property bool isVideo: model.kind === "video"
                readonly property bool isGif: model.kind === "gif"
                readonly property bool here: body.fileState === 2
                readonly property string fileSource: page.single ? page.source : "image://threesea/" + model.id + "/file"
                // A photo that is here can be looked at closely; everything else is just shown.
                readonly property bool zoomable: here && !isVideo
                readonly property bool zoomed: flickable.contentWidth > flickable.width + 1

                width: view.width
                height: view.height

                onCurrentChanged: {
                    if (!current && zoomed) {
                        flickable.resizeContent(flickable.width, flickable.height, Qt.point(0, 0))
                        flickable.returnToBounds()
                    }
                }

                SilicaFlickable {
                    id: flickable
                    anchors.fill: parent
                    contentWidth: width
                    contentHeight: height
                    boundsBehavior: Flickable.StopAtBounds
                    // Not zoomed, its drags are the slideshow's and the page's.
                    interactive: item.zoomed
                    clip: item.zoomed

                    PinchArea {
                        width: Math.max(flickable.contentWidth, flickable.width)
                        height: Math.max(flickable.contentHeight, flickable.height)
                        enabled: item.zoomable
                        property real startWidth
                        property real startHeight
                        onPinchStarted: {
                            startWidth = flickable.contentWidth
                            startHeight = flickable.contentHeight
                        }
                        onPinchUpdated: {
                            var scale = Math.max(1, Math.min(4 * flickable.width / startWidth, pinch.scale))
                            flickable.resizeContent(startWidth * scale, startHeight * scale, pinch.center)
                        }
                        onPinchFinished: flickable.returnToBounds()

                        Image {
                            id: image
                            anchors.fill: parent
                            fillMode: Image.PreserveAspectFit
                            asynchronous: true
                            cache: false
                            // The neighbours are decoded too, ready for the swipe: at screen size.
                            sourceSize.width: Math.max(Screen.width, Screen.height)
                            sourceSize.height: Math.max(Screen.width, Screen.height)
                            source: item.zoomable ? item.fileSource
                                    : item.body.thumbnailState === 2 ? "image://threesea/" + model.id + "/thumb" : ""

                            // Zoomed in: up to twice the screen size, over it once decoded.
                            Image {
                                anchors.fill: parent
                                fillMode: Image.PreserveAspectFit
                                asynchronous: true
                                cache: false
                                visible: status === Image.Ready
                                sourceSize.width: 2 * Math.max(Screen.width, Screen.height)
                                sourceSize.height: 2 * Math.max(Screen.width, Screen.height)
                                source: item.zoomed && !item.isGif ? item.fileSource : ""
                            }

                            AnimatedMedia {
                                id: animation
                                // The picture's area within the image item (PreserveAspectFit).
                                anchors.centerIn: parent
                                width: image.paintedWidth
                                height: image.paintedHeight
                                visible: item.isGif && ready
                                message: item.isGif && item.here && item.current ? model.id : -1
                                playing: Qt.application.state === Qt.ApplicationActive
                            }

                            MouseArea {
                                anchors.fill: parent
                                onClicked: {
                                    if (item.zoomable) tap.restart()
                                    else page.overlayShown = !page.overlayShown
                                }
                                onDoubleClicked: {
                                    if (!item.zoomable) return
                                    tap.stop()
                                    if (item.zoomed) {
                                        flickable.resizeContent(flickable.width, flickable.height, Qt.point(mouse.x, mouse.y))
                                    } else {
                                        flickable.resizeContent(flickable.width * 2.5, flickable.height * 2.5,
                                                                Qt.point(mouse.x, mouse.y))
                                    }
                                    flickable.returnToBounds()
                                }
                                // A single tap only once it can't be the start of a double one.
                                Timer {
                                    id: tap
                                    interval: 300
                                    onTriggered: page.overlayShown = !page.overlayShown
                                }
                            }
                        }
                    }
                }

                BusyIndicator {
                    anchors.centerIn: parent
                    size: BusyIndicatorSize.Large
                    running: item.current && (image.status === Image.Loading || item.body.fileState === 1)
                }

                // On the device: a video plays; not yet: it comes on being asked for.
                IconButton {
                    anchors.centerIn: parent
                    visible: item.body.fileState !== 1 && (item.isVideo || !item.here)
                    icon.source: "image://theme/" + (!item.here ? Fmt.fileIcon("", item.body.fileState) : "icon-l-play")
                    onClicked: {
                        if (item.here) page.playVideo(model.id)
                        else backend.call("downloadFile", { message: model.id })
                    }
                }

                InfoLabel {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: item.zoomable && image.status === Image.Error && !animation.visible
                    //% "This picture could not be shown"
                    text: qsTrId("threesea-la-cannot-show-picture")
                }
            }
        }
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: !page.listReady
    }

    Item {
        anchors.fill: parent
        visible: opacity > 0 && !page.single
        opacity: page.overlayShown && page.currentItem !== null ? 1.0 : 0.0
        Behavior on opacity { FadeAnimation { } }

        // Like Gallery's: a shade under the actions, for pictures with something bright there.
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: actions.height + place.height + 2 * Theme.paddingLarge
            gradient: Gradient {
                GradientStop { position: 0.0; color: "transparent" }
                GradientStop { position: 1.0; color: Theme.rgba("black", Theme.opacityOverlay) }
            }
        }
        Label {
            id: place
            anchors {
                bottom: actions.top
                horizontalCenter: parent.horizontalCenter
            }
            visible: media.count > 1
            height: visible ? implicitHeight : 0
            color: Theme.lightPrimaryColor
            font.pixelSize: Theme.fontSizeExtraSmall
            //: The photo or video shown, and how many the chat has. %1 is its number, %2 how many
            //% "%1 of %2"
            text: qsTrId("threesea-la-media-position").arg(slideshow.item ? slideshow.item.currentIndex + 1 : 0).arg(media.count)
        }
        Components.ActionRow {
            id: actions
            anchors.bottom: parent.bottom
            color: Theme.lightPrimaryColor
            actions: {
                var list = [{ id: "chat", icon: "icon-m-message", text: qsTrId("threesea-me-show-in-chat") }]
                if (page.currentItem && page.currentItem.here) {
                    list.push({ id: "save", icon: "icon-m-downloads", text: qsTrId("threesea-me-save-downloads") })
                }
                return list
            }
            onTriggered: page.act(id)
        }
    }
}
