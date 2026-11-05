// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors

Page {
    id: page
    allowedOrientations: Orientation.All

    // Contact to open once the add dialog has finished closing (pushing during the dialog's
    // pop transition is ignored by the page stack).
    property string openWhenActive: ""

    function open(identity) {
        if (page.status === PageStatus.Active && !pageStack.busy) {
            pageStack.push(Qt.resolvedUrl("ContactPage.qml"), { identity: identity })
        } else {
            openWhenActive = identity
        }
    }

    // Our own profile for the first row (back from it: it may have changed).
    property var profile: ({})
    function loadProfile() {
        backend.call("profile", {}, function(ok, result) { if (ok) page.profile = result })
    }
    Component.onCompleted: loadProfile()
    Connections {
        target: backend
        onCoreEvent: if (name === "profileChanged") page.loadProfile()
    }

    onStatusChanged: {
        if (status === PageStatus.Active) loadProfile()
        if (status === PageStatus.Active && openWhenActive.length > 0) {
            var identity = openWhenActive
            openWhenActive = ""
            pageStack.push(Qt.resolvedUrl("ContactPage.qml"), { identity: identity })
        }
    }

    function addContact() {
        var dialog = pageStack.push(Qt.resolvedUrl("AddContactDialog.qml"))
        dialog.accepted.connect(function() {
            var identity = dialog.identity
            backend.call("addContact", { identity: identity }, function(ok, result, kind, message) {
                if (ok) {
                    backend.refreshContacts()
                    page.open(result.identity)
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    // Android's pull to refresh: the contacts' states and keys from the directory now, and the
    // address book matched again if that is switched on.
    property bool refreshing: false
    function refresh() {
        refreshing = true
        if (backend.addressBookSync) window.syncAddressBook(true)
        backend.call("refreshContacts", {}, function(ok, result, kind, message) {
            page.refreshing = false
            if (ok) backend.refreshContacts()
            else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
        })
    }

    SilicaListView {
        id: listView
        anchors.fill: parent
        model: backend.contacts

        // Busy while refreshing, like a list being synced in Jolla's apps.
        PullDownMenu {
            busy: page.refreshing
            MenuItem {
                // The chat list's, which this page is attached to; there it gives way while offline.
                text: qsTrId("threesea-me-new-list")
                onClicked: pageStack.find(function(p) { return p.objectName === "conversations" }).newList()
            }
            MenuItem {
                //% "Refresh"
                text: qsTrId("threesea-me-refresh")
                enabled: !page.refreshing
                onClicked: page.refresh()
            }
            MenuItem {
                //% "Scan QR code"
                text: qsTrId("threesea-he-scan-qr")
                onClicked: pageStack.push(Qt.resolvedUrl("ScanQrPage.qml"), { anyCode: true })
            }
            MenuItem {
                //% "Add contact"
                text: qsTrId("threesea-me-add-contact")
                onClicked: page.addContact()
            }
        }

        // The header, then our own card (like a contact page's), above the contacts as People
        // shows "Me" first.
        header: Column {
            width: listView.width

            PageHeader {
                //% "Contacts"
                title: qsTrId("threesea-he-contacts")
            }

            // Our own card, like the contact pages' (Jolla People's card): tap for the profile.
            MouseArea {
                id: me
                width: parent.width
                height: myCard.height + Theme.paddingLarge
                onClicked: pageStack.push(Qt.resolvedUrl("settings/ProfileSettingsPage.qml"))
                ProfileHeader {
                    id: myCard
                    topSpace: 0
                    pinnable: false
                    pictureTappable: true
                    // Our QR code, for others to scan (the theme's icon-m-qr is a viewfinder, i.e. scanning).
                    actionIcon: Qt.resolvedUrl("../icons/qr-code.png")
                    onActionClicked: pageStack.push(Qt.resolvedUrl("MyQrCodePage.qml"))
                    onPictureClicked: me.clicked(null)
                    highlighted: me.pressed && me.containsMouse
                    //% "Me"
                    title: page.profile.nickname ? "~" + page.profile.nickname : qsTrId("threesea-la-me")
                    description: page.profile.identity || backend.identity
                    source: page.profile.pictureVersion > 0
                            ? "image://threesea/contact/me?" + page.profile.pictureVersion : ""
                }
            }
        }

        section {
            property: "section"
            delegate: SectionHeader { text: section }
        }

        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeExtraLarge
            // The row opens the chat, the picture the contact's details.
            onClicked: pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: model.identity })

            Thumbnail {
                id: thumbnail
                height: item.contentHeight
                highlighted: item.highlighted || pressed
                pressed: pictureArea.pressed && pictureArea.containsMouse
                source: model.pictureVersion > 0
                        ? "image://threesea/contact/" + model.identity + "?" + model.pictureVersion : ""
                MouseArea {
                    id: pictureArea
                    anchors.fill: parent
                    onClicked: pageStack.push(Qt.resolvedUrl("ContactPage.qml"), { identity: model.identity })
                    onPressAndHold: item.openMenu()
                }
            }

            function remove() {
                //% "Deleting contact and chat"
                remorseAction(qsTrId("threesea-la-deleting-contact"), function() {
                    backend.call("deleteContact", { identity: model.identity }, function() {
                        backend.refreshContacts()
                    })
                })
            }

            menu: Component {
                ContextMenu {
                    MenuItem {
                        //% "Delete"
                        text: qsTrId("threesea-me-delete")
                        onClicked: item.remove()
                    }
                }
            }

            Column {
                anchors {
                    left: thumbnail.right
                    leftMargin: Theme.paddingLarge
                    right: indicator.left
                    rightMargin: Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                Label {
                    width: parent.width
                    text: model.displayName
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeLarge
                    color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
                    opacity: model.invalid || model.inactive ? Theme.opacityHigh : 1.0
                }
                Label {
                    width: parent.width
                    visible: model.displayName !== model.identity
                    text: model.identity
                    font.pixelSize: Theme.fontSizeMedium
                    color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                }
            }

            VerificationIndicator {
                id: indicator
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                level: model.verification
                highlighted: item.highlighted
            }
        }

        ViewPlaceholder {
            enabled: listView.count === 0
            // Below the own profile row.
            verticalOffset: listView.headerItem ? listView.headerItem.height / 2 : 0
            //% "No contacts"
            text: qsTrId("threesea-la-no-contacts")
            //% "Pull down to add a contact by Threema ID"
            hintText: qsTrId("threesea-la-no-contacts-hint")
        }

        VerticalScrollDecorator { }
    }
}
