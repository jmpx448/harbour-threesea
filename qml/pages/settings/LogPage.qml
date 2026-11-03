// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"
import "../../components/Errors.js" as Errors

// Settings → Advanced → View log: what Threesea logged (fixed texts, kinds, codes and
// counts; never message content or addresses, CLAUDE.md §7.3). The pulley saves a support
// package (device facts plus this log) to Downloads, for reporting a problem.
Page {
    id: page
    allowedOrientations: Orientation.All

    property string text
    property bool loaded: false

    function refresh() {
        backend.call("logText", {}, function(ok, result) {
            if (ok) {
                page.text = result.text
                page.loaded = true
                scrollDown.restart()
            }
        })
    }
    Component.onCompleted: refresh()

    // To the newest lines once the text is laid out.
    Timer {
        id: scrollDown
        interval: 50
        onTriggered: flickable.scrollToBottom()
    }

    RemorsePopup { id: clearRemorse }

    SilicaFlickable {
        id: flickable
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            MenuItem {
                //% "Refresh"
                text: qsTrId("threesea-me-log-refresh")
                onClicked: page.refresh()
            }
        }

        // The page opens at the newest lines (the bottom), so the actions sit right there.
        PushUpMenu {
            MenuItem {
                //% "Save support package"
                text: qsTrId("threesea-me-log-save")
                onClicked: backend.call("logPackage", { directory: StandardPaths.download },
                                        function(ok, result, kind, message) {
                    Notices.show(ok
                                 //% "Saved to Downloads"
                                 ? qsTrId("threesea-la-saved-downloads")
                                 : Errors.text(kind, message), Notice.Short, Notice.Center)
                })
            }
            MenuItem {
                //% "Clear log"
                text: qsTrId("threesea-me-log-clear")
                //% "Clearing log"
                onClicked: clearRemorse.execute(qsTrId("threesea-la-clearing-log"), function() {
                    backend.call("logClear", {}, function() { page.refresh() })
                })
            }
        }

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Log"
                title: qsTrId("threesea-he-log")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                visible: page.text.length > 0
                text: page.text
                wrapMode: Text.WrapAnywhere
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.primaryColor
            }
        }

        ViewPlaceholder {
            enabled: page.loaded && page.text.length === 0
            //% "No entries"
            text: qsTrId("threesea-la-log-empty")
            //% "What Threesea logs appears here. Settings → Advanced sets how much that is."
            hintText: qsTrId("threesea-la-log-empty-hint")
        }

        VerticalScrollDecorator { }
    }
}
