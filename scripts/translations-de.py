#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# German texts for translations/harbour-threesea-de.ts. Fills in what this table knows, keeps
# everything else as it is, so lupdate's new entries can be added here. Run after
# scripts/update-translations.sh, as the last step (it writes the file in the repository's layout).
# Style: Sailfish OS German ("du", ß), Threema's terms (Threema-ID, Lesebestätigungen, Umfrage,
# Weiterleiten, deblockieren, verknüpfte Geräte, Verteilerliste, Threema für Desktop).
#
# Each entry holds the English it was translated from, then the German. When the English in the
# .ts no longer matches, the text stays unfinished and is listed: a reworded text never keeps a
# translation of what it used to say. Translate it again and put the new English beside it.
import sys
import xml.etree.ElementTree as ET

DE = {
    "threesea-la-error-network":
        ("Could not reach the Threema servers. Check your internet connection and try again.",
         "Die Threema-Server sind nicht erreichbar. Prüfe deine Internetverbindung und versuche es erneut."),
    "threesea-la-error-license-format":
        ("A license key looks like ABCDE-12345.",
         "Ein Lizenzschlüssel sieht so aus: ABCDE-12345."),
    "threesea-la-error-backup-decrypt":
        ("Wrong password, or the backup is damaged.",
         "Falsches Passwort, oder das Backup ist beschädigt."),
    "threesea-la-error-password-short":
        ("The password needs at least 8 characters.",
         "Das Passwort braucht mindestens 8 Zeichen."),
    "threesea-la-error-password-long":
        ("The password is too long.",
         "Das Passwort ist zu lang."),
    "threesea-la-error-safe-not-found":
        ("No Threema Safe backup found for this Threema ID and password.",
         "Für diese Threema-ID und dieses Passwort gibt es kein Threema-Safe-Backup."),
    "threesea-la-error-safe-too-large":
        ("The backup is larger than the Threema Safe server allows.",
         "Das Backup ist größer, als der Threema-Safe-Server erlaubt."),
    "threesea-la-error-identity-format":
        ("A Threema ID has 8 characters, letters and digits.",
         "Eine Threema-ID hat 8 Zeichen, Buchstaben und Ziffern."),
    "threesea-la-error-contact-invalid":
        ("This Threema ID does not exist or was revoked.",
         "Diese Threema-ID gibt es nicht, oder sie wurde widerrufen."),
    "threesea-la-error-contact-self":
        ("This is your own Threema ID.",
         "Das ist deine eigene Threema-ID."),
    "threesea-la-error-no-license":
        ("Enter your license key first.",
         "Gib zuerst deinen Lizenzschlüssel ein."),
    "threesea-la-verification-full":
        ("Fully verified",
         "Vollständig überprüft"),
    "threesea-la-verification-none":
        ("Not verified",
         "Nicht überprüft"),
    "threesea-la-cover-no-new":
        ("No new messages",
         "Keine neuen Nachrichten"),
    "threesea-he-about":
        ("About",
         "Über"),
    "threesea-la-version":
        ("Version",
         "Version"),
    "threesea-se-license":
        ("License",
         "Lizenz"),
    "threesea-la-license-text":
        ("An unofficial Threema client for Sailfish OS, licensed under the GNU AGPL v3. Uses libthreema by Threema AG. Requires a Threema Libre license from the Threema Shop. Not affiliated with or supported by Threema AG.",
         "Ein inoffizieller Threema-Client für Sailfish OS unter der GNU AGPL v3. Verwendet libthreema von der Threema AG. Benötigt eine Threema-Libre-Lizenz aus dem Threema Shop. Nicht mit der Threema AG verbunden und nicht von ihr unterstützt."),
    "threesea-he-add":
        ("Add",
         "Hinzufügen"),
    "threesea-la-add-contact-intro":
        ("Enter the Threema ID of the person you want to add. Meet in person and scan each other's QR code later to fully verify the contact.",
         "Gib die Threema-ID der Person ein, die du hinzufügen möchtest. Trefft euch später und scannt gegenseitig eure QR-Codes, um den Kontakt vollständig zu überprüfen."),
    "threesea-la-threema-id":
        ("Threema ID",
         "Threema-ID"),
    "threesea-me-delete":
        ("Delete",
         "Löschen"),
    "threesea-la-deleting-contact":
        ("Deleting contact and chat",
         "Kontakt und Chat werden gelöscht"),
    "threesea-me-edit":
        ("Edit",
         "Bearbeiten"),
    "threesea-la-nickname":
        ("Nickname",
         "Nickname"),
    "threesea-la-type":
        ("Type",
         "Typ"),
    "threesea-la-threema-work":
        ("Threema Work",
         "Threema Work"),
    "threesea-la-status":
        ("Status",
         "Status"),
    "threesea-la-revoked":
        ("Revoked",
         "Widerrufen"),
    "threesea-la-inactive":
        ("Inactive",
         "Inaktiv"),
    "threesea-se-public-key":
        ("Public key",
         "Öffentlicher Schlüssel"),
    "threesea-me-add-contact":
        ("Add contact",
         "Kontakt hinzufügen"),
    "threesea-he-contacts":
        ("Contacts",
         "Kontakte"),
    "threesea-la-no-contacts":
        ("No contacts",
         "Keine Kontakte"),
    "threesea-la-no-contacts-hint":
        ("Pull down to add a contact by Threema ID",
         "Ziehe nach unten, um einen Kontakt per Threema-ID hinzuzufügen"),
    "threesea-me-settings":
        ("Settings",
         "Einstellungen"),
    "threesea-he-chats":
        ("Chats",
         "Chats"),
    "threesea-la-no-chats":
        ("No chats yet",
         "Noch keine Chats"),
    "threesea-la-no-chats-hint":
        ("Swipe left to choose a contact",
         "Wische nach links, um einen Kontakt zu wählen"),
    "threesea-he-remove":
        ("Remove",
         "Entfernen"),
    "threesea-he-save":
        ("Save",
         "Speichern"),
    "threesea-la-first-name":
        ("First name",
         "Vorname"),
    "threesea-la-last-name":
        ("Last name",
         "Nachname"),
    "threesea-he-export-backup":
        ("Export ID backup",
         "ID exportieren"),
    "threesea-la-export-intro":
        ("An ID backup lets you restore your Threema ID on another device. Keep it and its password safe: anyone with both can use your ID.",
         "Mit einem ID-Backup kannst du deine Threema-ID auf einem anderen Gerät wiederherstellen. Bewahre es und sein Passwort sicher auf: Wer beides hat, kann deine ID verwenden."),
    "threesea-la-new-backup-password":
        ("Password (at least 8 characters)",
         "Passwort (mindestens 8 Zeichen)"),
    "threesea-la-repeat-password":
        ("Repeat password",
         "Passwort wiederholen"),
    "threesea-bt-create-backup":
        ("Create backup",
         "Backup erstellen"),
    "threesea-bt-copy":
        ("Copy to clipboard",
         "In die Zwischenablage kopieren"),
    "threesea-he-identity-setup":
        ("Your Threema ID",
         "Deine Threema-ID"),
    "threesea-bt-create-identity":
        ("Create new ID",
         "Neue ID erstellen"),
    "threesea-bt-restore-safe":
        ("Restore from Threema Safe",
         "Aus Threema Safe wiederherstellen"),
    "threesea-bt-restore-backup":
        ("Restore ID backup",
         "ID-Backup wiederherstellen"),
    "threesea-he-welcome":
        ("Welcome to Threesea",
         "Willkommen bei Threesea"),
    "threesea-la-license-intro":
        ("Threesea is an unofficial Threema client for Sailfish OS. It needs a Threema Libre license key, which you can buy in the Threema Shop (shop.threema.ch).",
         "Threesea ist ein inoffizieller Threema-Client für Sailfish OS. Er braucht einen Threema-Libre-Lizenzschlüssel, den du im Threema Shop (shop.threema.ch) kaufen kannst."),
    "threesea-la-license-key":
        ("License key",
         "Lizenzschlüssel"),
    "threesea-bt-check-license":
        ("Check license",
         "Lizenz prüfen"),
    "threesea-he-restore-backup":
        ("Restore ID backup",
         "ID-Backup wiederherstellen"),
    "threesea-la-id-backup":
        ("ID backup",
         "ID-Backup"),
    "threesea-la-backup-password":
        ("Backup password",
         "Backup-Passwort"),
    "threesea-bt-restore":
        ("Restore",
         "Wiederherstellen"),
    "threesea-me-safe-change-password":
        ("Change password",
         "Passwort ändern"),
    "threesea-me-safe-backup-now":
        ("Back up now",
         "Jetzt sichern"),
    "threesea-la-threema-safe":
        ("Threema Safe",
         "Threema Safe"),
    "threesea-se-status":
        ("Status",
         "Status"),
    "threesea-la-safe-last-backup":
        ("Last backup",
         "Letztes Backup"),
    "threesea-la-safe-not-yet":
        ("Not yet",
         "Noch nicht"),
    "threesea-la-safe-size":
        ("Size",
         "Größe"),
    "threesea-la-safe-server":
        ("Server",
         "Server"),
    "threesea-la-safe-retention":
        ("Kept on the server",
         "Auf dem Server aufbewahrt"),
    "threesea-la-days":
        ("%1 days",
         "%1 Tage"),
    "threesea-he-enable":
        ("Enable",
         "Einschalten"),
    "threesea-la-password":
        ("Password",
         "Passwort"),
    "threesea-la-safe-password-weak":
        ("This password is easy to guess. Choose a stronger one, or accept to use it anyway.",
         "Dieses Passwort ist leicht zu erraten. Wähle ein stärkeres oder bestätige, um es trotzdem zu verwenden."),
    "threesea-he-restore-safe":
        ("Restore from Threema Safe",
         "Aus Threema Safe wiederherstellen"),
    "threesea-la-safe-password":
        ("Threema Safe password",
         "Threema-Safe-Passwort"),
    "threesea-la-safe-deriving":
        ("Unlocking the backup takes a moment.",
         "Das Entsperren des Backups dauert einen Moment."),
    "threesea-me-about":
        ("About",
         "Über"),
    "threesea-he-settings":
        ("Settings",
         "Einstellungen"),
    "threesea-se-profile":
        ("My profile",
         "Mein Profil"),
    "threesea-se-privacy":
        ("Privacy",
         "Privatsphäre"),
    "threesea-la-send-read-receipts":
        ("Send read receipts",
         "Lesebestätigungen senden"),
    "threesea-la-send-read-receipts-description":
        ("Let people know when you have read their messages.",
         "Andere sehen, wenn du ihre Nachrichten gelesen hast."),
    "threesea-la-send-typing":
        ("Send typing indicator",
         "Melden, wenn ich tippe"),
    "threesea-la-send-typing-description":
        ("Let people see when you are writing to them.",
         "Andere sehen, wenn du ihnen gerade schreibst."),
    "threesea-la-block-unknown":
        ("Block unknown senders",
         "Unbekannte blockieren"),
    "threesea-la-block-unknown-description":
        ("Ignore messages from people who are not in your contacts.",
         "Nachrichten von Personen ignorieren, die nicht in deinen Kontakten sind."),
    "threesea-se-backup":
        ("Backup",
         "Backup"),
    "threesea-la-on":
        ("On",
         "Ein"),
    "threesea-la-off":
        ("Off",
         "Aus"),
    "threesea-la-export-backup":
        ("Export ID backup",
         "ID exportieren"),
    "threesea-la-startup-failed":
        ("Threesea could not start",
         "Threesea konnte nicht starten"),
    "threesea-la-location":
        ("Location",
         "Standort"),
    "threesea-me-copy":
        ("Copy",
         "Kopieren"),
    "threesea-la-cover-unread-one":
        ("unread message",
         "ungelesene Nachricht"),
    "threesea-la-cover-unread-many":
        ("unread messages",
         "ungelesene Nachrichten"),
    "threesea-la-typing":
        ("typing…",
         "schreibt gerade …"),
    "threesea-la-offline-queue":
        ("Offline – messages will be sent later",
         "Offline – Nachrichten werden später gesendet"),
    "threesea-la-connecting":
        ("Connecting…",
         "Wird verbunden …"),
    "threesea-la-offline":
        ("Offline",
         "Offline"),
    "threesea-me-delete-chat":
        ("Delete chat",
         "Chat löschen"),
    "threesea-la-deleting-chat":
        ("Deleting chat",
         "Chat wird gelöscht"),
    "threesea-la-keep-running":
        ("Keep running in the background",
         "Im Hintergrund weiterlaufen"),
    "threesea-la-keep-running-description":
        ("Threesea stays connected after you close it, so new messages arrive right away. After you turn on the phone, it connects once you open it. Uses more battery.",
         "Threesea bleibt verbunden, wenn du es schließt, damit neue Nachrichten sofort ankommen. Nach dem Einschalten des Telefons verbindet es sich, sobald du es öffnest. Braucht mehr Akku."),
    "threesea-la-status-fs-established":
        ("Perfect Forward Secrecy is active for this chat.",
         "Perfect Forward Secrecy ist für diesen Chat aktiv."),
    "threesea-la-today":
        ("Today",
         "Heute"),
    "threesea-la-yesterday":
        ("Yesterday",
         "Gestern"),
    "threesea-la-type-message":
        ("Type message",
         "Nachricht schreiben"),
    "threesea-la-error-reaction-not-supported":
        ("This contact's app only understands thumbs up and thumbs down.",
         "Die App dieses Kontakts versteht nur Daumen hoch und Daumen runter."),
    "threesea-la-error-not-supported-by-contact":
        ("The app on the other side doesn't support this.",
         "Die App auf der anderen Seite unterstützt das nicht."),
    "threesea-la-error-message-too-old":
        ("Messages can be edited or deleted for everyone for 6 hours after sending.",
         "Nachrichten können bis 6 Stunden nach dem Senden bearbeitet oder für alle gelöscht werden."),
    "threesea-la-message-deleted":
        ("Message deleted",
         "Nachricht gelöscht"),
    "threesea-la-edited":
        ("Edited",
         "Bearbeitet"),
    "threesea-la-deleting-everyone":
        ("Deleting for everyone",
         "Wird für alle gelöscht"),
    "threesea-la-deleting-message":
        ("Deleting",
         "Wird gelöscht"),
    "threesea-la-editing-message":
        ("Editing message",
         "Nachricht bearbeiten"),
    "threesea-la-file":
        ("File",
         "Datei"),
    "threesea-la-photo":
        ("Photo",
         "Foto"),
    "threesea-la-error-file-too-large":
        ("Files can be at most 100 MB.",
         "Dateien dürfen höchstens 100 MB groß sein."),
    "threesea-la-add-caption":
        ("Add a caption",
         "Beschriftung hinzufügen"),
    "threesea-me-send-photo":
        ("Send photo",
         "Foto senden"),
    "threesea-me-send-file":
        ("Send file",
         "Datei senden"),
    "threesea-me-save-downloads":
        ("Save to Downloads",
         "In Downloads speichern"),
    "threesea-la-saved-downloads":
        ("Saved to Downloads",
         "In Downloads gespeichert"),
    "threesea-la-voice-message":
        ("Voice message",
         "Sprachnachricht"),
    "threesea-la-recording-failed":
        ("Recording failed",
         "Aufnahme fehlgeschlagen"),
    "threesea-la-recording":
        ("Recording %1",
         "Aufnahme %1"),
    "threesea-la-cannot-play":
        ("This voice message could not be played",
         "Diese Sprachnachricht konnte nicht abgespielt werden"),
    "threesea-la-error-group-left":
        ("You are no longer in this group.",
         "Du bist nicht mehr in dieser Gruppe."),
    "threesea-la-error-not-group-creator":
        ("Only the group's creator can do this.",
         "Das kann nur die Person, die die Gruppe erstellt hat."),
    "threesea-la-status-group-created":
        ("You created this group.",
         "Du hast diese Gruppe erstellt."),
    "threesea-la-status-group-joined":
        ("You were added to this group.",
         "Du wurdest zu dieser Gruppe hinzugefügt."),
    "threesea-la-status-group-removed":
        ("You were removed from this group.",
         "Du wurdest aus dieser Gruppe entfernt."),
    "threesea-la-status-group-disbanded":
        ("This group was dissolved.",
         "Diese Gruppe wurde aufgelöst."),
    "threesea-la-status-group-left":
        ("You left this group.",
         "Du hast diese Gruppe verlassen."),
    "threesea-la-status-group-renamed":
        ("Group renamed to %1.",
         "Gruppe umbenannt in %1."),
    "threesea-la-status-group-member-added":
        ("%1 was added.",
         "%1 wurde hinzugefügt."),
    "threesea-la-status-group-member-removed":
        ("%1 was removed.",
         "%1 wurde entfernt."),
    "threesea-la-status-group-member-left":
        ("%1 left the group.",
         "%1 hat die Gruppe verlassen."),
    "threesea-la-group-left-input":
        ("You are no longer in this group",
         "Du bist nicht mehr in dieser Gruppe"),
    "threesea-me-new-group":
        ("New group",
         "Neue Gruppe"),
    "threesea-he-rename-group":
        ("Rename group",
         "Gruppe umbenennen"),
    "threesea-la-group-name":
        ("Group name",
         "Gruppenname"),
    "threesea-me-delete-group":
        ("Delete group",
         "Gruppe löschen"),
    "threesea-la-deleting-group":
        ("Deleting group",
         "Gruppe wird gelöscht"),
    "threesea-me-dissolve-group":
        ("Dissolve group",
         "Gruppe auflösen"),
    "threesea-me-leave-group":
        ("Leave group",
         "Gruppe verlassen"),
    "threesea-la-dissolving-group":
        ("Dissolving group",
         "Gruppe wird aufgelöst"),
    "threesea-la-leaving-group":
        ("Leaving group",
         "Gruppe wird verlassen"),
    "threesea-me-rename":
        ("Rename",
         "Umbenennen"),
    "threesea-he-members":
        ("Members",
         "Mitglieder"),
    "threesea-he-group":
        ("Group",
         "Gruppe"),
    "threesea-la-member-count":
        ("%n member(s)",
         ("%n Mitglied",
          "%n Mitglieder")),
    "threesea-la-you-creator":
        ("You (creator)",
         "Du (Ersteller)"),
    "threesea-la-you":
        ("You",
         "Du"),
    "threesea-la-member-creator":
        ("%1 (creator)",
         "%1 (Ersteller)"),
    "threesea-he-create":
        ("Create",
         "Erstellen"),
    "threesea-he-new-group":
        ("New group",
         "Neue Gruppe"),
    "threesea-me-remove-picture":
        ("Remove picture",
         "Bild entfernen"),
    "threesea-me-set-picture":
        ("Set picture",
         "Bild festlegen"),
    "threesea-me-add-members":
        ("Add members",
         "Mitglieder hinzufügen"),
    "threesea-he-add-members":
        ("Add members",
         "Mitglieder hinzufügen"),
    "threesea-la-removing-member":
        ("Removing %1",
         "%1 wird entfernt"),
    "threesea-la-poll-preview":
        ("Poll: %1",
         "Umfrage: %1"),
    "threesea-me-close-poll":
        ("Close poll",
         "Umfrage abschließen"),
    "threesea-la-poll-closed":
        ("Closed",
         "Abgeschlossen"),
    "threesea-la-poll-multiple":
        ("Multiple choice",
         "Mehrfachauswahl"),
    "threesea-la-poll-single":
        ("Single choice",
         "Einfachauswahl"),
    "threesea-la-poll-votes":
        ("%n vote(s)",
         ("%n Stimme",
          "%n Stimmen")),
    "threesea-me-create-poll":
        ("Create poll",
         "Umfrage erstellen"),
    "threesea-he-send":
        ("Send",
         "Senden"),
    "threesea-he-new-poll":
        ("New poll",
         "Neue Umfrage"),
    "threesea-la-poll-question":
        ("Question",
         "Frage"),
    "threesea-he-poll-choices":
        ("Choices",
         "Antworten"),
    "threesea-la-poll-choice":
        ("Choice %1",
         "Antwort %1"),
    "threesea-la-poll-announce":
        ("Show votes before closing",
         "Stimmen vor dem Abschließen zeigen"),
    "threesea-la-poll-announce-description":
        ("Otherwise only you see the votes until you close the poll.",
         "Sonst siehst nur du die Stimmen, bis du die Umfrage abschließt."),
    "threesea-la-poll-you-voted":
        ("You voted",
         "Du hast abgestimmt"),
    "threesea-la-poll-tap-to-vote":
        ("Tap to vote",
         "Tippe zum Abstimmen"),
    "threesea-la-closing-poll":
        ("Closing poll",
         "Umfrage wird abgeschlossen"),
    "threesea-he-poll-not-voted":
        ("Not voted yet",
         "Noch nicht abgestimmt"),
    "threesea-la-poll-only-you":
        ("Only you see the votes until you close the poll.",
         "Nur du siehst die Stimmen, bis du die Umfrage abschließt."),
    "threesea-me-clear-choices":
        ("Clear choices",
         "Auswahl löschen"),
    "threesea-he-vote":
        ("Vote",
         "Abstimmen"),
    "threesea-me-reply":
        ("Reply",
         "Antworten"),
    "threesea-me-mark-read":
        ("Mark as read",
         "Als gelesen markieren"),
    "threesea-la-new-message":
        ("New message",
         "Neue Nachricht"),
    "threesea-se-notifications":
        ("Notifications",
         "Benachrichtigungen"),
    "threesea-la-notifications":
        ("Notify about new messages",
         "Über neue Nachrichten benachrichtigen"),
    "threesea-la-notification-preview":
        ("Show message content",
         "Nachrichteninhalt anzeigen"),
    "threesea-me-quote":
        ("Quote",
         "Zitieren"),
    "threesea-la-quote-missing":
        ("Message not available",
         "Nachricht nicht verfügbar"),
    "threesea-la-quoting":
        ("Quoting %1",
         "Zitat von %1"),
    "threesea-he-share":
        ("Share",
         "Teilen"),
    "threesea-la-share-files":
        ("%n file(s)",
         ("%n Datei",
          "%n Dateien")),
    "threesea-la-error-qr-invalid":
        ("This is not a Threema contact QR code.",
         "Das ist kein Threema-Kontakt-QR-Code."),
    "threesea-la-error-qr-mismatch":
        ("This code has the right Threema ID but a different key from the one known for it, so it may be forged. It was not accepted. Compare the codes with this person directly.",
         "Dieser Code hat die richtige Threema-ID, aber einen anderen Schlüssel als den bekannten, er ist also vielleicht gefälscht. Er wurde nicht angenommen. Vergleicht eure Codes direkt miteinander."),
    "threesea-me-verify-qr":
        ("Verify by QR code",
         "Per QR-Code überprüfen"),
    "threesea-he-scan-qr":
        ("Scan QR code",
         "QR-Code scannen"),
    "threesea-he-my-qr-code":
        ("My QR code",
         "Mein QR-Code"),
    "threesea-la-my-qr-code-description":
        ("When someone scans this code with Threema, you are verified for them in person (three dots).",
         "Wenn jemand diesen Code mit Threema scannt, bist du für diese Person persönlich überprüft (drei Punkte)."),
    "threesea-la-qr-other-contact":
        ("This is someone else's code.",
         "Das ist der Code von jemand anderem."),
    "threesea-la-qr-verified":
        ("%1 is verified",
         "%1 ist überprüft"),
    "threesea-me-scan-from-picture":
        ("Scan from picture",
         "Aus Bild scannen"),
    "threesea-la-no-camera":
        ("No camera available. Pull down to scan a code from a picture.",
         "Keine Kamera verfügbar. Ziehe nach unten, um einen Code aus einem Bild zu scannen."),
    "threesea-la-scan-hint":
        ("Point the camera at the Threema QR code of the person next to you.",
         "Richte die Kamera auf den Threema-QR-Code der Person neben dir."),
    "threesea-la-qr-not-found":
        ("No Threema QR code found in this picture.",
         "In diesem Bild ist kein Threema-QR-Code."),
    "threesea-la-address-book-unavailable":
        ("The address book is not available on this device.",
         "Das Adressbuch ist auf diesem Gerät nicht verfügbar."),
    "threesea-la-address-book-added":
        ("%n new contact(s) from your address book",
         ("%n neuer Kontakt aus deinem Adressbuch",
          "%n neue Kontakte aus deinem Adressbuch")),
    "threesea-la-address-book-sync":
        ("Find contacts from the address book",
         "Kontakte aus dem Adressbuch finden"),
    "threesea-la-address-book-sync-description":
        ("Daily, only hashed e-mail addresses and phone numbers are sent to Threema to find who uses it. Found people become contacts with their address book names.",
         "Einmal täglich werden nur gehashte E-Mail-Adressen und Telefonnummern an Threema gesendet, um herauszufinden, wer es verwendet. Gefundene Personen werden mit ihren Namen aus dem Adressbuch zu Kontakten."),
    "threesea-la-mention-all":
        ("All",
         "Alle"),
    "threesea-la-mention-me":
        ("Me",
         "Ich"),
    "threesea-la-chat-notifications":
        ("Notifications",
         "Benachrichtigungen"),
    "threesea-la-notifications-until":
        ("Until %1",
         "Bis %1"),
    "threesea-me-notifications-on":
        ("On",
         "Ein"),
    "threesea-me-notifications-1-hour":
        ("Off for 1 hour",
         "1 Stunde aus"),
    "threesea-me-notifications-8-hours":
        ("Off for 8 hours",
         "8 Stunden aus"),
    "threesea-me-notifications-1-day":
        ("Off for 1 day",
         "1 Tag aus"),
    "threesea-me-notifications-1-week":
        ("Off for 1 week",
         "1 Woche aus"),
    "threesea-me-notifications-off":
        ("Off",
         "Aus"),
    "threesea-me-notifications-mentioned":
        ("Only when I am mentioned",
         "Nur, wenn ich erwähnt werde"),
    "threesea-la-error-file-not-downloaded":
        ("Download the file first.",
         "Lade die Datei zuerst herunter."),
    "threesea-la-error-not-forwardable":
        ("This message cannot be forwarded.",
         "Diese Nachricht kann nicht weitergeleitet werden."),
    "threesea-me-forward":
        ("Forward",
         "Weiterleiten"),
    "threesea-he-forward":
        ("Forward",
         "Weiterleiten"),
    "threesea-la-error-contact-blocked":
        ("You have blocked this contact.",
         "Du hast diesen Kontakt blockiert."),
    "threesea-me-block-id":
        ("Block a Threema ID",
         "Threema-ID blockieren"),
    "threesea-he-block":
        ("Block",
         "Blockieren"),
    "threesea-la-block-intro":
        ("Messages from this Threema ID will be discarded, and you cannot write to it.",
         "Nachrichten von dieser Threema-ID werden verworfen, und du kannst ihr nicht schreiben."),
    "threesea-he-blocked-contacts":
        ("Blocked contacts",
         "Blockierte Kontakte"),
    "threesea-me-unblock":
        ("Unblock",
         "Deblockieren"),
    "threesea-la-no-blocked":
        ("No blocked contacts",
         "Keine blockierten Kontakte"),
    "threesea-la-blocked-input":
        ("You have blocked this contact",
         "Du hast diesen Kontakt blockiert"),
    "threesea-me-block":
        ("Block",
         "Blockieren"),
    "threesea-la-blocked":
        ("Blocked",
         "Blockiert"),
    "threesea-me-send-location":
        ("Send location",
         "Standort senden"),
    "threesea-la-location-unavailable":
        ("Positioning is not available. Turn on location in Settings → Location.",
         "Ortung ist nicht verfügbar. Schalte sie in Einstellungen → Standort ein."),
    "threesea-la-location-finding":
        ("Finding your location…",
         "Dein Standort wird ermittelt …"),
    "threesea-la-location-name":
        ("Name of the place (optional)",
         "Name des Orts (optional)"),
    "threesea-me-search":
        ("Search",
         "Suchen"),
    "threesea-he-search":
        ("Search",
         "Suchen"),
    "threesea-la-search-messages":
        ("Search messages",
         "Nachrichten durchsuchen"),
    "threesea-la-search-you-to":
        ("You → %1",
         "Du → %1"),
    "threesea-la-search-nothing":
        ("No messages found",
         "Keine Nachrichten gefunden"),
    "threesea-se-pinned":
        ("Pinned",
         "Angeheftet"),
    "threesea-me-unpin":
        ("Unpin",
         "Nicht mehr anheften"),
    "threesea-me-pin":
        ("Pin to top",
         "Oben anheften"),
    "threesea-me-move-up":
        ("Move up",
         "Nach oben"),
    "threesea-me-move-down":
        ("Move down",
         "Nach unten"),
    "threesea-me-send-video":
        ("Send video",
         "Video senden"),
    "threesea-la-cannot-play-video":
        ("This video could not be played",
         "Dieses Video konnte nicht abgespielt werden"),
    "threesea-me-select-messages":
        ("Select messages",
         "Nachrichten auswählen"),
    "threesea-he-n-selected":
        ("%n selected",
         ("%n ausgewählt",
          "%n ausgewählt")),
    "threesea-la-deleting-messages":
        ("Deleting %n message(s)",
         ("%n Nachricht wird gelöscht",
          "%n Nachrichten werden gelöscht")),
    "threesea-la-n-messages":
        ("%n message(s)",
         ("%n Nachricht",
          "%n Nachrichten")),
    "threesea-la-video":
        ("Video",
         "Video"),
    "threesea-me-cancel-selection":
        ("Cancel selection",
         "Auswahl beenden"),
    "threesea-la-select-hint":
        ("Tap messages to select them",
         "Tippe auf Nachrichten, um sie auszuwählen"),
    "threesea-la-status-list-created":
        ("You created this distribution list.",
         "Du hast diese Verteilerliste erstellt."),
    "threesea-he-new-list":
        ("New distribution list",
         "Neue Verteilerliste"),
    "threesea-me-new-list":
        ("New distribution list",
         "Neue Verteilerliste"),
    "threesea-me-delete-list":
        ("Delete list",
         "Liste löschen"),
    "threesea-la-deleting-list":
        ("Deleting list",
         "Liste wird gelöscht"),
    "threesea-he-distribution-list":
        ("Distribution list",
         "Verteilerliste"),
    "threesea-la-recipient-count":
        ("%n recipient(s)",
         ("%n Empfänger",
          "%n Empfänger")),
    "threesea-la-distribution-list-info":
        ("Each recipient gets the messages as a normal message from you; they don't see who else got them.",
         "Alle Empfänger erhalten die Nachrichten als normale Nachricht von dir; sie sehen nicht, wer sie sonst noch bekommen hat."),
    "threesea-he-recipients":
        ("Recipients",
         "Empfänger"),
    "threesea-la-list-name":
        ("List name",
         "Name der Liste"),
    "threesea-me-star":
        ("Star",
         "Markieren"),
    "threesea-he-starred":
        ("Starred messages",
         "Markierte Nachrichten"),
    "threesea-la-no-starred":
        ("No starred messages",
         "Keine markierten Nachrichten"),
    "threesea-la-no-starred-hint":
        ("Long press a message and choose Star to keep it here.",
         "Halte eine Nachricht gedrückt und wähle Markieren, um sie hier zu sammeln."),
    "threesea-la-draft":
        ("Draft: %1",
         "Entwurf: %1"),
    "threesea-he-archived-chats":
        ("Archived chats",
         "Archivierte Chats"),
    "threesea-me-unarchive":
        ("Move back to chats",
         "Zurück zu den Chats"),
    "threesea-me-archive":
        ("Archive",
         "Archivieren"),
    "threesea-la-no-archived":
        ("No archived chats",
         "Keine archivierten Chats"),
    "threesea-me-details":
        ("Details",
         "Details"),
    "threesea-he-message-details":
        ("Message details",
         "Nachrichtendetails"),
    "threesea-la-details-created":
        ("Created",
         "Erstellt"),
    "threesea-la-details-sent":
        ("Sent",
         "Gesendet"),
    "threesea-la-details-received":
        ("Received",
         "Empfangen"),
    "threesea-la-details-delivered":
        ("Delivered",
         "Zugestellt"),
    "threesea-la-details-read":
        ("Read",
         "Gelesen"),
    "threesea-la-details-deleted":
        ("Deleted",
         "Gelöscht"),
    "threesea-la-details-edited":
        ("Edited",
         "Bearbeitet"),
    "threesea-la-details-file-type":
        ("File type",
         "Dateityp"),
    "threesea-la-details-file-size":
        ("File size",
         "Dateigröße"),
    "threesea-la-details-message-id":
        ("Message ID",
         "Nachrichten-ID"),
    "threesea-me-show-in-chat":
        ("Show in chat",
         "Im Chat zeigen"),
    "threesea-la-no-media":
        ("No media",
         "Keine Medien"),
    "threesea-la-wrong-pin":
        ("Wrong PIN",
         "Falsche PIN"),
    "threesea-la-error-pin-short":
        ("The PIN needs at least 4 digits.",
         "Die PIN braucht mindestens 4 Ziffern."),
    "threesea-la-error-locked":
        ("Enter your PIN first.",
         "Gib zuerst deine PIN ein."),
    "threesea-la-private-needs-lock":
        ("Private chats need an app lock. Set one up in Settings, Security, App lock.",
         "Private Chats brauchen eine App-Sperre. Richte sie unter Einstellungen, Sicherheit, App-Sperre ein."),
    "threesea-la-error-private-chats-exist":
        ("Remove all chats from the private chats first.",
         "Entferne zuerst alle Chats aus den privaten Chats."),
    "threesea-la-private-chat":
        ("Private chat",
         "Privater Chat"),
    "threesea-me-lock-immediately":
        ("Immediately",
         "Sofort"),
    "threesea-me-lock-30-seconds":
        ("After 30 seconds",
         "Nach 30 Sekunden"),
    "threesea-me-lock-1-minute":
        ("After 1 minute",
         "Nach 1 Minute"),
    "threesea-me-lock-2-minutes":
        ("After 2 minutes",
         "Nach 2 Minuten"),
    "threesea-me-lock-5-minutes":
        ("After 5 minutes",
         "Nach 5 Minuten"),
    "threesea-me-lock-10-minutes":
        ("After 10 minutes",
         "Nach 10 Minuten"),
    "threesea-me-lock-30-minutes":
        ("After 30 minutes",
         "Nach 30 Minuten"),
    "threesea-he-app-lock":
        ("App lock",
         "App-Sperre"),
    "threesea-me-lock-off":
        ("Off",
         "Aus"),
    "threesea-me-lock-pin":
        ("PIN or passphrase",
         "PIN oder Passphrase"),
    "threesea-la-change-pin":
        ("Change PIN",
         "PIN ändern"),
    "threesea-la-lock-app":
        ("Lock the app",
         "App sperren"),
    "threesea-la-lock-app-description":
        ("Ask when Threesea starts, and when you come back to it after the time you choose. Private chats always ask.",
         "Fragt, wenn Threesea startet und wenn du nach der gewählten Zeit zurückkommst. Private Chats fragen immer."),
    "threesea-la-lock-after":
        ("Lock",
         "Sperren"),
    "threesea-he-private-chats":
        ("Private chats",
         "Private Chats"),
    "threesea-me-unmark-private":
        ("Remove from private chats",
         "Aus den privaten Chats entfernen"),
    "threesea-me-mark-private":
        ("Make private",
         "Privat machen"),
    "threesea-la-no-private":
        ("No private chats",
         "Keine privaten Chats"),
    "threesea-he-repeat-pin":
        ("Enter the PIN again",
         "PIN noch einmal eingeben"),
    "threesea-la-pin-hint":
        ("At least 4 digits",
         "Mindestens 4 Ziffern"),
    "threesea-la-pin-mismatch":
        ("They did not match. Try again.",
         "Sie stimmen nicht überein. Versuche es noch einmal."),
    "threesea-he-enter-pin":
        ("Enter your PIN",
         "Gib deine PIN ein"),
    "threesea-la-locked-notification":
        ("Threesea is locked",
         "Threesea ist gesperrt"),
    "threesea-la-locked-notification-body":
        ("Open it and enter your PIN or passphrase to receive messages.",
         "Öffne es und gib deine PIN oder Passphrase ein, um Nachrichten zu empfangen."),
    "threesea-la-key-encryption-failed":
        ("Could not change the encryption of the stored data.",
         "Die Verschlüsselung der gespeicherten Daten konnte nicht geändert werden."),
    "threesea-la-lock-passphrase":
        ("Passphrase",
         "Passphrase"),
    "threesea-la-lock-pin":
        ("PIN",
         "PIN"),
    "threesea-la-change-passphrase":
        ("Change passphrase",
         "Passphrase ändern"),
    "threesea-he-new-passphrase":
        ("Enter new passphrase",
         "Neue Passphrase eingeben"),
    "threesea-he-new-pin":
        ("Enter new PIN",
         "Neue PIN eingeben"),
    "threesea-he-repeat-passphrase":
        ("Enter the passphrase again",
         "Passphrase noch einmal eingeben"),
    "threesea-la-passphrase-hint":
        ("At least %1 characters",
         "Mindestens %1 Zeichen"),
    "threesea-he-enter-passphrase":
        ("Enter your passphrase",
         "Gib deine Passphrase ein"),
    "threesea-la-wrong-passphrase":
        ("Wrong passphrase",
         "Falsche Passphrase"),
    "threesea-la-revocation-intro":
        ("With this password you can revoke your Threema ID at myid.threema.ch/revoke in case you lose your phone or it gets stolen. Use at least 8 characters.",
         "Mit diesem Passwort kannst du deine Threema-ID unter myid.threema.ch/revoke widerrufen, falls dein Telefon verloren geht oder gestohlen wird. Verwende mindestens 8 Zeichen."),
    "threesea-la-error-phone-invalid":
        ("Enter the number with its country code, e.g. +41 79 123 45 67.",
         "Gib die Nummer mit Ländervorwahl ein, z. B. +41 79 123 45 67."),
    "threesea-la-error-email-invalid":
        ("This is not an e-mail address.",
         "Das ist keine E-Mail-Adresse."),
    "threesea-la-error-no-verification":
        ("No phone number is waiting for a code. Link it again.",
         "Keine Telefonnummer wartet auf einen Code. Verknüpfe sie noch einmal."),
    "threesea-me-unlink":
        ("Remove link",
         "Verknüpfung entfernen"),
    "threesea-he-link":
        ("Link",
         "Verknüpfen"),
    "threesea-la-link-email-info":
        ("People who have this e-mail address in their address book can find you on Threema. It is sent to Threema, which e-mails you a link to confirm it.",
         "Wer diese E-Mail-Adresse im Adressbuch hat, kann dich in Threema finden. Sie wird an Threema gesendet; Threema schickt dir einen Link zur Bestätigung."),
    "threesea-la-link-mobile-info":
        ("People who have this number in their address book can find you on Threema. It is sent to Threema, which texts you a code to confirm it.",
         "Wer diese Nummer im Adressbuch hat, kann dich in Threema finden. Sie wird an Threema gesendet; Threema schickt dir per SMS einen Code zur Bestätigung."),
    "threesea-la-email":
        ("E-mail address",
         "E-Mail-Adresse"),
    "threesea-la-phone":
        ("Phone number",
         "Telefonnummer"),
    "threesea-me-cancel-link":
        ("Cancel linking",
         "Verknüpfen abbrechen"),
    "threesea-me-call-me":
        ("Call me instead",
         "Stattdessen Anruf anfordern"),
    "threesea-la-call-coming":
        ("You will get a call with the code.",
         "Du bekommst einen Anruf mit dem Code."),
    "threesea-he-confirm":
        ("Confirm",
         "Bestätigen"),
    "threesea-la-enter-sms-code":
        ("Enter the code Threema sent to +%1.",
         "Gib den Code ein, den Threema an +%1 geschickt hat."),
    "threesea-la-sms-code":
        ("Code",
         "Code"),
    "threesea-he-media":
        ("Media",
         "Medien"),
    "threesea-me-media-show":
        ("Show",
         "Zeigen"),
    "threesea-la-kind-photos":
        ("Photos",
         "Fotos"),
    "threesea-la-kind-gifs":
        ("GIFs",
         "GIFs"),
    "threesea-la-kind-videos":
        ("Videos",
         "Videos"),
    "threesea-la-kind-voice":
        ("Voice messages",
         "Sprachnachrichten"),
    "threesea-la-kind-audio":
        ("Audio",
         "Audio"),
    "threesea-la-kind-files":
        ("Files",
         "Dateien"),
    "threesea-la-no-media-filtered":
        ("Nothing of the kinds shown",
         "Keine Medien der gewählten Arten"),
    "threesea-la-no-media-filtered-hint":
        ("Pull down to choose what to show",
         "Ziehe nach unten, um zu wählen, was gezeigt wird"),
    "threesea-la-enter-to-send":
        ("Enter sends",
         "Eingabetaste sendet"),
    "threesea-la-enter-to-send-description":
        ("Enter sends the message. Off: Enter starts a new line and the send button sends. On a hardware keyboard, Shift+Enter does the opposite.",
         "Die Eingabetaste sendet die Nachricht. Aus: Die Eingabetaste beginnt eine neue Zeile, und die Senden-Taste sendet. Auf einer Hardware-Tastatur macht Umschalt+Eingabe das Gegenteil."),
    "threesea-se-advanced":
        ("Advanced",
         "Erweitert"),
    "threesea-se-security":
        ("Security",
         "Sicherheit"),
    "threesea-se-account":
        ("Account",
         "Konto"),
    "threesea-se-privacy-security":
        ("Privacy and security",
         "Privatsphäre und Sicherheit"),
    "threesea-la-app-lock-on":
        ("App lock on",
         "App-Sperre an"),
    "threesea-la-app-lock-off":
        ("App lock off",
         "App-Sperre aus"),
    "threesea-se-app":
        ("App",
         "App"),
    "threesea-se-info":
        ("Info",
         "Info"),
    "threesea-me-delete-data":
        ("Remove ID from this device",
         "ID von diesem Gerät entfernen"),
    "threesea-la-removing-data":
        ("Removing ID and data",
         "ID und Daten werden entfernt"),
    "threesea-la-revocation":
        ("ID revocation",
         "ID-Widerruf"),
    "threesea-la-saving":
        ("Saving…",
         "Wird gespeichert …"),
    "threesea-la-revocation-set-at":
        ("Password set %1",
         "Passwort gesetzt %1"),
    "threesea-la-revocation-set":
        ("Password set",
         "Passwort gesetzt"),
    "threesea-la-revocation-not-set":
        ("No password set",
         "Kein Passwort gesetzt"),
    "threesea-la-link-email":
        ("E-mail",
         "E-Mail"),
    "threesea-la-not-linked":
        ("Not linked",
         "Nicht verknüpft"),
    "threesea-la-email-pending":
        ("Waiting for you to confirm the link in the e-mail.",
         "Wartet darauf, dass du die Verknüpfung in der E-Mail bestätigst."),
    "threesea-la-link-phone":
        ("Phone number",
         "Telefonnummer"),
    "threesea-la-phone-pending":
        ("+%1 (not confirmed)",
         "+%1 (nicht bestätigt)"),
    "threesea-la-nickname-placeholder":
        ("Shown to the people you write to",
         "Wird den Personen angezeigt, denen du schreibst"),
    "threesea-la-change-profile-picture":
        ("Change profile picture",
         "Profilbild ändern"),
    "threesea-la-set-profile-picture":
        ("Set profile picture",
         "Profilbild festlegen"),
    "threesea-me-share-everyone":
        ("Everyone I write to",
         "Allen, denen ich schreibe"),
    "threesea-me-share-nobody":
        ("Nobody",
         "Niemandem"),
    "threesea-me-share-selected":
        ("Selected contacts",
         "Ausgewählten Kontakten"),
    "threesea-la-picture-share":
        ("Share profile picture with",
         "Profilbild teilen mit"),
    "threesea-la-share-selected-count":
        ("%n selected contact(s)",
         ("%n ausgewähltem Kontakt",
          "%n ausgewählten Kontakten")),
    "threesea-me-exclude-id":
        ("Add a Threema ID",
         "Threema-ID hinzufügen"),
    "threesea-la-exclusion-intro":
        ("Finding contacts from the address book will not add or rename this Threema ID.",
         "Diese Threema-ID wird beim Finden von Kontakten aus dem Adressbuch weder hinzugefügt noch umbenannt."),
    "threesea-he-exclusion-list":
        ("Exclusion list",
         "Ausschlussliste"),
    "threesea-me-remove-excluded":
        ("Remove",
         "Entfernen"),
    "threesea-la-no-excluded":
        ("Nobody is excluded",
         "Niemand ist ausgeschlossen"),
    "threesea-me-refresh":
        ("Refresh",
         "Aktualisieren"),
    "threesea-me-empty-chat":
        ("Empty chat",
         "Chat leeren"),
    "threesea-la-emptying-chat":
        ("Emptying chat",
         "Chat wird geleert"),
    "threesea-se-other":
        ("Other",
         "Sonstiges"),
    "threesea-la-incognito-keyboard":
        ("Incognito keyboard",
         "Inkognito-Tastatur"),
    "threesea-la-incognito-keyboard-description":
        ("Ask the keyboard not to learn or suggest from what you type in chats.",
         "Die Tastatur bitten, aus dem, was du in Chats schreibst, nichts zu lernen oder vorzuschlagen."),
    "threesea-la-hide-cover":
        ("Hide content on the cover",
         "Inhalt auf dem Cover verbergen"),
    "threesea-la-hide-cover-description":
        ("The cover only shows the number of new messages, not who wrote.",
         "Das Cover zeigt nur die Zahl neuer Nachrichten, nicht, wer geschrieben hat."),
    "threesea-se-appearance":
        ("Appearance",
         "Darstellung"),
    "threesea-la-bigger-emojis":
        ("Bigger single emojis",
         "Größere einzelne Emojis"),
    "threesea-la-bigger-emojis-description":
        ("Messages of up to three emojis are shown larger.",
         "Nachrichten aus bis zu drei Emojis werden größer gezeigt."),
    "threesea-se-contacts":
        ("Contacts",
         "Kontakte"),
    "threesea-se-receipts":
        ("Receipts",
         "Bestätigungen"),
    "threesea-me-policy-default-on":
        ("Same as in Settings (on)",
         "Wie in den Einstellungen (an)"),
    "threesea-me-policy-default-off":
        ("Same as in Settings (off)",
         "Wie in den Einstellungen (aus)"),
    "threesea-me-policy-send":
        ("Always",
         "Immer"),
    "threesea-me-policy-dont-send":
        ("Never",
         "Nie"),
    "threesea-la-contact-read-receipts":
        ("Read receipts",
         "Lesebestätigungen"),
    "threesea-la-contact-typing":
        ("Typing indicator",
         "Melden, wenn ich tippe"),
    "threesea-me-clear-contact-policies":
        ("Clear individual settings",
         "Individuelle Einstellungen löschen"),
    "threesea-la-clearing-contact-policies":
        ("Clearing individual settings",
         "Individuelle Einstellungen werden gelöscht"),
    "threesea-la-no-contact-policies":
        ("No contact has its own setting.",
         "Kein Kontakt hat eine eigene Einstellung."),
    "threesea-se-data-encryption":
        ("Encryption of stored data",
         "Verschlüsselung der gespeicherten Daten"),
    "threesea-me-older-1-year":
        ("Older than 1 year",
         "Älter als 1 Jahr"),
    "threesea-me-older-6-months":
        ("Older than 6 months",
         "Älter als 6 Monate"),
    "threesea-me-older-3-months":
        ("Older than 3 months",
         "Älter als 3 Monate"),
    "threesea-me-older-1-month":
        ("Older than 1 month",
         "Älter als 1 Monat"),
    "threesea-me-older-1-week":
        ("Older than 1 week",
         "Älter als 1 Woche"),
    "threesea-me-older-all":
        ("All",
         "Alle"),
    "threesea-he-delete":
        ("Delete",
         "Löschen"),
    "threesea-se-clean-up-what":
        ("What",
         "Was"),
    "threesea-me-clean-up-media":
        ("Media files",
         "Mediendateien"),
    "threesea-me-clean-up-media-description":
        ("Photos, videos, voice messages and files are deleted from this device, usually for good; the messages and their previews stay. Starred messages and messages still on their way keep their files.",
         "Fotos, Videos, Sprachnachrichten und Dateien werden von diesem Gerät gelöscht, meist endgültig; die Nachrichten und ihre Vorschauen bleiben. Markierte Nachrichten und Nachrichten, die noch unterwegs sind, behalten ihre Dateien."),
    "threesea-me-clean-up-messages":
        ("Messages and media",
         "Nachrichten und Medien"),
    "threesea-me-clean-up-messages-description":
        ("Starred messages, open polls and messages still on their way stay.",
         "Markierte Nachrichten, offene Umfragen und Nachrichten, die noch unterwegs sind, bleiben."),
    "threesea-se-clean-up-age":
        ("Age",
         "Alter"),
    "threesea-me-image-small":
        ("Small (640 × 640)",
         "Klein (640 × 640)"),
    "threesea-me-image-medium":
        ("Medium (1024 × 1024)",
         "Mittel (1024 × 1024)"),
    "threesea-me-image-large":
        ("Large (1600 × 1600)",
         "Groß (1600 × 1600)"),
    "threesea-me-image-xlarge":
        ("Extra large (2592 × 2592)",
         "Sehr groß (2592 × 2592)"),
    "threesea-me-image-original":
        ("Original size",
         "Originalgröße"),
    "threesea-la-download-nothing":
        ("Nothing",
         "Nichts"),
    "threesea-se-media-storage":
        ("Media and storage",
         "Medien und Speicher"),
    "threesea-se-sending":
        ("Sending",
         "Senden"),
    "threesea-la-image-size":
        ("Image size",
         "Bildgröße"),
    "threesea-se-receiving":
        ("Receiving",
         "Empfangen"),
    "threesea-la-download-wifi":
        ("Download on Wi-Fi",
         "Im WLAN herunterladen"),
    "threesea-la-download-mobile":
        ("Download on mobile data",
         "Über mobile Daten herunterladen"),
    "threesea-la-auto-save":
        ("Save to Gallery",
         "In der Galerie speichern"),
    "threesea-la-auto-save-description":
        ("Received photos and videos are also saved, unencrypted, to the Threesea folder in Pictures.",
         "Empfangene Fotos und Videos werden zusätzlich unverschlüsselt im Ordner Threesea unter Bilder gespeichert."),
    "threesea-se-storage":
        ("Storage",
         "Speicher"),
    "threesea-la-storage-messages":
        ("Messages",
         "Nachrichten"),
    "threesea-me-clean-up":
        ("Clean up",
         "Aufräumen"),
    "threesea-la-twemoji-credit":
        ("Emoji graphics by Twemoji (github.com/jdecked/twemoji), licensed under CC-BY 4.0.",
         "Emoji-Grafiken von Twemoji (github.com/jdecked/twemoji), lizenziert unter CC-BY 4.0."),
    "threesea-me-emoji-system":
        ("System",
         "System"),
    "threesea-la-emoji-style":
        ("Emoji style",
         "Emoji-Stil"),
    "threesea-se-blocking":
        ("Blocking",
         "Blockieren"),
    "threesea-me-first-name-first":
        ("First name first",
         "Vorname zuerst"),
    "threesea-me-last-name-first":
        ("Last name first",
         "Nachname zuerst"),
    "threesea-la-name-order":
        ("Names",
         "Namen"),
    "threesea-me-sort-first-name":
        ("By first name",
         "Nach Vorname"),
    "threesea-me-sort-last-name":
        ("By last name",
         "Nach Nachname"),
    "threesea-la-sorting":
        ("Sorting",
         "Sortierung"),
    "threesea-la-show-inactive":
        ("Show inactive IDs",
         "Inaktive IDs anzeigen"),
    "threesea-la-show-inactive-description":
        ("Contacts whose Threema ID has not been used for a long time stay in the list.",
         "Kontakte, deren Threema-ID lange nicht benutzt wurde, bleiben in der Liste."),
    "threesea-se-notifications-chats":
        ("Chats",
         "Chats"),
    "threesea-la-notification-sound":
        ("Sound",
         "Ton"),
    "threesea-la-notification-sound-description":
        ("Played at the volume of the current sound profile.",
         "Wird in der Lautstärke des aktuellen Tonprofils abgespielt."),
    "threesea-la-notification-vibrate":
        ("Vibrate",
         "Vibration"),
    "threesea-se-notifications-groups":
        ("Groups",
         "Gruppen"),
    "threesea-se-network":
        ("Network",
         "Netzwerk"),
    "threesea-la-ipv6":
        ("IPv6",
         "IPv6"),
    "threesea-la-ipv6-description":
        ("Messages, files and calls use IPv6 where the network has it. Off: Threesea uses IPv4 for them.",
         "Nachrichten, Dateien und Anrufe verwenden IPv6, wo das Netz es anbietet. Aus: Threesea verwendet dafür IPv4."),
    "threesea-la-error-not-gif":
        ("This picture is not a GIF.",
         "Dieses Bild ist kein GIF."),
    "threesea-la-gif-favourite-added":
        ("Added to favourite GIFs",
         "Zu den Lieblings-GIFs hinzugefügt"),
    "threesea-he-add-gif":
        ("Add GIF",
         "GIF hinzufügen"),
    "threesea-me-remove-gif":
        ("Remove",
         "Entfernen"),
    "threesea-la-no-gif-favourites":
        ("No favourite GIFs",
         "Keine Lieblings-GIFs"),
    "threesea-la-no-gif-favourites-hint":
        ("Long press a GIF in a chat to add it, or add one from this device.",
         "Halte ein GIF in einem Chat gedrückt, um es hinzuzufügen, oder füge eines von diesem Gerät hinzu."),
    "threesea-la-autoplay-gifs":
        ("Play GIFs automatically",
         "GIFs automatisch abspielen"),
    "threesea-la-autoplay-gifs-description":
        ("Animated images play in the chat. Off: they play when you open them.",
         "Animierte Bilder laufen im Chat. Aus: Sie laufen, wenn du sie öffnest."),
    "threesea-la-notification-tone":
        ("Notification tone",
         "Benachrichtigungston"),
    "threesea-la-tone-all-chats":
        ("Same as all chats",
         "Wie bei allen Chats"),
    "threesea-la-tone-all-groups":
        ("Same as all groups",
         "Wie bei allen Gruppen"),
    "threesea-la-tone-profile":
        ("Instant message tone",
         "Ton für Sofortnachrichten"),
    "threesea-la-tone-from-music":
        ("Select from music",
         "Aus der Musik wählen"),
    "threesea-se-tones":
        ("Tones",
         "Töne"),
    "threesea-la-error-link-unknown-code":
        ("This is not a code of Threema for Desktop. Scan the code it shows when you open it.",
         "Das ist kein Code von Threema für Desktop. Scanne den Code, den es beim Öffnen zeigt."),
    "threesea-la-error-link-web-code":
        ("This code is for Threema Web. Scan the code of Threema for Desktop instead.",
         "Dieser Code ist für Threema Web. Scanne stattdessen den Code von Threema für Desktop."),
    "threesea-la-error-link-version":
        ("This version of Threema for Desktop can't be linked. Please update both apps.",
         "Diese Version von Threema für Desktop kann nicht verknüpft werden. Bitte aktualisiere beide Apps."),
    "threesea-la-error-link-failed":
        ("Linking didn't work. Check the connection on both devices and try again.",
         "Das Verknüpfen hat nicht geklappt. Prüfe die Verbindung auf beiden Geräten und versuche es erneut."),
    "threesea-la-qr-not-device":
        ("This is a contact's code. Scan the code on the computer.",
         "Das ist der Code eines Kontakts. Scanne den Code auf dem Computer."),
    "threesea-la-scan-hint-device":
        ("Point the camera at the QR code Threema for Desktop shows.",
         "Richte die Kamera auf den QR-Code, den Threema für Desktop zeigt."),
    "threesea-la-link-intro":
        ("Open Threema for Desktop on your computer. It shows a QR code; scan it on the next page.",
         "Öffne Threema für Desktop auf deinem Computer. Es zeigt einen QR-Code; scanne ihn auf der nächsten Seite."),
    "threesea-la-link-emoji-mismatch":
        ("The emojis don't match. Close Threema for Desktop, open it again and scan its new code.",
         "Die Emojis stimmen nicht überein. Schließe Threema für Desktop, öffne es erneut und scanne den neuen Code."),
    "threesea-he-link-device":
        ("Link device",
         "Gerät verknüpfen"),
    "threesea-la-link-connecting":
        ("Connecting to the computer…",
         "Verbindung zum Computer wird hergestellt …"),
    "threesea-la-link-verify":
        ("Which emojis does the computer show?",
         "Welche Emojis zeigt der Computer?"),
    "threesea-la-link-linking":
        ("Linking. Keep Threesea open until the computer is ready.",
         "Wird verknüpft. Lass Threesea offen, bis der Computer bereit ist."),
    "threesea-la-link-done":
        ("Threema for Desktop is linked.",
         "Threema für Desktop ist verknüpft."),
    "threesea-me-link-no-match":
        ("None of them",
         "Keine davon"),
    "threesea-me-done":
        ("Done",
         "Fertig"),
    "threesea-la-removing-device":
        ("Removing",
         "Wird entfernt"),
    "threesea-me-remove-all-devices":
        ("Remove all linked devices",
         "Alle verknüpften Geräte entfernen"),
    "threesea-la-removing-all-devices":
        ("Removing all linked devices",
         "Alle verknüpften Geräte werden entfernt"),
    "threesea-me-link-device":
        ("Link device",
         "Gerät verknüpfen"),
    "threesea-se-linked-devices":
        ("Linked devices",
         "Verknüpfte Geräte"),
    "threesea-la-device":
        ("Device",
         "Gerät"),
    "threesea-la-this-device":
        ("This device",
         "Dieses Gerät"),
    "threesea-la-device-connected":
        ("Connected",
         "Verbunden"),
    "threesea-la-device-last-active":
        ("Last active %1",
         "Zuletzt aktiv %1"),
    "threesea-me-remove-device":
        ("Remove",
         "Entfernen"),
    "threesea-la-no-linked-devices":
        ("No linked devices",
         "Keine verknüpften Geräte"),
    "threesea-la-no-linked-devices-hint":
        ("With Threema for Desktop you can read and write your chats on a computer. Open it there and pull down here to link it.",
         "Mit Threema für Desktop kannst du deine Chats am Computer lesen und schreiben. Öffne es dort und ziehe hier nach unten, um es zu verknüpfen."),
    "threesea-me-connect-now":
        ("Connect now",
         "Jetzt verbinden"),
    "threesea-la-removing-lock":
        ("Removing the app lock",
         "App-Sperre wird entfernt"),
    "threesea-me-unstar-short":
        ("Unstar",
         "Stern entfernen"),
    "threesea-la-camera-unavailable":
        ("The camera is not available.",
         "Die Kamera ist nicht verfügbar."),
    "threesea-me-take-photo":
        ("Take photo or video",
         "Foto oder Video aufnehmen"),
    "threesea-me-add-gif-favourite":
        ("Add to favourite GIFs",
         "Zu den Lieblings-GIFs"),
    "threesea-me-add-recipients":
        ("Add recipients",
         "Empfänger hinzufügen"),
    "threesea-la-status-call-outgoing":
        ("Outgoing call, %1",
         "Ausgehender Anruf, %1"),
    "threesea-la-status-call-incoming":
        ("Incoming call, %1",
         "Eingehender Anruf, %1"),
    "threesea-la-status-call-missed":
        ("Missed call",
         "Verpasster Anruf"),
    "threesea-la-status-call-declined":
        ("Call declined",
         "Anruf abgelehnt"),
    "threesea-la-status-call-you-declined":
        ("You declined the call",
         "Du hast den Anruf abgelehnt"),
    "threesea-la-status-call-busy":
        ("Call not answered: busy",
         "Anruf nicht angenommen: besetzt"),
    "threesea-la-status-call-unanswered":
        ("Call not answered",
         "Anruf nicht angenommen"),
    "threesea-la-status-call-disabled":
        ("Call not answered: calls are off",
         "Anruf nicht angenommen: Anrufe sind aus"),
    "threesea-la-status-call-cancelled":
        ("Call cancelled",
         "Anruf abgebrochen"),
    "threesea-la-status-call-failed":
        ("Call failed",
         "Anruf fehlgeschlagen"),
    "threesea-la-call-calling":
        ("Calling…",
         "Ruft an …"),
    "threesea-la-call-ringing":
        ("Ringing…",
         "Klingelt …"),
    "threesea-la-call-incoming":
        ("Incoming call",
         "Eingehender Anruf"),
    "threesea-la-call-connecting":
        ("Connecting…",
         "Verbindung wird hergestellt …"),
    "threesea-la-call-ended":
        ("Call ended",
         "Anruf beendet"),
    "threesea-me-call":
        ("Call",
         "Anrufen"),
    "threesea-se-calls":
        ("Calls",
         "Anrufe"),
    "threesea-la-threema-calls":
        ("Threema calls",
         "Threema-Anrufe"),
    "threesea-la-threema-calls-description":
        ("Contacts can call you, and you can call them from the chat's pull-down menu.",
         "Kontakte können dich anrufen, und du kannst sie über das Menü des Chats anrufen."),
    "threesea-la-relay-calls":
        ("Always relay calls",
         "Anrufe immer über Server"),
    "threesea-la-relay-calls-description":
        ("Calls go through a Threema server, which keeps your IP address hidden from the other side. Call quality may be lower.",
         "Anrufe laufen über einen Threema-Server, der deine IP-Adresse vor der Gegenseite verbirgt. Die Anrufqualität kann geringer sein."),
    "threesea-me-video-call":
        ("Video call",
         "Videoanruf"),
    "threesea-se-video":
        ("Video",
         "Video"),
    "threesea-la-video-calls":
        ("Video in one-to-one calls",
         "Video in Einzelanrufen"),
    "threesea-la-video-calls-description":
        ("One-to-one calls can show the camera of both sides; each side turns its camera on when it wants.",
         "Einzelanrufe können die Kamera beider Seiten zeigen; jede Seite schaltet ihre Kamera ein, wann sie will."),
    "threesea-la-video-quality":
        ("Video quality",
         "Videoqualität"),
    "threesea-la-video-quality-auto":
        ("Standard",
         "Standard"),
    "threesea-la-video-quality-low":
        ("Low data use",
         "Niedriger Datenverbrauch"),
    "threesea-la-video-quality-max":
        ("Highest quality",
         "Maximale Qualität"),
    "threesea-la-video-quality-description":
        ("Both sides use the lower of their two settings. Calls that go only through a relay (Always relay calls, or with an unverified contact) use Standard at most.",
         "Beide Seiten verwenden die niedrigere ihrer beiden Einstellungen. Anrufe, die nur über einen Relay-Server laufen (Anrufe immer über Server, oder mit einem nicht überprüften Kontakt), verwenden höchstens Standard."),
    "threesea-me-switch-camera":
        ("Switch camera",
         "Kamera wechseln"),
    "threesea-me-show-self":
        ("Show own picture",
         "Eigenes Bild zeigen"),
    "threesea-me-hide-self":
        ("Hide own picture",
         "Eigenes Bild verbergen"),
    "threesea-la-me":
        ("Me",
         "Ich"),
    "threesea-me-remove-profile-picture":
        ("Remove profile picture",
         "Profilbild entfernen"),
    "threesea-me-join-group-call":
        ("Join group call",
         "Am Gruppenanruf teilnehmen"),
    "threesea-la-group-call-joining":
        ("Joining…",
         "Verbindung wird hergestellt …"),
    "threesea-me-start-group-call":
        ("Start group call",
         "Gruppenanruf starten"),
    "threesea-la-group-call-waiting":
        ("Waiting for others",
         "Warten auf andere"),
    "threesea-la-group-call":
        ("Group call",
         "Gruppenanruf"),
    "threesea-la-group-call-participants":
        ("%n participant(s)",
         ("%n Teilnehmer",
          "%n Teilnehmer")),
    "threesea-la-group-call-return":
        ("Return",
         "Zurück"),
    "threesea-la-group-call-join":
        ("Join",
         "Teilnehmen"),
    "threesea-la-error-web-version":
        ("This version of Threema Web can't be used. Reload the page in the browser.",
         "Diese Version von Threema Web kann nicht verwendet werden. Lade die Seite im Browser neu."),
    "threesea-la-error-web-session":
        ("This Threema Web session no longer exists.",
         "Diese Threema-Web-Sitzung gibt es nicht mehr."),
    "threesea-he-web-trust":
        ("Trust",
         "Vertrauen"),
    "threesea-he-web-allow":
        ("Allow",
         "Erlauben"),
    "threesea-la-web-certificate-changed":
        ("This Threema Web server shows a different certificate than last time. Trust it only if you know its certificate was renewed; otherwise go back.",
         "Dieser Threema-Web-Server zeigt ein anderes Zertifikat als beim letzten Mal. Vertraue ihm nur, wenn du weißt, dass sein Zertifikat erneuert wurde; sonst geh zurück."),
    "threesea-la-web-self-hosted":
        ("This code is for a Threema Web server run by someone other than Threema. Your chats go through it end-to-end encrypted. Allow it only if you run it yourself or trust who does. Its certificate is remembered and you'll be asked if it changes.",
         "Dieser Code ist für einen Threema-Web-Server, den nicht Threema betreibt. Deine Chats laufen Ende-zu-Ende-verschlüsselt darüber. Erlaube ihn nur, wenn du ihn selbst betreibst oder dem Betreiber vertraust. Sein Zertifikat wird gespeichert, und du wirst gefragt, wenn es sich ändert."),
    "threesea-la-web-connecting":
        ("Connecting…",
         "Verbindung wird hergestellt …"),
    "threesea-la-web-waiting":
        ("Waiting for the browser",
         "Wartet auf den Browser"),
    "threesea-la-web-connected":
        ("Connected",
         "Verbunden"),
    "threesea-la-web-last-used":
        ("Last used %1",
         "Zuletzt verwendet %1"),
    "threesea-la-web-not-connected":
        ("Not connected",
         "Nicht verbunden"),
    "threesea-me-scan-web-code":
        ("Scan Threema Web code",
         "Threema-Web-Code scannen"),
    "threesea-he-threema-web":
        ("Threema Web",
         "Threema Web"),
    "threesea-la-web-none":
        ("No browsers",
         "Keine Browser"),
    "threesea-la-web-none-hint":
        ("Open web.threema.ch in a browser and pull down to scan its code",
         "Öffne web.threema.ch in einem Browser und ziehe nach unten, um den Code zu scannen"),
    "threesea-me-web-remove":
        ("Remove",
         "Entfernen"),
    "threesea-la-web-browser":
        ("Browser",
         "Browser"),
    "threesea-la-no-recent-emoji":
        ("No emoji used yet",
         "Noch keine Emojis verwendet"),
    "threesea-la-poll-closed-preview":
        ("Poll closed: %1",
         "Umfrage beendet: %1"),
    "threesea-la-poll-tap-results":
        ("Poll closed. Tap to view results",
         "Umfrage beendet. Tippe für die Ergebnisse"),
    "threesea-he-poll-results":
        ("Results",
         "Ergebnisse"),
    "threesea-se-chat":
        ("Chat",
         "Chat"),
    "threesea-la-scan-hint-any":
        ("Point the camera at a contact's Threema QR code, or at the one Threema Web or Threema for Desktop shows.",
         "Richte die Kamera auf den Threema-QR-Code eines Kontakts oder auf den, den Threema Web oder Threema für Desktop zeigt."),
    "threesea-la-pin-paused":
        ("Too many wrong attempts. Try again in %n second(s).",
         ("Zu viele falsche Versuche. Versuche es in %n Sekunde erneut.",
          "Zu viele falsche Versuche. Versuche es in %n Sekunden erneut.")),
    "threesea-se-troubleshooting":
        ("Troubleshooting",
         "Fehlersuche"),
    "threesea-la-log-level":
        ("Log",
         "Protokoll"),
    "threesea-la-log-level-warn":
        ("Problems only",
         "Nur Probleme"),
    "threesea-la-log-level-info":
        ("Normal",
         "Normal"),
    "threesea-la-log-level-debug":
        ("Detailed",
         "Ausführlich"),
    "threesea-la-log-level-description":
        ("How much Threesea logs, to help find the cause of a problem.",
         "Wie viel Threesea protokolliert, um die Ursache eines Problems zu finden."),
    "threesea-se-disclaimer":
        ("Disclaimer",
         "Haftungsausschluss"),
    "threesea-la-disclaimer-ai":
        ("Threesea was developed with Claude Code (Anthropic) and audited with OpenAI Codex, directed and tested by a human maintainer. It is a research and experimentation project.",
         "Threesea wurde mit Claude Code (Anthropic) entwickelt und mit OpenAI Codex geprüft, angeleitet und getestet von einem menschlichen Maintainer. Es ist ein Forschungs- und Experimentierprojekt."),
    "threesea-la-disclaimer-warranty":
        ("AI makes mistakes. The code, including its security-relevant parts, can contain errors nobody noticed. The developers give no warranty of any kind; you use the app at your own risk.",
         "KI macht Fehler. Der Code, auch seine sicherheitsrelevanten Teile, kann Fehler enthalten, die niemand bemerkt hat. Die Entwickler übernehmen keinerlei Gewährleistung; du verwendest die App auf eigenes Risiko."),
    "threesea-la-disclaimer-threema":
        ("Threesea is based on Threema Libre's open-source code and tries to play fair with Threema's APIs and licensing. Threema told us it doesn't actively block third-party apps that cause no stability issues or high server load, are clearly marked as unofficial without using its branding, and don't circumvent the licensing. Threesea aims to meet that. This is not a permission, and Threesea is not affiliated with, endorsed or supported by Threema AG. As with any Threema app, keep a backup of your ID.",
         "Threesea beruht auf dem Open-Source-Code von Threema Libre und versucht, fair mit Threemas Schnittstellen und Lizenzen umzugehen. Threema hat uns mitgeteilt, dass es Apps von Dritten nicht aktiv blockiert, solange sie keine Stabilitätsprobleme oder hohe Serverlast verursachen, klar als inoffiziell gekennzeichnet sind, ohne Threemas Marke zu verwenden, und die Lizenzierung nicht umgehen. Threesea will dem entsprechen. Das ist keine Erlaubnis, und Threesea ist nicht mit der Threema AG verbunden, nicht von ihr empfohlen und nicht von ihr unterstützt. Wie bei jeder Threema-App gilt: Bewahre ein Backup deiner ID auf."),
    "threesea-se-credits":
        ("Credits",
         "Danksagungen"),
    "threesea-la-credit-threema":
        ("Threema AG: threema-android and libthreema (AGPL-3.0), the protocols of Threema, Threema for Desktop and Threema Web.",
         "Threema AG: threema-android und libthreema (AGPL-3.0), die Protokolle von Threema, Threema für Desktop und Threema Web."),
    "threesea-la-credit-opus":
        ("opus-pure by Stephen Berry, from Rust ports of libopus by the Xiph.Org Foundation and others (BSD-3-Clause): voice messages.",
         "opus-pure von Stephen Berry, aus Rust-Portierungen von libopus der Xiph.Org Foundation und anderer (BSD-3-Clause): Sprachnachrichten."),
    "threesea-la-credit-webrtc":
        ("webrtc-audio-processing, from Google's WebRTC (BSD-3-Clause): echo cancellation in calls.",
         "webrtc-audio-processing, aus Googles WebRTC (BSD-3-Clause): Echounterdrückung bei Anrufen."),
    "threesea-la-credit-jolla":
        ("Jolla and Sailfish OS: Silica and its icons; Jolla's Messages, People, Settings and Gallery as the model for the design.",
         "Jolla und Sailfish OS: Silica und seine Symbole; Jollas Nachrichten, Personen, Einstellungen und Galerie als Vorbild für die Gestaltung."),
    "threesea-la-credit-whisperfish":
        ("Whisperfish, the Signal client for Sailfish OS: a reference for building such an app.",
         "Whisperfish, der Signal-Client für Sailfish OS: ein Vorbild dafür, wie man eine solche App baut."),
    "threesea-la-credit-libraries":
        ("SQLCipher, GStreamer, Qt and many Rust libraries (tokio, rustls, reqwest, rusqlite, RustCrypto and others).",
         "SQLCipher, GStreamer, Qt und viele Rust-Bibliotheken (tokio, rustls, reqwest, rusqlite, RustCrypto und andere)."),
    "threesea-la-credit-eu-icon":
        ("The “AI generated” label is the European Commission's icon for AI-generated content.",
         "Das Label „AI generated“ ist das Symbol der Europäischen Kommission für KI-generierte Inhalte."),
    "threesea-la-update-notification":
        ("Threesea %1 is available",
         "Threesea %1 ist verfügbar"),
    "threesea-la-update-notification-body":
        ("Tap for details and the download page.",
         "Tippe für Details und die Download-Seite."),
    "threesea-me-check-updates-now":
        ("Check for updates now",
         "Jetzt nach Updates suchen"),
    "threesea-la-keep-running-description-full":
        ("Threesea starts when you turn on the phone and stays connected after you close it, so new messages arrive right away. Uses more battery.",
         "Threesea startet beim Einschalten des Telefons und bleibt verbunden, wenn du es schließt, damit neue Nachrichten sofort ankommen. Braucht mehr Akku."),
    "threesea-se-updates":
        ("Updates",
         "Updates"),
    "threesea-la-update-check":
        ("Check for updates",
         "Nach Updates suchen"),
    "threesea-la-update-check-description":
        ("Once a day, Threesea asks GitHub whether a newer version has been released, and tells you. GitHub sees your IP address; nothing else about you is sent.",
         "Einmal am Tag fragt Threesea bei GitHub, ob eine neuere Version erschienen ist, und sagt es dir. GitHub sieht deine IP-Adresse; sonst wird nichts über dich gesendet."),
    "threesea-la-update-open":
        ("Open download page",
         "Download-Seite öffnen"),
    "threesea-he-update-ask-accept":
        ("Check",
         "Suchen"),
    "threesea-he-update-ask-cancel":
        ("Don't check",
         "Nicht suchen"),
    "threesea-la-update-ask-title":
        ("Check for updates?",
         "Nach Updates suchen?"),
    "threesea-la-update-ask-text":
        ("This version of Threesea comes from GitHub, so it isn't updated by a store. Once a day, Threesea can ask GitHub whether a newer version has been released and tell you; you install it yourself. GitHub sees your IP address; nothing else about you is sent. You can change this in Settings, Advanced.",
         "Diese Version von Threesea stammt von GitHub und wird deshalb nicht von einem Store aktualisiert. Einmal am Tag kann Threesea bei GitHub fragen, ob eine neuere Version erschienen ist, und es dir sagen; die Installation übernimmst du selbst. GitHub sieht deine IP-Adresse; sonst wird nichts über dich gesendet. Du kannst das in Einstellungen, Erweitert ändern."),
    "threesea-la-update-newest":
        ("Newest version",
         "Neueste Version"),
    "threesea-la-update-installed":
        ("Installed",
         "Installiert"),
    "threesea-la-update-checked":
        ("Last checked",
         "Zuletzt geprüft"),
    "threesea-la-n-contacts":
        ("%n contact(s)",
         ("%n Kontakt",
          "%n Kontakte")),
    "threesea-la-n-groups":
        ("%n group(s)",
         ("%n Gruppe",
          "%n Gruppen")),
    "threesea-la-n-lists":
        ("%n distribution list(s)",
         ("%n Verteilerliste",
          "%n Verteilerlisten")),
    "threesea-me-safe-retry-import":
        ("Import the rest now",
         "Rest jetzt importieren"),
    "threesea-se-safe-restore":
        ("Restore",
         "Wiederherstellung"),
    "threesea-la-safe-to-import":
        ("Waiting to import",
         "Wartet auf Import"),
    "threesea-la-safe-import-attempt":
        ("Import attempt",
         "Importversuch"),
    "threesea-la-safe-skipped-deleted":
        ("Deleted groups",
         "Gelöschte Gruppen"),
    "threesea-la-safe-skipped-creator":
        ("Creator not available",
         "Ersteller nicht verfügbar"),
    "threesea-la-safe-skipped-invalid":
        ("Unreadable entries",
         "Nicht lesbare Einträge"),
    "threesea-la-safe-revoked":
        ("No longer registered",
         "Nicht mehr registriert"),
    "threesea-la-error-backup-version":
        ("This backup's format is not supported.",
         "Das Format dieses Backups wird nicht unterstützt."),
    "threesea-la-error-backup-invalid":
        ("The file could not be read as a Threema backup.",
         "Die Datei konnte nicht als Threema-Backup gelesen werden."),
    "threesea-la-android-backup-file":
        ("Backup file",
         "Backup-Datei"),
    "threesea-la-select":
        ("Select",
         "Auswählen"),
    "threesea-la-android-backup-password":
        ("Backup password",
         "Backup-Passwort"),
    "threesea-bu-android-backup-check":
        ("Check backup",
         "Backup prüfen"),
    "threesea-se-android-backup-contents":
        ("Contents",
         "Inhalt"),
    "threesea-la-android-backup-identity":
        ("Threema ID",
         "Threema-ID"),
    "threesea-la-android-backup-id-match":
        ("This device's Threema ID",
         "Die Threema-ID dieses Geräts"),
    "threesea-la-android-backup-id-other":
        ("Another Threema ID",
         "Eine andere Threema-ID"),
    "threesea-la-android-backup-id-key":
        ("Same ID, different key",
         "Gleiche ID, anderer Schlüssel"),
    "threesea-la-android-backup-id-unreadable":
        ("Not readable with this password",
         "Mit diesem Passwort nicht lesbar"),
    "threesea-la-android-backup-id-missing":
        ("Not in the backup",
         "Nicht im Backup"),
    "threesea-la-android-backup-chats":
        ("Chats",
         "Chats"),
    "threesea-la-android-backup-group-chats":
        ("Group chats",
         "Gruppenchats"),
    "threesea-la-android-backup-lists":
        ("Distribution lists",
         "Verteilerlisten"),
    "threesea-la-android-backup-media":
        ("Media files",
         "Mediendateien"),
    "threesea-la-android-backup-pictures":
        ("Pictures",
         "Bilder"),
    "threesea-la-error-backup-identity":
        ("This backup belongs to a different Threema ID or key.",
         "Dieses Backup gehört zu einer anderen Threema-ID oder einem anderen Schlüssel."),
    "threesea-la-android-backup-imported-contacts":
        ("Contacts",
         "Kontakte"),
    "threesea-la-android-backup-imported-groups":
        ("Groups",
         "Gruppen"),
    "threesea-la-android-backup-existing":
        ("Already on this device",
         "Schon auf diesem Gerät"),
    "threesea-la-android-import-chats":
        ("Chats %1 of %2",
         "Chats %1 von %2"),
    "threesea-bu-android-backup-stop":
        ("Stop",
         "Anhalten"),
    "threesea-se-android-backup-history":
        ("Message history",
         "Nachrichtenverlauf"),
    "threesea-la-android-import-messages":
        ("Messages",
         "Nachrichten"),
    "threesea-la-android-import-reactions":
        ("Reactions",
         "Reaktionen"),
    "threesea-se-android-backup-create":
        ("Create backups",
         "Backups erstellen"),
    "threesea-la-android-export-folder":
        ("Backup folder",
         "Backup-Ordner"),
    "threesea-la-android-export-password-saved":
        ("Password saved",
         "Passwort gespeichert"),
    "threesea-la-android-export-auto":
        ("Automatic backup",
         "Automatisches Backup"),
    "threesea-la-android-export-off":
        ("Off",
         "Aus"),
    "threesea-la-android-export-daily":
        ("Daily",
         "Täglich"),
    "threesea-la-android-export-weekly":
        ("Weekly",
         "Wöchentlich"),
    "threesea-la-android-export-keep":
        ("Keep",
         "Behalten"),
    "threesea-la-n-backups":
        ("%n backup(s)",
         ("%n Backup",
          "%n Backups")),
    "threesea-bu-android-export-now":
        ("Create backup now",
         "Backup jetzt erstellen"),
    "threesea-la-android-export-written":
        ("Backup written: %1",
         "Backup geschrieben: %1"),
    "threesea-la-android-import-polls":
        ("Polls",
         "Umfragen"),
    "threesea-he-import":
        ("Import",
         "Importieren"),
    "threesea-la-android-backup-import-duration":
        ("A large backup can take a while. You can stop the import; importing again continues where it stopped.",
         "Ein großes Backup kann eine Weile dauern. Du kannst den Import anhalten; ein erneuter Import macht dort weiter, wo er angehalten hat."),
    "threesea-he-importing":
        ("Importing",
         "Import läuft"),
    "threesea-la-android-import-stopping":
        ("Stopping…",
         "Wird angehalten …"),
    "threesea-la-android-import-step-metadata":
        ("Contacts, groups and pictures",
         "Kontakte, Gruppen und Bilder"),
    "threesea-la-android-import-step-reading":
        ("Reading the backup",
         "Backup wird gelesen"),
    "threesea-la-android-import-step-history":
        ("Messages and media",
         "Nachrichten und Medien"),
    "threesea-la-android-import-chat-media":
        ("Chat %1 of %2, media %3 of %4",
         "Chat %1 von %2, Medien %3 von %4"),
    "threesea-la-android-import-keep-open":
        ("Keep the app open until the import is done. What is imported stays; importing again continues.",
         "Lass die App offen, bis der Import fertig ist. Was importiert ist, bleibt; ein erneuter Import macht weiter."),
    "threesea-he-data-backup":
        ("Data backup",
         "Daten-Backup"),
    "threesea-se-data-backup-import":
        ("Import a backup",
         "Backup importieren"),
    "threesea-la-data-backup-import-description":
        ("Checking a backup only reads the file and shows what it holds. The password never leaves this device.",
         "Beim Prüfen wird die Datei nur gelesen und gezeigt, was sie enthält. Das Passwort verlässt dieses Gerät nicht."),
    "threesea-la-data-backup-create-description":
        ("Writes a backup into the chosen folder, now or on a schedule. The password is kept in the encrypted storage so automatic backups can run.",
         "Schreibt ein Backup in den gewählten Ordner, jetzt oder regelmäßig. Das Passwort bleibt im verschlüsselten Speicher, damit automatische Backups laufen können."),
    "threesea-he-import-failed":
        ("Import failed",
         "Import fehlgeschlagen"),
    "threesea-he-import-stopped":
        ("Import stopped",
         "Import angehalten"),
    "threesea-he-import-finished":
        ("Import finished",
         "Import abgeschlossen"),
    "threesea-la-data-backup-resume-hint":
        ("What is imported stays. Import the backup again to continue where it stopped.",
         "Was importiert ist, bleibt. Importiere das Backup erneut, um dort weiterzumachen, wo es angehalten hat."),
    "threesea-la-data-backup-imported":
        ("The backup is imported. Everything already on this device stayed as it was.",
         "Das Backup ist importiert. Alles, was schon auf diesem Gerät war, ist unverändert geblieben."),
    "threesea-se-import-contacts-groups":
        ("Contacts and groups",
         "Kontakte und Gruppen"),
    "threesea-la-android-import-media-failed":
        ("Media not imported",
         "Nicht importierte Medien"),
    "threesea-la-forward-secrecy":
        ("Perfect Forward Secrecy",
         "Perfect Forward Secrecy"),
    "threesea-la-forward-secrecy-description":
        ("Protects chats with keys that change continuously. Messages are end-to-end encrypted either way; contacts see a security notice when the protection starts, resets or is turned off.",
         "Schützt Chats mit Schlüsseln, die sich laufend ändern. Nachrichten sind so oder so Ende-zu-Ende-verschlüsselt; Kontakte sehen einen Sicherheitshinweis, wenn der Schutz beginnt, zurückgesetzt oder ausgeschaltet wird."),
    "threesea-la-view-log":
        ("View log",
         "Protokoll ansehen"),
    "threesea-me-log-clear":
        ("Clear log",
         "Protokoll leeren"),
    "threesea-la-clearing-log":
        ("Clearing log",
         "Protokoll wird geleert"),
    "threesea-me-log-save":
        ("Save support package",
         "Supportpaket speichern"),
    "threesea-me-log-refresh":
        ("Refresh",
         "Aktualisieren"),
    "threesea-he-log":
        ("Log",
         "Protokoll"),
    "threesea-la-log-empty":
        ("No entries",
         "Keine Einträge"),
    "threesea-la-log-empty-hint":
        ("What Threesea logs appears here. Settings → Advanced sets how much that is.",
         "Hier erscheint, was Threesea protokolliert. Wie viel das ist, stellst du unter Einstellungen → Erweitert ein."),
    "threesea-se-encryption":
        ("Encryption",
         "Verschlüsselung"),
    "threesea-la-device-cookie-title":
        ("Threema ID used on another device",
         "Threema-ID auf einem anderen Gerät verwendet"),
    "threesea-la-device-cookie-body":
        ("Threema's server noticed a connection from another device with your Threema ID. If you recently used your ID on another phone, you can ignore this. An ID works on one phone at a time: new messages go only to the phone that connected last. If you didn't, your private key may be compromised: protect this phone and its data, and create a new ID.",
         "Der Threema-Server hat eine Verbindung von einem anderen Gerät mit deiner Threema-ID bemerkt. Wenn du deine ID kürzlich auf einem anderen Telefon verwendet hast, kannst du das ignorieren. Eine ID funktioniert immer nur auf einem Telefon: Neue Nachrichten gehen nur an das Telefon, das sich zuletzt verbunden hat. Warst du es nicht, ist dein privater Schlüssel vielleicht kompromittiert: Schütze dieses Telefon und seine Daten und erstelle eine neue ID."),
    "threesea-me-delete-for-me":
        ("Delete for me",
         "Für mich löschen"),
    "threesea-me-delete-for-everyone":
        ("Delete for everyone",
         "Für alle löschen"),
    "threesea-la-playback-speed":
        ("%1×",
         "%1×"),
    "threesea-la-ear-to-play":
        ("Play voice messages at the ear",
         "Sprachnachrichten am Ohr abspielen"),
    "threesea-la-ear-to-play-description":
        ("Hold the phone to your ear and the voice message plays on the earpiece, like a call. Take it away to pause.",
         "Halte das Telefon ans Ohr, dann spielt die Sprachnachricht über den Hörer, wie bei einem Anruf. Nimmst du es weg, hält sie an."),
    "threesea-se-notifications-inapp":
        ("In the app",
         "In der App"),
    "threesea-la-inapp-sound-description":
        ("A soft sound for a message arriving in the chat you are reading, and between two voice messages playing one after the other.",
         "Ein leiser Ton, wenn eine Nachricht im Chat ankommt, den du gerade liest, und zwischen zwei Sprachnachrichten, die nacheinander laufen."),
    "threesea-la-inapp-vibrate-description":
        ("A short vibration for a message arriving in the chat you are reading.",
         "Eine kurze Vibration, wenn eine Nachricht im Chat ankommt, den du gerade liest."),
    "threesea-me-mark-unread":
        ("Mark as unread",
         "Als ungelesen markieren"),
    "threesea-me-wallpaper-clear":
        ("Use the Ambience's background",
         "Hintergrund des Ambiente verwenden"),
    "threesea-me-font-size-small":
        ("Small",
         "Klein"),
    "threesea-me-font-size-normal":
        ("Normal",
         "Normal"),
    "threesea-me-font-size-large":
        ("Large",
         "Groß"),
    "threesea-la-chat-font-size":
        ("Message text",
         "Nachrichtentext"),
    "threesea-la-wallpaper":
        ("Chat background",
         "Chat-Hintergrund"),
    "threesea-la-wallpaper-ambience":
        ("The Ambience's background",
         "Hintergrund des Ambiente"),
    "threesea-la-receive-pictures":
        ("Show profile pictures",
         "Profilbilder zeigen"),
    "threesea-la-receive-pictures-description":
        ("Contacts who share their profile picture are shown with it. Pictures they send keep arriving while this is off, and appear when it is turned on again.",
         "Kontakte, die ihr Profilbild teilen, werden damit angezeigt. Bilder, die sie senden, kommen auch an, während das aus ist, und erscheinen, wenn es wieder eingeschaltet wird."),
    "threesea-la-group-calls":
        ("Group calls",
         "Gruppenanrufe"),
    "threesea-la-group-calls-description":
        ("You can start group calls and join the ones running in your groups.",
         "Du kannst Gruppenanrufe starten und an denen teilnehmen, die in deinen Gruppen laufen."),
    "threesea-he-rename-device":
        ("Rename device",
         "Gerät umbenennen"),
    "threesea-la-device-name":
        ("Device name",
         "Gerätename"),
    "threesea-la-device-name-own-description":
        ("The other devices show this name for this phone.",
         "Die anderen Geräte zeigen diesen Namen für dieses Telefon."),
    "threesea-la-device-name-alias-description":
        ("Only this phone shows this name; the device keeps its own name everywhere else.",
         "Nur dieses Telefon zeigt diesen Namen; überall sonst behält das Gerät seinen eigenen."),
    "threesea-me-rename-device":
        ("Rename",
         "Umbenennen"),
    "threesea-me-keep-forever":
        ("For good",
         "Für immer"),
    "threesea-me-keep-1-year":
        ("1 year",
         "1 Jahr"),
    "threesea-me-keep-6-months":
        ("6 months",
         "6 Monate"),
    "threesea-me-keep-3-months":
        ("3 months",
         "3 Monate"),
    "threesea-me-keep-1-month":
        ("1 month",
         "1 Monat"),
    "threesea-me-keep-1-week":
        ("1 week",
         "1 Woche"),
    "threesea-me-video-small":
        ("Small (480 px)",
         "Klein (480 px)"),
    "threesea-me-video-medium":
        ("Medium (848 px)",
         "Mittel (848 px)"),
    "threesea-la-video-size":
        ("Video size",
         "Videogröße"),
    "threesea-la-video-size-description":
        ("Smaller videos are quicker to send and use less data. The sound is kept as it is.",
         "Kleinere Videos sind schneller verschickt und brauchen weniger Daten. Der Ton bleibt, wie er ist."),
    "threesea-la-keep-messages":
        ("Keep messages",
         "Nachrichten behalten"),
    "threesea-la-keep-messages-description":
        ("Messages older than the chosen time are deleted by themselves, with their media. Starred messages, open polls, messages still on their way and groups with only you in them stay.",
         "Nachrichten, die älter als die gewählte Zeit sind, werden samt ihren Medien von selbst gelöscht. Markierte Nachrichten, offene Umfragen, Nachrichten, die noch unterwegs sind, und Gruppen, in denen nur du bist, bleiben."),
    "threesea-la-exporting-chat":
        ("Exporting chat",
         "Chat wird exportiert"),
    "threesea-me-export-chat":
        ("Export chat",
         "Chat exportieren"),
    "threesea-he-export":
        ("Export",
         "Exportieren"),
    "threesea-he-export-chat":
        ("Export chat",
         "Chat exportieren"),
    "threesea-la-export-chat-description":
        ("The messages of %1 are written as a text file into a ZIP archive in your Downloads folder.",
         "Die Nachrichten von %1 werden als Textdatei in ein ZIP-Archiv in deinem Download-Ordner geschrieben."),
    "threesea-la-export-chat-media":
        ("Include pictures and files",
         "Bilder und Dateien einschließen"),
    "threesea-la-export-chat-media-description":
        ("Everything that has been downloaded to this phone goes into the archive.",
         "Alles, was auf dieses Telefon heruntergeladen wurde, kommt ins Archiv."),
    "threesea-me-limit-any":
        ("Any size",
         "Jede Größe"),
    "threesea-me-limit-1":
        ("Up to 1 MB",
         "Bis 1 MB"),
    "threesea-me-limit-5":
        ("Up to 5 MB",
         "Bis 5 MB"),
    "threesea-me-limit-20":
        ("Up to 20 MB",
         "Bis 20 MB"),
    "threesea-me-limit-100":
        ("Up to 100 MB",
         "Bis 100 MB"),
    "threesea-la-download-limit":
        ("File size",
         "Dateigröße"),
    "threesea-la-download-limit-description":
        ("Larger files wait in the chat until you tap them. Previews always come.",
         "Größere Dateien warten im Chat, bis du sie antippst. Vorschaubilder kommen immer."),
    "threesea-me-photo-turn-right":
        ("Turn right",
         "Nach rechts drehen"),
    "threesea-me-photo-turn-left":
        ("Turn left",
         "Nach links drehen"),
    "threesea-me-photo-whole":
        ("Whole picture",
         "Ganzes Bild"),
    "threesea-he-photo-use":
        ("Use",
         "Übernehmen"),
    "threesea-he-edit-photo":
        ("Edit photo",
         "Foto bearbeiten"),
    "threesea-la-edit-photo-failed":
        ("The photo could not be edited.",
         "Das Foto konnte nicht bearbeitet werden."),
    "threesea-la-license-notification":
        ("Threesea is offline",
         "Threesea ist offline"),
    "threesea-la-license-notification-body":
        ("Threema no longer accepts this license key. Open Threesea to check it.",
         "Threema akzeptiert diesen Lizenzschlüssel nicht mehr. Öffne Threesea, um ihn zu prüfen."),
    "threesea-he-license-key":
        ("License key",
         "Lizenzschlüssel"),
    "threesea-la-license-refused":
        ("Threesea stays offline until a license key is accepted. Your chats stay on this device.",
         "Threesea bleibt offline, bis ein Lizenzschlüssel akzeptiert wird. Deine Chats bleiben auf diesem Gerät."),
    "threesea-la-spam-reported":
        ("Reported to Threema as spam.",
         "Als Spam an Threema gemeldet."),
    "threesea-la-spam-unknown-group":
        ("This group is from an unknown contact",
         "Diese Gruppe stammt von einem unbekannten Kontakt"),
    "threesea-la-spam-unknown-contact":
        ("This chat is from an unknown contact",
         "Dieser Chat stammt von einem unbekannten Kontakt"),
    "threesea-la-spam-tap-to-report":
        ("Tap to report spam",
         "Tippe, um Spam zu melden"),
    "threesea-he-spam-report":
        ("Report",
         "Melden"),
    "threesea-he-spam-report-name":
        ("Report %1",
         "%1 melden"),
    "threesea-la-spam-explain-group":
        ("This sends your Threema ID to Threema, with the group's ID and number of members and the ID and nickname of its creator. No messages and not the group's name.",
         "Das sendet deine Threema-ID an Threema, dazu die ID und Mitgliederzahl der Gruppe sowie die ID und den Nickname ihres Erstellers. Keine Nachrichten und nicht den Gruppennamen."),
    "threesea-la-spam-explain-contact":
        ("This sends your Threema ID to Threema, with the ID and nickname of this contact. No messages.",
         "Das sendet deine Threema-ID an Threema, dazu die ID und den Nickname dieses Kontakts. Keine Nachrichten."),
    "threesea-la-spam-block-group":
        ("Also block the creator, leave the group and delete the chat",
         "Auch den Ersteller blockieren, die Gruppe verlassen und den Chat löschen"),
    "threesea-la-spam-block-contact":
        ("Also block this contact and delete the chat",
         "Auch diesen Kontakt blockieren und den Chat löschen"),
    "threesea-la-spam-report-failed":
        ("The report could not be sent.",
         "Die Meldung konnte nicht gesendet werden."),
    "threesea-la-address-book-none-new":
        ("%n address book entries checked, no new contacts.",
         ("%n Adressbucheintrag geprüft, keine neuen Kontakte.",
          "%n Adressbucheinträge geprüft, keine neuen Kontakte.")),
    "threesea-la-address-book-empty":
        ("No phone numbers or e-mail addresses found in the address book.",
         "Im Adressbuch wurden keine Telefonnummern oder E-Mail-Adressen gefunden."),
    "threesea-la-link-mobile-target":
        ("The code goes to +%1.",
         "Der Code geht an +%1."),
    "threesea-la-email-already-linked":
        ("This address is already linked to your ID, so no e-mail is needed.",
         "Diese Adresse ist bereits mit deiner ID verknüpft, eine E-Mail ist nicht nötig."),
    "threesea-la-phone-already-linked":
        ("This number is already linked to your ID, so no SMS is needed.",
         "Diese Nummer ist bereits mit deiner ID verknüpft, eine SMS ist nicht nötig."),
    "threesea-la-country-ae":
        ("United Arab Emirates",
         "Vereinigte Arabische Emirate"),
    "threesea-la-country-at":
        ("Austria",
         "Österreich"),
    "threesea-la-country-au":
        ("Australia",
         "Australien"),
    "threesea-la-country-be":
        ("Belgium",
         "Belgien"),
    "threesea-la-country-bg":
        ("Bulgaria",
         "Bulgarien"),
    "threesea-la-country-br":
        ("Brazil",
         "Brasilien"),
    "threesea-la-country-ca":
        ("Canada",
         "Kanada"),
    "threesea-la-country-ch":
        ("Switzerland",
         "Schweiz"),
    "threesea-la-country-cn":
        ("China",
         "China"),
    "threesea-la-country-cy":
        ("Cyprus",
         "Zypern"),
    "threesea-la-country-cz":
        ("Czechia",
         "Tschechien"),
    "threesea-la-country-de":
        ("Germany",
         "Deutschland"),
    "threesea-la-country-dk":
        ("Denmark",
         "Dänemark"),
    "threesea-la-country-ee":
        ("Estonia",
         "Estland"),
    "threesea-la-country-es":
        ("Spain",
         "Spanien"),
    "threesea-la-country-fi":
        ("Finland",
         "Finnland"),
    "threesea-la-country-fr":
        ("France",
         "Frankreich"),
    "threesea-la-country-gb":
        ("United Kingdom",
         "Vereinigtes Königreich"),
    "threesea-la-country-gr":
        ("Greece",
         "Griechenland"),
    "threesea-la-country-hr":
        ("Croatia",
         "Kroatien"),
    "threesea-la-country-hu":
        ("Hungary",
         "Ungarn"),
    "threesea-la-country-ie":
        ("Ireland",
         "Irland"),
    "threesea-la-country-il":
        ("Israel",
         "Israel"),
    "threesea-la-country-in":
        ("India",
         "Indien"),
    "threesea-la-country-is":
        ("Iceland",
         "Island"),
    "threesea-la-country-it":
        ("Italy",
         "Italien"),
    "threesea-la-country-jp":
        ("Japan",
         "Japan"),
    "threesea-la-country-kr":
        ("South Korea",
         "Südkorea"),
    "threesea-la-country-li":
        ("Liechtenstein",
         "Liechtenstein"),
    "threesea-la-country-lu":
        ("Luxembourg",
         "Luxemburg"),
    "threesea-la-country-lv":
        ("Latvia",
         "Lettland"),
    "threesea-la-country-mt":
        ("Malta",
         "Malta"),
    "threesea-la-country-mx":
        ("Mexico",
         "Mexiko"),
    "threesea-la-country-nl":
        ("Netherlands",
         "Niederlande"),
    "threesea-la-country-no":
        ("Norway",
         "Norwegen"),
    "threesea-la-country-nz":
        ("New Zealand",
         "Neuseeland"),
    "threesea-la-country-pl":
        ("Poland",
         "Polen"),
    "threesea-la-country-pt":
        ("Portugal",
         "Portugal"),
    "threesea-la-country-ro":
        ("Romania",
         "Rumänien"),
    "threesea-la-country-rs":
        ("Serbia",
         "Serbien"),
    "threesea-la-country-ru":
        ("Russia",
         "Russland"),
    "threesea-la-country-se":
        ("Sweden",
         "Schweden"),
    "threesea-la-country-sg":
        ("Singapore",
         "Singapur"),
    "threesea-la-country-si":
        ("Slovenia",
         "Slowenien"),
    "threesea-la-country-sk":
        ("Slovakia",
         "Slowakei"),
    "threesea-la-country-tr":
        ("Turkey",
         "Türkei"),
    "threesea-la-country-ua":
        ("Ukraine",
         "Ukraine"),
    "threesea-la-country-us":
        ("United States",
         "Vereinigte Staaten"),
    "threesea-la-country-za":
        ("South Africa",
         "Südafrika"),
    "threesea-la-address-book-country-automatic":
        ("Automatic",
         "Automatisch"),
    "threesea-la-address-book-country-automatic-code":
        ("Automatic (+%1)",
         "Automatisch (+%1)"),
    "threesea-la-address-book-country":
        ("Numbers without a country code",
         "Nummern ohne Ländervorwahl"),
    "threesea-la-address-book-country-description":
        ("Address book numbers written without a country code count as from this country. Automatic goes by your linked phone number, else by the device's region. Numbers from other countries are found when written with + and their country code.",
         "Nummern im Adressbuch, die ohne Ländervorwahl geschrieben sind, gelten als aus diesem Land. Automatisch richtet sich nach deiner verknüpften Telefonnummer, sonst nach der Region des Geräts. Nummern aus anderen Ländern werden gefunden, wenn sie mit + und ihrer Ländervorwahl geschrieben sind."),
    "threesea-la-no-emoji-found":
        ("No emoji found",
         "Keine Emojis gefunden"),
    "threesea-la-upgrading-hint":
        ("Updating the stored chats. This can take a moment; keep Threesea open.",
         "Die gespeicherten Chats werden aktualisiert. Das kann einen Moment dauern; lass Threesea offen."),
    "threesea-la-error-text-too-long":
        ("The text is too long. Shorten it and try again.",
         "Der Text ist zu lang. Kürze ihn und versuche es erneut."),
    "threesea-la-error-poll-results-too-long":
        ("The results of this poll are too long to send. The poll stays open.",
         "Die Ergebnisse dieser Umfrage sind zu lang zum Senden. Die Umfrage bleibt offen."),
    "threesea-la-error-backup-no-identity":
        ("This backup holds no Threema ID. Restore the ID from Threema Safe or an ID backup, then import this backup in Settings, Backup, Data backup.",
         "Dieses Backup enthält keine Threema-ID. Stelle die ID aus Threema Safe oder einem ID-Backup wieder her und importiere dieses Backup dann unter Einstellungen, Backup, Daten-Backup."),
    "threesea-la-error-backup-write":
        ("The backup could not be written. Check the backup folder.",
         "Das Backup konnte nicht geschrieben werden. Prüfe den Backup-Ordner."),
    "threesea-la-error-backup-no-room":
        ("The backup folder's storage has too little free space for a backup. Free some space or choose another folder.",
         "Auf dem Speicher des Backup-Ordners ist zu wenig Platz für ein Backup. Gib Speicher frei oder wähle einen anderen Ordner."),
    "threesea-la-error-backup-running":
        ("A backup is being written right now.",
         "Gerade wird ein Backup geschrieben."),
    "threesea-la-error-backup-no-folder":
        ("Choose a backup folder first.",
         "Wähle zuerst einen Backup-Ordner."),
    "threesea-la-error-backup-no-password":
        ("Enter a backup password first.",
         "Gib zuerst ein Backup-Passwort ein."),
    "threesea-la-error-safe-damaged":
        ("This Threema Safe backup is damaged and can't be read.",
         "Dieses Threema-Safe-Backup ist beschädigt und lässt sich nicht lesen."),
    "threesea-la-error-contact-revoked":
        ("This Threema ID was revoked.",
         "Diese Threema-ID wurde widerrufen."),
    "threesea-la-error-contact-unknown":
        ("Block unknown senders is on, and this person is not one of your contacts.",
         "Unbekannte blockieren ist eingeschaltet, und diese Person ist keiner deiner Kontakte."),
    "threesea-la-error-qr-expired":
        ("This code has expired. Scan it directly from the other person's app.",
         "Dieser Code ist abgelaufen. Scanne ihn direkt in der App der anderen Person."),
    "threesea-la-error-video-metadata":
        ("A video was not sent, because its metadata, which can hold where and when it was made, could not be removed. It can still be sent as a file.",
         "Ein Video wurde nicht gesendet, weil seine Metadaten, die zeigen können, wo und wann es entstanden ist, nicht entfernt werden konnten. Als Datei lässt es sich trotzdem senden."),
    "threesea-la-error-file-picture-metadata":
        ("A picture was not sent, because its metadata, which can hold where and when it was made, could not be removed. It can still be sent as a file.",
         "Ein Bild wurde nicht gesendet, weil seine Metadaten, die zeigen können, wo und wann es entstanden ist, nicht entfernt werden konnten. Als Datei lässt es sich trotzdem senden."),
    "threesea-la-error-picture-metadata":
        ("A picture was not sent, because its metadata, which can hold where and when it was made, could not be removed. Save it, and it can be sent as a file.",
         "Ein Bild wurde nicht gesendet, weil seine Metadaten, die zeigen können, wo und wann es entstanden ist, nicht entfernt werden konnten. Speichere es, dann lässt es sich als Datei senden."),
    "threesea-la-error-not-web-code":
        ("This is not a Threema Web code. Scan the code Threema Web shows in the browser.",
         "Das ist kein Threema-Web-Code. Scanne den Code, den Threema Web im Browser zeigt."),
    "threesea-la-error-web-disabled":
        ("Threema Web is switched off. Turn it on in the settings under Threema Web.",
         "Threema Web ist ausgeschaltet. Schalte es in den Einstellungen unter Threema Web ein."),
    "threesea-la-error-web-already-paired":
        ("This browser is already in the list under Threema Web. Remove it there to connect it with a new code.",
         "Dieser Browser ist schon in der Liste unter Threema Web. Entferne ihn dort, um ihn mit einem neuen Code zu verbinden."),
    "threesea-la-error-link-removed":
        ("Linking didn't finish, so the devices in the device group were removed again. The computer had already received your ID. Close Threema for Desktop, open it again and scan its new code.",
         "Das Verknüpfen wurde nicht fertig, deshalb wurden die Geräte der Gerätegruppe wieder entfernt. Der Computer hatte deine ID schon bekommen. Schließe Threema für Desktop, öffne es erneut und scanne seinen neuen Code."),
    "threesea-la-error-link-uncertain":
        ("Linking didn't finish, but the computer received your ID and may be linked anyway. Linked devices shows what is linked once you are online, and you can remove it there.",
         "Das Verknüpfen wurde nicht fertig, aber der Computer hat deine ID bekommen und ist vielleicht trotzdem verknüpft. Unter Verknüpfte Geräte siehst du, was verknüpft ist, sobald du online bist, und kannst es dort entfernen."),
    "threesea-la-error-group-call-full":
        ("This group call is full.",
         "Dieser Gruppenanruf ist voll."),
    "threesea-la-error-group-call-not-running":
        ("This group call has ended.",
         "Dieser Gruppenanruf ist beendet."),
    "threesea-la-error-group-call-version":
        ("This group call needs a different version of the app.",
         "Dieser Gruppenanruf braucht eine andere Version der App."),
    "threesea-la-error-group-call-connection":
        ("The connection to the group call failed.",
         "Die Verbindung zum Gruppenanruf ist fehlgeschlagen."),
    "threesea-la-error-group-call-audio":
        ("The microphone or the speaker stopped working, so the group call ended.",
         "Das Mikrofon oder der Lautsprecher hat aufgehört zu funktionieren, deshalb wurde der Gruppenanruf beendet."),
    "threesea-la-error-contact-not-found":
        ("Add this ID to your contacts to write to it.",
         "Füge diese ID zu deinen Kontakten hinzu, um ihr zu schreiben."),
    "threesea-la-error-group-not-found":
        ("This group is no longer on this device.",
         "Diese Gruppe ist nicht mehr auf diesem Gerät."),
    "threesea-la-error-no-identity":
        ("Create or restore your Threema ID first.",
         "Erstelle zuerst deine Threema-ID oder stelle sie wieder her."),
    "threesea-la-error-server-trouble":
        ("Threema's server didn't accept this. Try again later.",
         "Der Threema-Server hat das nicht angenommen. Versuche es später erneut."),
    "threesea-la-error-rate-limited":
        ("Threema's servers are busy. Try again in a few minutes.",
         "Die Threema-Server sind ausgelastet. Versuche es in ein paar Minuten erneut."),
    "threesea-la-error-server-unexplained":
        ("Threema's servers turned this down and gave no reason. Try again later.",
         "Die Threema-Server haben das ohne Begründung abgelehnt. Versuche es später erneut."),
    "threesea-la-error-report-refused":
        ("Threema didn't accept the report. Try again later.",
         "Threema hat die Meldung nicht angenommen. Versuche es später erneut."),
    "threesea-la-error-id-search-input":
        ("Enter a phone number with its country code, or an e-mail address.",
         "Gib eine Telefonnummer mit Ländervorwahl oder eine E-Mail-Adresse ein."),
    "threesea-la-error-identity-exists":
        ("This device already has a Threema ID.",
         "Dieses Gerät hat schon eine Threema-ID."),
    "threesea-la-error-disk-full":
        ("The phone's storage is full. Free some space.",
         "Der Speicher des Telefons ist voll. Gib etwas Speicher frei."),
    "threesea-la-error-upgrade-no-room":
        ("Updating the stored chats needs more free space. Free some space; the chats stay as they were until then.",
         "Das Aktualisieren der gespeicherten Chats braucht mehr freien Speicher. Gib Speicher frei; bis dahin bleiben die Chats, wie sie waren."),
    "threesea-la-error-store-too-new":
        ("This version of Threesea is older than your stored data. Install the newest version.",
         "Diese Version von Threesea ist älter als deine gespeicherten Daten. Installiere die neueste Version."),
    "threesea-la-error-store-key":
        ("The stored data could not be opened: the key does not fit, or the file is damaged. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Die gespeicherten Daten konnten nicht geöffnet werden: Der Schlüssel passt nicht, oder die Datei ist beschädigt. Threesea lässt die Daten, wie sie sind. Es lohnt sich, Threesea zu schließen und wieder zu öffnen, dann das Telefon neu zu starten."),
    "threesea-la-error-store-damaged":
        ("Part of the stored data could not be read. It may be damaged. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Ein Teil der gespeicherten Daten konnte nicht gelesen werden. Er ist vielleicht beschädigt. Threesea lässt die Daten, wie sie sind. Es lohnt sich, Threesea zu schließen und wieder zu öffnen, dann das Telefon neu zu starten."),
    "threesea-la-error-store-unreadable":
        ("The stored data could not be read. Check that the phone has free storage.",
         "Die gespeicherten Daten konnten nicht gelesen werden. Prüfe, ob das Telefon freien Speicher hat."),
    "threesea-la-error-store":
        ("The stored data could not be opened.",
         "Die gespeicherten Daten konnten nicht geöffnet werden."),
    "threesea-la-error-key-missing":
        ("The key to the stored data was not found in the phone's key storage, so the data could not be opened. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Der Schlüssel zu den gespeicherten Daten wurde im Schlüsselspeicher des Telefons nicht gefunden, deshalb konnten die Daten nicht geöffnet werden. Threesea lässt die Daten, wie sie sind. Es lohnt sich, Threesea zu schließen und wieder zu öffnen, dann das Telefon neu zu starten."),
    "threesea-la-error-secrets":
        ("Threesea could not get the key to its data from the phone's key storage. Allow the access if the phone asks for it.",
         "Threesea hat den Schlüssel zu seinen Daten nicht aus dem Schlüsselspeicher des Telefons bekommen. Erlaube den Zugriff, wenn das Telefon danach fragt."),
    "threesea-la-error-internal-plain":
        ("Something went wrong.",
         "Etwas ist schiefgelaufen."),
    "threesea-la-gif":
        ("GIF",
         "GIF"),
    "threesea-la-lock-private-passphrase":
        ("Passphrase for private chats",
         "Passphrase für private Chats"),
    "threesea-la-lock-private-pin":
        ("PIN for private chats",
         "PIN für private Chats"),
    "threesea-la-status-group-no-longer-member":
        ("You are no longer a member of this group.",
         "Du bist kein Mitglied dieser Gruppe mehr."),
    "threesea-la-status-group-call-started-unnamed":
        ("A group call started.",
         "Ein Gruppenanruf hat begonnen."),
    "threesea-la-status-group-call-started":
        ("%1 started a group call.",
         "%1 hat einen Gruppenanruf gestartet."),
    "threesea-la-status-group-call-you-started":
        ("You started a group call.",
         "Du hast einen Gruppenanruf gestartet."),
    "threesea-la-status-group-call-ended":
        ("The group call ended.",
         "Der Gruppenanruf wurde beendet."),
    "threesea-la-status-fs-session-reset":
        ("Perfect Forward Secrecy was reset. This can happen when the contact changes devices or reinstalls the app.",
         "Perfect Forward Secrecy wurde zurückgesetzt. Das kann passieren, wenn der Kontakt das Gerät wechselt oder die App neu installiert."),
    "threesea-la-status-fs-message-rejected":
        ("Perfect Forward Secrecy was reset. Affected messages are sent again.",
         "Perfect Forward Secrecy wurde zurückgesetzt. Betroffene Nachrichten werden erneut gesendet."),
    "threesea-la-status-fs-message-without":
        ("A message without Perfect Forward Secrecy was received. Check with the contact that they turned it off on purpose.",
         "Eine Nachricht ohne Perfect Forward Secrecy wurde empfangen. Kläre mit dem Kontakt, ob er sie absichtlich ausgeschaltet hat."),
    "threesea-la-status-fs-unavailable":
        ("Perfect Forward Secrecy is off in this chat because this contact's app is not using it at the moment.",
         "Perfect Forward Secrecy ist in diesem Chat aus, weil die App dieses Kontakts sie gerade nicht verwendet."),
    "threesea-la-status-fs-session-ended":
        ("The Perfect Forward Secrecy session with this contact ended.",
         "Die Perfect-Forward-Secrecy-Sitzung mit diesem Kontakt wurde beendet."),
    "threesea-me-copy-id":
        ("Copy ID",
         "ID kopieren"),
    "threesea-la-copied":
        ("Copied",
         "Kopiert"),
    "threesea-me-share-id":
        ("Share ID",
         "ID teilen"),
    "threesea-he-share-id":
        ("Share ID",
         "ID teilen"),
    "threesea-me-send-again":
        ("Send again",
         "Erneut senden"),
    "threesea-me-stop-download":
        ("Stop download",
         "Download abbrechen"),
    "threesea-me-stop-upload":
        ("Stop sending",
         "Senden abbrechen"),
    "threesea-me-share":
        ("Share",
         "Teilen"),
    "threesea-me-draw-on-picture":
        ("Draw on it",
         "Darauf zeichnen"),
    "threesea-la-saved-n-downloads":
        ("Saved %n file(s) to Downloads",
         ("%n Datei in Downloads gespeichert",
          "%n Dateien in Downloads gespeichert")),
    "threesea-la-download-failed":
        ("The file could not be downloaded. It may no longer be on the server.",
         "Die Datei konnte nicht heruntergeladen werden. Vielleicht ist sie nicht mehr auf dem Server."),
    "threesea-la-send-in-turn-failed":
        ("Threesea did not send everything",
         "Threesea hat nicht alles gesendet"),
    "threesea-la-not-sent-to-chats":
        ("Not everything went to %n chat(s)",
         ("Nicht alles ging an %n Chat",
          "Nicht alles ging an %n Chats")),
    "threesea-la-sent-to-chats":
        ("On its way to %n chat(s)",
         ("Unterwegs an %n Chat",
          "Unterwegs an %n Chats")),
    "threesea-la-send-in-turn-lost":
        ("Some chats did not get what you sent them. Send it to them again.",
         "Einige Chats haben nicht bekommen, was du ihnen gesendet hast. Sende es ihnen noch einmal."),
    "threesea-la-locked-notification-reply-body":
        ("Open it and enter your PIN or passphrase to send your reply and receive messages.",
         "Öffne Threesea und gib deine PIN oder Passphrase ein, um deine Antwort zu senden und Nachrichten zu empfangen."),
    "threesea-la-startup-failed-notification-body":
        ("Open it to see why. Messages arrive again once it runs.",
         "Öffne es, um zu sehen, warum. Nachrichten kommen wieder an, sobald es läuft."),
    "threesea-la-safe-failing-notification":
        ("Threema Safe backup failing",
         "Threema-Safe-Backup schlägt fehl"),
    "threesea-la-safe-failing-notification-body":
        ("Threema Safe hasn't been able to back up for %n day(s). Tap to check.",
         ("Threema Safe konnte seit %n Tag kein Backup machen. Tippe, um nachzusehen.",
          "Threema Safe konnte seit %n Tagen kein Backup machen. Tippe, um nachzusehen.")),
    "threesea-la-reason-login-refused":
        ("Threema's server keeps closing the connection during login.",
         "Der Threema-Server schließt die Verbindung immer wieder während der Anmeldung."),
    "threesea-la-reason-another-connection":
        ("Another device keeps taking over this Threema ID. An ID works on one phone at a time, so Threesea tries again every 10 minutes.",
         "Ein anderes Gerät übernimmt immer wieder diese Threema-ID. Eine ID funktioniert immer nur auf einem Telefon, deshalb versucht Threesea es alle 10 Minuten erneut."),
    "threesea-la-reason-mediator-version":
        ("Threema's server for linked devices no longer accepts this version of Threesea.",
         "Der Threema-Server für verknüpfte Geräte nimmt diese Version von Threesea nicht mehr an."),
    "threesea-la-reason-mediator-device-limit":
        ("Threema's server for linked devices turned this device away: this Threema ID has reached its limit of linked devices.",
         "Der Threema-Server für verknüpfte Geräte hat dieses Gerät abgewiesen: Diese Threema-ID hat ihre Höchstzahl an verknüpften Geräten erreicht."),
    "threesea-la-reason-mediator-duplicate":
        ("Another connection under this device's login took over on Threema's server for linked devices.",
         "Eine andere Verbindung mit der Anmeldung dieses Geräts hat auf dem Threema-Server für verknüpfte Geräte übernommen."),
    "threesea-la-reason-mediator-queue-full":
        ("Too many messages are waiting for this device on Threema's server for linked devices, so it closed the connection.",
         "Auf dem Threema-Server für verknüpfte Geräte warten zu viele Nachrichten auf dieses Gerät, deshalb hat er die Verbindung geschlossen."),
    "threesea-la-reason-mediator-other":
        ("Threema's server for linked devices closed the connection (code %1).",
         "Der Threema-Server für verknüpfte Geräte hat die Verbindung geschlossen (Code %1)."),
    "threesea-la-server-message-notification":
        ("Message from Threema's server",
         "Nachricht vom Threema-Server"),
    "threesea-la-opens-after-call":
        ("Opens when the call is over",
         "Öffnet sich, wenn der Anruf vorbei ist"),
    "threesea-la-capture-discarded":
        ("The photo or video waiting to be sent was discarded",
         "Das Foto oder Video, das noch gesendet werden sollte, wurde verworfen"),
    "threesea-la-mobile-verified":
        ("Phone number confirmed",
         "Telefonnummer bestätigt"),
    "threesea-la-share-before-setup":
        ("Set up Threesea first, then share this again",
         "Richte zuerst Threesea ein und teile das dann noch einmal"),
    "threesea-la-storage-problem-title":
        ("Saving messages failed",
         "Nachrichten konnten nicht gespeichert werden"),
    "threesea-la-storage-problem-body":
        ("Threesea can't write to its data, so new messages wait on the server, for a limited time. Check that this device has free storage. The log in Settings, Advanced has the error code.",
         "Threesea kann nicht in seine Daten schreiben, deshalb warten neue Nachrichten auf dem Server, für begrenzte Zeit. Prüfe, ob dieses Gerät freien Speicher hat. Das Protokoll unter Einstellungen, Erweitert enthält den Fehlercode."),
    "threesea-la-web-connected-notification":
        ("Connected to Threema Web",
         "Mit Threema Web verbunden"),
    "threesea-la-web-connected-notification-body":
        ("%n browser(s) can read and send your messages through this phone. Tap to manage.",
         ("%n Browser kann über dieses Telefon deine Nachrichten lesen und senden. Tippe, um das zu verwalten.",
          "%n Browser können über dieses Telefon deine Nachrichten lesen und senden. Tippe, um sie zu verwalten.")),
    "threesea-la-web-certificate-notification":
        ("Threema Web server has a new certificate",
         "Threema-Web-Server hat ein neues Zertifikat"),
    "threesea-la-web-certificate-notification-body":
        ("Tap to decide whether to trust it.",
         "Tippe, um zu entscheiden, ob du ihm vertraust."),
    "threesea-me-call-decline":
        ("Decline",
         "Ablehnen"),
    "threesea-me-call-answer":
        ("Answer",
         "Annehmen"),
    "threesea-la-app-locked-reply-body":
        ("Open it and enter your PIN or passphrase to send your reply.",
         "Öffne Threesea und gib deine PIN oder Passphrase ein, um deine Antwort zu senden."),
    "threesea-la-updated":
        ("Threesea was updated",
         "Threesea wurde aktualisiert"),
    "threesea-la-removed":
        ("Threesea was removed from this device",
         "Threesea wurde von diesem Gerät entfernt"),
    "threesea-la-reply-not-sent":
        ("Your reply was not sent",
         "Deine Antwort wurde nicht gesendet"),
    "threesea-la-app-locked-body":
        ("Something new has arrived. Open it and enter your PIN or passphrase to see it.",
         "Es ist etwas Neues angekommen. Öffne Threesea und gib deine PIN oder Passphrase ein, um es zu sehen."),
    "threesea-la-removed-body":
        ("Close it to end it.",
         "Schließe es, um es zu beenden."),
    "threesea-la-updated-body":
        ("Close it and open it again to use the new version.",
         "Schließe es und öffne es wieder, um die neue Version zu verwenden."),
    "threesea-la-updated-open-body":
        ("Tap to open the new version.",
         "Tippe, um die neue Version zu öffnen."),
    "threesea-la-updated-closed-body":
        ("Open it to receive messages again.",
         "Öffne es, um wieder Nachrichten zu empfangen."),
    "threesea-la-reply-not-sent-ended-kept":
        ("Threesea ended before sending it. Your text is kept as a draft in its chat.",
         "Threesea wurde beendet, bevor es sie gesendet hat. Dein Text ist als Entwurf in seinem Chat gespeichert."),
    "threesea-la-reply-not-sent-ended":
        ("Threesea ended before sending it. Write it again in the chat.",
         "Threesea wurde beendet, bevor es sie gesendet hat. Schreibe sie im Chat noch einmal."),
    "threesea-la-delete-data-linked-remain":
        ("This phone could not reach the server, so your linked devices stay linked to this ID.",
         "Dieses Telefon hat den Server nicht erreicht, deshalb bleiben deine verknüpften Geräte mit dieser ID verknüpft."),
    "threesea-la-reply-not-sent-body":
        ("Threesea could not start. Open it to see why.",
         "Threesea konnte nicht starten. Öffne es, um zu sehen, warum."),
    "threesea-la-reply-not-sent-yet":
        ("It is sent when you next open Threesea.",
         "Sie wird gesendet, wenn du Threesea das nächste Mal öffnest."),
    "threesea-la-reply-not-sent-kept":
        ("%1 Your text is kept as a draft in the chat.",
         "%1 Dein Text ist als Entwurf im Chat gespeichert."),
    "threesea-la-reply-not-sent-late":
        ("It reached Threesea too late. Write it again in the chat.",
         "Sie hat Threesea zu spät erreicht. Schreibe sie im Chat noch einmal."),
    "threesea-la-share-empty":
        ("What was shared holds nothing to send",
         "Das Geteilte enthält nichts zum Senden"),
    "threesea-la-share-own-files":
        ("Files of Threesea's own were left out of what was shared",
         "Dateien, die Threesea selbst gehören, wurden beim Teilen ausgelassen"),
    "threesea-la-call-route-failed":
        ("The sound could not be switched",
         "Der Ton konnte nicht umgeschaltet werden"),
    "threesea-la-recording-discarded":
        ("Recording stopped and discarded",
         "Aufnahme beendet und verworfen"),
    "threesea-la-delete-data-failed":
        ("The ID was not removed: %1",
         "Die ID wurde nicht entfernt: %1"),
    "threesea-la-backup-reminder":
        ("Back up your Threema ID",
         "Sichere deine Threema-ID"),
    "threesea-la-backup-reminder-body":
        ("A backup brings your ID, contacts and groups back on a new phone. Tap to set up Threema Safe.",
         "Ein Backup bringt deine ID, Kontakte und Gruppen auf ein neues Telefon. Tippe, um Threema Safe einzurichten."),
    "threesea-la-safe-server-copy-left":
        ("Threema Safe is off. The backup could not be deleted from Threema's server, so it stays there until it expires.",
         "Threema Safe ist aus. Das Backup konnte nicht vom Threema-Server gelöscht werden und bleibt dort, bis es abläuft."),
    "threesea-la-device-removal-failed":
        ("Removing linked devices did not finish. Linked devices shows what is still linked, and you can try again there.",
         "Das Entfernen der verknüpften Geräte wurde nicht fertig. Unter Verknüpfte Geräte siehst du, was noch verknüpft ist, und kannst es dort erneut versuchen."),
    "threesea-la-flavour":
        ("Flavour",
         "Variante"),
    "threesea-la-flavour-full":
        ("Full",
         "Full"),
    "threesea-la-flavour-lite":
        ("Lite",
         "Lite"),
    "threesea-la-credit-emoji-search":
        ("Emoji search terms from threema-android (AGPL-3.0).",
         "Emoji-Suchbegriffe aus threema-android (AGPL-3.0)."),
    "threesea-la-license-copyright":
        ("Copyright © 2026 the Threesea developers. libthreema and the code taken from threema-android: copyright Threema AG.",
         "Copyright © 2026 die Entwickler von Threesea. libthreema und der aus threema-android übernommene Code: Copyright Threema AG."),
    "threesea-la-license-source":
        ("Source code and bug reports: %1",
         "Quellcode und Fehlermeldungen: %1"),
    "threesea-bt-license-agpl":
        ("License text",
         "Lizenztext"),
    "threesea-bt-license-notices":
        ("Third-party licenses",
         "Lizenzen von Drittanbietern"),
    "threesea-la-add-scanned-contact":
        ("Adds this Threema ID to your contacts with the key from Threema's directory. To verify it, scan the QR code in the other person's app.",
         "Fügt diese Threema-ID mit dem Schlüssel aus dem Threema-Verzeichnis zu deinen Kontakten hinzu. Um sie zu überprüfen, scanne den QR-Code in der App der anderen Person."),
    "threesea-la-app-lock-about":
        ("With a lock you can also make chats private: they are hidden from the chat list and are opened from its bottom menu. If you forget the app lock's PIN or passphrase, the way back in is to delete Threesea's data folder .local/share/org.threesea/harbour-threesea with the chats and media in it, restart the phone and restore from a backup. A passphrase for the stored data stays set: Threesea asks for it first.",
         "Mit einer Sperre kannst du Chats auch privat machen: Sie verschwinden aus der Chatliste und werden über deren unteres Menü geöffnet. Wenn du die PIN oder Passphrase der App-Sperre vergisst, kommst du nur wieder hinein, indem du den Datenordner von Threesea, .local/share/org.threesea/harbour-threesea, mit den Chats und Medien darin löschst, das Telefon neu startest und aus einem Backup wiederherstellst. Eine Passphrase für die gespeicherten Daten bleibt gesetzt: Threesea fragt zuerst nach ihr."),
    "threesea-la-call-interrupted":
        ("Connection interrupted…",
         "Verbindung unterbrochen …"),
    "threesea-la-call-unverified":
        ("The connection could not be verified. The call was ended.",
         "Die Verbindung konnte nicht überprüft werden. Der Anruf wurde beendet."),
    "threesea-la-call-connection-failed":
        ("Connection failed",
         "Verbindung fehlgeschlagen"),
    "threesea-la-camera-photo-failed":
        ("The photo could not be taken.",
         "Das Foto konnte nicht aufgenommen werden."),
    "threesea-la-camera-video-failed":
        ("The video could not be recorded.",
         "Das Video konnte nicht aufgenommen werden."),
    "threesea-la-sending-drawing":
        ("Sending drawing",
         "Zeichnung wird gesendet"),
    "threesea-la-discarding-drawing":
        ("Discarding drawing",
         "Zeichnung wird verworfen"),
    "threesea-la-discarding-recording":
        ("Discarding recording",
         "Aufnahme wird verworfen"),
    "threesea-la-recording-held":
        ("Recording stopped. Send it or discard it.",
         "Aufnahme beendet. Sende sie oder verwirf sie."),
    "threesea-la-recording-not-playable":
        ("This recording could not be played. It can still be sent.",
         "Diese Aufnahme konnte nicht abgespielt werden. Senden lässt sie sich trotzdem."),
    "threesea-la-spam-reporting-group":
        ("Reporting, blocking, leaving and deleting",
         "Wird gemeldet, blockiert, verlassen und gelöscht"),
    "threesea-la-spam-reporting-contact":
        ("Reporting, blocking and deleting",
         "Wird gemeldet, blockiert und gelöscht"),
    "threesea-me-polls":
        ("Polls",
         "Umfragen"),
    "threesea-la-polls-waiting":
        ("%n poll(s) waiting for your vote",
         ("%n Umfrage wartet auf deine Stimme",
          "%n Umfragen warten auf deine Stimme")),
    "threesea-la-revoked-input":
        ("This Threema ID was revoked",
         "Diese Threema-ID wurde widerrufen"),
    "threesea-la-recording-paused":
        ("Paused %1",
         "Angehalten %1"),
    "threesea-he-choose-chats-accept":
        ("Choose",
         "Auswählen"),
    "threesea-la-search-chats-contacts":
        ("Search chats and contacts",
         "Chats und Kontakte durchsuchen"),
    "threesea-la-search-no-chats-contacts":
        ("No chats or contacts found",
         "Keine Chats oder Kontakte gefunden"),
    "threesea-me-clean-up-scope":
        ("Groups with only you in them are cleaned up too, and private chats while they are unlocked.",
         "Gruppen, in denen nur du bist, werden auch aufgeräumt, und private Chats, solange sie entsperrt sind."),
    "threesea-he-clean-up-all-chats":
        ("Clean up all chats",
         "Alle Chats aufräumen"),
    "threesea-he-confirm-send":
        ("Send",
         "Senden"),
    "threesea-la-confirm-forward-to":
        ("Forward to %1",
         "Weiterleiten an %1"),
    "threesea-la-confirm-send-to":
        ("Send to %1",
         "Senden an %1"),
    "threesea-la-confirm-caption":
        ("Caption",
         "Beschriftung"),
    "threesea-la-confirm-message":
        ("Message",
         "Nachricht"),
    "threesea-la-confirm-include-captions":
        ("Include captions",
         "Beschriftungen mitsenden"),
    "threesea-me-add-to-contacts":
        ("Add to contacts",
         "Zu den Kontakten hinzufügen"),
    "threesea-la-contact-share-picture":
        ("Share profile picture",
         "Profilbild teilen"),
    "threesea-la-search-contacts":
        ("Search contacts",
         "Kontakte durchsuchen"),
    "threesea-la-search-no-contacts":
        ("No contacts found",
         "Keine Kontakte gefunden"),
    "threesea-me-lock-now":
        ("Lock now",
         "Jetzt sperren"),
    "threesea-la-data-import-unfinished":
        ("Your Data backup isn't fully imported yet. Tap to import it again; it continues where it stopped.",
         "Dein Daten-Backup ist noch nicht ganz importiert. Tippe, um es erneut zu importieren; der Import macht dort weiter, wo er angehalten hat."),
    "threesea-me-notifications-back-on":
        ("Notifications on",
         "Benachrichtigungen ein"),
    "threesea-me-notifications-off-8-hours":
        ("Notifications off for 8 hours",
         "Benachrichtigungen für 8 Stunden aus"),
    "threesea-la-chats-not-loaded":
        ("The chats could not be loaded",
         "Die Chats konnten nicht geladen werden"),
    "threesea-la-data-backup-import-intro":
        ("Adds what is missing: contacts, groups and pictures, then messages and media. Everything already on this device stays as it is. No messages are sent.",
         "Ergänzt, was fehlt: Kontakte, Gruppen und Bilder, dann Nachrichten und Medien. Alles, was schon auf diesem Gerät ist, bleibt unverändert. Es werden keine Nachrichten gesendet."),
    "threesea-la-data-backup-import-other":
        ("This backup belongs to another Threema ID or key. To import it, first restore that ID on this device.",
         "Dieses Backup gehört zu einer anderen Threema-ID oder einem anderen Schlüssel. Um es zu importieren, stelle zuerst diese ID auf diesem Gerät wieder her."),
    "threesea-la-data-backup-import-unreadable":
        ("The Threema ID in this backup isn't readable with this password. The file may be damaged. Only a backup of this device's ID is imported.",
         "Die Threema-ID in diesem Backup ist mit diesem Passwort nicht lesbar. Die Datei ist vielleicht beschädigt. Importiert wird nur ein Backup der ID dieses Geräts."),
    "threesea-la-data-backup-import-no-id":
        ("This backup holds no Threema ID. Only a backup of this device's ID is imported.",
         "Dieses Backup enthält keine Threema-ID. Importiert wird nur ein Backup der ID dieses Geräts."),
    "threesea-la-data-backup-contents":
        ("Encrypted backups of your Threema ID, chats, contacts, groups and media, in the format of Threema for Android.",
         "Verschlüsselte Backups deiner Threema-ID, Chats, Kontakte, Gruppen und Medien im Format von Threema für Android."),
    "threesea-la-android-export-running":
        ("Writing the backup",
         "Backup wird geschrieben"),
    "threesea-la-android-export-left-out":
        ("%n chat(s) stayed out of the last backup: restoring them needs the contact's key, which could not be fetched. The next backup tries again.",
         ("%n Chat fehlt im letzten Backup: Zum Wiederherstellen braucht es den Schlüssel des Kontakts, der sich nicht abrufen ließ. Das nächste Backup versucht es erneut.",
          "%n Chats fehlen im letzten Backup: Zum Wiederherstellen braucht es die Schlüssel der Kontakte, die sich nicht abrufen ließen. Das nächste Backup versucht es erneut.")),
    "threesea-la-android-export-files-left-out":
        ("%n file(s) stayed out of the last backup: they could not be read on this phone.",
         ("%n Datei fehlt im letzten Backup: Sie ließ sich auf diesem Telefon nicht lesen.",
          "%n Dateien fehlen im letzten Backup: Sie ließen sich auf diesem Telefon nicht lesen.")),
    "threesea-la-safe-last-attempt-failed":
        ("Last attempt failed",
         "Letzter Versuch fehlgeschlagen"),
    "threesea-he-restore-data-backup":
        ("Restore from Data backup",
         "Aus Daten-Backup wiederherstellen"),
    "threesea-la-restore-data-backup-intro":
        ("Choose a Data backup from Threema for Android or Threesea and enter its password. It brings your ID, contacts, groups, chats and media. The password stays on this device.",
         "Wähle ein Daten-Backup von Threema für Android oder Threesea und gib sein Passwort ein. Es bringt deine ID, Kontakte, Gruppen, Chats und Medien. Das Passwort bleibt auf diesem Gerät."),
    "threesea-la-restore-one-phone":
        ("An ID works on one phone at a time: new messages go only to the phone that connected last.",
         "Eine ID funktioniert immer nur auf einem Telefon: Neue Nachrichten gehen nur an das Telefon, das sich zuletzt verbunden hat."),
    "threesea-la-data-backup-reason-too-large":
        ("too large to import",
         "zu groß für den Import"),
    "threesea-la-data-backup-reason-damaged":
        ("damaged in the backup",
         "im Backup beschädigt"),
    "threesea-la-data-backup-reason-other-key":
        ("a different key in the backup",
         "anderer Schlüssel im Backup"),
    "threesea-la-data-backup-reason-not-here":
        ("not on this device",
         "nicht auf diesem Gerät"),
    "threesea-la-data-backup-imported-except":
        ("The backup is imported, except for what is listed below. Everything already on this device stayed as it was.",
         "Das Backup ist importiert, bis auf das, was unten steht. Alles, was schon auf diesem Gerät war, ist unverändert geblieben."),
    "threesea-la-data-backup-restored-private":
        ("Chats that were marked private come back as ordinary, visible chats. To hide them again, set up an app lock in Settings and choose Make private in the chat list.",
         "Chats, die privat waren, kommen als normale, sichtbare Chats zurück. Um sie wieder zu verbergen, richte in den Einstellungen eine App-Sperre ein und wähle in der Chatliste Privat machen."),
    "threesea-la-data-backup-received-list-missing":
        ("The list of messages already received isn't part of this import. Messages sent again are recognised from what this device receives from now on.",
         "Die Liste der schon empfangenen Nachrichten gehört nicht zu diesem Import. Erneut gesendete Nachrichten werden ab jetzt an dem erkannt, was dieses Gerät empfängt."),
    "threesea-la-data-backup-key-conflicts":
        ("Contacts with a different key (kept as here)",
         "Kontakte mit anderem Schlüssel (wie hier behalten)"),
    "threesea-la-data-backup-local-avatars":
        ("Pictures chosen for contacts on the other device, left out",
         "Auf dem anderen Gerät gewählte Kontaktbilder, ausgelassen"),
    "threesea-la-data-backup-groups-left-out":
        ("Groups left out",
         "Ausgelassene Gruppen"),
    "threesea-la-data-backup-unsent":
        ("Messages that were never sent (kept as failed)",
         "Nie gesendete Nachrichten (als fehlgeschlagen behalten)"),
    "threesea-la-data-backup-chats-left-out":
        ("Chats left out",
         "Ausgelassene Chats"),
    "threesea-la-data-backup-chats-too-large":
        ("Chats too large to import",
         "Chats, zu groß für den Import"),
    "threesea-la-data-backup-chats-damaged":
        ("Damaged chats left out",
         "Beschädigte Chats, ausgelassen"),
    "threesea-la-data-backup-messages-left-out":
        ("Messages left out",
         "Ausgelassene Nachrichten"),
    "threesea-la-data-backup-collisions":
        ("Messages here from another sender (kept as here)",
         "Nachrichten hier von einem anderen Absender (wie hier behalten)"),
    "threesea-la-data-backup-reactions-left-out":
        ("Reactions left out",
         "Ausgelassene Reaktionen"),
    "threesea-la-data-backup-reaction-files-left-out":
        ("Lists of reactions left out",
         "Ausgelassene Reaktionslisten"),
    "threesea-la-data-backup-entries-left-out":
        ("Entries left out",
         "Ausgelassene Einträge"),
    "threesea-la-data-backup-media-missing":
        ("Media not in the backup",
         "Medien, die nicht im Backup sind"),
    "threesea-se-data-backup-left-out":
        ("Chats that weren't imported",
         "Nicht importierte Chats"),
    "threesea-la-data-backup-uid-conflicts":
        ("Messages also found in another chat",
         "Nachrichten, die auch in einem anderen Chat sind"),
    "threesea-la-data-backup-foreign-group-chats":
        ("Group chats mostly from non-members",
         "Gruppenchats meist von Nichtmitgliedern"),
    "threesea-la-delete-data-goes":
        ("This removes your Threema ID, contacts, chats, media and settings from this device.",
         "Damit werden deine Threema-ID, Kontakte, Chats, Medien und Einstellungen von diesem Gerät entfernt."),
    "threesea-la-delete-data-comes-back-data-backup":
        ("Your ID itself is not revoked: you can restore it later from Threema Safe, an ID backup or a Data backup — without one of them, the ID is lost. Chats and media come back only from a Data backup.",
         "Die ID selbst wird nicht widerrufen: Du kannst sie später aus Threema Safe, einem ID-Backup oder einem Daten-Backup wiederherstellen – ohne eines davon ist die ID verloren. Chats und Medien kommen nur aus einem Daten-Backup zurück."),
    "threesea-la-delete-data-stays":
        ("Pictures saved to the Threesea folder in Pictures, and backups and files you exported, stay on this device. A Threema Safe backup stays on the server.",
         "Bilder im Ordner Threesea unter Bilder sowie Backups und Dateien, die du exportiert hast, bleiben auf diesem Gerät. Ein Threema-Safe-Backup bleibt auf dem Server."),
    "threesea-la-delete-data-linked":
        ("Your linked devices are removed from this ID first, which takes a connection. If this phone can't reach the server then, they stay linked and keep the ID.",
         "Deine verknüpften Geräte werden zuerst von dieser ID entfernt, wofür es eine Verbindung braucht. Erreicht dieses Telefon den Server dann nicht, bleiben sie verknüpft und behalten die ID."),
    "threesea-la-delete-data-have-a-backup":
        ("I have a Threema Safe, ID or Data backup, or no longer need this ID",
         "Ich habe ein Threema-Safe-, ID- oder Daten-Backup oder brauche diese ID nicht mehr"),
    "threesea-he-share-id-backup":
        ("Share ID backup",
         "ID-Backup teilen"),
    "threesea-bt-share":
        ("Share",
         "Teilen"),
    "threesea-bt-done":
        ("Done",
         "Fertig"),
    "threesea-la-closing-id-backup":
        ("Closing ID backup",
         "ID-Backup wird geschlossen"),
    "threesea-la-export-chat-password-note":
        ("At least 8 characters. With a password the messages and files in the archive are encrypted with AES-256, and it opens in ZIP programs with AES support, such as 7-Zip; the names of the files stay readable. Empty: not encrypted, anyone who gets the file can read it.",
         "Mindestens 8 Zeichen. Mit einem Passwort werden die Nachrichten und Dateien im Archiv mit AES-256 verschlüsselt, und es lässt sich mit ZIP-Programmen öffnen, die AES unterstützen, etwa 7-Zip; die Namen der Dateien bleiben lesbar. Leer: unverschlüsselt, jeder, der die Datei bekommt, kann sie lesen."),
    "threesea-he-rename-list":
        ("Rename list",
         "Liste umbenennen"),
    "threesea-me-choose-picture":
        ("Choose picture",
         "Bild wählen"),
    "threesea-me-take-picture":
        ("Take photo",
         "Foto aufnehmen"),
    "threesea-la-removing-picture":
        ("Removing picture",
         "Bild wird entfernt"),
    "threesea-la-license-key-changed":
        ("License key changed",
         "Lizenzschlüssel geändert"),
    "threesea-me-change-license-key":
        ("Change license key",
         "Lizenzschlüssel ändern"),
    "threesea-la-identity-intro-id-export":
        ("Your Threema ID identifies you to your contacts. Create a new one, or restore the ID you already use from Threema Safe, an ID backup (Threema for Android calls it an ID export) or a Data backup. A Data backup brings your chats as well.",
         "Mit deiner Threema-ID erkennen dich deine Kontakte. Erstelle eine neue oder stelle die ID, die du schon verwendest, aus Threema Safe, einem ID-Backup (Threema für Android nennt es ID-Export) oder einem Daten-Backup wieder her. Ein Daten-Backup bringt auch deine Chats mit."),
    "threesea-bt-restore-data-backup":
        ("Restore from Data backup",
         "Aus Daten-Backup wiederherstellen"),
    "threesea-la-cannot-show-picture":
        ("This picture could not be shown",
         "Dieses Bild kann nicht gezeigt werden"),
    "threesea-la-media-position":
        ("%1 of %2",
         "%1 von %2"),
    "threesea-me-select-all":
        ("Select all",
         "Alle auswählen"),
    "threesea-me-select":
        ("Select",
         "Auswählen"),
    "threesea-la-select-media-hint":
        ("Tap to select",
         "Zum Auswählen tippen"),
    "threesea-la-all-contacts-on-list":
        ("All your contacts are on this list",
         "Alle deine Kontakte sind auf dieser Liste"),
    "threesea-la-all-contacts-in-group":
        ("All your contacts are in this group",
         "Alle deine Kontakte sind in dieser Gruppe"),
    "threesea-he-reactions":
        ("Reactions",
         "Reaktionen"),
    "threesea-la-nickname-ask-title":
        ("Choose a nickname",
         "Wähle einen Nickname"),
    "threesea-la-nickname-ask-text":
        ("This name goes with your messages. Contacts who haven't given you a name of their own see it. You can change it in Settings, My profile.",
         "Dieser Name geht mit deinen Nachrichten mit. Kontakte, die dir keinen eigenen Namen gegeben haben, sehen ihn. Du kannst ihn in Einstellungen, Mein Profil ändern."),
    "threesea-he-photo-text":
        ("Text on the photo",
         "Text auf dem Foto"),
    "threesea-la-photo-text":
        ("Text",
         "Text"),
    "threesea-la-data-passphrase-note":
        ("If you forget it, Threesea can start fresh only without what it has stored on this phone. Your ID and chats can then come back from a backup, if you have one.",
         "Wenn du sie vergisst, kann Threesea nur ohne das, was es auf diesem Telefon gespeichert hat, neu anfangen. Deine ID und Chats kommen dann aus einem Backup zurück, falls du eines hast."),
    "threesea-me-try-again":
        ("Try again",
         "Erneut versuchen"),
    "threesea-he-choose-place":
        ("Choose a place",
         "Ort wählen"),
    "threesea-la-place-search":
        ("Search by name",
         "Nach Namen suchen"),
    "threesea-la-place-use-own":
        ("Use your position",
         "Deinen Standort verwenden"),
    "threesea-he-places-found":
        ("Places found",
         "Gefundene Orte"),
    "threesea-he-places-nearby":
        ("Places nearby",
         "Orte in der Nähe"),
    "threesea-la-places-need-position":
        ("Places are found around your position",
         "Orte werden um deinen Standort herum gefunden"),
    "threesea-la-places-failed":
        ("Threema's map server did not answer",
         "Der Kartenserver von Threema hat nicht geantwortet"),
    "threesea-la-places-none":
        ("No places found",
         "Keine Orte gefunden"),
    "threesea-la-places-failed-hint":
        ("Check your internet connection and pull down to try again.",
         "Prüfe deine Internetverbindung und ziehe nach unten, um es erneut zu versuchen."),
    "threesea-la-places-none-hint":
        ("Search by name to look further away.",
         "Suche nach Namen, um weiter entfernt zu suchen."),
    "threesea-he-polls":
        ("Polls",
         "Umfragen"),
    "threesea-la-no-polls":
        ("No polls in this chat",
         "Keine Umfragen in diesem Chat"),
    "threesea-se-polls-closed":
        ("Closed",
         "Abgeschlossen"),
    "threesea-se-polls-open":
        ("Open",
         "Offen"),
    "threesea-la-poll-waiting":
        ("Waiting for your vote",
         "Wartet auf deine Stimme"),
    "threesea-la-restore-intro-scan":
        ("Scan the QR code of the ID backup or paste its text (80 characters in groups of four, like XXXX-XXXX-…), and enter the password you chose when you created it. Threema for Android calls it an ID export. It brings your ID. Chats come from a Data backup, which Settings, Backup can import afterwards.",
         "Scanne den QR-Code des ID-Backups oder füge seinen Text ein (80 Zeichen in Vierergruppen, wie XXXX-XXXX-…), und gib das Passwort ein, das du beim Erstellen gewählt hast. Threema für Android nennt es ID-Export. Es bringt deine ID. Chats kommen aus einem Daten-Backup, das du danach unter Einstellungen, Backup importieren kannst."),
    "threesea-la-safe-find-id-none":
        ("No Threema ID is linked to this.",
         "Damit ist keine Threema-ID verknüpft."),
    "threesea-he-safe-find-id":
        ("Find my ID",
         "Meine ID finden"),
    "threesea-la-safe-find-id-intro":
        ("Enter the phone number or e-mail address linked to your Threema ID. It is hashed on this device, and Threema's directory is asked with the hash.",
         "Gib die Telefonnummer oder E-Mail-Adresse ein, die mit deiner Threema-ID verknüpft ist. Sie wird auf diesem Gerät gehasht, und das Threema-Verzeichnis wird mit dem Hash gefragt."),
    "threesea-la-safe-find-id-phone":
        ("Phone number",
         "Telefonnummer"),
    "threesea-la-safe-find-id-email":
        ("E-mail address",
         "E-Mail-Adresse"),
    "threesea-bt-safe-find-id":
        ("Find",
         "Suchen"),
    "threesea-se-safe-find-id-found":
        ("Linked IDs",
         "Verknüpfte IDs"),
    "threesea-la-safe-description-changes":
        ("Encrypted backup of your ID, profile, contacts, groups and settings, updated when something changes. For your chats, use Data backup.",
         "Verschlüsseltes Backup von ID, Profil, Kontakten, Gruppen und Einstellungen, aktualisiert, wenn sich etwas ändert. Für deine Chats verwende das Daten-Backup."),
    "threesea-la-safe-disabling-delete":
        ("Turning off and deleting the backup",
         "Wird ausgeschaltet und das Backup gelöscht"),
    "threesea-he-continue":
        ("Continue",
         "Weiter"),
    "threesea-he-change":
        ("Change",
         "Ändern"),
    "threesea-la-safe-password-change-intro":
        ("Choose a new password for Threema Safe. A new backup replaces the one made with the old password. Only your password (at least 8 characters) unlocks it: if you forget it, the backup cannot be recovered.",
         "Wähle ein neues Passwort für Threema Safe. Ein neues Backup ersetzt das mit dem alten Passwort. Nur dein Passwort (mindestens 8 Zeichen) entsperrt es: Wenn du es vergisst, kann das Backup nicht wiederhergestellt werden."),
    "threesea-la-safe-password-intro-changes":
        ("Threema Safe stores an encrypted backup of your ID, profile, contacts, groups and settings on Threema's servers, updated when something changes. For your chats, use Data backup. Only your password (at least 8 characters) unlocks it — if you forget it, the backup cannot be recovered.",
         "Threema Safe speichert ein verschlüsseltes Backup deiner ID, deines Profils, deiner Kontakte, Gruppen und Einstellungen auf den Servern von Threema und aktualisiert es, wenn sich etwas ändert. Für deine Chats verwende das Daten-Backup. Nur dein Passwort (mindestens 8 Zeichen) entsperrt es – wenn du es vergisst, kann das Backup nicht wiederhergestellt werden."),
    "threesea-la-safe-setup-skip":
        ("Continue without a backup",
         "Ohne Backup weiter"),
    "threesea-la-safe-setup-skip-description":
        ("Your ID, contacts and groups then exist on this phone alone, and go with it. Settings, Backup can turn Threema Safe on or export an ID backup at any time.",
         "Deine ID, Kontakte und Gruppen gibt es dann nur auf diesem Telefon, und sie gehen mit ihm. Unter Einstellungen, Backup kannst du Threema Safe jederzeit einschalten oder ein ID-Backup exportieren."),
    "threesea-me-safe-find-id":
        ("Find my ID",
         "Meine ID finden"),
    "threesea-la-restore-safe-intro-data-backup":
        ("Enter your Threema ID and your Threema Safe password. Your profile, contacts, groups and settings are restored with it. Chats come from a Data backup, which Settings, Backup can import afterwards.",
         "Gib deine Threema-ID und dein Threema-Safe-Passwort ein. Profil, Kontakte, Gruppen und Einstellungen werden damit wiederhergestellt. Chats kommen aus einem Daten-Backup, das du danach unter Einstellungen, Backup importieren kannst."),
    "threesea-la-qr-not-id-backup":
        ("This is not an ID backup. Scan the QR code of your ID backup.",
         "Das ist kein ID-Backup. Scanne den QR-Code deines ID-Backups."),
    "threesea-la-qr-not-web-code":
        ("This is a contact's code. Scan the code Threema Web shows.",
         "Das ist der Code eines Kontakts. Scanne den Code, den Threema Web zeigt."),
    "threesea-la-qr-link-verify":
        ("This is a link to a Threema ID. To verify it, scan the QR code in the other person's app.",
         "Das ist ein Link zu einer Threema-ID. Um sie zu überprüfen, scanne den QR-Code in der App der anderen Person."),
    "threesea-la-qr-id-backup-elsewhere":
        ("This is an ID backup. It restores an ID while the app is being set up.",
         "Das ist ein ID-Backup. Damit wird eine ID wiederhergestellt, während die App eingerichtet wird."),
    "threesea-la-qr-unknown":
        ("This is not a Threema QR code.",
         "Das ist kein Threema-QR-Code."),
    "threesea-la-scan-hint-id-backup":
        ("Point the camera at the QR code of your ID backup.",
         "Richte die Kamera auf den QR-Code deines ID-Backups."),
    "threesea-la-scan-hint-web":
        ("Point the camera at the QR code Threema Web shows.",
         "Richte die Kamera auf den QR-Code, den Threema Web zeigt."),
    "threesea-la-search-names-messages":
        ("Names and messages",
         "Namen und Nachrichten"),
    "threesea-he-messages":
        ("Messages",
         "Nachrichten"),
    "threesea-me-remove-star":
        ("Remove star",
         "Stern entfernen"),
    "threesea-la-search-nothing-at-all":
        ("No chats, contacts or messages found",
         "Keine Chats, Kontakte oder Nachrichten gefunden"),
    "threesea-la-location-waiting":
        ("No position yet. Check that location is on in Settings → Location. Outdoors this goes faster.",
         "Noch kein Standort. Prüfe, ob die Ortung unter Einstellungen → Standort eingeschaltet ist. Im Freien geht es schneller."),
    "threesea-la-location-own":
        ("Your position",
         "Dein Standort"),
    "threesea-la-location-place":
        ("Place",
         "Ort"),
    "threesea-la-location-last-known":
        ("Your last known position, from %1",
         "Dein letzter bekannter Standort, von %1"),
    "threesea-la-place-server":
        ("To find places, Threema's map server gets your position and what you search for.",
         "Um Orte zu finden, bekommt der Kartenserver von Threema deinen Standort und das, wonach du suchst."),
    "threesea-la-location-use-last-known":
        ("Use your last known position, from",
         "Letzten bekannten Standort verwenden, von"),
    "threesea-la-update-check-failed":
        ("Could not check for updates. Try again later.",
         "Die Suche nach Updates hat nicht geklappt. Versuche es später erneut."),
    "threesea-me-emoji-twemoji":
        ("Twemoji",
         "Twemoji"),
    "threesea-la-safe-on-failing":
        ("On, the last attempt failed",
         "Ein, der letzte Versuch ist fehlgeschlagen"),
    "threesea-he-leave-device-group":
        ("Leave",
         "Verlassen"),
    "threesea-la-leave-device-group-text":
        ("This device connects directly again. The other devices keep your ID and stay in their group, and only they can remove themselves from then on.",
         "Dieses Gerät verbindet sich wieder direkt. Die anderen Geräte behalten deine ID und bleiben in ihrer Gruppe, und nur sie selbst können sich ab dann entfernen."),
    "threesea-he-link-desktop":
        ("Link Threema for Desktop",
         "Threema für Desktop verknüpfen"),
    "threesea-he-link-next":
        ("Next",
         "Weiter"),
    "threesea-la-link-grant":
        ("Threema for Desktop gets your Threema ID with its private key, your contacts and groups, and can read and send messages as you. Link only your own computer.",
         "Threema für Desktop bekommt deine Threema-ID mit ihrem privaten Schlüssel, deine Kontakte und Gruppen und kann in deinem Namen Nachrichten lesen und senden. Verknüpfe nur deinen eigenen Computer."),
    "threesea-la-link-pfs-if-on":
        ("While a device is linked, chats with your contacts use end-to-end encryption without Perfect Forward Secrecy. If it was on, it comes back when the last linked device is removed.",
         "Solange ein Gerät verknüpft ist, sind Chats mit deinen Kontakten Ende-zu-Ende-verschlüsselt, aber ohne Perfect Forward Secrecy. War sie eingeschaltet, kommt sie zurück, wenn das letzte verknüpfte Gerät entfernt ist."),
    "threesea-me-link-scan-again":
        ("Scan again",
         "Erneut scannen"),
    "threesea-me-clear-reflected-left-out":
        ("Clear count of messages not kept",
         "Zähler der nicht behaltenen Nachrichten löschen"),
    "threesea-la-reflected-left-out":
        ("%n message(s) received on your other devices weren't kept on this phone.",
         ("%n Nachricht, die deine anderen Geräte empfangen haben, wurde auf diesem Telefon nicht behalten.",
          "%n Nachrichten, die deine anderen Geräte empfangen haben, wurden auf diesem Telefon nicht behalten.")),
    "threesea-la-linked-devices-offline":
        ("Waiting for the connection",
         "Wartet auf die Verbindung"),
    "threesea-la-linked-devices-failed":
        ("The list of linked devices did not load",
         "Die Liste der verknüpften Geräte wurde nicht geladen"),
    "threesea-la-linked-devices-offline-hint":
        ("The linked devices appear here once the server answers.",
         "Die verknüpften Geräte erscheinen hier, sobald der Server antwortet."),
    "threesea-la-linked-devices-failed-hint":
        ("Open this page again to load it.",
         "Öffne diese Seite erneut, um sie zu laden."),
    "threesea-la-log-save-failed":
        ("Could not save the support package to Downloads.",
         "Das Supportpaket konnte nicht in Downloads gespeichert werden."),
    "threesea-la-keep-messages-remorse":
        ("Deleting older messages",
         "Ältere Nachrichten werden gelöscht"),
    "threesea-la-cleaning-up":
        ("Cleaning up",
         "Wird aufgeräumt"),
    "threesea-la-notifications-calls":
        ("Notify about new messages and missed calls",
         "Über neue Nachrichten und verpasste Anrufe benachrichtigen"),
    "threesea-la-notification-preview-who-description":
        ("Otherwise notifications show who wrote, but not what, e.g. on the lock screen.",
         "Sonst zeigen Benachrichtigungen, wer geschrieben hat, aber nicht was, z. B. auf dem Sperrbildschirm."),
    "threesea-se-notifications-calls":
        ("Calls",
         "Anrufe"),
    "threesea-la-call-tone":
        ("Ringtone",
         "Klingelton"),
    "threesea-la-call-tone-system":
        ("System call tone",
         "Klingelton des Systems"),
    "threesea-la-inapp-sound-voice-description":
        ("A soft sound between two voice messages playing one after the other.",
         "Ein leiser Ton zwischen zwei Sprachnachrichten, die nacheinander laufen."),
    "threesea-la-exclusion-list-description":
        ("Finding contacts from the address book will not add or rename these Threema IDs.",
         "Beim Finden von Kontakten aus dem Adressbuch werden diese Threema-IDs weder hinzugefügt noch umbenannt."),
    "threesea-la-clear-contact-policies-description":
        ("The read receipt and typing indicator choices made on single contacts' pages go back to the settings above.",
         "Was auf den Seiten einzelner Kontakte für Lesebestätigungen und „Melden, wenn ich tippe“ gewählt wurde, geht zurück auf die Einstellungen oben."),
    "threesea-la-contact-policies-cleared":
        ("Cleared for %n contact(s)",
         ("Für %n Kontakt gelöscht",
          "Für %n Kontakte gelöscht")),
    "threesea-la-forward-secrecy-linked":
        ("Off while other devices are linked to this ID. Your choice here applies again when the last one is removed.",
         "Aus, solange andere Geräte mit dieser ID verknüpft sind. Deine Wahl hier gilt wieder, wenn das letzte entfernt ist."),
    "threesea-la-removing-link":
        ("Removing link",
         "Verknüpfung wird entfernt"),
    "threesea-la-cancelling-link":
        ("Cancelling link",
         "Verknüpfen wird abgebrochen"),
    "threesea-me-share-my-id":
        ("Share my ID",
         "Meine ID teilen"),
    "threesea-la-revocation-not-checked":
        ("Not checked yet",
         "Noch nicht geprüft"),
    "threesea-la-data-passphrase-about":
        ("Your messages are stored encrypted with a key that is kept encrypted with this passphrase. Threesea asks for it every time it starts, also after restarting the phone or deleting its data folder, and only this passphrase opens it. It receives messages once it is entered. If you forget it, pull down where it is asked for to start fresh, without what Threesea has stored on this phone: Threema Safe or an ID backup brings your ID back, a Data backup your chats.",
         "Deine Nachrichten werden mit einem Schlüssel verschlüsselt gespeichert, der mit dieser Passphrase verschlüsselt ist. Threesea fragt bei jedem Start danach, auch nach einem Neustart des Telefons oder dem Löschen seines Datenordners, und nur diese Passphrase öffnet ihn. Nachrichten empfängt es, sobald sie eingegeben ist. Wenn du sie vergisst, ziehe dort, wo sie abgefragt wird, nach unten, um ohne das, was Threesea auf diesem Telefon gespeichert hat, neu anzufangen: Threema Safe oder ein ID-Backup bringt deine ID zurück, ein Daten-Backup deine Chats."),
    "threesea-la-web-pair-grants":
        ("This code connects a browser to your Threema ID. That browser can read your contacts and chats and send messages in your name.",
         "Dieser Code verbindet einen Browser mit deiner Threema-ID. Dieser Browser kann deine Kontakte und Chats lesen und in deinem Namen Nachrichten senden."),
    "threesea-la-web-pair-only-yours":
        ("Allow it only if you opened Threema Web yourself and the code is on your own screen. If someone sent you this code, go back.",
         "Erlaube es nur, wenn du Threema Web selbst geöffnet hast und der Code auf deinem eigenen Bildschirm ist. Hat dir jemand diesen Code geschickt, geh zurück."),
    "threesea-la-web-pair-remembered":
        ("If the browser asks to be remembered, it can connect again without a new code until you remove it from Threema Web in the settings.",
         "Bittet der Browser darum, sich ihn zu merken, kann er sich ohne neuen Code wieder verbinden, bis du ihn in den Einstellungen unter Threema Web entfernst."),
    "threesea-he-rename-browser":
        ("Rename browser",
         "Browser umbenennen"),
    "threesea-la-browser-name":
        ("Browser name",
         "Name des Browsers"),
    "threesea-la-browser-name-description":
        ("Only this phone shows this name. Left empty, the browser's own name is shown.",
         "Nur dieses Telefon zeigt diesen Namen. Bleibt er leer, wird der eigene Name des Browsers gezeigt."),
    "threesea-la-web-scan-again":
        ("The new certificate is trusted. Scan the code in the browser again to connect it.",
         "Dem neuen Zertifikat wird vertraut. Scanne den Code im Browser noch einmal, um ihn zu verbinden."),
    "threesea-la-web-disconnected-kept":
        ("Threema Web disconnected. Tap the browser to connect it again.",
         "Threema Web getrennt. Tippe auf den Browser, um ihn wieder zu verbinden."),
    "threesea-la-web-disconnected":
        ("Threema Web disconnected.",
         "Threema Web getrennt."),
    "threesea-la-web-removing":
        ("Removing",
         "Wird entfernt"),
    "threesea-me-web-remove-all":
        ("Remove all browsers",
         "Alle Browser entfernen"),
    "threesea-la-web-removing-all":
        ("Removing all browsers",
         "Alle Browser werden entfernt"),
    "threesea-la-web-enabled":
        ("Threema Web",
         "Threema Web"),
    "threesea-la-web-enabled-description":
        ("Lets the browsers listed here connect to this phone and use your chats. Turning it off disconnects them and keeps them in the list.",
         "Lässt die hier aufgeführten Browser sich mit diesem Telefon verbinden und deine Chats verwenden. Ausschalten trennt sie und behält sie in der Liste."),
    "threesea-la-web-tap-hint":
        ("Tap a browser to connect or disconnect it. Threema Web must be open in the browser to connect. A disconnected browser can connect again when it is opened; removing it or turning off the switch above keeps it out.",
         "Tippe auf einen Browser, um ihn zu verbinden oder zu trennen. Zum Verbinden muss Threema Web im Browser offen sein. Ein getrennter Browser kann sich wieder verbinden, wenn er geöffnet wird; Entfernen oder der Schalter oben hält ihn fern."),
    "threesea-me-web-rename":
        ("Rename",
         "Umbenennen"),
    "threesea-se-web-servers":
        ("Allowed servers",
         "Erlaubte Server"),
    "threesea-la-web-servers-description":
        ("Self-hosted Threema Web servers you allowed. Removing one also removes its browsers, and the next code naming it asks again.",
         "Selbst betriebene Threema-Web-Server, die du erlaubt hast. Wenn du einen entfernst, werden auch seine Browser entfernt, und beim nächsten Code, der ihn nennt, wirst du wieder gefragt."),
    "threesea-la-share-refused-several":
        ("Nothing was sent. Blocked or revoked, and taken off the choice: %1",
         "Nichts wurde gesendet. Blockiert oder widerrufen und aus der Auswahl genommen: %1"),
    "threesea-me-choose-several-chats":
        ("Choose several chats",
         "Mehrere Chats wählen"),
    "threesea-la-startup-try-again":
        ("Pull down to try again.",
         "Ziehe nach unten, um es erneut zu versuchen."),
    "threesea-la-startup-code":
        ("Code: %1",
         "Code: %1"),
    "threesea-la-removal-stuck":
        ("Removing the ID did not finish",
         "Das Entfernen der ID wurde nicht fertig"),
    "threesea-la-removal-stuck-hint":
        ("Pull down to try again, or close Threesea and open it again (%1).",
         "Ziehe nach unten, um es erneut zu versuchen, oder schließe Threesea und öffne es wieder (%1)."),
    "threesea-he-enter-data-passphrase":
        ("Enter the passphrase of your stored data",
         "Gib die Passphrase deiner gespeicherten Daten ein"),
    "threesea-he-video-keeps-metadata-send":
        ("Send as it is",
         "So senden"),
    "threesea-he-video-keeps-metadata-cancel":
        ("Don't send",
         "Nicht senden"),
    "threesea-la-video-keeps-metadata":
        ("This video keeps what its file says about it",
         "Dieses Video behält, was seine Datei über es verrät"),
    "threesea-la-picture-keeps-metadata":
        ("This picture keeps what its file says about it",
         "Dieses Bild behält, was seine Datei über es verrät"),
    "threesea-la-video-keeps-metadata-description":
        ("Threesea can't remove the metadata of this file. It can hold where and when the video was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file.",
         "Threesea kann die Metadaten dieser Datei nicht entfernen. Sie können zeigen, wo, wann und mit welchem Gerät das Video entstanden ist, und der Empfänger bekommt sie alle, dazu den Namen der Datei: %1. So gesendet, geht es als einfache Datei hinaus."),
    "threesea-la-picture-keeps-metadata-description":
        ("Threesea can't remove the metadata of this file. It can hold where and when the picture was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file.",
         "Threesea kann die Metadaten dieser Datei nicht entfernen. Sie können zeigen, wo, wann und mit welchem Gerät das Bild entstanden ist, und der Empfänger bekommt sie alle, dazu den Namen der Datei: %1. So gesendet, geht es als einfache Datei hinaus."),
    "threesea-la-ended-early":
        ("Threesea ended before it was done",
         "Threesea wurde beendet, bevor es fertig war"),
    "threesea-la-ended-early-body":
        ("Your last files or draft did not reach the chat. Add them again there.",
         "Deine letzten Dateien oder dein Entwurf sind nicht im Chat angekommen. Füge sie dort noch einmal hinzu."),
    "threesea-la-error-share-too-large":
        ("That is too much to share at once. Select fewer files or smaller ones, or try again in a few minutes.",
         "Das ist zu viel, um es auf einmal zu teilen. Wähle weniger oder kleinere Dateien, oder versuche es in ein paar Minuten erneut."),
    "threesea-la-verification-partly":
        ("Partly verified",
         "Teilweise überprüft"),
    "threesea-la-credit-cldr":
        ("Unicode CLDR: those search terms are likely derived from its emoji annotations (Unicode License v3).",
         "Unicode CLDR: Diese Suchbegriffe stammen wahrscheinlich aus seinen Emoji-Annotationen (Unicode License v3)."),
    "threesea-la-verification-partly-explained":
        ("Partly verified: found in your address book, or checked with their code from a picture. Scanning their code with the camera, in person, verifies them fully.",
         "Teilweise überprüft: im Adressbuch gefunden oder mit dem Code aus einem Bild geprüft. Wenn du den Code persönlich mit der Kamera scannst, ist der Kontakt vollständig überprüft."),
    "threesea-la-qr-verified-picture":
        ("The code in the picture matches %1's key. For full verification, scan their code with the camera, in person.",
         "Der Code im Bild passt zum Schlüssel von %1. Für die vollständige Überprüfung scanne den Code persönlich mit der Kamera."),
    "threesea-he-remove-devices-again":
        ("Remove",
         "Entfernen"),
    "threesea-he-leave-device-group-choice":
        ("Your linked devices",
         "Deine verknüpften Geräte"),
    "threesea-la-leave-device-group-not-finished":
        ("Removing your linked devices did not finish, so they may still be linked.",
         "Das Entfernen deiner verknüpften Geräte wurde nicht fertig, sie sind also vielleicht noch verknüpft."),
    "threesea-la-remove-devices-first":
        ("Remove the linked devices first",
         "Zuerst die verknüpften Geräte entfernen"),
    "threesea-la-remove-devices-first-description":
        ("Tries again now, best once you are online. This device stays in the group until they are removed, so it can still remove a lost or stolen computer.",
         "Versucht es jetzt erneut, am besten, wenn du online bist. Dieses Gerät bleibt in der Gruppe, bis sie entfernt sind, und kann so noch einen verlorenen oder gestohlenen Computer entfernen."),
    "threesea-la-leave-device-group-only":
        ("Leave on this device only",
         "Nur auf diesem Gerät verlassen"),
    "threesea-la-linked-devices-read-failed":
        ("The list of linked devices could not be read",
         "Die Liste der verknüpften Geräte konnte nicht gelesen werden"),
    "threesea-la-image-cache":
        ("Keep pictures in memory",
         "Bilder im Speicher behalten"),
    "threesea-la-image-cache-description":
        ("Chat pictures scroll back in faster: they stay decoded and unencrypted in the app's memory while it runs, until the app locks or a chat is made private. This uses more memory. Private chats load their pictures anew each time.",
         "Chatbilder erscheinen beim Zurückscrollen schneller: Sie bleiben, solange die App läuft, entschlüsselt im Arbeitsspeicher der App, bis sie sich sperrt oder ein Chat privat gemacht wird. Das braucht mehr Speicher. Private Chats laden ihre Bilder jedes Mal neu."),
    "threesea-he-start-fresh":
        ("Start fresh",
         "Neu anfangen"),
    "threesea-la-start-fresh-goes":
        ("This deletes Threesea's own data on this phone: chats, contacts, media, settings and your Threema ID. Backups and pictures saved to your folders stay.",
         "Damit werden Threeseas eigene Daten auf diesem Telefon gelöscht: Chats, Kontakte, Medien, Einstellungen und deine Threema-ID. Backups und Bilder, die in deinen Ordnern gespeichert sind, bleiben."),
    "threesea-la-start-fresh-comes-back":
        ("You can then restore your ID from Threema Safe, an ID backup or a Data backup, and your chats from a Data backup. Without one of them, the ID is lost.",
         "Danach kannst du deine ID aus Threema Safe, einem ID-Backup oder einem Daten-Backup wiederherstellen und deine Chats aus einem Daten-Backup. Ohne eines davon ist die ID verloren."),
    "threesea-la-start-fresh-linked":
        ("Devices linked to your ID stay linked to it.",
         "Mit deiner ID verknüpfte Geräte bleiben mit ihr verknüpft."),
    "threesea-la-start-fresh-understood":
        ("Delete the stored data",
         "Die gespeicherten Daten löschen"),
    "threesea-la-starting-fresh":
        ("Deleting the stored data",
         "Gespeicherte Daten werden gelöscht"),
    "threesea-la-startup-try-again-or-fresh":
        ("Pull down to try again, or to start fresh without the stored data.",
         "Ziehe nach unten, um es erneut zu versuchen oder ohne die gespeicherten Daten neu anzufangen."),
    "threesea-me-start-fresh":
        ("Start fresh",
         "Neu anfangen"),
    "threesea-la-data-passphrase-forgotten":
        ("Forgotten it? Pull down to start fresh without the stored data.",
         "Vergessen? Ziehe nach unten, um ohne die gespeicherten Daten neu anzufangen."),
    "threesea-la-credit-maplibre":
        ("The map: MapLibre Native and its Qt bindings (BSD-2-Clause), mapbox-gl-qml by rinigus (LGPL-3.0) and his Sailfish packaging pkg-maplibre-gl-native (MIT). Their exact sources: github.com/maplibre/maplibre-native-qt 3.0.0, github.com/rinigus/pkg-maplibre-gl-native 3.0.0+git2 and github.com/rinigus/mapbox-gl-qml 3.2.1, with this app's patches in its source under scripts/maplibre.",
         "Die Karte: MapLibre Native und seine Qt-Anbindung (BSD-2-Clause), mapbox-gl-qml von rinigus (LGPL-3.0) und seine Sailfish-Pakete pkg-maplibre-gl-native (MIT). Ihre genauen Quellen: github.com/maplibre/maplibre-native-qt 3.0.0, github.com/rinigus/pkg-maplibre-gl-native 3.0.0+git2 und github.com/rinigus/mapbox-gl-qml 3.2.1, mit den Patches dieser App in ihrem Quellcode unter scripts/maplibre."),
    "threesea-la-credit-map-data":
        ("Map data © OpenMapTiles © OpenStreetMap contributors (ODbL), from Threema's map server.",
         "Kartendaten © OpenMapTiles © OpenStreetMap-Mitwirkende (ODbL), vom Kartenserver von Threema."),
    "threesea-me-open-in-maps":
        ("Open in maps app",
         "In Maps-App öffnen"),
    "threesea-la-no-maps-app-installed":
        ("No maps app installed",
         "Keine Maps-App installiert"),
    "threesea-me-copy-coordinates":
        ("Copy coordinates",
         "Koordinaten kopieren"),
    "threesea-he-choose":
        ("Choose",
         "Auswählen"),
    "threesea-la-location-map-point":
        ("A point on the map",
         "Ein Punkt auf der Karte"),
    "threesea-la-location-on-map":
        ("Choose on the map",
         "Auf der Karte wählen"),
    "threesea-la-location-map-server":
        ("The map comes from Threema's map server, which learns the area shown. The places on it come from Threema's places server, which gets the middle of the map.",
         "Die Karte kommt vom Kartenserver von Threema, der den gezeigten Ausschnitt erfährt. Die Orte darauf kommen vom Ortsserver von Threema, der die Mitte der Karte bekommt."),
}

