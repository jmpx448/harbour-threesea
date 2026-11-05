// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components/Format.js" as Fmt

// Cut, turn and draw on a photo before it is sent (Android's editor and ImagePaintActivity, with a
// pen and text). Whoever opened it calls save() when it is accepted, for a new picture in the app's
// capture folder, or hold() for the picture of a message; the original is never changed. state()
// and `earlier` bring back where it was left, if the user takes back sending or leaving.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    // The photo to work on.
    property string path
    // Or the picture of this message (its id), which stays in memory: see send().
    property var message: -1
    readonly property bool ofMessage: message >= 0
    // A state() to go on from.
    property var earlier: null
    // The frame stays a square, and nothing is drawn (profile and group pictures).
    property bool square

    // What a touch on the picture does: "cut", "draw" or "text".
    property string tool: "cut"
    property color penColor: "#e60003"

    // What was drawn, in fractions of the picture before turning, so that it turns with the picture
    // and is the same at any resolution (PhotoEdit::edited). The lines; the texts are in `texts`.
    property var strokes: []
    // For undoing: "stroke" or "text", the latest last.
    property var history: []
    ListModel { id: texts }

    // Turned clockwise, in steps of 90° (`rotation` itself is Item's own).
    property int turn: 0
    readonly property bool sideways: turn % 180 !== 0

    // The frame in fractions of the picture as it is shown, which stay when the phone is turned.
    property var kept: ({ x: 0, y: 0, width: 1, height: 1 })
    // Sending a picture that nothing was done to would only be a copy of it.
    readonly property bool changed: history.length > 0 || turn !== 0
                                    || kept.x !== 0 || kept.y !== 0 || kept.width !== 1 || kept.height !== 1

    canAccept: photo.status === Image.Ready && frame.width > 0 && frame.height > 0 && (!ofMessage || changed)

    Component.onCompleted: {
        if (!earlier) return
        strokes = earlier.strokes
        history = earlier.history
        turn = earlier.turn
        tool = earlier.tool
        penColor = earlier.penColor
        for (var i = 0; i < earlier.texts.length; ++i) texts.append(earlier.texts[i])
    }

    function state() {
        var list = []
        for (var i = 0; i < texts.count; ++i) {
            var text = texts.get(i)
            list.push({ text: text.text, x: text.x, y: text.y, color: text.color, size: text.size,
                        angle: text.angle })
        }
        return { strokes: strokes, texts: list, history: history, turn: turn, tool: tool,
                 penColor: "" + penColor, crop: kept }
    }

    // The frame's place inside the picture as it is shown, as fractions of it.
    function crop() {
        return Qt.rect(frame.x / view.width, frame.y / view.height,
                       frame.width / view.width, frame.height / view.height)
    }

    function marks() {
        var list = strokes.slice()
        for (var i = 0; i < texts.count; ++i) {
            var text = texts.get(i)
            list.push({ text: text.text, x: text.x, y: text.y, color: text.color, size: text.size,
                        angle: text.angle, family: Theme.fontFamily })
        }
        return list
    }

    // See ThreeseBackend::editPhoto for `callback`.
    function save(callback) {
        backend.editPhoto(dialog.path, crop(), dialog.turn, marks(), callback)
    }

    // See ThreeseBackend::holdPicture.
    function hold() {
        return backend.holdPicture(dialog.message, crop(), dialog.turn, marks())
    }

    function undo() {
        var kinds = history.slice()
        if (kinds.pop() === "text") {
            texts.remove(texts.count - 1)
        } else {
            var lines = strokes.slice()
            lines.pop()
            strokes = lines
            drawing.requestPaint()
        }
        history = kinds
    }

    // Where a touch on the picture is, in fractions of it.
    function fraction(value, whole) {
        return Math.max(0, Math.min(1, value / whole))
    }

    function addText(x, y) {
        var editor = pageStack.push(Qt.resolvedUrl("PhotoTextDialog.qml"))
        editor.accepted.connect(function() {
            texts.append({ text: editor.text.trim(), x: x, y: y, color: "" + dialog.penColor,
                           size: Theme.fontSizeLarge / Math.max(picture.width, picture.height),
                           // Upright as the picture lies now.
                           angle: (360 - dialog.turn) % 360 })
            dialog.history = dialog.history.concat(["text"])
        })
    }

    // The whole picture again, or the largest square in its middle.
    function reset() {
        var side = Math.min(view.width, view.height)
        var width = square && view.width > 0 ? side / view.width : 1
        var height = square && view.height > 0 ? side / view.height : 1
        kept = { x: (1 - width) / 2, y: (1 - height) / 2, width: width, height: height }
        applyFrame()
    }

    function applyFrame() {
        frame.x = kept.x * view.width
        frame.y = kept.y * view.height
        frame.width = kept.width * view.width
        frame.height = kept.height * view.height
    }

    // After a finger moved the frame.
    function rememberFrame() {
        kept = crop()
    }

    SilicaFlickable {
        anchors.fill: parent

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
                visible: !dialog.square
                //% "Whole picture"
                text: qsTrId("threesea-me-photo-whole")
                onClicked: dialog.reset()
            }
        }

        DialogHeader {
            id: header
            acceptText: dialog.ofMessage
                        ? qsTrId("threesea-he-send")
                        //% "Use"
                        : qsTrId("threesea-he-photo-use")
            //% "Edit photo"
            title: qsTrId("threesea-he-edit-photo")
        }

        Item {
            id: area
            anchors {
                top: header.bottom
                left: parent.left
                right: parent.right
                bottom: tools.top
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
                onWidthChanged: dialog.applyFrame()
                onHeightChanged: dialog.applyFrame()

                // The picture as it lies before turning, with what is drawn on it.
                Item {
                    id: picture
                    objectName: "picture"
                    anchors.centerIn: parent
                    width: dialog.sideways ? view.height : view.width
                    height: dialog.sideways ? view.width : view.height
                    rotation: dialog.turn

                    Image {
                        id: photo
                        anchors.fill: parent
                        source: dialog.ofMessage ? "image://threesea/" + dialog.message + "/file"
                              : dialog.path ? Fmt.fileUrl(dialog.path) : ""
                        fillMode: Image.PreserveAspectFit
                        asynchronous: true
                        // Like every reader of message media: nothing decoded stays behind in Qt's
                        // cache once the editor has gone, a private chat's least of all.
                        cache: false
                        // As the camera stored it: QML's Image leaves the EXIF rotation alone
                        // unless asked. The cut is made on the turned picture (editPhoto), and
                        // what is sent is turned too (FilePreparer), so the view must be.
                        autoTransform: true
                        // Shown no larger than the screen; the cut is made from the original.
                        sourceSize.width: Screen.width
                        sourceSize.height: Screen.height
                        onStatusChanged: {
                            if (status !== Image.Ready) return
                            if (dialog.earlier) {
                                dialog.kept = dialog.earlier.crop
                                dialog.applyFrame()
                            } else {
                                dialog.reset()
                            }
                        }
                    }

                    Canvas {
                        id: drawing
                        anchors.fill: parent
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                        onPaint: {
                            var context = getContext("2d")
                            context.clearRect(0, 0, width, height)
                            context.lineCap = "round"
                            context.lineJoin = "round"
                            var longest = Math.max(width, height)
                            for (var i = 0; i < dialog.strokes.length; ++i) {
                                var stroke = dialog.strokes[i]
                                var points = stroke.points
                                context.lineWidth = stroke.width * longest
                                context.strokeStyle = stroke.color
                                context.beginPath()
                                context.moveTo(points[0] * width, points[1] * height)
                                // A single point is a dot: a line too short to see, with its round ends.
                                if (points.length === 2) {
                                    context.lineTo(points[0] * width + 0.01, points[1] * height)
                                }
                                for (var at = 2; at < points.length; at += 2) {
                                    context.lineTo(points[at] * width, points[at + 1] * height)
                                }
                                context.stroke()
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: dialog.tool !== "cut"
                        // Or the dialog's own swipe takes the line away after a few pixels.
                        preventStealing: true
                        onPressed: {
                            if (dialog.tool !== "draw") return
                            var lines = dialog.strokes.slice()
                            lines.push({ points: [dialog.fraction(mouse.x, width), dialog.fraction(mouse.y, height)],
                                         color: "" + dialog.penColor,
                                         width: Theme.paddingSmall / Math.max(width, height) })
                            dialog.strokes = lines
                            dialog.history = dialog.history.concat(["stroke"])
                            drawing.requestPaint()
                        }
                        onPositionChanged: {
                            if (dialog.tool !== "draw") return
                            var points = dialog.strokes[dialog.strokes.length - 1].points
                            points.push(dialog.fraction(mouse.x, width), dialog.fraction(mouse.y, height))
                            drawing.requestPaint()
                        }
                        onClicked: {
                            if (dialog.tool === "text") {
                                dialog.addText(dialog.fraction(mouse.x, width), dialog.fraction(mouse.y, height))
                            }
                        }
                    }

                    // Moved with a finger; the picture gets them as they lie (PhotoEdit: on top of
                    // the lines, centred on x, y).
                    Repeater {
                        model: texts
                        Label {
                            x: model.x * picture.width - width / 2
                            y: model.y * picture.height - height / 2
                            rotation: model.angle
                            text: model.text
                            textFormat: Text.PlainText
                            color: model.color
                            font.pixelSize: Math.max(1, model.size * Math.max(picture.width, picture.height))

                            MouseArea {
                                anchors.fill: parent
                                enabled: dialog.tool === "text"
                                preventStealing: true
                                onPositionChanged: {
                                    var point = mapToItem(picture, mouse.x, mouse.y)
                                    texts.setProperty(index, "x", dialog.fraction(point.x, picture.width))
                                    texts.setProperty(index, "y", dialog.fraction(point.y, picture.height))
                                }
                            }
                        }
                    }
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
                    border { width: Math.max(1, Theme.paddingSmall / 4); color: Theme.highlightColor }

                    MouseArea {
                        anchors.fill: parent
                        enabled: dialog.tool === "cut"
                        // Or the dialog's own left/right swipe (accept, cancel) takes the drag
                        // away after a few pixels and the frame stops following the finger.
                        preventStealing: true
                        onPositionChanged: dialog.rememberFrame()
                        onReleased: dialog.rememberFrame()
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
                        objectName: "grip" + index
                        visible: dialog.tool === "cut"
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
                                if (dialog.square) {
                                    // The opposite corner stays, the side follows the finger as
                                    // far as the picture has room.
                                    var fixedX = parent.atRight ? frame.x : frame.x + frame.width
                                    var fixedY = parent.atBottom ? frame.y : frame.y + frame.height
                                    var wanted = Math.max(parent.atRight ? point.x - fixedX : fixedX - point.x,
                                                          parent.atBottom ? point.y - fixedY : fixedY - point.y)
                                    var room = Math.min(parent.atRight ? view.width - fixedX : fixedX,
                                                        parent.atBottom ? view.height - fixedY : fixedY)
                                    var side = Math.min(room, Math.max(least, wanted))
                                    frame.width = side
                                    frame.height = side
                                    frame.x = parent.atRight ? fixedX : fixedX - side
                                    frame.y = parent.atBottom ? fixedY : fixedY - side
                                    dialog.rememberFrame()
                                    return
                                }
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
                                dialog.rememberFrame()
                            }
                        }
                    }
                }
            }
        }

        Row {
            id: tools
            // Not by `visible`: that is off too while another page covers this one, and the picture
            // would change size meanwhile.
            visible: !dialog.square
            height: dialog.square ? 0 : Theme.itemSizeSmall
            anchors {
                bottom: parent.bottom
                bottomMargin: dialog.square ? 0 : Theme.paddingMedium
                horizontalCenter: parent.horizontalCenter
            }

            Repeater {
                model: [{ tool: "cut", icon: "icon-m-crop" },
                        { tool: "draw", icon: "icon-m-edit" },
                        { tool: "text", icon: "icon-m-text-input" }]
                IconButton {
                    icon.source: "image://theme/" + modelData.icon
                    highlighted: down || dialog.tool === modelData.tool
                    onClicked: dialog.tool = modelData.tool
                }
            }
            // The colour of what is drawn next, as Silica's picker offers them.
            BackgroundItem {
                width: Theme.itemSizeSmall
                height: Theme.itemSizeSmall
                onClicked: pageStack.push(colorPage)
                Rectangle {
                    anchors.centerIn: parent
                    width: Theme.iconSizeSmall
                    height: width
                    radius: width / 2
                    color: dialog.penColor
                }
            }
            IconButton {
                icon.source: "image://theme/icon-m-backspace"
                enabled: dialog.history.length > 0
                onClicked: dialog.undo()
            }
        }
    }

    Component {
        id: colorPage
        ColorPickerPage {
            onColorClicked: {
                dialog.penColor = color
                pageStack.pop()
            }
        }
    }
}
