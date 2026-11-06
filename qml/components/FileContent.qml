// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "Format.js" as Fmt

// The file part of a file message bubble: an image (thumbnail, then the full image once on the
// device) for media, or an icon with name and size for other files. States from the core:
// 0 on the server, 1 loading, 2 on the device, 3 failed (fileState / thumbnailState).
Item {
    id: root

    property int messageId
    property var body: ({})
    property real maxWidth
    property bool highlighted
    // Whether the bubble is in view: a GIF outside it needn't be decoded frame by frame.
    property bool onScreen: true
    // The list is moving: the full picture waits until it stops, since reading one means reading
    // and decrypting the whole file, for a bubble that may be gone again in a moment. Once the
    // list has rested the picture stays wanted, so scrolling on doesn't take it away.
    property bool listMoving: false
    property bool fullWanted: false
    // Qt's image cache may keep the decoded pictures (the setting; never for a private chat).
    property bool cachePictures: false
    onListMovingChanged: if (!listMoving) fullWanted = true
    Component.onCompleted: if (!listMoving) fullWanted = true
    property color textColor: Theme.primaryColor
    // Corners like the surrounding bubble (concentric radius, same corners rounded).
    property real radius: 0
    property bool roundTopLeft: true
    property bool roundTopRight: true
    property bool roundBottomLeft: true
    property bool roundBottomRight: true

    readonly property bool isImage: body && body.rendering !== "file" && Fmt.isPicture(body.mediaType)
    // An audio file rendered as media is a voice message (Threema Android's rule).
    // GIFs play in the bubble (Android's "Auto-play animated images"; off: in the viewer).
    readonly property bool isGif: isImage && String(body.mediaType).toLowerCase() === "image/gif"
    readonly property bool isVoice: body && body.rendering !== "file"
                                    && String(body.mediaType).indexOf("audio/") === 0
    readonly property bool isVideo: body && body.rendering !== "file"
                                    && String(body.mediaType).indexOf("video/") === 0
    readonly property bool hasPicture: !isVoice && (isImage || (body && body.thumbnail === true))
    readonly property bool active: voicePlayer.messageId === messageId
    readonly property bool loading: body && (body.fileState === 1 || body.thumbnailState === 1
                                             || body.uploadState === 1)
    readonly property bool uploadFailed: body && body.uploadState === 3
    readonly property bool fileHere: body && body.fileState === 2
    // An incoming voice message not played yet: its play icon and length glow until then.
    property bool newVoice: false

    // Picture width: two thirds of the screen, like the bubbles' text width; height from the
    // metadata (or the loaded thumbnail), kept within sane proportions.
    readonly property real pictureWidth: Math.min(maxWidth, Screen.width * 2 / 3)
    readonly property real aspect: {
        var w = body && body.width, h = body && body.height
        // The full picture only carries on from a thumbnail it replaced. Without one the bubble
        // stays as it was built and crops: going by the picture would change its height, and
        // move the list, at each rest after a rebuild.
        if (!(w > 0 && h > 0) && body && body.thumbnailState === 2 && fullPicture.status === Image.Ready) {
            w = fullPicture.implicitWidth
            h = fullPicture.implicitHeight
        } else if (!(w > 0 && h > 0) && thumbnail.status === Image.Ready) {
            w = thumbnail.implicitWidth
            h = thumbnail.implicitHeight
        }
        var a = w > 0 && h > 0 ? h / w : 0.75
        return Math.max(0.4, Math.min(1.5, a))
    }

    width: hasPicture ? pictureWidth : isVoice ? voiceRow.width : fileRow.width
    height: hasPicture ? Math.round(pictureWidth * aspect) : isVoice ? voiceRow.height : fileRow.height

    // Voice message: play/pause, progress, duration (position while playing).
    Row {
        id: voiceRow
        visible: root.isVoice
        spacing: Theme.paddingMedium

        Item {
            width: Theme.iconSizeMedium
            height: Theme.iconSizeMedium
            anchors.verticalCenter: parent.verticalCenter
            Image {
                anchors.centerIn: parent
                visible: !voiceBusy.running
                source: "image://theme/" + (!root.fileHere ? Fmt.fileIcon(root.body.fileType,
                                                                           root.uploadFailed ? 3 : root.body.fileState)
                                            : root.active && voicePlayer.playing ? "icon-m-pause" : "icon-m-play")
                        + "?" + (root.highlighted || root.newVoice ? Theme.highlightColor : root.textColor)
            }
            BusyIndicator {
                id: voiceBusy
                anchors.centerIn: parent
                size: BusyIndicatorSize.Small
                running: root.loading
            }
        }
        // Silica's slider as the playback bar (user request 2026-10-28): the standard groove,
        // and dragging it seeks while this message is loaded in the player.
        Slider {
            id: voiceSlider
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(Screen.width / 3,
                            root.maxWidth - Theme.iconSizeMedium - 2 * Theme.paddingMedium
                            - durationLabel.width
                            - (speedLabel.visible ? speedLabel.width + Theme.paddingMedium : 0))
            leftMargin: Theme.paddingMedium
            rightMargin: Theme.paddingMedium
            handleVisible: root.active
            enabled: root.active && !voiceBusy.running
            minimumValue: 0
            maximumValue: root.active && voicePlayer.duration > 0
                          ? voicePlayer.duration : Math.max(1, (root.body.duration || 0) * 1000)
            onReleased: if (root.active) voicePlayer.seek(value)
        }
        Binding {
            target: voiceSlider
            property: "value"
            value: root.active ? voicePlayer.position : 0
            when: !voiceSlider.pressed
        }
        Label {
            id: durationLabel
            anchors.verticalCenter: parent.verticalCenter
            text: Fmt.duration(root.active ? voicePlayer.position / 1000 : root.body.duration)
            font.pixelSize: Theme.fontSizeExtraSmall
            color: root.newVoice ? Theme.highlightColor
                   : root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
        }
        // Playback speed, like Threema Android's chip on the bubble: tapping steps
        // 1× → 1.5× → 2×, and the choice holds for the next messages. Only on the message in
        // the player, so the other bubbles stay as narrow as they were.
        Label {
            id: speedLabel
            anchors.verticalCenter: parent.verticalCenter
            visible: root.active
            //% "%1×"
            text: qsTrId("threesea-la-playback-speed").arg(
                      voicePlayer.speed === 1 ? "1"
                      : voicePlayer.speed === 2 ? "2"
                      : Number(voicePlayer.speed).toLocaleString(Qt.locale(), "f", 1))
            font.pixelSize: Theme.fontSizeExtraSmall
            color: speedArea.pressed ? Theme.highlightColor
                   : root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            MouseArea {
                id: speedArea
                anchors.fill: parent
                anchors.margins: -Theme.paddingMedium
                onClicked: voicePlayer.speed = voicePlayer.speed >= 2 ? 1 : voicePlayer.speed + 0.5
            }
        }
    }

    // The picture with rounded corners: rendered into a layer and masked by a small shader
    // (per-corner radius, antialiased edge).
    Item {
        id: picture
        visible: root.hasPicture
        anchors.fill: parent
        layer.enabled: root.hasPicture && root.radius > 0
        layer.effect: ShaderEffect {
            property size pixelSize: Qt.size(picture.width, picture.height)
            // top left, top right, bottom right, bottom left
            property vector4d radii: Qt.vector4d(root.roundTopLeft ? root.radius : 0,
                                                 root.roundTopRight ? root.radius : 0,
                                                 root.roundBottomRight ? root.radius : 0,
                                                 root.roundBottomLeft ? root.radius : 0)
            fragmentShader: "
                uniform sampler2D source;
                uniform lowp float qt_Opacity;
                uniform highp vec2 pixelSize;
                uniform highp vec4 radii;
                varying highp vec2 qt_TexCoord0;
                void main() {
                    highp vec2 p = qt_TexCoord0 * pixelSize;
                    bool left = p.x < pixelSize.x * 0.5;
                    bool top = p.y < pixelSize.y * 0.5;
                    highp float r = top ? (left ? radii.x : radii.y) : (left ? radii.w : radii.z);
                    highp vec2 inner = clamp(p, vec2(r), pixelSize - vec2(r));
                    lowp float alpha = r > 0.0 ? clamp(r - length(p - inner) + 0.5, 0.0, 1.0) : 1.0;
                    gl_FragColor = texture2D(source, qt_TexCoord0) * alpha * qt_Opacity;
                }"
        }

        // Thumbnail first, the full image on top once it is here, and then the thumbnail goes.
        Image {
            id: thumbnail
            // Not under a playing GIF (dimmed, both would show).
            visible: root.hasPicture && !animation.ready
            anchors.fill: parent
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            cache: root.cachePictures
            sourceSize.width: root.pictureWidth
            source: root.hasPicture && root.body.thumbnailState === 2 && fullPicture.status !== Image.Ready
                    ? "image://threesea/" + root.messageId + "/thumb" : ""
        }

        Image {
            id: fullPicture
            visible: root.isImage && status === Image.Ready && !animation.ready
            anchors.fill: parent
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            cache: root.cachePictures
            sourceSize.width: root.pictureWidth
            // GIFs: AnimatedMedia draws them (the first frame too).
            source: root.isImage && !root.isGif && root.fileHere && root.fullWanted
                    ? "image://threesea/" + root.messageId + "/file" : ""
        }
        AnimatedMedia {
            id: animation
            visible: root.isGif && ready
            anchors.fill: parent
            message: root.isGif && root.fileHere ? root.messageId : -1
            playing: backend.autoPlayGifs && Qt.application.state === Qt.ApplicationActive
                     && root.onScreen
        }
        // Placeholder until something is loaded.
        Rectangle {
            visible: root.hasPicture && thumbnail.status !== Image.Ready && fullPicture.status !== Image.Ready
                     && !animation.ready
            anchors.fill: parent
            color: Theme.rgba(Theme.primaryColor, Fmt.alpha(0.05, backend.emulator))
        }
    }
    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Medium
        running: root.hasPicture && root.loading
    }
    // Not here (yet): download on tap; after a failure the same, with the retry icon.
    Image {
        visible: root.hasPicture && !root.loading && (!root.fileHere || root.uploadFailed)
        anchors.centerIn: parent
        source: (root.body.fileState === 3 || root.uploadFailed ? "image://theme/icon-m-refresh?"
                                            : "image://theme/icon-m-cloud-download?")
                + (root.highlighted ? Theme.highlightColor : Theme.primaryColor)
    }

    Row {
        id: fileRow
        visible: !root.hasPicture && !root.isVoice
        spacing: Theme.paddingMedium

        Item {
            width: Theme.iconSizeMedium
            height: Theme.iconSizeMedium
            Image {
                anchors.centerIn: parent
                visible: !fileBusy.running
                source: "image://theme/" + Fmt.fileIcon(root.body.fileType,
                                                        root.uploadFailed ? 3 : root.body.fileState)
                        + "?" + (root.highlighted ? Theme.highlightColor : root.textColor)
            }
            BusyIndicator {
                id: fileBusy
                anchors.centerIn: parent
                size: BusyIndicatorSize.Small
                running: root.body && (root.body.fileState === 1 || root.body.uploadState === 1)
            }
        }
        Column {
            width: Math.min(Math.max(nameLabel.implicitWidth, sizeLabel.implicitWidth),
                            root.maxWidth - Theme.iconSizeMedium - Theme.paddingMedium)
            anchors.verticalCenter: parent.verticalCenter
            Label {
                id: nameLabel
                width: parent.width
                //% "File"
                text: root.body.name || qsTrId("threesea-la-file")
                truncationMode: TruncationMode.Fade
                color: root.textColor
            }
            Label {
                id: sizeLabel
                width: parent.width
                text: Format.formatFileSize(root.body.size || 0)
                font.pixelSize: Theme.fontSizeExtraSmall
                color: root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            }
        }
    }

    // Videos: a play button over the picture and the length, like Jolla Gallery.
    Rectangle {
        visible: root.isVideo && root.hasPicture && root.fileHere && !root.loading && !root.uploadFailed
        anchors.centerIn: parent
        width: Theme.itemSizeMedium
        height: width
        radius: width / 2
        color: Theme.rgba("black", 0.4)
        Image {
            anchors.centerIn: parent
            source: "image://theme/icon-m-play?white"
        }
    }
    // GIFs: marked like that (Threema's label), with a play button while they don't play here.
    Rectangle {
        visible: root.isGif && root.fileHere && !backend.autoPlayGifs && !root.loading && !root.uploadFailed
        anchors.centerIn: parent
        width: Theme.itemSizeMedium
        height: width
        radius: width / 2
        color: Theme.rgba("black", 0.4)
        Image {
            anchors.centerIn: parent
            source: "image://theme/icon-m-play?white"
        }
    }
    Label {
        visible: root.isGif && root.hasPicture
        anchors {
            right: parent.right
            bottom: parent.bottom
            margins: Theme.paddingMedium
        }
        //% "GIF"
        text: qsTrId("threesea-la-gif")
        font.pixelSize: Theme.fontSizeExtraSmall
        font.bold: true
        color: "white"
        style: Text.Raised
        styleColor: "black"
    }
    Label {
        visible: root.isVideo && root.hasPicture && root.body.duration > 0
        anchors {
            right: parent.right
            bottom: parent.bottom
            margins: Theme.paddingMedium
        }
        text: Fmt.duration(root.body.duration || 0)
        font.pixelSize: Theme.fontSizeExtraSmall
        color: "white"
        style: Text.Raised
        styleColor: "black"
    }
}
