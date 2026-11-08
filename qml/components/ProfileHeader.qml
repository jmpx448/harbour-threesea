// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Format.js" as Fmt

// The top of a contact or group page like a contact card in Jolla People: the square picture at
// the left edge, next to it the name on the ambience's highlight background, and room above
// for the page indicators. Tapping the picture shows it full screen.
Item {
    id: root

    property string title
    property string description
    property string source
    property bool group
    property bool list
    // Pinned to the top of the chat list: the pin top right, like People's favourite star.
    property bool pinned
    property bool pinnable: true
    signal pinToggled()
    // Tapping the picture: full screen by default; `pictureTappable` + pictureClicked for own
    // handling (e.g. choosing one on the own profile).
    property bool pictureTappable: false
    // Without the title line (e.g. an editable name in `content` instead).
    property bool showTitle: true
    signal pictureClicked()
    // Holding the description down (a Threema ID, to copy or share).
    property bool descriptionHoldable: false
    signal descriptionHeld()
    // Room above the card (page indicators, the pin, a display cutout); 0 under a page header
    // (the own card on the contacts page), which brings its own cutout padding.
    property real topSpace: Theme.itemSizeLarge + cutout.inset
    // Pressed (when the card as a whole is a button).
    property bool highlighted
    // An icon button in the card's lower right corner (e.g. the own QR code), if set.
    property string actionIcon
    signal actionClicked()
    // Below the description, e.g. the verification dots.
    default property alias content: extra.data

    width: parent ? parent.width : Screen.width
    height: topSpace + card.height

    CutoutInset { id: cutout }

    IconButton {
        visible: root.pinnable
        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
            verticalCenter: parent.top
            verticalCenterOffset: cutout.inset + Theme.itemSizeLarge / 2
        }
        // The theme has no pushpin (icon-m-pin is the SIM PIN): People's favourite star.
        icon.source: "image://theme/" + (root.pinned ? "icon-m-favorite-selected" : "icon-m-favorite")
                     + "?" + (pressed ? Theme.highlightColor : Theme.primaryColor)
        onClicked: root.pinToggled()
    }

    Item {
        id: card
        y: root.topSpace
        width: parent.width
        height: Math.round(Math.min(Screen.width, Screen.height) / 3)

        Thumbnail {
            id: picture
            height: parent.height
            group: root.group
            list: root.list
            source: root.source
            highlighted: pressed || root.highlighted
            pressed: pictureArea.pressed && pictureArea.containsMouse
            MouseArea {
                id: pictureArea
                anchors.fill: parent
                enabled: root.source.length > 0 || root.pictureTappable
                onClicked: {
                    if (root.pictureTappable) root.pictureClicked()
                    else pageStack.push(Qt.resolvedUrl("../pages/ImageViewerPage.qml"), { source: root.source })
                }
            }
        }

        Rectangle {
            anchors {
                left: picture.right
                right: parent.right
                top: parent.top
                bottom: parent.bottom
            }
            color: Theme.rgba(Theme.highlightBackgroundColor,
                              Fmt.alpha(root.highlighted ? Math.min(1, 2 * Theme.highlightBackgroundOpacity)
                                                         : Theme.highlightBackgroundOpacity, backend.emulator))

            Column {
                x: Theme.paddingLarge
                y: Theme.paddingMedium
                width: parent.width - Theme.paddingLarge - Theme.horizontalPageMargin

                Label {
                    visible: root.showTitle
                    width: parent.width
                    text: root.title
                    font.pixelSize: Theme.fontSizeLarge
                    font.family: Theme.fontFamilyHeading
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                    color: root.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
                Label {
                    width: parent.width
                    visible: text.length > 0
                    text: root.description
                    font.pixelSize: Theme.fontSizeSmall
                    truncationMode: TruncationMode.Fade
                    color: descriptionArea.pressed ? Theme.highlightColor : Theme.secondaryColor
                    MouseArea {
                        id: descriptionArea
                        anchors.fill: parent
                        enabled: root.descriptionHoldable
                        onPressAndHold: root.descriptionHeld()
                    }
                }
                Item {
                    id: extra
                    width: parent.width
                    height: childrenRect.height
                }
            }

            IconButton {
                visible: root.actionIcon.length > 0
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin - Theme.paddingMedium
                    bottom: parent.bottom
                    bottomMargin: Theme.paddingSmall
                }
                icon.source: root.actionIcon
                // Our own icons (qml/icons) at the theme's icon size.
                icon.sourceSize: Qt.size(Theme.iconSizeMedium, Theme.iconSizeMedium)
                onClicked: root.actionClicked()
            }
        }
    }
}
