// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0

// One participant of a group call: the square picture (the theme's contact icon without one),
// their camera's picture over it while it is on, the name at the bottom on a dark ramp, a
// crossed-out microphone while it is off. Corners like
// the call page's videos and the chat's bubbles: rounded, except the bottom left for the others
// and the bottom right for us.
Item {
    id: tile

    property int size
    property alias source: picture.source
    property string name
    property bool microphone: true
    // Joined, the handshake with them not done yet.
    property bool connecting: false
    // Us: the bottom right corner square instead of the bottom left.
    property bool own: false
    // Their camera is on: its picture (ours with `own`; `participant` the others').
    property bool camera: false
    property int participant: -1
    property bool mirror: false

    width: size
    height: size
    opacity: connecting ? Theme.opacityHigh : 1.0

    layer.enabled: true
    layer.effect: ShaderEffect {
        property size area: Qt.size(tile.width, tile.height)
        property real radius: Theme.paddingLarge
        // Top left, top right, bottom right, bottom left: 1 rounded.
        property vector4d corners: tile.own ? Qt.vector4d(1, 1, 0, 1) : Qt.vector4d(1, 1, 1, 0)
        fragmentShader: "
            uniform sampler2D source;
            uniform lowp float qt_Opacity;
            uniform highp vec2 area;
            uniform highp float radius;
            uniform highp vec4 corners;
            varying highp vec2 qt_TexCoord0;
            void main() {
                highp vec2 p = qt_TexCoord0 * area;
                highp vec2 q = vec2(0.0);
                highp float rounded = 0.0;
                if (p.x < radius && p.y < radius) {
                    q = vec2(radius) - p;
                    rounded = corners.x;
                } else if (p.x > area.x - radius && p.y < radius) {
                    q = vec2(p.x - area.x + radius, radius - p.y);
                    rounded = corners.y;
                } else if (p.x > area.x - radius && p.y > area.y - radius) {
                    q = p - area + vec2(radius);
                    rounded = corners.z;
                } else if (p.x < radius && p.y > area.y - radius) {
                    q = vec2(radius - p.x, p.y - area.y + radius);
                    rounded = corners.w;
                }
                highp float alpha = rounded > 0.5 ? clamp(radius - length(q) + 0.5, 0.0, 1.0) : 1.0;
                gl_FragColor = texture2D(source, qt_TexCoord0) * alpha * qt_Opacity;
            }"
    }

    Thumbnail {
        id: picture
        anchors.fill: parent
    }

    CallVideo {
        id: video
        anchors.fill: parent
        manager: groupCalls
        remote: !tile.own
        participant: tile.own ? -1 : tile.participant
        mirror: tile.mirror
        visible: tile.camera && hasFrame
    }

    Rectangle {
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        height: nameLabel.height + 2 * Theme.paddingMedium
        gradient: Gradient {
            GradientStop { position: 0.0; color: "transparent" }
            GradientStop { position: 1.0; color: Theme.rgba("black", Theme.opacityHigh) }
        }
    }

    Label {
        id: nameLabel
        anchors {
            left: parent.left
            leftMargin: Theme.paddingMedium
            right: mute.visible ? mute.left : parent.right
            rightMargin: Theme.paddingSmall
            bottom: parent.bottom
            bottomMargin: Theme.paddingMedium
        }
        text: tile.name
        truncationMode: TruncationMode.Fade
        font.pixelSize: Theme.fontSizeSmall
        color: "white"
    }

    Image {
        id: mute
        visible: !tile.microphone && !tile.connecting
        anchors {
            right: parent.right
            rightMargin: Theme.paddingSmall
            verticalCenter: nameLabel.verticalCenter
        }
        width: Theme.iconSizeSmall
        height: Theme.iconSizeSmall
        sourceSize.width: width
        sourceSize.height: height
        source: "image://theme/icon-m-mic-mute?white"
    }
}
