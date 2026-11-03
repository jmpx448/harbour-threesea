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

    function load() {
        backend.call("contact", { identity: identity }, function(ok, result) {
            if (ok) page.contact = result
        })
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
                //% "Delete"
                text: qsTrId("threesea-me-delete")
                //% "Deleting contact"
                onClicked: remorse.execute(qsTrId("threesea-la-deleting-contact"), function() {
                    backend.call("deleteContact", { identity: page.identity }, function() {
                        backend.refreshContacts()
                        pageStack.pop()
                    })
                })
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
                // Sharing the profile picture with selected contacts (Settings); not with
                // ECHOECHO or gateway IDs, which never get it.
                visible: typeof page.contact.pictureRecipient === "boolean"
                         && page.identity !== "ECHOECHO" && page.identity.charAt(0) !== "*"
                text: page.contact.pictureRecipient
                      //% "Stop sharing profile picture"
                      ? qsTrId("threesea-me-stop-sharing-picture")
                      //% "Share profile picture"
                      : qsTrId("threesea-me-share-picture")
                onClicked: backend.call("setProfilePictureRecipient",
                                        { identity: page.identity, share: !page.contact.pictureRecipient },
                                        function(ok, result) { if (ok) page.contact = result })
            }
            MenuItem {
                visible: (page.contact.verification || 0) < 2
                //% "Verify by QR code"
                text: qsTrId("threesea-me-verify-qr")
                onClicked: {
                    var scan = pageStack.push(Qt.resolvedUrl("ScanQrPage.qml"), { expectedIdentity: page.identity })
                    scan.verified.connect(function(contact) { page.contact = contact })
                }
            }
            MenuItem {
                //% "Edit"
                text: qsTrId("threesea-me-edit")
                onClicked: page.edit()
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

            ChatNotificationsButton {
                identity: page.identity
                settings: page.contact.notifications || ({})
            }
            // Device-local, like the tone of all chats.
            ToneButton {
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
                text: qsTrId("threesea-se-privacy")
            }
            ContactPolicyButton {
                identity: page.identity
                field: "readReceipts"
                choice: page.contact.readReceipts || 0
                appDefault: window.appSettings.readReceipts !== false
                //% "Read receipts"
                label: qsTrId("threesea-la-contact-read-receipts")
                onChanged: page.contact = contact
            }

            ContactPolicyButton {
                identity: page.identity
                field: "typing"
                choice: page.contact.typing || 0
                appDefault: window.appSettings.sendTyping !== false
                //% "Typing indicator"
                label: qsTrId("threesea-la-contact-typing")
                onChanged: page.contact = contact
            }

            SectionHeader {
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
        }

        VerticalScrollDecorator { }
    }
}
