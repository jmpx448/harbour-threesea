// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

Label {
    x: Theme.horizontalPageMargin
    width: parent.width - 2 * Theme.horizontalPageMargin
    visible: text.length > 0
    wrapMode: Text.Wrap
    font.pixelSize: Theme.fontSizeSmall
    color: Theme.errorColor
}
