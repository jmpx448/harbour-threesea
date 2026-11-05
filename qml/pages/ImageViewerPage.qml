// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0

// A received or sent image, full screen on black like Jolla Gallery: pinch or double tap to
// zoom, tap to show/hide the page indicators.
Page {
    id: page

    property int messageId
    // Or any picture of ours (a profile or group picture: image://threesea/contact/…).
    property string source
    // A GIF: plays over the (first frame) image.
    property bool animated

    allowedOrientations: Orientation.All
    // Full screen on black also beside a landscape cutout, like Gallery (its full-screen page
    // sets the same); nothing interactive sits at the edges.
    cutoutMode: CutoutMode.FullScreen
    backNavigation: !zoomed
    showNavigationIndicator: !zoomed
    readonly property bool zoomed: flickable.contentWidth > flickable.width + 1

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    SilicaFlickable {
        id: flickable
        anchors.fill: parent
        contentWidth: width
        contentHeight: height
        boundsBehavior: Flickable.StopAtBounds

        PinchArea {
            width: Math.max(flickable.contentWidth, flickable.width)
            height: Math.max(flickable.contentHeight, flickable.height)
            property real startWidth
            property real startHeight
            onPinchStarted: {
                startWidth = flickable.contentWidth
                startHeight = flickable.contentHeight
            }
            onPinchUpdated: {
                var scale = Math.max(1, Math.min(4 * flickable.width / startWidth, pinch.scale))
                flickable.resizeContent(startWidth * scale, startHeight * scale, pinch.center)
            }
            onPinchFinished: flickable.returnToBounds()

            Image {
                id: image
                anchors.fill: parent
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                cache: false
                // Decode at up to twice the screen size: enough to zoom in a bit.
                sourceSize.width: 2 * Math.max(Screen.width, Screen.height)
                sourceSize.height: 2 * Math.max(Screen.width, Screen.height)
                source: page.source.length > 0 ? page.source : "image://threesea/" + page.messageId + "/file"

                AnimatedMedia {
                    id: animation
                    // The picture's area within the image item (PreserveAspectFit).
                    anchors.centerIn: parent
                    width: image.paintedWidth
                    height: image.paintedHeight
                    visible: page.animated && ready
                    message: page.animated ? page.messageId : -1
                    playing: Qt.application.state === Qt.ApplicationActive
                }

                MouseArea {
                    anchors.fill: parent
                    onDoubleClicked: {
                        if (page.zoomed) {
                            flickable.resizeContent(flickable.width, flickable.height, Qt.point(mouse.x, mouse.y))
                        } else {
                            flickable.resizeContent(flickable.width * 2.5, flickable.height * 2.5,
                                                    Qt.point(mouse.x, mouse.y))
                        }
                        flickable.returnToBounds()
                    }
                }
            }
        }
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: image.status === Image.Loading
    }

    InfoLabel {
        anchors.verticalCenter: parent.verticalCenter
        visible: image.status === Image.Error && !animation.visible
        //% "This picture could not be shown"
        text: qsTrId("threesea-la-cannot-show-picture")
    }
}
