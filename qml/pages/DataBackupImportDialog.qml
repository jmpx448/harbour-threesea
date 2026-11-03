// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Data backup, step 2: what the checked backup holds. Accepting imports it (the progress
// page replaces this one); a backup of another ID cannot be accepted.
Dialog {
    id: dialog
    allowedOrientations: Orientation.All

    // The inspection result (version, identity, inventory).
    property var preview
    property string filePath
    property string fileName
    property string password

    readonly property bool matches: preview !== null && preview !== undefined
                                    && preview.identity === "match"

    canAccept: matches
    acceptDestination: Qt.resolvedUrl("DataBackupImportPage.qml")
    acceptDestinationAction: PageStackAction.Replace
    acceptDestinationProperties: {
        "filePath": filePath,
        "password": password
    }

    Component.onDestruction: password = ""

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        Column {
            id: column
            width: parent.width

            DialogHeader {
                //% "Import"
                acceptText: qsTrId("threesea-he-import")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: dialog.matches
                      //% "Adds what is missing: contacts, groups and pictures, then messages and media. Everything already on this device stays as it is. Nothing is sent."
                      ? qsTrId("threesea-la-android-backup-import-intro")
                      //% "This backup belongs to another Threema ID or key. It can only be imported on the device of that ID."
                      : qsTrId("threesea-la-android-backup-import-other")
            }

            SectionHeader {
                //% "Contents"
                text: qsTrId("threesea-se-android-backup-contents")
            }

            DetailItem {
                label: qsTrId("threesea-la-android-backup-file")
                value: dialog.fileName
            }

            DetailItem {
                //% "Format version"
                label: qsTrId("threesea-la-android-backup-version")
                value: dialog.preview ? dialog.preview.version : ""
            }

            DetailItem {
                //% "Threema ID"
                label: qsTrId("threesea-la-android-backup-identity")
                value: {
                    if (!dialog.preview) return ""
                    switch (dialog.preview.identity) {
                    //% "This device's Threema ID"
                    case "match": return qsTrId("threesea-la-android-backup-id-match")
                    //% "Another Threema ID"
                    case "otherIdentity": return qsTrId("threesea-la-android-backup-id-other")
                    //% "Same ID, different key"
                    case "keyMismatch": return qsTrId("threesea-la-android-backup-id-key")
                    //% "Not readable with this password"
                    case "unreadable": return qsTrId("threesea-la-android-backup-id-unreadable")
                    //% "Not in the backup"
                    default: return qsTrId("threesea-la-android-backup-id-missing")
                    }
                }
            }

            DetailItem {
                //% "Chats"
                label: qsTrId("threesea-la-android-backup-chats")
                value: dialog.preview ? dialog.preview.inventory.contactChats : 0
            }

            DetailItem {
                //% "Group chats"
                label: qsTrId("threesea-la-android-backup-group-chats")
                value: dialog.preview ? dialog.preview.inventory.groupChats : 0
            }

            DetailItem {
                visible: !!dialog.preview && dialog.preview.inventory.listChats > 0
                //% "Distribution lists"
                label: qsTrId("threesea-la-android-backup-lists")
                value: dialog.preview ? dialog.preview.inventory.listChats : 0
            }

            DetailItem {
                //% "Media files"
                label: qsTrId("threesea-la-android-backup-media")
                value: !dialog.preview ? "" : dialog.preview.inventory.mediaEntries
                       + (dialog.preview.inventory.mediaBytes > 0
                          ? " (" + Format.formatFileSize(dialog.preview.inventory.mediaBytes) + ")" : "")
            }

            DetailItem {
                //% "Pictures"
                label: qsTrId("threesea-la-android-backup-pictures")
                value: !dialog.preview ? 0
                       : dialog.preview.inventory.contactAvatars
                         + dialog.preview.inventory.contactProfilePictures
                         + dialog.preview.inventory.groupAvatars
                         + (dialog.preview.inventory.ownAvatar ? 1 : 0)
            }

            Item { width: 1; height: Theme.paddingLarge }

            Label {
                visible: dialog.matches
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                //% "A large backup can take a while. You can stop the import; importing again continues where it stopped."
                text: qsTrId("threesea-la-android-backup-import-duration")
            }
        }
    }
}
