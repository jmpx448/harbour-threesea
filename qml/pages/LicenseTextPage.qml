// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// About → a licence text the package installs (/usr/share/harbour-threesea/licenses): the AGPL or
// the third-party notices. Shown in pieces of a few paragraphs, so that the long notices never
// become one huge text item.
Page {
    id: page
    allowedOrientations: Orientation.All

    property string title
    // A file name under licenses/.
    property string file
    property var pieces: []
    property bool loading: true

    Component.onCompleted: {
        var xhr = new XMLHttpRequest()
        xhr.onreadystatechange = function() {
            if (xhr.readyState !== XMLHttpRequest.DONE) return
            var paragraphs = (xhr.responseText || "").split("\n\n")
            var result = []
            var piece = ""
            for (var i = 0; i < paragraphs.length; i++) {
                piece = piece ? piece + "\n\n" + paragraphs[i] : paragraphs[i]
                if (piece.length > 3000) {
                    result.push(piece)
                    piece = ""
                }
            }
            if (piece) result.push(piece)
            pieces = result
            loading = false
        }
        // The QML lives in /usr/share/harbour-threesea/qml/pages.
        xhr.open("GET", Qt.resolvedUrl("../../licenses/" + file))
        xhr.send()
    }

    SilicaListView {
        anchors.fill: parent
        spacing: Theme.paddingLarge
        header: PageHeader { title: page.title }
        model: page.pieces
        delegate: Label {
            x: Theme.horizontalPageMargin
            width: ListView.view.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.highlightColor
            text: modelData
        }
        footer: Item { width: 1; height: Theme.paddingLarge }

        BusyIndicator {
            anchors.centerIn: parent
            size: BusyIndicatorSize.Large
            running: page.loading
        }

        VerticalScrollDecorator { }
    }
}