# The engineering-English base file: only the plural forms need filling in.
EN_PLURALS = {
    "threesea-la-member-count":
        ("%n member(s)",
         ("%n member",
          "%n members")),
    "threesea-la-poll-votes":
        ("%n vote(s)",
         ("%n vote",
          "%n votes")),
    "threesea-la-share-files":
        ("%n file(s)",
         ("%n file",
          "%n files")),
    "threesea-la-address-book-added":
        ("%n new contact(s) from your address book",
         ("%n new contact from your address book",
          "%n new contacts from your address book")),
    "threesea-he-n-selected":
        ("%n selected",
         ("%n selected",
          "%n selected")),
    "threesea-la-deleting-messages":
        ("Deleting %n message(s)",
         ("Deleting %n message",
          "Deleting %n messages")),
    "threesea-la-n-messages":
        ("%n message(s)",
         ("%n message",
          "%n messages")),
    "threesea-la-recipient-count":
        ("%n recipient(s)",
         ("%n recipient",
          "%n recipients")),
    "threesea-la-share-selected-count":
        ("%n selected contact(s)",
         ("%n selected contact",
          "%n selected contacts")),
    "threesea-la-group-call-participants":
        ("%n participant(s)",
         ("%n participant",
          "%n participants")),
    "threesea-la-pin-paused":
        ("Too many wrong attempts. Try again in %n second(s).",
         ("Too many wrong attempts. Try again in %n second.",
          "Too many wrong attempts. Try again in %n seconds.")),
    "threesea-la-n-contacts":
        ("%n contact(s)",
         ("%n contact",
          "%n contacts")),
    "threesea-la-n-groups":
        ("%n group(s)",
         ("%n group",
          "%n groups")),
    "threesea-la-n-lists":
        ("%n distribution list(s)",
         ("%n distribution list",
          "%n distribution lists")),
    "threesea-la-n-backups":
        ("%n backup(s)",
         ("%n backup",
          "%n backups")),
    "threesea-la-address-book-none-new":
        ("%n address book entries checked, no new contacts.",
         ("%n address book entry checked, no new contacts.",
          "%n address book entries checked, no new contacts.")),
    "threesea-la-saved-n-downloads":
        ("Saved %n file(s) to Downloads",
         ("Saved %n file to Downloads",
          "Saved %n files to Downloads")),
    "threesea-la-not-sent-to-chats":
        ("Not everything went to %n chat(s)",
         ("Not everything went to %n chat",
          "Not everything went to %n chats")),
    "threesea-la-sent-to-chats":
        ("On its way to %n chat(s)",
         ("On its way to %n chat",
          "On its way to %n chats")),
    "threesea-la-safe-failing-notification-body":
        ("Threema Safe hasn't been able to back up for %n day(s). Tap to check.",
         ("Threema Safe hasn't been able to back up for %n day. Tap to check.",
          "Threema Safe hasn't been able to back up for %n days. Tap to check.")),
    "threesea-la-web-connected-notification-body":
        ("%n browser(s) can read and send your messages through this phone. Tap to manage.",
         ("%n browser can read and send your messages through this phone. Tap to manage.",
          "%n browsers can read and send your messages through this phone. Tap to manage.")),
    "threesea-la-polls-waiting":
        ("%n poll(s) waiting for your vote",
         ("%n poll waiting for your vote",
          "%n polls waiting for your vote")),
    "threesea-la-android-export-left-out":
        ("%n chat(s) stayed out of the last backup: restoring them needs the contact's key, which could not be fetched. The next backup tries again.",
         ("%n chat stayed out of the last backup: restoring it needs the contact's key, which could not be fetched. The next backup tries again.",
          "%n chats stayed out of the last backup: restoring them needs the contact's key, which could not be fetched. The next backup tries again.")),
    "threesea-la-android-export-files-left-out":
        ("%n file(s) stayed out of the last backup: they could not be read on this phone.",
         ("%n file stayed out of the last backup: it could not be read on this phone.",
          "%n files stayed out of the last backup: they could not be read on this phone.")),
    "threesea-la-reflected-left-out":
        ("%n message(s) received on your other devices weren't kept on this phone.",
         ("%n message received on your other devices wasn't kept on this phone.",
          "%n messages received on your other devices weren't kept on this phone.")),
    "threesea-la-contact-policies-cleared":
        ("Cleared for %n contact(s)",
         ("Cleared for %n contact",
          "Cleared for %n contacts")),
}


