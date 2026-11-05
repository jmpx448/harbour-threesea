// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0

// A profile or group picture: from the gallery or the camera, then cut to a square. Android's
// AvatarEditView. `chosen` gets the cut copy for the time of the signal: it is removed right after.
Item {
    id: chooser

    // The page this belongs to: the editor opens when that is what shows again.
    property Item host
    // Picked or taken, waiting for the picker or the camera to have gone.
    property string waiting
    // In the editor. A shot stays no longer than this does, also when the editor is taken away
    // without an answer (the stack settled, the chooser's page went).
    property string editing

    signal chosen(string path)

    function pick() {
        pageStack.push(picker)
    }
    function shoot() {
        var camera = pageStack.push(Qt.resolvedUrl("../pages/CameraPage.qml"), { photoOnly: true })
        camera.captured.connect(function(path) { chooser.edit(path) })
    }
    function edit(path) {
        forget(waiting)
        waiting = path
        editorLater.start()
    }
    // A photo just taken isn't kept (a picked one isn't ours: this leaves it alone).
    function forget(path) {
        if (path) backend.removeCapture(path)
        if (path === editing) editing = ""
    }
    function openEditor() {
        forget(editing)
        var source = waiting
        waiting = ""
        editing = source
        var editor = pageStack.push(Qt.resolvedUrl("../pages/EditPhotoPage.qml"), { path: source, square: true })
        editor.accepted.connect(function() {
            editor.save(function(path) {
                chooser.forget(source)
                if (path) {
                    chooser.chosen(path)
                } else {
                    Notices.show(qsTrId("threesea-la-edit-photo-failed"), Notice.Long, Notice.Center)
                }
                return ""
            })
        })
        editor.rejected.connect(function() { chooser.forget(source) })
    }

    Component.onDestruction: {
        backend.removeCapture(waiting)
        backend.removeCapture(editing)
    }

    Timer {
        id: editorLater
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy || pageStack.currentPage !== chooser.host) return
            stop()
            chooser.openEditor()
        }
    }

    Component {
        id: picker
        ImagePickerPage {
            onSelectedContentPropertiesChanged: chooser.edit(selectedContentProperties.filePath)
        }
    }
}
