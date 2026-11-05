// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Pages.js" as Pages

// The app started and ready; the page opened and given time to get its answers. Then all rows of
// its lists are built (the screen here is small) and each row's context menu is opened once. The
// page is closed again at the end, for what it does on its way out.
QtObject {
    id: root

    readonly property var stack: window.pageStack
    readonly property var backend: fakes.backend
    property var page: null
    property int step: 0
    property var rows: []
    property var openRow: null

    function collect(item, wanted, into) {
        if (wanted(item)) into.push(item)
        var children = item.children || []
        for (var i = 0; i < children.length; ++i) collect(children[i], wanted, into)
        return into
    }

    readonly property var steps: [
        [function() {
            harness.setActive(true)
            backend.unlockedAtStart = true
            backend.state = "ready"
        }, 600],
        [function() {
            Pages.prepare(pagePath, fakes)
            page = stack.push(Qt.resolvedUrl("../qml/" + pagePath), Pages.properties(pagePath),
                              PageStackAction.Immediate)
            if (!page) console.warn("could not be opened:", pagePath)
        }, 800],
        [function() {
            if (stack.currentPage !== page) return
            var lists = collect(page, function(item) { return item.cacheBuffer !== undefined }, [])
            for (var i = 0; i < lists.length; ++i) lists[i].cacheBuffer = 20000
        }, 400],
        [function() {
            if (stack.currentPage !== page) return
            rows = collect(page, function(item) { return item.openMenu !== undefined && item.closeMenu !== undefined && !!item.menu }, [])
            menus.start()
        }, 1],
    ]
    readonly property var finishing: [
        [function() { if (stack.currentPage === page) stack.pop(undefined, PageStackAction.Immediate) }, 400],
    ]

    readonly property var menus: Timer {
        interval: 60
        repeat: true
        onTriggered: {
            if (root.openRow) root.openRow.closeMenu()
            root.openRow = null
            if (root.rows.length === 0) {
                stop()
                return
            }
            root.openRow = root.rows.shift()
            root.openRow.openMenu()
        }
    }

    readonly property var runner: Timer {
        running: true
        interval: 300
        onTriggered: {
            if (menus.running) {
                restart()
                return
            }
            var all = steps.concat(finishing)
            if (step >= all.length) {
                harness.finish()
                return
            }
            var current = all[step]
            step = step + 1
            current[0]()
            interval = Math.max(current[1], 1)
            restart()
        }
    }
}
