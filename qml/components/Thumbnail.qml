// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Format.js" as Fmt

// A list entry's picture like Jolla Media's album list: a square at the left edge, full item
// height, no rounding, entries touching; without a picture the theme icon on a faint tile.
Item {
    id: thumbnail

    property string source
    property bool group: false
    // A distribution list: one message going out to many.
    property bool list: false
    property bool highlighted: false
    // Pressed on its own (it opens something else than its row): Silica's press highlight.
    property bool pressed: false

    width: height

    Rectangle {
        anchors.fill: parent
        visible: image.status !== Image.Ready
        gradient: Gradient {
            GradientStop { position: 0.0; color: Theme.rgba(Theme.primaryColor, Fmt.alpha(0.12, backend.emulator)) }
            GradientStop { position: 1.0; color: Theme.rgba(Theme.primaryColor, Fmt.alpha(0.05, backend.emulator)) }
        }
        Image {
            anchors.centerIn: parent
            source: "image://theme/" + (thumbnail.list ? "icon-m-message-forward" : thumbnail.group ? "icon-m-users" : "icon-m-contact")
                    + "?" + (thumbnail.highlighted ? Theme.highlightColor : Theme.primaryColor)
        }
    }

    Image {
        id: image
        anchors.fill: parent
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        // Cached: the source names the picture's version, so a new picture is a new URL; rows
        // scrolled back in don't decode (and decrypt) it again.
        cache: true
        sourceSize.width: width
        sourceSize.height: height
        source: thumbnail.source
    }

    Rectangle {
        anchors.fill: parent
        visible: thumbnail.pressed
        color: Theme.rgba(Theme.highlightBackgroundColor, Theme.highlightBackgroundOpacity)
    }
}
