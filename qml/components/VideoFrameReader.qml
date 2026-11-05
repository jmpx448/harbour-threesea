// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtMultimedia 5.6
import "Format.js" as Fmt

// A video's thumbnail (first frame, from the system thumbnailer) and length: `start` takes an entry
// with a `path`, `finished` gives it back with `frame` (none if there is none) and `seconds`.
Item {
    id: reader
    // Off screen, not invisible: grabToImage only renders visible items.
    x: -2 * width
    width: 512
    height: 512
    property string path
    property var file
    property real seconds: 0

    signal finished(var entry)

    function start(entry) {
        reader.path = entry.path
        reader.file = entry
        seconds = 0
        frame.path = entry.path
        durationProbe.source = Fmt.fileUrl(entry.path)
        giveUp.restart()
    }
    function finish(image) {
        giveUp.stop()
        if (reader.path.length === 0) return
        reader.path = ""
        var entry = file
        entry.frame = image
        entry.seconds = seconds
        reader.finished(entry)
        frame.path = ""
        durationProbe.source = ""
    }

    // The thumbnailer's image provider, as lipstick uses it. It always gives a centred
    // square, so video thumbnails are square (the video keeps its proportions).
    Image {
        id: frame
        property string path
        width: implicitWidth
        height: implicitHeight
        sourceSize.width: 512
        sourceSize.height: 512
        asynchronous: true
        cache: false
        source: path.length > 0 ? "image://nemoThumbnail/" + path.replace(/^file:\/\//, "") : ""
        onStatusChanged: {
            if (reader.path.length === 0) return
            // A moment more for the length (the media service may not exist).
            if (status === Image.Ready) grabLater.restart()
            else if (status === Image.Error) reader.finish(null)
        }
    }
    MediaPlayer {
        id: durationProbe
        autoLoad: true
        onDurationChanged: if (duration > 0) reader.seconds = duration / 1000
    }
    Timer {
        id: grabLater
        interval: 800
        onTriggered: {
            if (!frame.grabToImage(function(result) { reader.finish(result.image) })) {
                reader.finish(null)
            }
        }
    }
    Timer {
        id: giveUp
        interval: 10000
        onTriggered: reader.finish(null)
    }
}
