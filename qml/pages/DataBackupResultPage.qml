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
    property string errorKind

    // What can pass: room on the phone, a store that answers again. A file that is refused stays
    // refused, however often it is imported.
    readonly property bool errorCanPass: ["diskFull", "storeUnreadable", "store", "internal"]
                                         .indexOf(errorKind) >= 0

    readonly property bool stopped: !!summary && summary.cancelled === true

    readonly property int pictures: !imported ? 0
                                    : imported.pictures.contacts + imported.pictures.groups
                                      + (imported.pictures.own ? 1 : 0)
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
    readonly property int keyConflicts: imported ? imported.contacts.keyConflicts : 0
    readonly property int groupsLeftOut: imported ? imported.groups.skippedDeleted
                                                    + imported.groups.skippedUnknownCreator : 0
    readonly property int chatsLeftOut: summary ? summary.skippedChats + summary.keyConflictChats : 0
    readonly property int chatsTooLarge: summary ? summary.tooLargeChats : 0
    readonly property int chatsDamaged: summary ? summary.damagedChats : 0
    readonly property int entriesLeftOut: (imported ? imported.invalidRows + imported.pictures.skipped : 0)
                                          + (summary ? summary.invalidRows + summary.keyConflictRows : 0)
    readonly property int localAvatars: imported ? imported.pictures.localAvatarsNotImported : 0
    readonly property int collisions: summary ? summary.collisions : 0
    readonly property int reactionsLeftOut: summary ? summary.reactionsUnmatched : 0
    readonly property int mediaMissing: summary ? summary.mediaMissing : 0
    readonly property int mediaFailed: summary ? summary.mediaFailed : 0
    // Anything above zero: the headline must not say the whole backup arrived.
    readonly property int notImported: keyConflicts + groupsLeftOut + chatsLeftOut + chatsTooLarge + chatsDamaged
                                       + entriesLeftOut + localAvatars + collisions + reactionsLeftOut
                                       + mediaMissing + mediaFailed + unsupported

    function leftOutReason(reason) {
        switch (reason) {
        case "tooLarge":
            //% "too large to import"
            return qsTrId("threesea-la-data-backup-reason-too-large")
        case "damaged":
            //% "damaged in the backup"
            return qsTrId("threesea-la-data-backup-reason-damaged")
        case "otherKey":
            //% "a different key in the backup"
            return qsTrId("threesea-la-data-backup-reason-other-key")
        default:
            //% "not on this device"
            return qsTrId("threesea-la-data-backup-reason-not-here")
        }
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
                        : page.notImported > 0
                          //% "The backup is imported, except for what is listed below. Everything already on this device stayed as it was."
                          ? qsTrId("threesea-la-data-backup-imported-except")
                          //% "The backup is imported. Everything already on this device stayed as it was."
                          : qsTrId("threesea-la-data-backup-imported")
            }

            Label {
                visible: page.errorCanPass
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

            Label {
                visible: page.noncesMissing
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                //% "The list of messages already received isn't part of this import. Messages sent again are recognised from what this device receives from now on."
                text: qsTrId("threesea-la-data-backup-received-list-missing")
            }

            DetailItem {
                visible: page.keyConflicts > 0
                //% "Contacts with a different key (kept as here)"
                label: qsTrId("threesea-la-data-backup-key-conflicts")
                value: page.keyConflicts
            }

            DetailItem {
                visible: page.localAvatars > 0
                //% "Pictures chosen for contacts on the other device, left out"
                label: qsTrId("threesea-la-data-backup-local-avatars")
                value: page.localAvatars
            }

            DetailItem {
                visible: page.groupsLeftOut > 0
                //% "Groups left out"
                label: qsTrId("threesea-la-data-backup-groups-left-out")
                value: page.groupsLeftOut
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
                visible: page.summary !== null && page.summary.unsent > 0
                //% "Messages that were never sent (kept as failed)"
                label: qsTrId("threesea-la-data-backup-unsent")
                value: page.summary ? page.summary.unsent : 0
            }

            DetailItem {
                visible: page.chatsLeftOut > 0
                //% "Chats left out"
                label: qsTrId("threesea-la-data-backup-chats-left-out")
                value: page.chatsLeftOut
            }

            DetailItem {
                visible: page.chatsTooLarge > 0
                //% "Chats too large to import"
                label: qsTrId("threesea-la-data-backup-chats-too-large")
                value: page.chatsTooLarge
            }

            DetailItem {
                visible: page.chatsDamaged > 0
                //% "Damaged chats left out"
                label: qsTrId("threesea-la-data-backup-chats-damaged")
                value: page.chatsDamaged
            }

            DetailItem {
                visible: page.unsupported > 0
                //% "Messages left out"
                label: qsTrId("threesea-la-data-backup-messages-left-out")
                value: page.unsupported
            }

            DetailItem {
                visible: page.collisions > 0
                //% "Messages here from another sender (kept as here)"
                label: qsTrId("threesea-la-data-backup-collisions")
                value: page.collisions
            }

            DetailItem {
                visible: page.reactionsLeftOut > 0
                //% "Reactions left out"
                label: qsTrId("threesea-la-data-backup-reactions-left-out")
                value: page.reactionsLeftOut
            }

            DetailItem {
                visible: page.entriesLeftOut > 0
                //% "Entries left out"
                label: qsTrId("threesea-la-data-backup-entries-left-out")
                value: page.entriesLeftOut
            }

            DetailItem {
                visible: page.mediaMissing > 0
                //% "Media not in the backup"
                label: qsTrId("threesea-la-data-backup-media-missing")
                value: page.mediaMissing
            }

            DetailItem {
                visible: page.mediaFailed > 0
                //% "Media not imported"
                label: qsTrId("threesea-la-android-import-media-failed")
                value: page.mediaFailed
            }

            SectionHeader {
                visible: leftOutChats.count > 0
                //% "Chats that weren't imported"
                text: qsTrId("threesea-se-data-backup-left-out")
            }

            Repeater {
                id: leftOutChats
                model: page.summary ? page.summary.leftOutChats : []

                // Names come from other people: plain text.
                Label {
                    x: Theme.horizontalPageMargin
                    width: column.width - 2 * x
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.highlightColor
                    text: modelData.name + ": " + page.leftOutReason(modelData.reason)
                }
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
