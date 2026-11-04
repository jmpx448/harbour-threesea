// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"

// Data backup, last step: what the import brought, in one place. Replaces the progress page;
// back leads to the backup page.
Page {
    id: page
    allowedOrientations: Orientation.All

    // The metadata import's counts, the history job's summary, and the error if one ended it.
    property var imported: null
    property var summary: null
    property string error

    readonly property bool stopped: !!summary && summary.cancelled === true

    readonly property int pictures: !imported ? 0
                                    : imported.pictures.contacts + imported.pictures.groups
                                      + (imported.pictures.own ? 1 : 0)
    readonly property int nonces: imported && imported.nonces
                                  ? imported.nonces.csp + imported.nonces.d2d : 0
    readonly property bool noncesMissing: !!imported && !!imported.nonces
                                          && imported.nonces.missing === true
    readonly property int existing: (imported ? imported.contacts.existing + imported.groups.existing
                                                + imported.lists.existing : 0)
                                    + (summary ? summary.existing : 0)
    readonly property int unsupported: {
        if (!summary) return 0
        var sum = 0
        for (var category in summary.unsupported) sum += summary.unsupported[category]
        return sum
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                title: page.error.length > 0
                       //% "Import failed"
                       ? qsTrId("threesea-he-import-failed")
                       : page.stopped
                         //% "Import stopped"
                         ? qsTrId("threesea-he-import-stopped")
                         //% "Import finished"
                         : qsTrId("threesea-he-import-finished")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: page.error.length > 0
                      ? page.error
                      : page.stopped
                        //% "What is imported stays. Import the backup again to continue where it stopped."
                        ? qsTrId("threesea-la-data-backup-resume-hint")
                        //% "The backup is imported. Everything already on this device stayed as it was."
                        : qsTrId("threesea-la-data-backup-imported")
            }

            Label {
                visible: page.error.length > 0
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTrId("threesea-la-data-backup-resume-hint")
            }

            SectionHeader {
                visible: page.imported !== null
                //% "Contacts and groups"
                text: qsTrId("threesea-se-import-contacts-groups")
            }

            DetailItem {
                visible: page.imported !== null
                //% "Contacts"
                label: qsTrId("threesea-la-android-backup-imported-contacts")
                value: page.imported ? page.imported.contacts.added : 0
            }

            DetailItem {
                visible: page.imported !== null
                //% "Groups"
                label: qsTrId("threesea-la-android-backup-imported-groups")
                value: page.imported ? page.imported.groups.added : 0
            }

            DetailItem {
                visible: page.imported !== null && page.imported.lists.added > 0
                //% "Distribution lists"
                label: qsTrId("threesea-la-android-backup-lists")
                value: page.imported ? page.imported.lists.added : 0
            }

            DetailItem {
                visible: page.imported !== null
                //% "Pictures"
                label: qsTrId("threesea-la-android-backup-pictures")
                value: page.pictures
            }

            DetailItem {
                visible: page.nonces > 0
                //% "Replay protection entries"
                label: qsTrId("threesea-la-android-backup-nonces")
                value: page.nonces
            }

            Label {
                visible: page.noncesMissing
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                //% "This backup holds no replay protection entries. Protection covers what this device receives from now on."
                text: qsTrId("threesea-la-data-backup-nonces-missing")
            }

            DetailItem {
                visible: page.imported !== null && page.imported.contacts.keyConflicts > 0
                //% "Conflicting keys"
                label: qsTrId("threesea-la-android-backup-key-conflicts")
                value: page.imported ? page.imported.contacts.keyConflicts : 0
            }

            SectionHeader {
                visible: page.summary !== null
                //% "Message history"
                text: qsTrId("threesea-se-android-backup-history")
            }

            DetailItem {
                visible: page.summary !== null
                //% "Chats"
                label: qsTrId("threesea-la-android-backup-chats")
                value: page.summary ? page.summary.chats : 0
            }

            DetailItem {
                visible: page.summary !== null
                //% "Messages"
                label: qsTrId("threesea-la-android-import-messages")
                value: page.summary ? page.summary.imported : 0
            }

            DetailItem {
                visible: page.summary !== null
                //% "Media files"
                label: qsTrId("threesea-la-android-backup-media")
                value: page.summary ? page.summary.mediaFiles : 0
            }

            DetailItem {
                visible: page.summary !== null && page.summary.reactions > 0
                //% "Reactions"
                label: qsTrId("threesea-la-android-import-reactions")
                value: page.summary ? page.summary.reactions : 0
            }

            DetailItem {
                visible: page.summary !== null && page.summary.polls > 0
                //% "Polls"
                label: qsTrId("threesea-la-android-import-polls")
                value: page.summary ? page.summary.polls : 0
            }

            DetailItem {
                visible: page.existing > 0
                //% "Already on this device"
                label: qsTrId("threesea-la-android-backup-existing")
                value: page.existing
            }

            DetailItem {
                visible: page.unsupported > 0
                //% "Not supported yet"
                label: qsTrId("threesea-la-android-import-unsupported")
                value: page.unsupported
            }

            DetailItem {
                visible: page.summary !== null && page.summary.mediaFailed > 0
                //% "Media not imported"
                label: qsTrId("threesea-la-android-import-media-failed")
                value: page.summary ? page.summary.mediaFailed : 0
            }

            DetailItem {
                visible: page.summary !== null && page.summary.uidConflicts > 0
                //% "Messages also found in another chat"
                label: qsTrId("threesea-la-data-backup-uid-conflicts")
                value: page.summary ? page.summary.uidConflicts : 0
            }

            DetailItem {
                visible: page.summary !== null && page.summary.foreignGroupChats > 0
                //% "Group chats mostly from non-members"
                label: qsTrId("threesea-la-data-backup-foreign-group-chats")
                value: page.summary ? page.summary.foreignGroupChats : 0
            }
        }

        VerticalScrollDecorator { }
    }
}
