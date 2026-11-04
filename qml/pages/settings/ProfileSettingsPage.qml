// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "../../components"
import "../../components/Errors.js" as Errors
import "../../components/Verification.js" as Verification

// My profile (Settings, and the first row of Contacts), laid out like a contact's page: the
// People card with picture, nickname and ID, then nickname, revocation password, linked e-mail
// and number, who gets the picture, the public key; picture, QR code and removing the ID in the
// pulley (Android: My ID).
Page {
    id: page
    allowedOrientations: Orientation.All

    property var profile: ({})
    // "email" / "mobile" while a link request runs.
    property string linking: ""

    function loadProfile() {
        backend.call("profile", {}, function(ok, result) { if (ok) page.profile = result })
    }

    function link(command, args, kind, done) {
        linking = kind
        backend.call(command, args, function(ok, result, errorKind, message) {
            page.linking = ""
            if (ok) {
                page.profile = result
                if (done) done(result)
            } else {
                Notices.show(Errors.text(errorKind, message), Notice.Long, Notice.Center)
            }
        })
    }

    // The SMS code of a pending phone link.
    function askCode() {
        var dialog = pageStack.push(Qt.resolvedUrl("../MobileCodeDialog.qml"),
                                    { number: page.profile.linkedMobilePending })
        dialog.accepted.connect(function() { page.link("verifyMobile", { code: dialog.code }, "mobile") })
        dialog.cancelRequested.connect(function() { page.link("linkMobile", { number: "" }, "mobile") })
    }

    Connections {
        target: backend
        onCoreEvent: if (name === "profileChanged") page.loadProfile()
    }

    // After the link dialog has closed.
    Timer {
        id: askCodeLater
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy) return
            stop()
            page.askCode()
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            MenuItem {
                //% "Remove ID from this device"
                text: qsTrId("threesea-me-delete-data")
                onClicked: window.pushWithPin(Qt.resolvedUrl("../DeleteDataDialog.qml"), {}, function(dialog) {
                    dialog.accepted.connect(function() {
                        var pin = dialog.pin
                        // Last chance to undo, like Jolla's own destructive actions.
                        //% "Removing ID and data"
                        Remorse.popupAction(page, qsTrId("threesea-la-removing-data"), function() {
                            backend.call("deleteAllData", { pin: pin }, function() {
                                backend.refreshContacts()
                                backend.refresh()
                            })
                        })
                    })
                })
            }
            MenuItem {
                //% "My QR code"
                text: qsTrId("threesea-he-my-qr-code")
                onClicked: pageStack.push(Qt.resolvedUrl("../MyQrCodePage.qml"))
            }
        }

        Component {
            id: profilePicturePicker
            ImagePickerPage {
                onSelectedContentPropertiesChanged: backend.setProfilePicture(selectedContentProperties.filePath,
                                                                              function(ok, result) {
                    if (ok) page.profile = result
                })
            }
        }

        Column {
            id: column
            width: parent.width

            // Like a contact's page (Jolla People's card); the picture opens it full screen, or
            // the picker while there is none.
            ProfileHeader {
                id: header
                pinnable: false
                pictureTappable: true
                onPictureClicked: pictureMenu.open(menuHost)
                showTitle: false
                source: page.profile.pictureVersion > 0
                        ? "image://threesea/contact/me?" + page.profile.pictureVersion : ""

                Column {
                    width: parent.width
                    // The nickname, edited in place next to the picture.
                    TextField {
                        id: nicknameField
                        width: parent.width
                        // The card's own margins already apply.
                        textLeftMargin: 0
                        textRightMargin: 0
                        //% "Nickname"
                        label: qsTrId("threesea-la-nickname")
                        //% "Shown to new contacts"
                        placeholderText: qsTrId("threesea-la-nickname-placeholder")
                        font.pixelSize: Theme.fontSizeLarge
                        Component.onCompleted: backend.call("profile", {}, function(ok, result) {
                            if (!ok) return
                            page.profile = result
                            nicknameField.text = result.nickname || ""
                        })
                        function save() {
                            backend.call("setNickname", { nickname: text }, function() { page.loadProfile() })
                            focus = false
                        }
                        onActiveFocusChanged: if (!activeFocus) save()
                        EnterKey.iconSource: "image://theme/icon-m-enter-close"
                        EnterKey.onClicked: save()
                    }
                    Label {
                        width: parent.width
                        text: page.profile.identity || backend.identity
                        font.pixelSize: Theme.fontSizeSmall
                        color: Theme.secondaryColor
                    }
                }
            }

            // The picture's menu opens right under the card.
            Item {
                id: menuHost
                width: parent.width
                height: pictureMenu.height
            }

            ContextMenu {
                id: pictureMenu
                MenuItem {
                    text: page.profile.pictureVersion > 0
                          //% "Change profile picture"
                          ? qsTrId("threesea-la-change-profile-picture")
                          //% "Set profile picture"
                          : qsTrId("threesea-la-set-profile-picture")
                    onClicked: pageStack.push(profilePicturePicker)
                }
                MenuItem {
                    visible: page.profile.pictureVersion > 0
                    //% "Remove profile picture"
                    text: qsTrId("threesea-me-remove-profile-picture")
                    onClicked: backend.call("removeProfilePicture", {}, function(ok, result) {
                        if (ok) page.profile = result
                    })
                }
            }

            Item { width: 1; height: Theme.paddingLarge }


            // Android's "ID Revocation": the last known state at once, then the server's.
            ValueButton {
                id: revocationButton
                property var status: ({})
                property bool working: false
                function refresh(cached) {
                    backend.call("revocationStatus", { cached: cached }, function(ok, result) {
                        if (ok) revocationButton.status = result
                    })
                }
                Component.onCompleted: { refresh(true); refresh(false) }
                //% "ID revocation"
                label: qsTrId("threesea-la-revocation")
                value: working
                       //% "Saving…"
                       ? qsTrId("threesea-la-saving")
                       : status.set
                         ? (status.setAt
                            //% "Password set %1"
                            ? qsTrId("threesea-la-revocation-set-at").arg(Format.formatDate(new Date(status.setAt), Formatter.DateMedium))
                            //% "Password set"
                            : qsTrId("threesea-la-revocation-set"))
                         //% "No password set"
                         : qsTrId("threesea-la-revocation-not-set")
                enabled: !working
                onClicked: window.pushWithPin(Qt.resolvedUrl("../RevocationPasswordDialog.qml"), {}, function(dialog) {
                    dialog.accepted.connect(function() {
                        revocationButton.working = true
                        backend.call("setRevocationPassword", { password: dialog.password, pin: dialog.pin },
                                     function(ok, result, kind, message) {
                            revocationButton.working = false
                            if (ok) {
                                revocationButton.status = result
                            } else {
                                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                            }
                        })
                    })
                })
            }

            // Identity links (Android's): only on the user's request (CLAUDE.md §7.4).
            ValueButton {
                //% "E-mail"
                label: qsTrId("threesea-la-link-email")
                value: page.linking === "email"
                       ? qsTrId("threesea-la-saving")
                       : !page.profile.linkedEmail
                         //% "Not linked"
                         ? qsTrId("threesea-la-not-linked")
                         : page.profile.linkedEmail
                description: page.profile.linkedEmailPending
                             //% "Waiting for you to confirm the link in the e-mail."
                             ? qsTrId("threesea-la-email-pending") : ""
                enabled: page.linking === ""
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("../LinkDialog.qml"),
                                                { kind: "email", current: page.profile.linkedEmail || "" })
                    dialog.accepted.connect(function() {
                        page.link("linkEmail", { email: dialog.value.trim() }, "email", function(result) {
                            // Threema answers "linked" at once, and sends nothing.
                            //% "This address is already linked to your ID, so no e-mail is needed."
                            if (!result.linkedEmailPending) Notices.show(qsTrId("threesea-la-email-already-linked"), Notice.Long, Notice.Center)
                        })
                    })
                    dialog.unlinkRequested.connect(function() { page.link("linkEmail", { email: "" }, "email") })
                }
            }
            ValueButton {
                //% "Phone number"
                label: qsTrId("threesea-la-link-phone")
                value: page.linking === "mobile"
                       ? qsTrId("threesea-la-saving")
                       : page.profile.linkedMobilePending
                         //% "+%1 (not confirmed)"
                         ? qsTrId("threesea-la-phone-pending").arg(page.profile.linkedMobilePending)
                         : page.profile.linkedMobile
                           ? "+" + String(page.profile.linkedMobile).replace(/^\+/, "")
                           : qsTrId("threesea-la-not-linked")
                enabled: page.linking === ""
                onClicked: {
                    if (page.profile.linkedMobilePending) {
                        page.askCode()
                        return
                    }
                    var dialog = pageStack.push(Qt.resolvedUrl("../LinkDialog.qml"), {
                        kind: "mobile",
                        current: page.profile.linkedMobile ? "+" + String(page.profile.linkedMobile).replace(/^\+/, "") : ""
                    })
                    dialog.accepted.connect(function() {
                        // What the dialog showed, exactly.
                        page.link("linkMobile", { number: "+" + dialog.normalized }, "mobile", function(result) {
                            if (result.linkedMobilePending) askCodeLater.start()
                            // Threema answers "linked" at once, and sends nothing.
                            //% "This number is already linked to your ID, so no SMS is needed."
                            else Notices.show(qsTrId("threesea-la-phone-already-linked"), Notice.Long, Notice.Center)
                        })
                    })
                    dialog.unlinkRequested.connect(function() { page.link("linkMobile", { number: "" }, "mobile") })
                }
            }

            // Like Android: everyone, nobody, or an allow list; "Selected contacts" continues to
            // the contact picker (also to change them), which saves both.
            ChoiceBox {
                id: shareButton
                readonly property var shares: ["everyone", "nobody", "selected"]
                readonly property var recipients: page.profile.pictureRecipients || []

                function apply(share, recipients) {
                    backend.call("setProfilePictureShare", { share: share, recipients: recipients },
                                 function(ok, result) { if (ok) page.profile = result })
                }

                options: [
                    //% "Everyone I write to"
                    qsTrId("threesea-me-share-everyone"),
                    //% "Nobody"
                    qsTrId("threesea-me-share-nobody"),
                    //% "Selected contacts"
                    qsTrId("threesea-me-share-selected")
                ]
                //% "Share profile picture with"
                label: qsTrId("threesea-la-picture-share")
                choice: Math.max(0, shares.indexOf(page.profile.pictureShare))
                valueText: choice === 2
                           //% "%n selected contact(s)"
                           ? qsTrId("threesea-la-share-selected-count", recipients.length) : ""
                onChosen: {
                    if (index !== 2) {
                        apply(shares[index])
                        return
                    }
                    var picker = pageStack.push(Qt.resolvedUrl("../MemberPickerDialog.qml"), {
                        //% "Save"
                        acceptText: qsTrId("threesea-he-save"),
                        title: qsTrId("threesea-la-picture-share"),
                        selected: recipients.slice(),
                        // It never gets profile pictures.
                        excluded: ["ECHOECHO"]
                    })
                    picker.accepted.connect(function() { apply("selected", picker.selected) })
                }
            }

            SectionHeader {
                text: qsTrId("threesea-se-public-key")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                text: Verification.formatKey(page.profile.publicKey)
                font.family: "monospace"
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.highlightColor
            }
        }

        VerticalScrollDecorator { }
    }
}