def fill(path, table, plurals_only=False):
    tree = ET.parse(path)
    missing, stale = [], []
    for message in tree.iter("message"):
        key = message.get("id")
        entry = table.get(key)
        translation = message.find("translation")
        if entry is None:
            if not plurals_only and translation.get("type") == "unfinished":
                missing.append(key)
            continue
        english, text = entry
        if english != message.findtext("source"):
            translation.set("type", "unfinished")
            stale.append(key)
            continue
        for child in list(translation):
            translation.remove(child)
        translation.text = None
        if isinstance(text, tuple):
            message.set("numerus", "yes")
            for form in text:
                ET.SubElement(translation, "numerusform").text = form
        else:
            translation.text = text
        translation.attrib.pop("type", None)
    ET.indent(tree, "    ")
    with open(path, "w", encoding="utf-8") as out:
        out.write('<?xml version="1.0" encoding="utf-8"?>\n<!DOCTYPE TS>\n')
        out.write(ET.tostring(tree.getroot(), encoding="unicode"))
        out.write("\n")
    return missing, stale


missing, stale = fill("translations/harbour-threesea-de.ts", DE)
_, stale_en = fill("translations/harbour-threesea.ts", EN_PLURALS, plurals_only=True)
for key in stale + stale_en:
    print(f"English changed, translate again: {key}", file=sys.stderr)
# New texts are translated at the very end (user decision 2026-10-22): until then they only warn,
# and scripts/check.sh --release fails on them.
if missing:
    print(f"{len(missing)} texts without German yet (left unfinished)", file=sys.stderr)
