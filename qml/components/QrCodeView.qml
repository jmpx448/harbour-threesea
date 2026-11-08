// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// A QR code from the core ({size, bits}): dark modules on white with a quiet zone, whatever the
// ambience (scanners need contrast). As high as it is wide.
Rectangle {
    id: root

    property var code: ({ size: 0, bits: "" })

    height: width
    color: "white"
    radius: Theme.paddingSmall

    Canvas {
        anchors.fill: parent
        property var code: root.code
        onCodeChanged: requestPaint()
        // Turning the phone changes the size of the code.
        onWidthChanged: requestPaint()
        onPaint: {
            var ctx = getContext("2d")
            ctx.fillStyle = "white"
            ctx.fillRect(0, 0, width, height)
            var n = code.size
            if (!n) return
            var module = width / (n + 8)
            ctx.fillStyle = "black"
            for (var i = 0; i < code.bits.length; ++i) {
                if (code.bits.charAt(i) === "1") {
                    ctx.fillRect(Math.floor((i % n + 4) * module), Math.floor((Math.floor(i / n) + 4) * module),
                                 Math.ceil(module), Math.ceil(module))
                }
            }
        }
    }
}
