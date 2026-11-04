// SPDX-License-Identifier: AGPL-3.0-only
.pragma library

// User-facing text for an error kind returned by the core (see core/src/app.rs ApiError).
function text(kind, message) {
    switch (kind) {
    case "network":
        //% "Could not reach the Threema servers. Check your internet connection and try again."
        return qsTrId("threesea-la-error-network")
    case "licenseFormat":
        //% "A license key looks like ABCDE-12345."
        return qsTrId("threesea-la-error-license-format")
    case "backupDecrypt":
        //% "Wrong password, or the ID backup is damaged."
        return qsTrId("threesea-la-error-backup-decrypt")
    case "backupVersion":
        //% "This backup's format is not supported."
        return qsTrId("threesea-la-error-backup-version")
    case "backupInvalid":
        //% "The file could not be read as a Threema backup."
        return qsTrId("threesea-la-error-backup-invalid")
    case "backupIdentity":
        //% "This backup belongs to a different Threema ID or key."
        return qsTrId("threesea-la-error-backup-identity")
    case "passwordTooShort":
        //% "The password needs at least 8 characters."
        return qsTrId("threesea-la-error-password-short")
    case "passwordTooLong":
        //% "The password is too long."
        return qsTrId("threesea-la-error-password-long")
    case "safeNotFound":
        //% "No Threema Safe backup found for this Threema ID and password."
        return qsTrId("threesea-la-error-safe-not-found")
    case "safeTooLarge":
        //% "The backup is larger than the Threema Safe server allows."
        return qsTrId("threesea-la-error-safe-too-large")
    case "identityFormat":
        //% "A Threema ID has 8 characters, letters and digits."
        return qsTrId("threesea-la-error-identity-format")
    case "contactInvalid":
        //% "This Threema ID does not exist or was revoked."
        return qsTrId("threesea-la-error-contact-invalid")
    case "contactIsSelf":
        //% "This is your own Threema ID."
        return qsTrId("threesea-la-error-contact-self")
    case "noLicense":
        //% "Enter your license key first."
        return qsTrId("threesea-la-error-no-license")
    case "contactBlocked":
        //% "You have blocked this contact."
        return qsTrId("threesea-la-error-contact-blocked")
    case "fileNotDownloaded":
        //% "Download the file first."
        return qsTrId("threesea-la-error-file-not-downloaded")
    case "notForwardable":
        //% "This message cannot be forwarded."
        return qsTrId("threesea-la-error-not-forwardable")
    case "qrInvalid":
        //% "This is not a Threema contact QR code."
        return qsTrId("threesea-la-error-qr-invalid")
    case "qrMismatch":
        //% "The code does not match this contact's key. Ask them to show their code again."
        return qsTrId("threesea-la-error-qr-mismatch")
    case "qrExpired":
        //% "This code has expired. Scan it directly from the other person's app."
        return qsTrId("threesea-la-error-qr-expired")
    case "groupLeft":
        //% "You are no longer in this group."
        return qsTrId("threesea-la-error-group-left")
    case "notGroupCreator":
        //% "Only the group's creator can do this."
        return qsTrId("threesea-la-error-not-group-creator")
    case "fileTooLarge":
        //% "Files can be at most 100 MB."
        return qsTrId("threesea-la-error-file-too-large")
    case "linkUnknownCode":
        //% "This is not a code of Threema for Desktop. Scan the code it shows when you open it."
        return qsTrId("threesea-la-error-link-unknown-code")
    case "linkWebCode":
        //% "This code is for Threema Web. Scan the code of Threema for Desktop instead."
        return qsTrId("threesea-la-error-link-web-code")
    case "webCodeInvalid":
        //% "This is not a Threema Web code. Open web.threema.ch and scan the code it shows."
        return qsTrId("threesea-la-error-web-code")
    case "webVersion":
        //% "This version of Threema Web can't be used. Reload the page in the browser."
        return qsTrId("threesea-la-error-web-version")
    case "webSessionNotFound":
        //% "This Threema Web session no longer exists."
        return qsTrId("threesea-la-error-web-session")
    case "linkVersion":
        //% "This version of Threema for Desktop can't be linked. Please update both apps."
        return qsTrId("threesea-la-error-link-version")
    case "linkFailed":
        //% "Linking didn't work. Check the connection on both devices and try again."
        return qsTrId("threesea-la-error-link-failed")
    case "linkRemoved":
        //% "Linking didn't finish, so the devices in the device group were removed again. The computer had already received your ID. Close Threema for Desktop, open it again and scan its new code."
        return qsTrId("threesea-la-error-link-removed")
    case "linkUncertain":
        //% "Linking didn't finish, but the computer received your ID and may be linked anyway. Linked devices shows what is linked once you are online, and you can remove it there."
        return qsTrId("threesea-la-error-link-uncertain")
    case "groupCallFull":
        //% "This group call is full."
        return qsTrId("threesea-la-error-group-call-full")
    case "groupCallNotRunning":
        //% "This group call has ended."
        return qsTrId("threesea-la-error-group-call-not-running")
    case "groupCallVersion":
        //% "This group call needs a different version of the app."
        return qsTrId("threesea-la-error-group-call-version")
    case "groupCallConnection":
        //% "The connection to the group call failed."
        return qsTrId("threesea-la-error-group-call-connection")
    case "notGif":
        //% "This picture is not a GIF."
        return qsTrId("threesea-la-error-not-gif")
    case "notSupportedByContact":
        //% "This contact's app doesn't support this."
        return qsTrId("threesea-la-error-not-supported-by-contact")
    case "messageTooOld":
        //% "Messages can only be changed for 6 hours."
        return qsTrId("threesea-la-error-message-too-old")
    case "reactionNotSupported":
        //% "This contact's app only understands thumbs up and thumbs down."
        return qsTrId("threesea-la-error-reaction-not-supported")
    case "phoneInvalid":
        //% "Enter the number with its country code, e.g. +41 79 123 45 67."
        return qsTrId("threesea-la-error-phone-invalid")
    case "emailInvalid":
        //% "This is not an e-mail address."
        return qsTrId("threesea-la-error-email-invalid")
    case "noVerification":
        //% "No phone number is waiting for a code. Link it again."
        return qsTrId("threesea-la-error-no-verification")
    case "wrongPin":
        //% "Wrong PIN"
        return qsTrId("threesea-la-wrong-pin")
    case "pinLockout":
        return qsTrId("threesea-la-pin-paused", parseInt(message, 10))
    case "pinTooShort":
        //% "The PIN needs at least 4 digits."
        return qsTrId("threesea-la-error-pin-short")
    case "locked":
        //% "Enter your PIN first."
        return qsTrId("threesea-la-error-locked")
    case "lockNeeded":
        return qsTrId("threesea-la-private-needs-lock")
    case "privateChatsExist":
        //% "Remove all chats from the private chats first."
        return qsTrId("threesea-la-error-private-chats-exist")
    case "server":
        // Message from the Threema server, possibly already localized.
        return message
    default:
        //% "Something went wrong: %1"
        return qsTrId("threesea-la-error-internal").arg(message)
    }
}
