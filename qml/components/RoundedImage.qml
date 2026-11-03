// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6

// An image with rounded corners (cropped to fill), antialiased by a small shader.
Image {
    id: image

    property real radius: 0

    fillMode: Image.PreserveAspectCrop
    asynchronous: true
    cache: false
    sourceSize.width: width
    sourceSize.height: height
    layer.enabled: radius > 0
    layer.effect: ShaderEffect {
        property size pixelSize: Qt.size(image.width, image.height)
        property real radius: image.radius
        fragmentShader: "
            uniform sampler2D source;
            uniform lowp float qt_Opacity;
            uniform highp vec2 pixelSize;
            uniform highp float radius;
            varying highp vec2 qt_TexCoord0;
            void main() {
                highp vec2 p = qt_TexCoord0 * pixelSize;
                highp vec2 inner = clamp(p, vec2(radius), pixelSize - vec2(radius));
                lowp float alpha = clamp(radius - length(p - inner) + 0.5, 0.0, 1.0);
                gl_FragColor = texture2D(source, qt_TexCoord0) * alpha * qt_Opacity;
            }"
    }
}
