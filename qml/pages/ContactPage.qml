// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../components"
import "../components/Errors.js" as Errors
import "../components/Verification.js" as Verification

Page {
    id: page
    allowedOrientations: Orientation.All

    property string identity
    property var contact: ({})
    // An ID without a contact (earlier versions left chats like that behind).
    property bool noContact: false
    // Attached to this contact's chat: that is one swipe back, nothing to offer here.
    property bool besideChat: false
    onStatusChanged: {
        if (status === PageStatus.Activating || status === PageStatus.Active) {
            var below = pageStack.previousPage(page)
            besideChat = !!below && below.inputField !== undefined && below.identity === identity
        }
    }

    function load() {
        backend.call("contact", { identity: identity }, function(ok, result, kind) {
            if (ok) {
                page.noContact = false
                page.contact = result
            } else if (kind === "contactNotFound") {
                page.noContact = true
            }
        })
    }

    function addToContacts() {
        backend.call("addContact", { identity: identity }, function(ok, result, kind, message) {
            if (ok) {
                page.noContact = false
                page.contact = result
                backend.refreshContacts()
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
    }

    // The chat went with the contact: back past it, if this page is attached to it.
    function leaveDeleted() {
        var below = pageStack.previousPage(page)
        if (below && below.inputField !== undefined && below.identity === identity) {
            below = pageStack.previousPage(below)
        }
        pageStack.pop(below || undefined)
    }

    function edit() {
        var dialog = pageStack.push(Qt.resolvedUrl("EditContactDialog.qml"),
                                    { firstName: contact.firstName || "", lastName: contact.lastName || "" })
        dialog.accepted.connect(function() {
            backend.call("updateContact", { identity: identity, firstName: dialog.firstName, lastName: dialog.lastName },
                         function(ok, result, kind, message) {
                if (ok) {
                    page.contact = result
                    backend.refreshContacts()
                } else {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    Component.onCompleted: load()

    Connections {
        target: backend
        onCoreEvent: if (name === "contactsChanged") page.load()
    }

    RemorsePopup { id: remorse }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            MenuItem {
                // Someone a group of ours needs stays until they leave it.
                visible: page.contact.neededByGroup !== true
                //% "Delete"
                text: qsTrId("threesea-me-delete")
                //% "Deleting contact and chat"
                onClicked: remorse.execute(qsTrId("threesea-la-deleting-contact"), function() {
                    backend.call("deleteContact", { identity: page.identity }, function() {
                        backend.refreshContacts()
                        page.leaveDeleted()
                    })
                })
            }
            MenuItem {
                visible: page.contact.hidden === true || page.noContact
                //% "Add to contacts"
                text: qsTrId("threesea-me-add-to-contacts")
                onClicked: page.addToContacts()
            }
            MenuItem {
                text: page.contact.blocked
                      //% "Unblock"
                      ? qsTrId("threesea-me-unblock")
                      //% "Block"
                      : qsTrId("threesea-me-block")
                onClicked: backend.call("setBlocked", { identity: page.identity, blocked: !page.contact.blocked },
                                        function(ok, result) {
                    if (ok) page.contact = result
                })
            }
            MenuItem {
                // Not saved yet: added first.
                visible: !page.noContact && page.contact.hidden !== true
                //% "Edit"
                text: qsTrId("threesea-me-edit")
                onClicked: page.edit()
            }
            MenuItem {
                // The chat asks for the PIN itself if it is a private one. A blocked or revoked
                // contact's chat has no input.
                visible: !page.besideChat && !page.noContact
                         && page.contact.blocked !== true && page.contact.invalid !== true
                //% "Write message"
                text: qsTrId("threesea-me-write-message")
                onClicked: pageStack.push(Qt.resolvedUrl("ChatPage.qml"), { identity: page.identity })
            }
        }

        Column {
            id: column
            width: parent.width

            ProfileHeader {
                pinned: page.contact.pinned === true
                onPinToggled: backend.call("setPinned", { identity: page.identity, pinned: !pinned },
                                           function() { page.load() })
                title: page.contact.displayName || page.identity
                description: page.identity
                descriptionHoldable: true
                onDescriptionHeld: identityMenu.open(menuHost)
                source: page.contact.pictureVersion > 0
                        ? "image://threesea/contact/" + page.identity + "?" + page.contact.pictureVersion : ""
                Row {
                    y: Theme.paddingMedium
                    spacing: Theme.paddingMedium
                    VerificationIndicator {
                        anchors.verticalCenter: parent.verticalCenter
                        level: page.contact.verification || 0
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        text: Verification.text(page.contact.verification || 0)
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.secondaryColor
                    }
                }
            }

            // The ID's menu opens right under the card.
            Item {
                id: menuHost
                width: parent.width
                height: identityMenu.height
            }

            IdentityMenu {
                id: identityMenu
                identity: page.identity
            }

            // Only when the name on the card is our own one for them (else it is the nickname).
            DetailItem {
                visible: !!page.contact.nickname && page.contact.displayName !== "~" + page.contact.nickname
                //% "Nickname"
                label: qsTrId("threesea-la-nickname")
                value: page.contact.nickname ? "~" + page.contact.nickname : ""
            }

            DetailItem {
                visible: !!page.contact.work
                //% "Type"
                label: qsTrId("threesea-la-type")
                //% "Threema Work"
                value: qsTrId("threesea-la-threema-work")
            }

            DetailItem {
                visible: !!page.contact.inactive || !!page.contact.invalid || !!page.contact.blocked
                //% "Status"
                label: qsTrId("threesea-la-status")
                //% "Blocked"
                value: page.contact.blocked ? qsTrId("threesea-la-blocked")
                       : page.contact.invalid
                       //% "Revoked"
                       ? qsTrId("threesea-la-revoked")
                       //% "Inactive"
                       : qsTrId("threesea-la-inactive")
            }

            SectionHeader {
                //% "Chat"
                text: qsTrId("threesea-se-chat")
            }
            MediaButton {
                identity: page.identity
                name: page.contact.displayName || page.identity
            }
            ExportChatButton {
                visible: !page.noContact
                identity: page.identity
                name: page.contact.displayName || page.identity
            }

            ChatNotificationsButton {
                visible: !page.noContact
                identity: page.identity
                settings: page.contact.notifications || ({})
            }
            // Device-local, like the tone of all chats.
            ToneButton {
                visible: !page.noContact
                property int revision: 0
                tone: revision >= 0 ? backend.chatTone(page.identity) : ""
                //% "Same as all chats"
                defaultText: qsTrId("threesea-la-tone-all-chats")
                onChosen: {
                    backend.setChatTone(page.identity, tone)
                    revision++
                }
            }

            SectionHeader {
                visible: !page.noContact
                text: qsTrId("threesea-se-privacy")
            }
            ContactPolicyButton {
                visible: !page.noContact
                identity: page.identity
                field: "readReceipts"
                choice: page.contact.readReceipts || 0
                appDefault: window.appSettings.readReceipts !== false
                //% "Read receipts"
                label: qsTrId("threesea-la-contact-read-receipts")
                onChanged: page.contact = contact
            }

            ContactPolicyButton {
                visible: !page.noContact
                identity: page.identity
                field: "typing"
                choice: page.contact.typing || 0
                appDefault: window.appSettings.sendTyping !== false
                //% "Typing indicator"
                label: qsTrId("threesea-la-contact-typing")
                onChanged: page.contact = contact
            }

            TextSwitch {
                // Sharing the profile picture with selected contacts (Settings); not with
                // ECHOECHO or gateway IDs, which never get it.
                visible: !page.noContact && typeof page.contact.pictureRecipient === "boolean"
                         && page.identity !== "ECHOECHO" && page.identity.charAt(0) !== "*"
                automaticCheck: false
                checked: page.contact.pictureRecipient === true
                //% "Share profile picture"
                text: qsTrId("threesea-la-contact-share-picture")
                onClicked: backend.call("setProfilePictureRecipient",
                                        { identity: page.identity, share: !page.contact.pictureRecipient },
                                        function(ok, result) { if (ok) page.contact = result })
            }

            SectionHeader {
                visible: !page.noContact
                //% "Public key"
                text: qsTrId("threesea-se-public-key")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                text: Verification.formatKey(page.contact.publicKey)
                font.family: "monospace"
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                bottomPadding: Theme.paddingLarge
            }

            Button {
                // Here beside the key it is about; the pulley has its four items.
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !page.noContact && (page.contact.verification || 0) < 2
                //% "Verify by QR code"
                text: qsTrId("threesea-me-verify-qr")
                onClicked: {
                    var scan = pageStack.push(Qt.resolvedUrl("ScanQrPage.qml"), { expectedIdentity: page.identity })
                    scan.verified.connect(function(contact) { page.contact = contact })
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
