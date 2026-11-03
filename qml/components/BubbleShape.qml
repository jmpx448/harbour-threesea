// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6

// Rectangle with individually rounded corners, built from non-overlapping pieces so that a
// translucent colour stays even (Qt 5.6 has no per-corner radius and layers change the colour).
Item {
    id: shape

    property color color
    property real radius
    property bool roundTopLeft: true
    property bool roundTopRight: true
    property bool roundBottomLeft: true
    property bool roundBottomRight: true

    // Whole pixels: at fractional sizes (e.g. text with emoji pictures) the pieces' edges are
    // anti-aliased and leave a visible seam between them.
    readonly property real w: Math.round(width)
    readonly property real h: Math.round(height)
    readonly property real r: Math.round(Math.min(radius, w / 2, h / 2))

    // Middle band, full width.
    Rectangle { y: shape.r; width: shape.w; height: shape.h - 2 * shape.r; color: shape.color }
    // Top and bottom bands between the corners.
    Rectangle { x: shape.r; width: shape.w - 2 * shape.r; height: shape.r; color: shape.color }
    Rectangle {
        x: shape.r; y: shape.h - shape.r
        width: shape.w - 2 * shape.r; height: shape.r; color: shape.color
    }

    // Corners: a square, or a quarter of a circle (clipped).
    Repeater {
        model: [
            { x: 0, y: 0, round: shape.roundTopLeft, cx: 0, cy: 0 },
            { x: 1, y: 0, round: shape.roundTopRight, cx: -1, cy: 0 },
            { x: 0, y: 1, round: shape.roundBottomLeft, cx: 0, cy: -1 },
            { x: 1, y: 1, round: shape.roundBottomRight, cx: -1, cy: -1 }
        ]
        Item {
            x: modelData.x * (shape.w - shape.r)
            y: modelData.y * (shape.h - shape.r)
            width: shape.r
            height: shape.r
            clip: modelData.round
            Rectangle {
                x: modelData.round ? modelData.cx * shape.r : 0
                y: modelData.round ? modelData.cy * shape.r : 0
                width: modelData.round ? 2 * shape.r : shape.r
                height: width
                radius: modelData.round ? shape.r : 0
                color: shape.color
            }
        }
    }
}
