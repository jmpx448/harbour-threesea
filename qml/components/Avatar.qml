// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Contact avatar. Until profile pictures exist (Phase 3), the theme's contact icon without a
// background (head and shoulders), like Jolla Messages shows for contacts without a picture.
Item {
    id: avatar

    property string name
    property bool highlighted: false
    // A group: the theme's group icon.
    property bool group: false
    property bool list: false
    // A picture (e.g. the group's), shown instead of the icon.
    property string source

    width: Theme.itemSizeSmall
    height: width

    // Square, like the pictures in Jolla's People and Media (user request: no rounded corners).
    Image {
        id: picture
        anchors.fill: parent
        visible: avatar.source.length > 0 && status === Image.Ready
        source: avatar.source
        fillMode: Image.PreserveAspectCrop
        sourceSize.width: width
        sourceSize.height: height
        asynchronous: true
        // Cached: the source names the picture's version, so a new picture is a new URL; rows
        // scrolled back in don't decode (and decrypt) it again.
        cache: true
    }

    Image {
        anchors.centerIn: parent
        // The icon stays while the picture loads and when it can't be decoded (like
        // Thumbnail.qml), never a blank square.
        visible: avatar.source.length === 0 || picture.status !== Image.Ready
        source: "image://theme/" + (avatar.list ? "icon-m-message-forward" : avatar.group ? "icon-m-users" : "icon-m-contact") + "?" + (avatar.highlighted ? Theme.highlightColor : Theme.primaryColor)
    }
}
