// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Delivery state of an outgoing message, Threema's symbols filled like Threema's own so they read
// this small: waiting = clock, sent to the server = envelope, delivered = inbox, read = eye
// (ours, scripts/make-ui-icons.py; the theme has outlines only), failed = the theme's warning.
Icon {
    property int state: 0

    width: Theme.iconSizeExtraSmall
    height: width
    sourceSize.width: width
    sourceSize.height: height
    color: state > 3 ? Theme.errorColor : Theme.secondaryHighlightColor
    highlightColor: state > 3 ? Theme.errorColor : Theme.highlightColor
    source: {
        switch (state) {
        case 0: return Qt.resolvedUrl("../icons/delivery-pending.png")
        case 1: return Qt.resolvedUrl("../icons/delivery-sent.png")
        case 2: return Qt.resolvedUrl("../icons/delivery-delivered.png")
        case 3: return Qt.resolvedUrl("../icons/delivery-read.png")
        default: return "image://theme/icon-s-warning"
        }
    }
}
