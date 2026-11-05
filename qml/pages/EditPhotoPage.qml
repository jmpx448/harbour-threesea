// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// Cut and turn a photo before it is sent (Android's editor, without the drawing). Accepting
// writes a new picture into the app's capture folder and hands back its path; the original is
// never changed.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    // The photo to work on, and the result once the dialog was accepted.
    property string path
    property string editedPath

    // Turned clockwise, in steps of 90° (`rotation` itself is Item's own).
    property int turn: 0
    readonly property bool sideways: turn % 180 !== 0

    canAccept: frame.width > Theme.itemSizeExtraSmall && frame.height > Theme.itemSizeExtraSmall

    onAccepted: {
        // The frame's place inside the picture as it is shown, as fractions of it.
        var crop = Qt.rect(frame.x / view.width, frame.y / view.height,
                           frame.width / view.width, frame.height / view.height)
        dialog.editedPath = backend.editPhoto(dialog.path, crop, dialog.turn)
    }

    // The whole picture again.
    function reset() {
        frame.x = 0
        frame.y = 0
        frame.width = view.width
        frame.height = view.height
    }

    SilicaFlickable {
        anchors.fill: parent
        interactive: false

        PullDownMenu {
            MenuItem {
                //% "Turn right"
                text: qsTrId("threesea-me-photo-turn-right")
                onClicked: { dialog.turn = (dialog.turn + 90) % 360; dialog.reset() }
            }
            MenuItem {
                //% "Turn left"
                text: qsTrId("threesea-me-photo-turn-left")
                onClicked: { dialog.turn = (dialog.turn + 270) % 360; dialog.reset() }
            }
            MenuItem {
                //% "Whole picture"
                text: qsTrId("threesea-me-photo-whole")
                onClicked: dialog.reset()
            }
        }

        DialogHeader {
            id: header
            //% "Use"
            acceptText: qsTrId("threesea-he-photo-use")
            //% "Edit photo"
            title: qsTrId("threesea-he-edit-photo")
        }

        Item {
            id: area
            anchors {
                top: header.bottom
                left: parent.left
                right: parent.right
                bottom: parent.bottom
                topMargin: Theme.paddingLarge
                bottomMargin: Theme.paddingLarge
                // Wide enough that a grip never sits where a swipe closes the dialog.
                leftMargin: Theme.itemSizeSmall
                rightMargin: Theme.itemSizeSmall
            }

            // Where the turned picture really sits: the area, in the picture's proportions.
            // Everything below lives inside it, so the frame is in the picture's own
            // coordinates and cannot drift away from it.
            Item {
                id: view
                readonly property real ratio: {
                    var w = photo.implicitWidth || 1
                    var h = photo.implicitHeight || 1
                    return dialog.sideways ? h / w : w / h
                }
                width: Math.min(area.width, area.height * ratio)
                height: Math.min(area.height, area.width / ratio)
                anchors.centerIn: parent
                onWidthChanged: dialog.reset()
                onHeightChanged: dialog.reset()

                Image {
                    id: photo
                    anchors.centerIn: parent
                    width: dialog.sideways ? view.height : view.width
                    height: dialog.sideways ? view.width : view.height
                    rotation: dialog.turn
                    source: dialog.path ? Fmt.fileUrl(dialog.path) : ""
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    // As the camera stored it: QML's Image leaves the EXIF rotation alone
                    // unless asked. The cut is made on the turned picture (editPhoto), and
                    // what is sent is turned too (FilePreparer), so the view must be.
                    autoTransform: true
                    // Shown no larger than the screen; the cut is made from the original.
                    sourceSize.width: Screen.width
                    sourceSize.height: Screen.height
                    onStatusChanged: if (status === Image.Ready) dialog.reset()
                }

                // Everything outside the frame is dimmed: left, right, above, below.
                Repeater {
                    model: 4
                    Rectangle {
                        color: Qt.rgba(0, 0, 0, 0.5)
                        x: index === 1 ? frame.x + frame.width : index === 0 ? 0 : frame.x
                        y: index < 2 ? 0 : index === 2 ? 0 : frame.y + frame.height
                        width: index === 0 ? frame.x
                             : index === 1 ? view.width - frame.x - frame.width
                                           : frame.width
                        height: index < 2 ? view.height
                              : index === 2 ? frame.y
                                            : view.height - frame.y - frame.height
                        visible: width > 0 && height > 0
                    }
                }

                Rectangle {
                    id: frame
                    color: "transparent"
                    border { width: 2; color: Theme.highlightColor }

                    MouseArea {
                        anchors.fill: parent
                        // Or the dialog's own left/right swipe (accept, cancel) takes the drag
                        // away after a few pixels and the frame stops following the finger.
                        preventStealing: true
                        drag {
                            target: frame
                            minimumX: 0
                            maximumX: Math.max(0, view.width - frame.width)
                            minimumY: 0
                            maximumY: Math.max(0, view.height - frame.height)
                        }
                    }
                }

                // One grip per corner; dragging moves that corner only.
                Repeater {
                    model: 4
                    Item {
                        readonly property bool atRight: index === 1 || index === 2
                        readonly property bool atBottom: index >= 2
                        width: Theme.itemSizeSmall
                        height: Theme.itemSizeSmall
                        x: (atRight ? frame.x + frame.width : frame.x) - width / 2
                        y: (atBottom ? frame.y + frame.height : frame.y) - height / 2

                        Rectangle {
                            anchors.centerIn: parent
                            width: Theme.paddingLarge
                            height: Theme.paddingLarge
                            radius: width / 2
                            color: Theme.highlightColor
                            opacity: Theme.opacityHigh
                        }

                        MouseArea {
                            anchors.fill: parent
                            // As above: the dialog must not take this drag.
                            preventStealing: true
                            // The corner goes where the finger is, in the picture's own
                            // coordinates: the grip moves with the frame, so a drag measured
                            // inside the grip would chase its own tail and lag behind.
                            onPositionChanged: {
                                var point = mapToItem(view, mouse.x, mouse.y)
                                var least = Theme.itemSizeExtraSmall
                                if (parent.atRight) {
                                    var right = Math.max(frame.x + least,
                                                         Math.min(view.width, point.x))
                                    frame.width = right - frame.x
                                } else {
                                    var left = Math.max(0, Math.min(
                                        frame.x + frame.width - least, point.x))
                                    frame.width += frame.x - left
                                    frame.x = left
                                }
                                if (parent.atBottom) {
                                    var bottom = Math.max(frame.y + least,
                                                          Math.min(view.height, point.y))
                                    frame.height = bottom - frame.y
                                } else {
                                    var top = Math.max(0, Math.min(
                                        frame.y + frame.height - least, point.y))
                                    frame.height += frame.y - top
                                    frame.y = top
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
