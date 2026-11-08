// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

// The block list (Settings → Privacy), like Android's "Blocked contacts": long press to unblock,
// pulley to block an ID that is not a contact. With `exclusion` the same for Android's exclusion
// list: IDs that address book matching leaves alone.
Page {
    id: page
    allowedOrientations: Orientation.All

    property bool exclusion: false
    property var blocked: []

    function load() {
        backend.call(exclusion ? "syncExcluded" : "blockedContacts", {}, function(ok, result) {
            if (ok) page.blocked = exclusion ? result.excluded : result.blocked
        })
    }

    function setBlocked(identity, value) {
        var args = exclusion ? { identity: identity, excluded: value } : { identity: identity, blocked: value }
        backend.call(exclusion ? "setSyncExcluded" : "setBlocked", args, function(ok, result, kind, message) {
            if (ok) page.load()
            else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
        })
    }

    Component.onCompleted: load()
    Connections {
        target: backend
        onCoreEvent: if (name === "settingsChanged") page.load()
    }

    SilicaListView {
        anchors.fill: parent
        model: page.blocked

        PullDownMenu {
            MenuItem {
                text: page.exclusion
                      //% "Add a Threema ID"
                      ? qsTrId("threesea-me-exclude-id")
                      //% "Block a Threema ID"
                      : qsTrId("threesea-me-block-id")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("AddContactDialog.qml"), {
                        acceptText: page.exclusion
                                    //% "Add"
                                    ? qsTrId("threesea-he-add")
                                    //% "Block"
                                    : qsTrId("threesea-he-block"),
                        intro: page.exclusion
                               //% "Finding contacts from the address book will not add or rename this Threema ID."
                               ? qsTrId("threesea-la-exclusion-intro")
                               //% "Messages from this Threema ID will be discarded, and you cannot write to it."
                               : qsTrId("threesea-la-block-intro")
                    })
                    dialog.accepted.connect(function() { page.setBlocked(dialog.identity.trim(), true) })
                }
            }
        }

        header: PageHeader {
            title: page.exclusion
                   ? qsTrId("threesea-he-exclusion-list")
                   //% "Blocked contacts"
                   : qsTrId("threesea-he-blocked-contacts")
        }

        delegate: ListItem {
            id: item
            menu: Component {
                ContextMenu {
                    MenuItem {
                        text: page.exclusion
                              //% "Remove"
                              ? qsTrId("threesea-me-remove-excluded")
                              //% "Unblock"
                              : qsTrId("threesea-me-unblock")
                        onClicked: page.setBlocked(modelData.identity, false)
                    }
                }
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                text: modelData.displayName !== modelData.identity
                      ? modelData.displayName + " (" + modelData.identity + ")" : modelData.identity
                truncationMode: TruncationMode.Fade
                color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
            }
        }

        ViewPlaceholder {
            enabled: page.blocked.length === 0
            text: page.exclusion
                  //% "Nobody is excluded"
                  ? qsTrId("threesea-la-no-excluded")
                  //% "No blocked contacts"
                  : qsTrId("threesea-la-no-blocked")
        }

        VerticalScrollDecorator { }
    }
}
