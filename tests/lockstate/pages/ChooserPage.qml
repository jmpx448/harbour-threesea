// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// A page with a PictureChooser, as the profile and group pages have.
Page {
    objectName: "chooserPage"
    property alias chooser: pictureChooser
    PictureChooser {
        id: pictureChooser
        host: parent
    }
}
