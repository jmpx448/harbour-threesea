// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import QtMultimedia 5.6
import "../components/Format.js" as Fmt

// Choose a notification tone, like Jolla's sound settings: the default first, "Select from
// music", then the system's tones; choosing one plays it. Nothing changes until the dialog is
// accepted. `tone`: the chosen file ("": the default).
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    property string tone
    // What "" means here, e.g. the sound profile's tone or the tone of all chats.
    property string defaultText
    readonly property var tones: backend.alertTones()
    // A music file (not one of the system's tones), chosen before or here.
    property string customPath

    Component.onCompleted: if (tone.length > 0 && !isSystemTone(tone)) customPath = tone

    function isSystemTone(path) {
        for (var i = 0; i < tones.length; ++i) {
            if (tones[i].path === path) return true
        }
        return false
    }

    function choose(path) {
        tone = path
        preview.stop()
        if (path.length > 0) {
            preview.source = Fmt.fileUrl(path)
            preview.play()
        }
    }

    onDone: preview.stop()

    Audio {
        id: preview
    }

    Component {
        id: musicPicker
        MusicPickerPage {
            onSelectedContentPropertiesChanged: {
                dialog.customPath = selectedContentProperties.filePath
                dialog.choose(selectedContentProperties.filePath)
            }
        }
    }

    // A column, not a list with a header: the header's height settles after the list has
    // positioned itself, which left it scrolled past the top.
    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            DialogHeader { }

            TextSwitch {
                automaticCheck: false
                checked: dialog.tone.length === 0
                text: dialog.defaultText
                onClicked: dialog.choose("")
            }

            // A music file: shown while chosen.
            TextSwitch {
                visible: dialog.customPath.length > 0
                automaticCheck: false
                checked: dialog.tone === dialog.customPath
                text: backend.toneTitle(dialog.customPath)
                onClicked: dialog.choose(dialog.customPath)
            }

            BackgroundItem {
                id: selectFromMusic
                width: parent.width
                onClicked: {
                    preview.stop()
                    pageStack.push(musicPicker)
                }
                Image {
                    id: musicIcon
                    x: Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    source: "image://theme/icon-m-sounds" + (selectFromMusic.highlighted ? "?" + Theme.highlightColor : "")
                }
                Label {
                    anchors {
                        left: musicIcon.right
                        leftMargin: Theme.paddingMedium
                        right: parent.right
                        rightMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    //% "Select from music"
                    text: qsTrId("threesea-la-tone-from-music")
                    color: selectFromMusic.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }

            SectionHeader {
                //% "Tones"
                text: qsTrId("threesea-se-tones")
            }

            Repeater {
                model: dialog.tones
                TextSwitch {
                    automaticCheck: false
                    checked: dialog.tone === modelData.path
                    text: modelData.title
                    onClicked: dialog.choose(modelData.path)
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
