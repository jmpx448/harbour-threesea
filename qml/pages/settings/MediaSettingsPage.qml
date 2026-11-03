// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"
import "../../components/Errors.js" as Errors

// Settings → Media & storage (Android's): image size, auto-save to the Gallery, what downloads
// by itself on Wi-Fi and on mobile data, and the space used with a clean-up.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var media: ({})
    property var storage: ({})

    readonly property var kinds: ["image", "audio", "video", "file"]
    readonly property var kindNames: [
        qsTrId("threesea-la-kind-photos"),
        //% "Voice messages"
        qsTrId("threesea-la-kind-voice"),
        qsTrId("threesea-la-kind-videos"),
        qsTrId("threesea-la-kind-files")
    ]
    // Android's auto-delete: how long messages stay (0: for good).
    readonly property var keepDays: [0, 365, 180, 90, 30, 7]
    readonly property var keepNames: [
        //% "For good"
        qsTrId("threesea-me-keep-forever"),
        //% "1 year"
        qsTrId("threesea-me-keep-1-year"),
        //% "6 months"
        qsTrId("threesea-me-keep-6-months"),
        //% "3 months"
        qsTrId("threesea-me-keep-3-months"),
        //% "1 month"
        qsTrId("threesea-me-keep-1-month"),
        //% "1 week"
        qsTrId("threesea-me-keep-1-week")
    ]
    // Android's video sizes (VideoConfig): the longest edge, or the video as it is.
    readonly property var videoSizeNames: [
        //% "Original size"
        qsTrId("threesea-me-image-original"),
        //% "Small (480 px)"
        qsTrId("threesea-me-video-small"),
        //% "Medium (848 px)"
        qsTrId("threesea-me-video-medium")
    ]
    // Largest file that comes by itself, in MiB (0: any size). Android's limit is a fixed 5 MB.
    readonly property var limits: [0, 1, 5, 20, 100]
    readonly property var limitNames: [
        //% "Any size"
        qsTrId("threesea-me-limit-any"),
        //% "Up to 1 MB"
        qsTrId("threesea-me-limit-1"),
        //% "Up to 5 MB"
        qsTrId("threesea-me-limit-5"),
        //% "Up to 20 MB"
        qsTrId("threesea-me-limit-20"),
        //% "Up to 100 MB"
        qsTrId("threesea-me-limit-100")
    ]
    readonly property var sizes: [640, 1024, 1600, 2592, 0]
    readonly property var sizeNames: [
        //% "Small (640 × 640)"
        qsTrId("threesea-me-image-small"),
        //% "Medium (1024 × 1024)"
        qsTrId("threesea-me-image-medium"),
        //% "Large (1600 × 1600)"
        qsTrId("threesea-me-image-large"),
        //% "Extra large (2592 × 2592)"
        qsTrId("threesea-me-image-xlarge"),
        //% "Original size"
        qsTrId("threesea-me-image-original")
    ]

    function load() {
        backend.call("mediaSettings", {}, function(ok, result) { if (ok) page.media = result })
        backend.call("storageInfo", {}, function(ok, result) { if (ok) page.storage = result })
    }

    function set(args) {
        backend.call("setMediaSettings", args, function(ok, result) { if (ok) page.media = result })
    }

    function kindsText(list) {
        if (!list || list.length === 0) {
            //% "Nothing"
            return qsTrId("threesea-la-download-nothing")
        }
        return kinds.filter(function(k) { return list.indexOf(k) >= 0 })
                    .map(function(k) { return kindNames[kinds.indexOf(k)] }).join(", ")
    }

    function chooseKinds(field, title) {
        var dialog = pageStack.push(Qt.resolvedUrl("../MediaFilterDialog.qml"), {
            kinds: kinds, names: kindNames, selected: (media[field] || []).slice(), title: title,
            allowNone: true
        })
        dialog.accepted.connect(function() {
            var args = {}
            args[field] = dialog.selected
            page.set(args)
        })
    }

    Component.onCompleted: load()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Media and storage"
                title: qsTrId("threesea-se-media-storage")
            }

            SectionHeader {
                //% "Sending"
                text: qsTrId("threesea-se-sending")
            }

            ChoiceBox {
                //% "Image size"
                label: qsTrId("threesea-la-image-size")
                options: page.sizeNames
                choice: Math.max(0, page.sizes.indexOf(backend.imageSize))
                onChosen: backend.imageSize = page.sizes[index]
            }

            ChoiceBox {
                // Scaling needs the system's GStreamer: only the Full flavour has it.
                visible: backend.full
                //% "Video size"
                label: qsTrId("threesea-la-video-size")
                options: page.videoSizeNames
                choice: backend.videoSize
                //% "Smaller videos are quicker to send and use less data. The sound is kept as it is."
                description: qsTrId("threesea-la-video-size-description")
                onChosen: backend.videoSize = index
            }

            SectionHeader {
                //% "Receiving"
                text: qsTrId("threesea-se-receiving")
            }

            ValueButton {
                //% "Download on Wi-Fi"
                label: qsTrId("threesea-la-download-wifi")
                value: page.kindsText(page.media.autoDownloadWifi)
                onClicked: page.chooseKinds("autoDownloadWifi", label)
            }

            ValueButton {
                //% "Download on mobile data"
                label: qsTrId("threesea-la-download-mobile")
                value: page.kindsText(page.media.autoDownloadMobile)
                onClicked: page.chooseKinds("autoDownloadMobile", label)
            }

            ChoiceBox {
                //% "File size"
                label: qsTrId("threesea-la-download-limit")
                options: page.limitNames
                choice: Math.max(0, page.limits.indexOf(page.media.autoDownloadLimit || 0))
                //% "Larger files wait in the chat until you tap them. Previews always come."
                description: qsTrId("threesea-la-download-limit-description")
                onChosen: page.set({ autoDownloadLimit: page.limits[index] })
            }

            TextSwitch {
                visible: page.media.picturesAvailable === true
                automaticCheck: false
                checked: page.media.autoSave === true
                //% "Save to Gallery"
                text: qsTrId("threesea-la-auto-save")
                //% "Received photos and videos are also saved, unencrypted, to the Threesea folder in Pictures."
                description: qsTrId("threesea-la-auto-save-description")
                onClicked: page.set({ autoSave: !checked })
            }

            SectionHeader {
                //% "Storage"
                text: qsTrId("threesea-se-storage")
            }

            DetailItem {
                //% "Media"
                label: qsTrId("threesea-he-media")
                value: Format.formatFileSize(page.storage.mediaBytes || 0)
            }
            DetailItem {
                //% "Messages"
                label: qsTrId("threesea-la-storage-messages")
                value: Format.formatFileSize(page.storage.databaseBytes || 0)
            }

            ChoiceBox {
                //% "Keep messages"
                label: qsTrId("threesea-la-keep-messages")
                options: page.keepNames
                choice: Math.max(0, page.keepDays.indexOf(page.media.autoDeleteDays || 0))
                //% "Older messages are deleted by themselves, with their media. Starred messages, open polls and messages still on their way stay."
                description: qsTrId("threesea-la-keep-messages-description")
                onChosen: page.set({ autoDeleteDays: page.keepDays[index] })
            }

            SettingsButton {
                //% "Clean up"
                text: qsTrId("threesea-me-clean-up")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("../CleanUpDialog.qml"))
                    dialog.accepted.connect(function() {
                        var args = { days: dialog.days, messages: dialog.messages }
                        //% "Deleting"
                        Remorse.popupAction(page, qsTrId("threesea-la-deleting-message"), function() {
                            backend.call("cleanUp", args, function(ok, result, kind, message) {
                                if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                                page.load()
                            })
                        })
                    })
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
