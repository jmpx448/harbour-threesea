// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import harbour.threesea 1.0
import "EmojiPickerData.js" as EmojiData
import "Errors.js" as Errors
import "Format.js" as Fmt

// Threema's emoji picker in place of the keyboard (Android's EmojiPicker), looking like the
// keyboard: its tinted background and top line, tabs as keys. Tabs: recently used, Threema's
// categories, the favourite GIFs; backspace at the right (repeats while held). Long press an
// emoji for its skin tones (a context menu, like a bubble's reactions). Threema's pictures always
// (the "emoji style" setting is about how text is drawn; the picker shows Threema's collection).
Item {
    id: panel

    signal emojiPicked(string emoji)
    signal backspace()
    signal gifPicked(var id)
    // Swiped down at the top of its content, like the keyboard (the chat closes the panel).
    signal closeRequested()

    // A finger moved down this far from where it pressed, with the content at its top (Silica's
    // grids don't take that drag, so the item under the finger sees it): close.
    property real pressY
    function pressed(item, mouse) {
        pressY = item.mapToItem(panel, mouse.x, mouse.y).y
    }
    function moved(item, mouse, view) {
        if ((!view || view.atYBeginning)
                && item.mapToItem(panel, mouse.x, mouse.y).y - pressY > Theme.itemSizeSmall) {
            closeRequested()
        }
    }

    // Picking a reaction (a bubble's menu → more emoji): emoji only, no GIF tab, no backspace.
    property bool reactionMode: false

    // -1: recently used, 0..8: the categories, 9: GIFs.
    property int tab: 0
    property var recent: []
    property var gifs: []

    readonly property int gifTab: EmojiData.names.length

    // Each chat has a panel, most are never opened: the emoji list is read at the first opening.
    property bool opened: false

    function load() {
        opened = true
        // Opens on the recently used, if any (set once: picking must not switch tabs).
        backend.call("recentEmoji", {}, function(ok, result) {
            if (!ok) return
            panel.recent = result.emoji
            if (panel.tab !== panel.gifTab || panel.reactionMode) {
                panel.tab = result.emoji.length > 0 ? -1 : Math.max(Math.min(panel.tab, panel.gifTab - 1), 0)
            }
        })
        backend.call("gifFavourites", {}, function(ok, result) { if (ok) panel.setGifs(result.favourites) })
    }

    // The grid builds its tiles anew from a new list: only for a list that did change, and where
    // the view was, so removing one of many GIFs doesn't throw you back to the first.
    function setGifs(list) {
        if (JSON.stringify(list) === JSON.stringify(gifs)) return
        var position = gifGrid.contentY
        gifs = list
        gifGrid.contentY = position
        gifGrid.returnToBounds()
    }

    function pick(emoji) {
        emojiPicked(emoji)
        backend.call("useEmoji", { emoji: emoji }, function(ok, result) {
            // Keep the recent tab as it is while it shows (it would jump under the finger).
            if (ok && panel.tab !== -1) panel.recent = result.emoji
        })
    }

    // Here, not in the context menu: its scope is gone when the remorse runs out.
    function removeGif(tile, id) {
        tile.remorseDelete(function() {
            backend.call("removeGifFavourite", { favourite: id }, function(ok, result) {
                if (ok) panel.setGifs(result.favourites)
            })
        })
    }

    // The theme's icon where one fits, else ours (scripts/make-ui-icons.py).
    readonly property var themeIcons: ({ recent: "history", people: "contact", activity: "game-controller", travel: "car" })
    function iconSource(name) {
        return themeIcons[name] ? "image://theme/icon-m-" + themeIcons[name]
                                : Qt.resolvedUrl("../icons/emoji-" + name + ".png")
    }

    // The grid's items: [text, cell, tones?].
    readonly property var items: {
        if (!opened) return []
        if (tab === -1) {
            var list = []
            for (var i = 0; i < recent.length; ++i) {
                var ranges = Fmt.emojiRanges(recent[i])
                if (ranges.length === 1) list.push([recent[i], ranges[0].cell])
            }
            return list
        }
        if (tab >= 0 && tab < gifTab) return EmojiData.items(tab)
        return []
    }

    Component {
        id: gifPicker
        ImagePickerPage {
            //% "Add GIF"
            title: qsTrId("threesea-he-add-gif")
            onSelectedContentPropertiesChanged: backend.addGifFavourite(selectedContentProperties.filePath,
                                                                        function(ok, result, kind, message) {
                if (ok) {
                    panel.setGifs(result.favourites)
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        }
    }

    // The keyboard's background: the highlight colour over the page, a little stronger towards
    // the bottom, under a line in the highlight colour.
    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0.0; color: Theme.rgba(Theme.highlightBackgroundColor, 0.3) }
            GradientStop { position: 1.0; color: Theme.rgba(Theme.highlightBackgroundColor, 0.42) }
        }
    }
    Rectangle {
        id: topLine
        width: parent.width
        height: Math.round(Theme.paddingSmall)
        color: Theme.highlightBackgroundColor
    }

    // Tabs as keyboard keys: the current one on a key's background.
    Row {
        id: tabs
        anchors.top: topLine.bottom
        width: parent.width
        height: Theme.itemSizeSmall
        readonly property real cellWidth: Math.floor(width / (EmojiData.names.length
                                                             + (panel.reactionMode ? 1 : 3)))
        readonly property real iconSize: Math.min(Theme.iconSizeSmallPlus, Math.round(cellWidth * 0.7))

        Repeater {
            model: EmojiData.names.length + (panel.reactionMode ? 1 : 2)
            MouseArea {
                id: tabKey
                readonly property int tabIndex: index - 1
                readonly property bool current: panel.tab === tabIndex
                width: tabs.cellWidth
                height: tabs.height
                onPressed: panel.pressed(tabKey, mouse)
                onPositionChanged: panel.moved(tabKey, mouse, null)
                onClicked: {
                    panel.tab = tabIndex
                    grid.positionViewAtBeginning()
                }
                Rectangle {
                    anchors {
                        fill: parent
                        topMargin: Theme.paddingSmall
                        bottomMargin: Theme.paddingSmall
                        leftMargin: Theme.paddingSmall / 2
                        rightMargin: Theme.paddingSmall / 2
                    }
                    radius: Theme.paddingSmall
                    visible: tabKey.pressed || tabKey.current
                    color: tabKey.pressed ? Theme.highlightBackgroundColor : Theme.primaryColor
                    opacity: tabKey.pressed ? 0.6 : 0.17
                }
                Icon {
                    anchors.centerIn: parent
                    width: tabs.iconSize
                    height: width
                    sourceSize.width: width
                    sourceSize.height: height
                    highlighted: tabKey.current
                    source: panel.iconSource(tabKey.tabIndex === -1 ? "recent"
                                             : tabKey.tabIndex === panel.gifTab ? "gif"
                                             : EmojiData.names[tabKey.tabIndex])
                }
            }
        }
        // Backspace, repeating while held like the keyboard's.
        MouseArea {
            id: backspaceKey
            visible: !panel.reactionMode
            width: parent.width - (EmojiData.names.length + 2) * tabs.cellWidth
            height: tabs.height
            onPressed: {
                panel.backspace()
                repeat.interval = 500
                repeat.start()
            }
            onReleased: repeat.stop()
            onCanceled: repeat.stop()
            Timer {
                id: repeat
                repeat: true
                onTriggered: {
                    interval = 80
                    panel.backspace()
                }
            }
            Icon {
                anchors.centerIn: parent
                width: tabs.iconSize
                height: width
                sourceSize.width: width
                sourceSize.height: height
                highlighted: backspaceKey.pressed
                source: "image://theme/icon-m-backspace"
            }
        }
    }

    SilicaGridView {
        id: grid
        visible: panel.tab !== panel.gifTab
        anchors {
            top: tabs.bottom
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        clip: true
        readonly property real cell: Math.floor(width / Math.max(1, Math.floor(width / Theme.itemSizeSmall)))
        cellWidth: cell
        cellHeight: cell
        model: panel.items
        delegate: GridItem {
            id: emojiItem
            readonly property var tones: modelData.length > 2 ? modelData[2] : null
            width: grid.cellWidth
            contentHeight: grid.cellHeight
            onClicked: panel.pick(modelData[0])
            onPressed: panel.pressed(emojiItem, mouse)
            onPositionChanged: panel.moved(emojiItem, mouse, grid)
            menu: tones ? toneMenu : null
            Image {
                anchors.centerIn: parent
                width: Theme.iconSizeMedium
                height: width
                sourceSize.width: width
                sourceSize.height: height
                asynchronous: true
                source: "image://emoji/" + modelData[1]
            }
            Component {
                id: toneMenu
                ContextMenu {
                    id: menu
                    Row {
                        anchors.horizontalCenter: parent.horizontalCenter
                        Repeater {
                            model: [[modelData[0], modelData[1]]].concat(emojiItem.tones)
                            BackgroundItem {
                                width: Math.min(Theme.itemSizeMedium, Math.floor(grid.width / 6))
                                height: Theme.itemSizeSmall
                                onClicked: {
                                    panel.pick(modelData[0])
                                    menu.close()
                                }
                                Image {
                                    anchors.centerIn: parent
                                    width: Theme.iconSizeMedium
                                    height: width
                                    sourceSize.width: width
                                    sourceSize.height: height
                                    source: "image://emoji/" + modelData[1]
                                }
                            }
                        }
                    }
                }
            }
        }
        VerticalScrollDecorator { }
    }

    SilicaGridView {
        id: gifGrid
        visible: panel.tab === panel.gifTab && panel.gifs.length > 0
        anchors.fill: grid
        clip: true
        readonly property real cell: Math.floor(width / Math.max(1, Math.floor(width / Theme.itemSizeHuge)))
        cellWidth: cell
        cellHeight: cell
        // The last tile adds one from the device.
        model: visible ? panel.gifs.concat([null]) : []
        delegate: GridItem {
            id: tile
            width: gifGrid.cellWidth
            contentHeight: gifGrid.cellHeight
            onClicked: modelData ? panel.gifPicked(modelData.id) : pageStack.push(gifPicker)
            menu: modelData ? removeMenu : null
            onPressed: panel.pressed(tile, mouse)
            onPositionChanged: panel.moved(tile, mouse, gifGrid)
            Component {
                id: removeMenu
                ContextMenu {
                    MenuItem {
                        //% "Remove"
                        text: qsTrId("threesea-me-remove-gif")
                        onClicked: panel.removeGif(tile, modelData.id)
                    }
                }
            }
            Icon {
                visible: !modelData
                anchors.centerIn: parent
                source: "image://theme/icon-m-add"
                highlighted: tile.highlighted
            }
            // Until the GIF plays over it.
            Image {
                visible: !!modelData && !animation.ready
                anchors.fill: parent
                anchors.margins: Theme.paddingSmall / 2
                fillMode: Image.PreserveAspectCrop
                clip: true
                asynchronous: true
                cache: false
                sourceSize.width: gifGrid.cell
                sourceSize.height: gifGrid.cell
                source: modelData ? "image://threesea/gif/" + modelData.id : ""
                opacity: tile.highlighted ? Theme.opacityHigh : 1
            }
            // Only what is on screen plays, and nothing while the grid moves: the thumbnails
            // are there for that, and a list of many GIFs stays as quick as a list of a few.
            AnimatedMedia {
                id: animation
                anchors.fill: parent
                anchors.margins: Theme.paddingSmall / 2
                visible: !!modelData
                favourite: modelData ? modelData.id : -1
                onDemand: true
                playing: backend.autoPlayGifs && Qt.application.state === Qt.ApplicationActive
                         && !gifGrid.moving
                         && tile.y + tile.height > gifGrid.contentY
                         && tile.y < gifGrid.contentY + gifGrid.height
                opacity: tile.highlighted ? Theme.opacityHigh : 1
            }
        }
        VerticalScrollDecorator { }
    }

    // Nothing to show: centred in the panel (ViewPlaceholder centres on the page).
    Column {
        anchors {
            left: parent.left
            right: parent.right
            verticalCenter: grid.verticalCenter
        }
        spacing: Theme.paddingLarge
        visible: (panel.tab === -1 && grid.count === 0) || (panel.tab === panel.gifTab && panel.gifs.length === 0)
        InfoLabel {
            text: panel.tab === panel.gifTab
                  //% "No favourite GIFs"
                  ? qsTrId("threesea-la-no-gif-favourites")
                  //% "No emoji used yet"
                  : qsTrId("threesea-la-no-recent-emoji")
        }
        Label {
            visible: panel.tab === panel.gifTab
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * x
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeMedium
            color: Theme.secondaryHighlightColor
            //% "Long press a GIF in a chat to add it, or add one from this device."
            text: qsTrId("threesea-la-no-gif-favourites-hint")
        }
        Button {
            visible: panel.tab === panel.gifTab
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTrId("threesea-he-add-gif")
            onClicked: pageStack.push(gifPicker)
        }
    }
}
