#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# French texts for translations/harbour-threesea-fr.ts. Fills in what this table knows, keeps
# everything else as it is, so lupdate's new entries can be added here. Run after
# scripts/update-translations.sh, as the last step (it writes the file in the repository's
# layout). Style: Sailfish OS French (vouvoiement, typographie française: fill() puts a
# non-breaking space before : ; ! ? » and after «), Threema's terms (ID Threema, Threema Safe,
# accusés de lecture, sondage, transférer, débloquer, surnom, liste de distribution, appareils
# associés; a phone number or e-mail address is lié).
#
# Each entry holds the English it was translated from, then the French. When the English in the
# .ts no longer matches, the text stays unfinished and is listed: a reworded text never keeps a
# translation of what it used to say. Translate it again and put the new English beside it.
import re
import sys
import xml.etree.ElementTree as ET

FR = {
    "threesea-la-error-network":
        ("Could not reach the Threema servers. Check your internet connection and try again.",
         "Les serveurs Threema sont injoignables. Vérifiez votre connexion Internet et réessayez."),
    "threesea-la-error-license-format":
        ("A license key looks like ABCDE-12345.",
         "Une clé de licence ressemble à ceci : ABCDE-12345."),
    "threesea-la-error-backup-decrypt":
        ("Wrong password, or the backup is damaged.",
         "Mot de passe incorrect, ou la sauvegarde est endommagée."),
    "threesea-la-error-password-short":
        ("The password needs at least 8 characters.",
         "Le mot de passe doit comporter au moins 8 caractères."),
    "threesea-la-error-password-long":
        ("The password is too long.",
         "Le mot de passe est trop long."),
    "threesea-la-error-safe-not-found":
        ("No Threema Safe backup found for this Threema ID and password.",
         "Aucune sauvegarde Threema Safe pour cet ID Threema et ce mot de passe."),
    "threesea-la-error-safe-too-large":
        ("The backup is larger than the Threema Safe server allows.",
         "La sauvegarde dépasse la taille autorisée par le serveur Threema Safe."),
    "threesea-la-error-identity-format":
        ("A Threema ID has 8 characters, letters and digits.",
         "Un ID Threema compte 8 caractères, lettres et chiffres."),
    "threesea-la-error-contact-invalid":
        ("This Threema ID does not exist or was revoked.",
         "Cet ID Threema n’existe pas ou a été révoqué."),
    "threesea-la-error-contact-self":
        ("This is your own Threema ID.",
         "C’est votre propre ID Threema."),
    "threesea-la-error-no-license":
        ("Enter your license key first.",
         "Saisissez d’abord votre clé de licence."),
    "threesea-la-verification-full":
        ("Fully verified",
         "Entièrement vérifié"),
    "threesea-la-verification-server":
        ("Verified by the server",
         "Vérifié par le serveur"),
    "threesea-la-verification-none":
        ("Not verified",
         "Non vérifié"),
    "threesea-la-cover-no-new":
        ("No new messages",
         "Aucun nouveau message"),
    "threesea-he-about":
        ("About",
         "À propos"),
    "threesea-la-version":
        ("Version",
         "Version"),
    "threesea-se-license":
        ("License",
         "Licence"),
    "threesea-la-license-text":
        ("An unofficial Threema client for Sailfish OS, licensed under the GNU AGPL v3. Uses libthreema by Threema AG. Requires a Threema Libre license from the Threema Shop. Not affiliated with or supported by Threema AG.",
         "Un client Threema non officiel pour Sailfish OS, sous licence GNU AGPL v3. Utilise libthreema de Threema AG. Nécessite une licence Threema Libre achetée dans la boutique Threema. Sans lien avec Threema AG, ni soutenu par elle."),
    "threesea-he-add":
        ("Add",
         "Ajouter"),
    "threesea-la-add-contact-intro":
        ("Enter the Threema ID of the person you want to add. Meet in person and scan each other's QR code later to fully verify the contact.",
         "Saisissez l’ID Threema de la personne à ajouter. Plus tard, vous pourrez vous rencontrer en personne et scanner vos codes QR respectifs pour vérifier entièrement le contact."),
    "threesea-la-threema-id":
        ("Threema ID",
         "ID Threema"),
    "threesea-me-delete":
        ("Delete",
         "Supprimer"),
    "threesea-la-deleting-contact":
        ("Deleting contact and chat",
         "Suppression du contact et de la discussion"),
    "threesea-me-edit":
        ("Edit",
         "Modifier"),
    "threesea-la-nickname":
        ("Nickname",
         "Surnom"),
    "threesea-la-type":
        ("Type",
         "Type"),
    "threesea-la-threema-work":
        ("Threema Work",
         "Threema Work"),
    "threesea-la-status":
        ("Status",
         "État"),
    "threesea-la-revoked":
        ("Revoked",
         "Révoqué"),
    "threesea-la-inactive":
        ("Inactive",
         "Inactif"),
    "threesea-se-public-key":
        ("Public key",
         "Clé publique"),
    "threesea-me-add-contact":
        ("Add contact",
         "Ajouter un contact"),
    "threesea-he-contacts":
        ("Contacts",
         "Contacts"),
    "threesea-la-no-contacts":
        ("No contacts",
         "Aucun contact"),
    "threesea-la-no-contacts-hint":
        ("Pull down to add a contact by Threema ID",
         "Tirez vers le bas pour ajouter un contact par ID Threema"),
    "threesea-me-settings":
        ("Settings",
         "Paramètres"),
    "threesea-he-chats":
        ("Chats",
         "Discussions"),
    "threesea-la-no-chats":
        ("No chats yet",
         "Aucune discussion"),
    "threesea-la-no-chats-hint":
        ("Swipe left to choose a contact",
         "Balayez vers la gauche pour choisir un contact"),
    "threesea-he-remove":
        ("Remove",
         "Supprimer"),
    "threesea-he-save":
        ("Save",
         "Enregistrer"),
    "threesea-la-first-name":
        ("First name",
         "Prénom"),
    "threesea-la-last-name":
        ("Last name",
         "Nom"),
    "threesea-he-export-backup":
        ("Export ID backup",
         "Exporter la sauvegarde d’ID"),
    "threesea-la-export-intro":
        ("An ID backup lets you restore your Threema ID on another device. Keep it and its password safe: anyone with both can use your ID.",
         "Une sauvegarde d’ID vous permet de restaurer votre ID Threema sur un autre appareil. Conservez-la ainsi que son mot de passe en lieu sûr : toute personne qui a les deux peut utiliser votre ID."),
    "threesea-la-new-backup-password":
        ("Password (at least 8 characters)",
         "Mot de passe (au moins 8 caractères)"),
    "threesea-la-repeat-password":
        ("Repeat password",
         "Répétez le mot de passe"),
    "threesea-bt-create-backup":
        ("Create backup",
         "Créer la sauvegarde"),
    "threesea-bt-copy":
        ("Copy to clipboard",
         "Copier dans le presse-papiers"),
    "threesea-he-identity-setup":
        ("Your Threema ID",
         "Votre ID Threema"),
    "threesea-bt-create-identity":
        ("Create new ID",
         "Créer un ID"),
    "threesea-bt-restore-safe":
        ("Restore from Threema Safe",
         "Restaurer depuis Threema Safe"),
    "threesea-bt-restore-backup":
        ("Restore ID backup",
         "Restaurer une sauvegarde d’ID"),
    "threesea-he-welcome":
        ("Welcome to Threesea",
         "Bienvenue dans Threesea"),
    "threesea-la-license-intro":
        ("Threesea is an unofficial Threema client for Sailfish OS. It needs a Threema Libre license key, which you can buy in the Threema Shop (shop.threema.ch).",
         "Threesea est un client Threema non officiel pour Sailfish OS. Il lui faut une clé de licence Threema Libre, que vous pouvez acheter dans la boutique Threema (shop.threema.ch)."),
    "threesea-la-license-key":
        ("License key",
         "Clé de licence"),
    "threesea-bt-check-license":
        ("Check license",
         "Vérifier la licence"),
    "threesea-he-restore-backup":
        ("Restore ID backup",
         "Restaurer une sauvegarde d’ID"),
    "threesea-la-id-backup":
        ("ID backup",
         "Sauvegarde d’ID"),
    "threesea-la-backup-password":
        ("Backup password",
         "Mot de passe de la sauvegarde"),
    "threesea-bt-restore":
        ("Restore",
         "Restaurer"),
    "threesea-me-safe-change-password":
        ("Change password",
         "Changer le mot de passe"),
    "threesea-me-safe-backup-now":
        ("Back up now",
         "Sauvegarder maintenant"),
    "threesea-la-threema-safe":
        ("Threema Safe",
         "Threema Safe"),
    "threesea-se-status":
        ("Status",
         "État"),
    "threesea-la-safe-last-backup":
        ("Last backup",
         "Dernière sauvegarde"),
    "threesea-la-safe-not-yet":
        ("Not yet",
         "Pas encore"),
    "threesea-la-safe-size":
        ("Size",
         "Taille"),
    "threesea-la-safe-server":
        ("Server",
         "Serveur"),
    "threesea-la-safe-retention":
        ("Kept on the server",
         "Conservée sur le serveur"),
    "threesea-la-days":
        ("%1 days",
         "%1 jours"),
    "threesea-he-enable":
        ("Enable",
         "Activer"),
    "threesea-la-password":
        ("Password",
         "Mot de passe"),
    "threesea-la-safe-password-weak":
        ("This password is easy to guess. Choose a stronger one, or accept to use it anyway.",
         "Ce mot de passe est facile à deviner. Choisissez-en un plus solide, ou acceptez pour l’utiliser quand même."),
    "threesea-he-restore-safe":
        ("Restore from Threema Safe",
         "Restaurer depuis Threema Safe"),
    "threesea-la-safe-password":
        ("Threema Safe password",
         "Mot de passe Threema Safe"),
    "threesea-la-safe-deriving":
        ("Unlocking the backup takes a moment.",
         "Le déverrouillage de la sauvegarde prend un moment."),
    "threesea-me-about":
        ("About",
         "À propos"),
    "threesea-he-settings":
        ("Settings",
         "Paramètres"),
    "threesea-se-profile":
        ("My profile",
         "Mon profil"),
    "threesea-se-privacy":
        ("Privacy",
         "Confidentialité"),
    "threesea-la-send-read-receipts":
        ("Send read receipts",
         "Envoyer les accusés de lecture"),
    "threesea-la-send-read-receipts-description":
        ("Let people know when you have read their messages.",
         "Indique aux autres quand vous avez lu leurs messages."),
    "threesea-la-send-typing":
        ("Send typing indicator",
         "Envoyer l’indicateur de saisie"),
    "threesea-la-send-typing-description":
        ("Let people see when you are writing to them.",
         "Montre aux autres quand vous êtes en train de leur écrire."),
    "threesea-la-block-unknown":
        ("Block unknown senders",
         "Bloquer les expéditeurs inconnus"),
    "threesea-la-block-unknown-description":
        ("Ignore messages from people who are not in your contacts.",
         "Ignorer les messages des personnes absentes de vos contacts."),
    "threesea-se-backup":
        ("Backup",
         "Sauvegarde"),
    "threesea-la-on":
        ("On",
         "Activé"),
    "threesea-la-off":
        ("Off",
         "Désactivé"),
    "threesea-la-export-backup":
        ("Export ID backup",
         "Exporter la sauvegarde d’ID"),
    "threesea-la-startup-failed":
        ("Threesea could not start",
         "Threesea n’a pas pu démarrer"),
    "threesea-la-location":
        ("Location",
         "Position"),
    "threesea-me-copy":
        ("Copy",
         "Copier"),
    "threesea-la-cover-unread-one":
        ("unread message",
         "message non lu"),
    "threesea-la-cover-unread-many":
        ("unread messages",
         "messages non lus"),
    "threesea-la-typing":
        ("typing…",
         "écrit…"),
    "threesea-la-offline-queue":
        ("Offline – messages will be sent later",
         "Hors ligne – les messages partiront plus tard"),
    "threesea-la-connecting":
        ("Connecting…",
         "Connexion…"),
    "threesea-la-offline":
        ("Offline",
         "Hors ligne"),
    "threesea-me-delete-chat":
        ("Delete chat",
         "Supprimer la discussion"),
    "threesea-la-deleting-chat":
        ("Deleting chat",
         "Suppression de la discussion"),
    "threesea-la-keep-running":
        ("Keep running in the background",
         "Garder actif en arrière-plan"),
    "threesea-la-keep-running-description":
        ("Threesea stays connected after you close it, so new messages arrive right away. After you turn on the phone, it connects once you open it. Uses more battery.",
         "Threesea reste connecté après sa fermeture, pour que les nouveaux messages arrivent tout de suite. Après l’allumage du téléphone, il se connecte dès que vous l’ouvrez. Consomme davantage de batterie."),
    "threesea-la-status-fs-established":
        ("Perfect Forward Secrecy is active for this chat.",
         "La confidentialité persistante est active pour cette discussion."),
    "threesea-la-today":
        ("Today",
         "Aujourd’hui"),
    "threesea-la-yesterday":
        ("Yesterday",
         "Hier"),
    "threesea-la-type-message":
        ("Type message",
         "Écrire un message"),
    "threesea-la-error-reaction-not-supported":
        ("This contact's app only understands thumbs up and thumbs down.",
         "L’application de ce contact ne comprend que le pouce levé et le pouce baissé."),
    "threesea-la-error-not-supported-by-contact":
        ("The app on the other side doesn't support this.",
         "L’application de l’autre côté ne prend pas cela en charge."),
    "threesea-la-error-message-too-old":
        ("Messages can be edited or deleted for everyone for 6 hours after sending.",
         "Les messages peuvent être modifiés ou supprimés pour tous pendant 6 heures après leur envoi."),
    "threesea-la-message-deleted":
        ("Message deleted",
         "Message supprimé"),
    "threesea-la-edited":
        ("Edited",
         "Modifié"),
    "threesea-la-deleting-everyone":
        ("Deleting for everyone",
         "Suppression pour tout le monde"),
    "threesea-la-deleting-message":
        ("Deleting",
         "Suppression"),
    "threesea-la-editing-message":
        ("Editing message",
         "Modification du message"),
    "threesea-la-file":
        ("File",
         "Fichier"),
    "threesea-la-photo":
        ("Photo",
         "Photo"),
    "threesea-la-error-file-too-large":
        ("Files can be at most 100 MB.",
         "Les fichiers ne peuvent pas dépasser 100 Mo."),
    "threesea-la-add-caption":
        ("Add a caption",
         "Ajouter une légende"),
    "threesea-me-send-photo":
        ("Send photo",
         "Envoyer une photo"),
    "threesea-me-send-file":
        ("Send file",
         "Envoyer un fichier"),
    "threesea-me-save-downloads":
        ("Save to Downloads",
         "Enregistrer dans Téléchargements"),
    "threesea-la-saved-downloads":
        ("Saved to Downloads",
         "Enregistré dans Téléchargements"),
    "threesea-la-voice-message":
        ("Voice message",
         "Message vocal"),
    "threesea-la-recording-failed":
        ("Recording failed",
         "L’enregistrement a échoué"),
    "threesea-la-recording":
        ("Recording %1",
         "Enregistrement %1"),
    "threesea-la-cannot-play":
        ("This voice message could not be played",
         "Ce message vocal n’a pas pu être lu"),
    "threesea-la-error-group-left":
        ("You are no longer in this group.",
         "Vous ne faites plus partie de ce groupe."),
    "threesea-la-error-not-group-creator":
        ("Only the group's creator can do this.",
         "Seul le créateur du groupe peut le faire."),
    "threesea-la-status-group-created":
        ("You created this group.",
         "Vous avez créé ce groupe."),
    "threesea-la-status-group-joined":
        ("You were added to this group.",
         "Vous avez été ajouté à ce groupe."),
    "threesea-la-status-group-removed":
        ("You were removed from this group.",
         "Vous avez été retiré de ce groupe."),
    "threesea-la-status-group-disbanded":
        ("This group was dissolved.",
         "Ce groupe a été dissous."),
    "threesea-la-status-group-left":
        ("You left this group.",
         "Vous avez quitté ce groupe."),
    "threesea-la-status-group-renamed":
        ("Group renamed to %1.",
         "Le groupe a été renommé en %1."),
    "threesea-la-status-group-member-added":
        ("%1 was added.",
         "%1 a été ajouté."),
    "threesea-la-status-group-member-removed":
        ("%1 was removed.",
         "%1 a été retiré."),
    "threesea-la-status-group-member-left":
        ("%1 left the group.",
         "%1 a quitté le groupe."),
    "threesea-la-group-left-input":
        ("You are no longer in this group",
         "Vous ne faites plus partie de ce groupe"),
    "threesea-me-new-group":
        ("New group",
         "Nouveau groupe"),
    "threesea-he-rename-group":
        ("Rename group",
         "Renommer le groupe"),
    "threesea-la-group-name":
        ("Group name",
         "Nom du groupe"),
    "threesea-me-delete-group":
        ("Delete group",
         "Supprimer le groupe"),
    "threesea-la-deleting-group":
        ("Deleting group",
         "Suppression du groupe"),
    "threesea-me-dissolve-group":
        ("Dissolve group",
         "Dissoudre le groupe"),
    "threesea-me-leave-group":
        ("Leave group",
         "Quitter le groupe"),
    "threesea-la-dissolving-group":
        ("Dissolving group",
         "Dissolution du groupe"),
    "threesea-la-leaving-group":
        ("Leaving group",
         "Sortie du groupe"),
    "threesea-me-rename":
        ("Rename",
         "Renommer"),
    "threesea-he-members":
        ("Members",
         "Membres"),
    "threesea-he-group":
        ("Group",
         "Groupe"),
    "threesea-la-member-count":
        ("%n member(s)",
         ("%n membre",
          "%n membres")),
    "threesea-la-you-creator":
        ("You (creator)",
         "Vous (créateur)"),
    "threesea-la-you":
        ("You",
         "Vous"),
    "threesea-la-member-creator":
        ("%1 (creator)",
         "%1 (créateur)"),
    "threesea-he-create":
        ("Create",
         "Créer"),
    "threesea-he-new-group":
        ("New group",
         "Nouveau groupe"),
    "threesea-me-remove-picture":
        ("Remove picture",
         "Supprimer l’image"),
    "threesea-me-set-picture":
        ("Set picture",
         "Choisir une image"),
    "threesea-me-add-members":
        ("Add members",
         "Ajouter des membres"),
    "threesea-he-add-members":
        ("Add members",
         "Ajouter des membres"),
    "threesea-la-removing-member":
        ("Removing %1",
         "Retrait de %1"),
    "threesea-la-poll-preview":
        ("Poll: %1",
         "Sondage : %1"),
    "threesea-me-close-poll":
        ("Close poll",
         "Fermer le sondage"),
    "threesea-la-poll-closed":
        ("Closed",
         "Fermé"),
    "threesea-la-poll-multiple":
        ("Multiple choice",
         "Choix multiple"),
    "threesea-la-poll-single":
        ("Single choice",
         "Choix unique"),
    "threesea-la-poll-votes":
        ("%n vote(s)",
         ("%n vote",
          "%n votes")),
    "threesea-me-create-poll":
        ("Create poll",
         "Créer un sondage"),
    "threesea-he-send":
        ("Send",
         "Envoyer"),
    "threesea-he-new-poll":
        ("New poll",
         "Nouveau sondage"),
    "threesea-la-poll-question":
        ("Question",
         "Question"),
    "threesea-he-poll-choices":
        ("Choices",
         "Choix"),
    "threesea-la-poll-choice":
        ("Choice %1",
         "Choix %1"),
    "threesea-la-poll-announce":
        ("Show votes before closing",
         "Montrer les votes avant la fermeture"),
    "threesea-la-poll-announce-description":
        ("Otherwise only you see the votes until you close the poll.",
         "Sinon, vous seul voyez les votes jusqu’à la fermeture du sondage."),
    "threesea-la-poll-you-voted":
        ("You voted",
         "Vous avez voté"),
    "threesea-la-poll-tap-to-vote":
        ("Tap to vote",
         "Touchez pour voter"),
    "threesea-la-closing-poll":
        ("Closing poll",
         "Fermeture du sondage"),
    "threesea-he-poll-not-voted":
        ("Not voted yet",
         "Pas encore voté"),
    "threesea-la-poll-only-you":
        ("Only you see the votes until you close the poll.",
         "Vous seul voyez les votes jusqu’à la fermeture du sondage."),
    "threesea-me-clear-choices":
        ("Clear choices",
         "Effacer les choix"),
    "threesea-he-vote":
        ("Vote",
         "Voter"),
    "threesea-me-reply":
        ("Reply",
         "Répondre"),
    "threesea-me-mark-read":
        ("Mark as read",
         "Marquer comme lu"),
    "threesea-la-new-message":
        ("New message",
         "Nouveau message"),
    "threesea-se-notifications":
        ("Notifications",
         "Notifications"),
    "threesea-la-notifications":
        ("Notify about new messages",
         "Notifier les nouveaux messages"),
    "threesea-la-notification-preview":
        ("Show message content",
         "Afficher le contenu des messages"),
    "threesea-me-quote":
        ("Quote",
         "Citer"),
    "threesea-la-quote-missing":
        ("Message not available",
         "Message indisponible"),
    "threesea-la-quoting":
        ("Quoting %1",
         "Citation de %1"),
    "threesea-he-share":
        ("Share",
         "Partager"),
    "threesea-la-share-files":
        ("%n file(s)",
         ("%n fichier",
          "%n fichiers")),
    "threesea-la-error-qr-invalid":
        ("This is not a Threema contact QR code.",
         "Ceci n’est pas un code QR de contact Threema."),
    "threesea-la-error-qr-mismatch":
        ("This code has the right Threema ID but a different key from the one known for it, so it may be forged. It was not accepted. Compare the codes with this person directly.",
         "Ce code a le bon ID Threema, mais une clé différente de celle connue pour lui : il est peut-être falsifié. Il n’a pas été accepté. Comparez les codes directement avec cette personne."),
    "threesea-me-verify-qr":
        ("Verify by QR code",
         "Vérifier par code QR"),
    "threesea-he-scan-qr":
        ("Scan QR code",
         "Scanner un code QR"),
    "threesea-he-my-qr-code":
        ("My QR code",
         "Mon code QR"),
    "threesea-la-my-qr-code-description":
        ("When someone scans this code with Threema, you are verified for them in person (three dots).",
         "Quand quelqu’un scanne ce code avec Threema, vous êtes vérifié en personne pour lui (trois points)."),
    "threesea-la-qr-other-contact":
        ("This is someone else's code.",
         "Ce code est celui de quelqu’un d’autre."),
    "threesea-la-qr-verified":
        ("%1 is verified",
         "%1 est vérifié"),
    "threesea-me-scan-from-picture":
        ("Scan from picture",
         "Scanner depuis une image"),
    "threesea-la-no-camera":
        ("No camera available. Pull down to scan a code from a picture.",
         "Aucun appareil photo disponible. Tirez vers le bas pour scanner un code depuis une image."),
    "threesea-la-scan-hint":
        ("Point the camera at the Threema QR code of the person next to you.",
         "Dirigez l’appareil photo vers le code QR Threema de la personne à côté de vous."),
    "threesea-la-qr-not-found":
        ("No Threema QR code found in this picture.",
         "Aucun code QR Threema trouvé dans cette image."),
    "threesea-la-address-book-unavailable":
        ("The address book is not available on this device.",
         "Le carnet d’adresses n’est pas disponible sur cet appareil."),
    "threesea-la-address-book-added":
        ("%n new contact(s) from your address book",
         ("%n nouveau contact depuis votre carnet d’adresses",
          "%n nouveaux contacts depuis votre carnet d’adresses")),
    "threesea-la-address-book-sync":
        ("Find contacts from the address book",
         "Trouver des contacts dans le carnet d’adresses"),
    "threesea-la-address-book-sync-description":
        ("Daily, only hashed e-mail addresses and phone numbers are sent to Threema to find who uses it. Found people become contacts with their address book names.",
         "Chaque jour, seules des empreintes des adresses e-mail et des numéros de téléphone sont envoyées à Threema pour savoir qui l’utilise. Les personnes trouvées deviennent des contacts avec le nom du carnet d’adresses."),
    "threesea-la-mention-all":
        ("All",
         "Tous"),
    "threesea-la-mention-me":
        ("Me",
         "Moi"),
    "threesea-la-chat-notifications":
        ("Notifications",
         "Notifications"),
    "threesea-la-notifications-until":
        ("Until %1",
         "Jusqu’à %1"),
    "threesea-me-notifications-on":
        ("On",
         "Activées"),
    "threesea-me-notifications-1-hour":
        ("Off for 1 hour",
         "Coupées pendant 1 heure"),
    "threesea-me-notifications-8-hours":
        ("Off for 8 hours",
         "Coupées pendant 8 heures"),
    "threesea-me-notifications-1-day":
        ("Off for 1 day",
         "Coupées pendant 1 jour"),
    "threesea-me-notifications-1-week":
        ("Off for 1 week",
         "Coupées pendant 1 semaine"),
    "threesea-me-notifications-off":
        ("Off",
         "Coupées"),
    "threesea-me-notifications-mentioned":
        ("Only when I am mentioned",
         "Seulement si l’on me mentionne"),
    "threesea-la-error-file-not-downloaded":
        ("Download the file first.",
         "Téléchargez d’abord le fichier."),
    "threesea-la-error-not-forwardable":
        ("This message cannot be forwarded.",
         "Ce message ne peut pas être transféré."),
    "threesea-me-forward":
        ("Forward",
         "Transférer"),
    "threesea-he-forward":
        ("Forward",
         "Transférer"),
    "threesea-la-error-contact-blocked":
        ("You have blocked this contact.",
         "Vous avez bloqué ce contact."),
    "threesea-me-block-id":
        ("Block a Threema ID",
         "Bloquer un ID Threema"),
    "threesea-he-block":
        ("Block",
         "Bloquer"),
    "threesea-la-block-intro":
        ("Messages from this Threema ID will be discarded, and you cannot write to it.",
         "Les messages venant de cet ID Threema seront ignorés, et vous ne pourrez pas lui écrire."),
    "threesea-he-blocked-contacts":
        ("Blocked contacts",
         "Contacts bloqués"),
    "threesea-me-unblock":
        ("Unblock",
         "Débloquer"),
    "threesea-la-no-blocked":
        ("No blocked contacts",
         "Aucun contact bloqué"),
    "threesea-la-blocked-input":
        ("You have blocked this contact",
         "Vous avez bloqué ce contact"),
    "threesea-me-block":
        ("Block",
         "Bloquer"),
    "threesea-la-blocked":
        ("Blocked",
         "Bloqué"),
    "threesea-la-no-maps-app":
        ("No maps app installed. Long press to copy the coordinates.",
         "Aucune application de cartes installée. Appui long pour copier les coordonnées."),
    "threesea-me-send-location":
        ("Send location",
         "Envoyer la position"),
    "threesea-la-location-unavailable":
        ("Positioning is not available. Turn on location in Settings → Location.",
         "La localisation n’est pas disponible. Activez-la dans Paramètres → Localisation."),
    "threesea-la-location-finding":
        ("Finding your location…",
         "Recherche de votre position…"),
    "threesea-la-location-name":
        ("Name of the place (optional)",
         "Nom du lieu (facultatif)"),
    "threesea-me-search":
        ("Search",
         "Rechercher"),
    "threesea-he-search":
        ("Search",
         "Recherche"),
    "threesea-la-search-messages":
        ("Search messages",
         "Rechercher dans les messages"),
    "threesea-la-search-you-to":
        ("You → %1",
         "Vous → %1"),
    "threesea-la-search-nothing":
        ("No messages found",
         "Aucun message trouvé"),
    "threesea-se-pinned":
        ("Pinned",
         "Épinglées"),
    "threesea-me-unpin":
        ("Unpin",
         "Désépingler"),
    "threesea-me-pin":
        ("Pin to top",
         "Épingler en haut"),
    "threesea-me-move-up":
        ("Move up",
         "Monter"),
    "threesea-me-move-down":
        ("Move down",
         "Descendre"),
    "threesea-me-send-video":
        ("Send video",
         "Envoyer une vidéo"),
    "threesea-la-cannot-play-video":
        ("This video could not be played",
         "Cette vidéo n’a pas pu être lue"),
    "threesea-me-select-messages":
        ("Select messages",
         "Sélectionner des messages"),
    "threesea-he-n-selected":
        ("%n selected",
         ("%n sélectionné",
          "%n sélectionnés")),
    "threesea-la-deleting-messages":
        ("Deleting %n message(s)",
         ("Suppression de %n message",
          "Suppression de %n messages")),
    "threesea-la-n-messages":
        ("%n message(s)",
         ("%n message",
          "%n messages")),
    "threesea-la-video":
        ("Video",
         "Vidéo"),
    "threesea-me-cancel-selection":
        ("Cancel selection",
         "Annuler la sélection"),
    "threesea-la-select-hint":
        ("Tap messages to select them",
         "Touchez des messages pour les sélectionner"),
    "threesea-la-status-list-created":
        ("You created this distribution list.",
         "Vous avez créé cette liste de distribution."),
    "threesea-he-new-list":
        ("New distribution list",
         "Nouvelle liste de distribution"),
    "threesea-me-new-list":
        ("New distribution list",
         "Nouvelle liste de distribution"),
    "threesea-me-delete-list":
        ("Delete list",
         "Supprimer la liste"),
    "threesea-la-deleting-list":
        ("Deleting list",
         "Suppression de la liste"),
    "threesea-he-distribution-list":
        ("Distribution list",
         "Liste de distribution"),
    "threesea-la-recipient-count":
        ("%n recipient(s)",
         ("%n destinataire",
          "%n destinataires")),
    "threesea-la-distribution-list-info":
        ("Each recipient gets the messages as a normal message from you; they don't see who else got them.",
         "Chaque destinataire reçoit les messages comme un message normal de votre part ; il ne voit pas qui d’autre les a reçus."),
    "threesea-he-recipients":
        ("Recipients",
         "Destinataires"),
    "threesea-la-list-name":
        ("List name",
         "Nom de la liste"),
    "threesea-me-star":
        ("Star",
         "Ajouter l’étoile"),
    "threesea-he-starred":
        ("Starred messages",
         "Messages marqués"),
    "threesea-la-no-starred":
        ("No starred messages",
         "Aucun message marqué"),
    "threesea-la-no-starred-hint":
        ("Long press a message and choose Star to keep it here.",
         "Appui long sur un message, puis « Ajouter l’étoile » pour le garder ici."),
    "threesea-la-draft":
        ("Draft: %1",
         "Brouillon : %1"),
    "threesea-he-archived-chats":
        ("Archived chats",
         "Discussions archivées"),
    "threesea-me-unarchive":
        ("Move back to chats",
         "Remettre dans les discussions"),
    "threesea-me-archive":
        ("Archive",
         "Archiver"),
    "threesea-la-no-archived":
        ("No archived chats",
         "Aucune discussion archivée"),
    "threesea-me-details":
        ("Details",
         "Détails"),
    "threesea-he-message-details":
        ("Message details",
         "Détails du message"),
    "threesea-la-details-created":
        ("Created",
         "Créé"),
    "threesea-la-details-sent":
        ("Sent",
         "Envoyé"),
    "threesea-la-details-received":
        ("Received",
         "Reçu"),
    "threesea-la-details-delivered":
        ("Delivered",
         "Remis"),
    "threesea-la-details-read":
        ("Read",
         "Lu"),
    "threesea-la-details-deleted":
        ("Deleted",
         "Supprimé"),
    "threesea-la-details-edited":
        ("Edited",
         "Modifié"),
    "threesea-la-details-file-type":
        ("File type",
         "Type de fichier"),
    "threesea-la-details-file-size":
        ("File size",
         "Taille du fichier"),
    "threesea-la-details-message-id":
        ("Message ID",
         "ID du message"),
    "threesea-me-show-in-chat":
        ("Show in chat",
         "Afficher dans la discussion"),
    "threesea-la-no-media":
        ("No media",
         "Aucun média"),
    "threesea-la-wrong-pin":
        ("Wrong PIN",
         "Code PIN incorrect"),
    "threesea-la-error-pin-short":
        ("The PIN needs at least 4 digits.",
         "Le code PIN doit comporter au moins 4 chiffres."),
    "threesea-la-error-locked":
        ("Enter your PIN first.",
         "Saisissez d’abord votre code PIN."),
    "threesea-la-private-needs-lock":
        ("Private chats need an app lock. Set one up in Settings, Security, App lock.",
         "Les discussions privées nécessitent un verrouillage de l’application. Configurez-le dans Paramètres, Sécurité, Verrouillage de l’application."),
    "threesea-la-error-private-chats-exist":
        ("Remove all chats from the private chats first.",
         "Retirez d’abord toutes vos discussions de la liste des discussions privées."),
    "threesea-la-private-chat":
        ("Private chat",
         "Discussion privée"),
    "threesea-me-lock-immediately":
        ("Immediately",
         "Immédiatement"),
    "threesea-me-lock-30-seconds":
        ("After 30 seconds",
         "Après 30 secondes"),
    "threesea-me-lock-1-minute":
        ("After 1 minute",
         "Après 1 minute"),
    "threesea-me-lock-2-minutes":
        ("After 2 minutes",
         "Après 2 minutes"),
    "threesea-me-lock-5-minutes":
        ("After 5 minutes",
         "Après 5 minutes"),
    "threesea-me-lock-10-minutes":
        ("After 10 minutes",
         "Après 10 minutes"),
    "threesea-me-lock-30-minutes":
        ("After 30 minutes",
         "Après 30 minutes"),
    "threesea-he-app-lock":
        ("App lock",
         "Verrouillage de l’application"),
    "threesea-me-lock-off":
        ("Off",
         "Désactivé"),
    "threesea-me-lock-pin":
        ("PIN or passphrase",
         "Code PIN ou phrase secrète"),
    "threesea-la-change-pin":
        ("Change PIN",
         "Changer le code PIN"),
    "threesea-la-lock-app":
        ("Lock the app",
         "Verrouiller l’application"),
    "threesea-la-lock-app-description":
        ("Ask when Threesea starts, and when you come back to it after the time you choose. Private chats always ask.",
         "Déverrouillage demandé au démarrage de Threesea, et quand vous y revenez après le délai choisi. Les discussions privées le demandent toujours."),
    "threesea-la-lock-after":
        ("Lock",
         "Verrouiller"),
    "threesea-he-private-chats":
        ("Private chats",
         "Discussions privées"),
    "threesea-me-unmark-private":
        ("Remove from private chats",
         "Retirer des discussions privées"),
    "threesea-me-mark-private":
        ("Make private",
         "Rendre privée"),
    "threesea-la-no-private":
        ("No private chats",
         "Aucune discussion privée"),
    "threesea-he-repeat-pin":
        ("Enter the PIN again",
         "Saisissez à nouveau le code PIN"),
    "threesea-la-pin-hint":
        ("At least 4 digits",
         "Au moins 4 chiffres"),
    "threesea-la-pin-mismatch":
        ("They did not match. Try again.",
         "Saisies différentes. Réessayez."),
    "threesea-he-enter-pin":
        ("Enter your PIN",
         "Saisissez votre code PIN"),
    "threesea-la-locked-notification":
        ("Threesea is locked",
         "Threesea est verrouillé"),
    "threesea-la-locked-notification-body":
        ("Open it and enter your PIN or passphrase to receive messages.",
         "Ouvrez-le et saisissez votre code PIN ou votre phrase secrète pour recevoir les messages."),
    "threesea-la-key-encryption-failed":
        ("Could not change the encryption of the stored data.",
         "Impossible de changer le chiffrement des données enregistrées."),
    "threesea-la-lock-passphrase":
        ("Passphrase",
         "Phrase secrète"),
    "threesea-la-lock-pin":
        ("PIN",
         "Code PIN"),
    "threesea-la-change-passphrase":
        ("Change passphrase",
         "Changer la phrase secrète"),
    "threesea-he-new-passphrase":
        ("Enter new passphrase",
         "Saisissez la nouvelle phrase secrète"),
    "threesea-he-new-pin":
        ("Enter new PIN",
         "Saisissez le nouveau code PIN"),
    "threesea-he-repeat-passphrase":
        ("Enter the passphrase again",
         "Saisissez à nouveau la phrase secrète"),
    "threesea-la-passphrase-hint":
        ("At least %1 characters",
         "Au moins %1 caractères"),
    "threesea-he-enter-passphrase":
        ("Enter your passphrase",
         "Saisissez votre phrase secrète"),
    "threesea-la-wrong-passphrase":
        ("Wrong passphrase",
         "Phrase secrète incorrecte"),
    "threesea-la-revocation-intro":
        ("With this password you can revoke your Threema ID at myid.threema.ch/revoke in case you lose your phone or it gets stolen. Use at least 8 characters.",
         "Avec ce mot de passe, vous pouvez révoquer votre ID Threema sur myid.threema.ch/revoke si vous perdez votre téléphone ou s’il vous est volé. Utilisez au moins 8 caractères."),
    "threesea-la-error-phone-invalid":
        ("Enter the number with its country code, e.g. +41 79 123 45 67.",
         "Saisissez le numéro avec son indicatif international, par exemple +41 79 123 45 67."),
    "threesea-la-error-email-invalid":
        ("This is not an e-mail address.",
         "Ceci n’est pas une adresse e-mail."),
    "threesea-la-error-no-verification":
        ("No phone number is waiting for a code. Link it again.",
         "Aucun numéro de téléphone n’attend de code. Associez-le à nouveau."),
    "threesea-me-unlink":
        ("Remove link",
         "Supprimer le lien"),
    "threesea-he-link":
        ("Link",
         "Associer"),
    "threesea-la-link-email-info":
        ("People who have this e-mail address in their address book can find you on Threema. It is sent to Threema, which e-mails you a link to confirm it.",
         "Les personnes qui ont cette adresse e-mail dans leur carnet d’adresses peuvent vous trouver sur Threema. Elle est envoyée à Threema, qui vous envoie un lien de confirmation par e-mail."),
    "threesea-la-link-mobile-info":
        ("People who have this number in their address book can find you on Threema. It is sent to Threema, which texts you a code to confirm it.",
         "Les personnes qui ont ce numéro dans leur carnet d’adresses peuvent vous trouver sur Threema. Il est envoyé à Threema, qui vous envoie un code de confirmation par SMS."),
    "threesea-la-email":
        ("E-mail address",
         "Adresse e-mail"),
    "threesea-la-phone":
        ("Phone number",
         "Numéro de téléphone"),
    "threesea-me-cancel-link":
        ("Cancel linking",
         "Annuler l’association"),
    "threesea-me-call-me":
        ("Call me instead",
         "M’appeler à la place"),
    "threesea-la-call-coming":
        ("You will get a call with the code.",
         "Vous allez recevoir un appel avec le code."),
    "threesea-he-confirm":
        ("Confirm",
         "Confirmer"),
    "threesea-la-enter-sms-code":
        ("Enter the code Threema sent to +%1.",
         "Saisissez le code que Threema a envoyé au +%1."),
    "threesea-la-sms-code":
        ("Code",
         "Code"),
    "threesea-he-media":
        ("Media",
         "Médias"),
    "threesea-me-media-show":
        ("Show",
         "Afficher"),
    "threesea-la-kind-photos":
        ("Photos",
         "Photos"),
    "threesea-la-kind-gifs":
        ("GIFs",
         "GIF"),
    "threesea-la-kind-videos":
        ("Videos",
         "Vidéos"),
    "threesea-la-kind-voice":
        ("Voice messages",
         "Messages vocaux"),
    "threesea-la-kind-audio":
        ("Audio",
         "Audio"),
    "threesea-la-kind-files":
        ("Files",
         "Fichiers"),
    "threesea-la-no-media-filtered":
        ("Nothing of the kinds shown",
         "Aucun élément des types affichés"),
    "threesea-la-no-media-filtered-hint":
        ("Pull down to choose what to show",
         "Tirez vers le bas pour choisir ce qui est affiché"),
    "threesea-la-enter-to-send":
        ("Enter sends",
         "Entrée envoie"),
    "threesea-la-enter-to-send-description":
        ("Enter sends the message. Off: Enter starts a new line and the send button sends. On a hardware keyboard, Shift+Enter does the opposite.",
         "Entrée envoie le message. Désactivé : Entrée va à la ligne et le bouton d’envoi envoie. Sur un clavier physique, Maj+Entrée fait l’inverse."),
    "threesea-se-advanced":
        ("Advanced",
         "Avancé"),
    "threesea-se-security":
        ("Security",
         "Sécurité"),
    "threesea-se-account":
        ("Account",
         "Compte"),
    "threesea-se-privacy-security":
        ("Privacy and security",
         "Confidentialité et sécurité"),
    "threesea-la-app-lock-on":
        ("App lock on",
         "Verrouillage activé"),
    "threesea-la-app-lock-off":
        ("App lock off",
         "Verrouillage désactivé"),
    "threesea-se-app":
        ("App",
         "Application"),
    "threesea-se-info":
        ("Info",
         "Informations"),
    "threesea-me-delete-data":
        ("Remove ID from this device",
         "Supprimer l’ID de cet appareil"),
    "threesea-la-removing-data":
        ("Removing ID and data",
         "Suppression de l’ID et des données"),
    "threesea-la-revocation":
        ("ID revocation",
         "Révocation de l’ID"),
    "threesea-la-saving":
        ("Saving…",
         "Enregistrement…"),
    "threesea-la-revocation-set-at":
        ("Password set %1",
         "Mot de passe défini le %1"),
    "threesea-la-revocation-set":
        ("Password set",
         "Mot de passe défini"),
    "threesea-la-revocation-not-set":
        ("No password set",
         "Aucun mot de passe défini"),
    "threesea-la-link-email":
        ("E-mail",
         "E-mail"),
    "threesea-la-not-linked":
        ("Not linked",
         "Non lié"),
    "threesea-la-email-pending":
        ("Waiting for you to confirm the link in the e-mail.",
         "En attente : confirmez le lien reçu par e-mail."),
    "threesea-la-link-phone":
        ("Phone number",
         "Numéro de téléphone"),
    "threesea-la-phone-pending":
        ("+%1 (not confirmed)",
         "+%1 (non confirmé)"),
    "threesea-la-nickname-placeholder":
        ("Shown to the people you write to",
         "Montré aux personnes à qui vous écrivez"),
    "threesea-la-change-profile-picture":
        ("Change profile picture",
         "Changer la photo de profil"),
    "threesea-la-set-profile-picture":
        ("Set profile picture",
         "Choisir une photo de profil"),
    "threesea-me-share-everyone":
        ("Everyone I write to",
         "Toutes les personnes à qui j’écris"),
    "threesea-me-share-nobody":
        ("Nobody",
         "Personne"),
    "threesea-me-share-selected":
        ("Selected contacts",
         "Contacts choisis"),
    "threesea-la-picture-share":
        ("Share profile picture with",
         "Partager la photo de profil avec"),
    "threesea-la-share-selected-count":
        ("%n selected contact(s)",
         ("%n contact choisi",
          "%n contacts choisis")),
    "threesea-me-exclude-id":
        ("Add a Threema ID",
         "Ajouter un ID Threema"),
    "threesea-la-exclusion-intro":
        ("Finding contacts from the address book will not add or rename this Threema ID.",
         "La recherche de contacts dans le carnet d’adresses n’ajoutera ni ne renommera cet ID Threema."),
    "threesea-he-exclusion-list":
        ("Exclusion list",
         "Liste d’exclusion"),
    "threesea-me-remove-excluded":
        ("Remove",
         "Retirer"),
    "threesea-la-no-excluded":
        ("Nobody is excluded",
         "Personne n’est exclu"),
    "threesea-me-refresh":
        ("Refresh",
         "Actualiser"),
    "threesea-me-empty-chat":
        ("Empty chat",
         "Vider la discussion"),
    "threesea-la-emptying-chat":
        ("Emptying chat",
         "Vidage de la discussion"),
    "threesea-se-other":
        ("Other",
         "Autres"),
    "threesea-la-incognito-keyboard":
        ("Incognito keyboard",
         "Clavier incognito"),
    "threesea-la-incognito-keyboard-description":
        ("Ask the keyboard not to learn or suggest from what you type in chats.",
         "Demander au clavier de ne rien apprendre ni suggérer à partir de ce que vous écrivez dans les discussions."),
    "threesea-la-hide-cover":
        ("Hide content on the cover",
         "Masquer le contenu sur la couverture"),
    "threesea-la-hide-cover-description":
        ("The cover only shows the number of new messages, not who wrote.",
         "La couverture n’affiche que le nombre de nouveaux messages, pas qui a écrit."),
    "threesea-se-appearance":
        ("Appearance",
         "Apparence"),
    "threesea-la-bigger-emojis":
        ("Bigger single emojis",
         "Émojis seuls plus grands"),
    "threesea-la-bigger-emojis-description":
        ("Messages of up to three emojis are shown larger.",
         "Les messages d’au plus trois émojis sont affichés en plus grand."),
    "threesea-se-contacts":
        ("Contacts",
         "Contacts"),
    "threesea-se-receipts":
        ("Receipts",
         "Accusés de réception"),
    "threesea-me-policy-default-on":
        ("Same as in Settings (on)",
         "Comme dans les paramètres (activé)"),
    "threesea-me-policy-default-off":
        ("Same as in Settings (off)",
         "Comme dans les paramètres (désactivé)"),
    "threesea-me-policy-send":
        ("Always",
         "Toujours"),
    "threesea-me-policy-dont-send":
        ("Never",
         "Jamais"),
    "threesea-la-contact-read-receipts":
        ("Read receipts",
         "Accusés de lecture"),
    "threesea-la-contact-typing":
        ("Typing indicator",
         "Indicateur de saisie"),
    "threesea-me-clear-contact-policies":
        ("Clear individual settings",
         "Effacer les réglages individuels"),
    "threesea-la-clearing-contact-policies":
        ("Clearing individual settings",
         "Effacement des réglages individuels"),
    "threesea-la-no-contact-policies":
        ("No contact has its own setting.",
         "Aucun contact n’a son propre réglage."),
    "threesea-se-data-encryption":
        ("Encryption of stored data",
         "Chiffrement des données enregistrées"),
    "threesea-me-older-1-year":
        ("Older than 1 year",
         "Plus d’un an"),
    "threesea-me-older-6-months":
        ("Older than 6 months",
         "Plus de 6 mois"),
    "threesea-me-older-3-months":
        ("Older than 3 months",
         "Plus de 3 mois"),
    "threesea-me-older-1-month":
        ("Older than 1 month",
         "Plus d’un mois"),
    "threesea-me-older-1-week":
        ("Older than 1 week",
         "Plus d’une semaine"),
    "threesea-me-older-all":
        ("All",
         "Tout"),
    "threesea-he-delete":
        ("Delete",
         "Supprimer"),
    "threesea-se-clean-up-what":
        ("What",
         "Que supprimer"),
    "threesea-me-clean-up-media":
        ("Media files",
         "Fichiers médias"),
    "threesea-me-clean-up-media-description":
        ("Photos, videos, voice messages and files are deleted from this device, usually for good; the messages and their previews stay. Starred messages and messages still on their way keep their files.",
         "Les photos, vidéos, messages vocaux et fichiers sont supprimés de cet appareil, en général définitivement ; les messages et leurs aperçus restent. Les messages marqués d’une étoile et les messages encore en route gardent leurs fichiers."),
    "threesea-me-clean-up-messages":
        ("Messages and media",
         "Messages et médias"),
    "threesea-me-clean-up-messages-description":
        ("Starred messages, open polls and messages still on their way stay.",
         "Les messages marqués d’une étoile, les sondages ouverts et les messages encore en route restent."),
    "threesea-se-clean-up-age":
        ("Age",
         "Ancienneté"),
    "threesea-me-image-small":
        ("Small (640 × 640)",
         "Petite (640 × 640)"),
    "threesea-me-image-medium":
        ("Medium (1024 × 1024)",
         "Moyenne (1024 × 1024)"),
    "threesea-me-image-large":
        ("Large (1600 × 1600)",
         "Grande (1600 × 1600)"),
    "threesea-me-image-xlarge":
        ("Extra large (2592 × 2592)",
         "Très grande (2592 × 2592)"),
    "threesea-me-image-original":
        ("Original size",
         "Taille d’origine"),
    "threesea-la-download-nothing":
        ("Nothing",
         "Rien"),
    "threesea-se-media-storage":
        ("Media and storage",
         "Médias et stockage"),
    "threesea-se-sending":
        ("Sending",
         "Envoi"),
    "threesea-la-image-size":
        ("Image size",
         "Taille des images"),
    "threesea-se-receiving":
        ("Receiving",
         "Réception"),
    "threesea-la-download-wifi":
        ("Download on Wi-Fi",
         "Télécharger en Wi-Fi"),
    "threesea-la-download-mobile":
        ("Download on mobile data",
         "Télécharger en données mobiles"),
    "threesea-la-auto-save":
        ("Save to Gallery",
         "Enregistrer dans la galerie"),
    "threesea-la-auto-save-description":
        ("Received photos and videos are also saved, unencrypted, to the Threesea folder in Pictures.",
         "Les photos et vidéos reçues sont aussi enregistrées, non chiffrées, dans le dossier Threesea des images."),
    "threesea-se-storage":
        ("Storage",
         "Stockage"),
    "threesea-la-storage-messages":
        ("Messages",
         "Messages"),
    "threesea-me-clean-up":
        ("Clean up",
         "Nettoyer"),
    "threesea-la-twemoji-credit":
        ("Emoji graphics by Twemoji (github.com/jdecked/twemoji), licensed under CC-BY 4.0.",
         "Graphismes des émojis par Twemoji (github.com/jdecked/twemoji), sous licence CC-BY 4.0."),
    "threesea-me-emoji-system":
        ("System",
         "Système"),
    "threesea-la-emoji-style":
        ("Emoji style",
         "Style des émojis"),
    "threesea-se-blocking":
        ("Blocking",
         "Blocage"),
    "threesea-me-first-name-first":
        ("First name first",
         "Prénom d’abord"),
    "threesea-me-last-name-first":
        ("Last name first",
         "Nom d’abord"),
    "threesea-la-name-order":
        ("Names",
         "Noms"),
    "threesea-me-sort-first-name":
        ("By first name",
         "Par prénom"),
    "threesea-me-sort-last-name":
        ("By last name",
         "Par nom"),
    "threesea-la-sorting":
        ("Sorting",
         "Tri"),
    "threesea-la-show-inactive":
        ("Show inactive IDs",
         "Afficher les ID inactifs"),
    "threesea-la-show-inactive-description":
        ("Contacts whose Threema ID has not been used for a long time stay in the list.",
         "Les contacts dont l’ID Threema n’a pas servi depuis longtemps restent dans la liste."),
    "threesea-se-notifications-chats":
        ("Chats",
         "Discussions"),
    "threesea-la-notification-sound":
        ("Sound",
         "Son"),
    "threesea-la-notification-sound-description":
        ("Played at the volume of the current sound profile.",
         "Joué au volume du profil sonore actuel."),
    "threesea-la-notification-vibrate":
        ("Vibrate",
         "Vibration"),
    "threesea-se-notifications-groups":
        ("Groups",
         "Groupes"),
    "threesea-se-network":
        ("Network",
         "Réseau"),
    "threesea-la-ipv6":
        ("IPv6",
         "IPv6"),
    "threesea-la-ipv6-description":
        ("Messages, files and calls use IPv6 where the network has it. Off: Threesea uses IPv4 for them.",
         "Les messages, les fichiers et les appels utilisent IPv6 là où le réseau le propose. Désactivé : Threesea utilise IPv4 pour eux."),
    "threesea-la-error-not-gif":
        ("This picture is not a GIF.",
         "Cette image n’est pas un GIF."),
    "threesea-la-gif-favourite-added":
        ("Added to favourite GIFs",
         "Ajouté aux GIF favoris"),
    "threesea-he-add-gif":
        ("Add GIF",
         "Ajouter un GIF"),
    "threesea-me-remove-gif":
        ("Remove",
         "Retirer"),
    "threesea-la-no-gif-favourites":
        ("No favourite GIFs",
         "Aucun GIF favori"),
    "threesea-la-no-gif-favourites-hint":
        ("Long press a GIF in a chat to add it, or add one from this device.",
         "Appui long sur un GIF dans une discussion pour l’ajouter, ou ajoutez-en un depuis cet appareil."),
    "threesea-la-autoplay-gifs":
        ("Play GIFs automatically",
         "Lire les GIF automatiquement"),
    "threesea-la-autoplay-gifs-description":
        ("Animated images play in the chat. Off: they play when you open them.",
         "Les images animées sont lues dans la discussion. Désactivé : elles sont lues quand vous les ouvrez."),
    "threesea-la-notification-tone":
        ("Notification tone",
         "Son de notification"),
    "threesea-la-tone-all-chats":
        ("Same as all chats",
         "Comme toutes les discussions"),
    "threesea-la-tone-all-groups":
        ("Same as all groups",
         "Comme tous les groupes"),
    "threesea-la-tone-profile":
        ("Instant message tone",
         "Son de messagerie instantanée"),
    "threesea-la-tone-from-music":
        ("Select from music",
         "Choisir dans la musique"),
    "threesea-se-tones":
        ("Tones",
         "Sons"),
    "threesea-la-error-link-unknown-code":
        ("This is not a code of Threema for Desktop. Scan the code it shows when you open it.",
         "Ceci n’est pas un code de Threema for Desktop. Scannez le code qu’il affiche à son ouverture."),
    "threesea-la-error-link-web-code":
        ("This code is for Threema Web. Scan the code of Threema for Desktop instead.",
         "Ce code est celui de Threema Web. Scannez plutôt celui de Threema for Desktop."),
    "threesea-la-error-link-version":
        ("This version of Threema for Desktop can't be linked. Please update both apps.",
         "Cette version de Threema for Desktop ne peut pas être associée. Mettez les deux applications à jour."),
    "threesea-la-error-link-failed":
        ("Linking didn't work. Check the connection on both devices and try again.",
         "L’association n’a pas abouti. Vérifiez la connexion des deux appareils et réessayez."),
    "threesea-la-qr-not-device":
        ("This is a contact's code. Scan the code on the computer.",
         "Ceci est le code d’un contact. Scannez le code affiché sur l’ordinateur."),
    "threesea-la-scan-hint-device":
        ("Point the camera at the QR code Threema for Desktop shows.",
         "Dirigez l’appareil photo vers le code QR affiché par Threema for Desktop."),
    "threesea-la-link-intro":
        ("Open Threema for Desktop on your computer. It shows a QR code; scan it on the next page.",
         "Ouvrez Threema for Desktop sur votre ordinateur. Il affiche un code QR ; scannez-le à la page suivante."),
    "threesea-la-link-emoji-mismatch":
        ("The emojis don't match. Close Threema for Desktop, open it again and scan its new code.",
         "Les émojis ne correspondent pas. Fermez Threema for Desktop, rouvrez-le et scannez son nouveau code."),
    "threesea-he-link-device":
        ("Link device",
         "Associer un appareil"),
    "threesea-la-link-connecting":
        ("Connecting to the computer…",
         "Connexion à l’ordinateur…"),
    "threesea-la-link-verify":
        ("Which emojis does the computer show?",
         "Quels émojis l’ordinateur affiche-t-il ?"),
    "threesea-la-link-linking":
        ("Linking. Keep Threesea open until the computer is ready.",
         "Association en cours. Laissez Threesea ouvert jusqu’à ce que l’ordinateur soit prêt."),
    "threesea-la-link-done":
        ("Threema for Desktop is linked.",
         "Threema for Desktop est associé."),
    "threesea-me-link-no-match":
        ("None of them",
         "Aucun d’eux"),
    "threesea-me-done":
        ("Done",
         "Terminé"),
    "threesea-la-removing-device":
        ("Removing",
         "Retrait"),
    "threesea-me-remove-all-devices":
        ("Remove all linked devices",
         "Retirer tous les appareils associés"),
    "threesea-la-removing-all-devices":
        ("Removing all linked devices",
         "Retrait de tous les appareils associés"),
    "threesea-me-link-device":
        ("Link device",
         "Associer un appareil"),
    "threesea-se-linked-devices":
        ("Linked devices",
         "Appareils associés"),
    "threesea-la-device":
        ("Device",
         "Appareil"),
    "threesea-la-this-device":
        ("This device",
         "Cet appareil"),
    "threesea-la-device-connected":
        ("Connected",
         "Connecté"),
    "threesea-la-device-last-active":
        ("Last active %1",
         "Dernière activité %1"),
    "threesea-me-remove-device":
        ("Remove",
         "Retirer"),
    "threesea-la-no-linked-devices":
        ("No linked devices",
         "Aucun appareil associé"),
    "threesea-la-no-linked-devices-hint":
        ("With Threema for Desktop you can read and write your chats on a computer. Open it there and pull down here to link it.",
         "Avec Threema for Desktop, vous pouvez lire et écrire vos discussions sur un ordinateur. Ouvrez-le là-bas et tirez vers le bas ici pour l’associer."),
    "threesea-me-connect-now":
        ("Connect now",
         "Se connecter"),
    "threesea-la-removing-lock":
        ("Removing the app lock",
         "Suppression du verrouillage"),
    "threesea-me-unstar-short":
        ("Unstar",
         "Retirer l’étoile"),
    "threesea-la-camera-unavailable":
        ("The camera is not available.",
         "L’appareil photo n’est pas disponible."),
    "threesea-me-take-photo":
        ("Take photo or video",
         "Prendre une photo ou une vidéo"),
    "threesea-me-add-gif-favourite":
        ("Add to favourite GIFs",
         "Ajouter aux GIF favoris"),
    "threesea-me-add-recipients":
        ("Add recipients",
         "Ajouter des destinataires"),
    "threesea-la-status-call-outgoing":
        ("Outgoing call, %1",
         "Appel sortant, %1"),
    "threesea-la-status-call-incoming":
        ("Incoming call, %1",
         "Appel entrant, %1"),
    "threesea-la-status-call-missed":
        ("Missed call",
         "Appel manqué"),
    "threesea-la-status-call-declined":
        ("Call declined",
         "Appel refusé"),
    "threesea-la-status-call-you-declined":
        ("You declined the call",
         "Vous avez refusé l’appel"),
    "threesea-la-status-call-busy":
        ("Call not answered: busy",
         "Appel sans réponse : occupé"),
    "threesea-la-status-call-unanswered":
        ("Call not answered",
         "Appel sans réponse"),
    "threesea-la-status-call-disabled":
        ("Call not answered: calls are off",
         "Appel sans réponse : les appels sont désactivés"),
    "threesea-la-status-call-cancelled":
        ("Call cancelled",
         "Appel annulé"),
    "threesea-la-status-call-failed":
        ("Call failed",
         "Échec de l’appel"),
    "threesea-la-call-calling":
        ("Calling…",
         "Appel…"),
    "threesea-la-call-ringing":
        ("Ringing…",
         "Cela sonne…"),
    "threesea-la-call-incoming":
        ("Incoming call",
         "Appel entrant"),
    "threesea-la-call-connecting":
        ("Connecting…",
         "Connexion…"),
    "threesea-la-call-ended":
        ("Call ended",
         "Appel terminé"),
    "threesea-me-call":
        ("Call",
         "Appeler"),
    "threesea-se-calls":
        ("Calls",
         "Appels"),
    "threesea-la-threema-calls":
        ("Threema calls",
         "Appels Threema"),
    "threesea-la-threema-calls-description":
        ("Contacts can call you, and you can call them from the chat's pull-down menu.",
         "Vos contacts peuvent vous appeler, et vous pouvez les appeler depuis le menu déroulant de la discussion."),
    "threesea-la-relay-calls":
        ("Always relay calls",
         "Toujours relayer les appels"),
    "threesea-la-relay-calls-description":
        ("Calls go through a Threema server, which keeps your IP address hidden from the other side. Call quality may be lower.",
         "Les appels passent par un serveur Threema, qui masque votre adresse IP à l’autre partie. La qualité des appels peut être moindre."),
    "threesea-me-video-call":
        ("Video call",
         "Appel vidéo"),
    "threesea-se-video":
        ("Video",
         "Vidéo"),
    "threesea-la-video-calls":
        ("Video in one-to-one calls",
         "Vidéo dans les appels individuels"),
    "threesea-la-video-calls-description":
        ("One-to-one calls can show the camera of both sides; each side turns its camera on when it wants.",
         "Les appels individuels peuvent montrer la caméra des deux côtés ; chacun allume la sienne quand il le souhaite."),
    "threesea-la-video-quality":
        ("Video quality",
         "Qualité vidéo"),
    "threesea-la-video-quality-auto":
        ("Standard",
         "Standard"),
    "threesea-la-video-quality-low":
        ("Low data use",
         "Faible consommation de données"),
    "threesea-la-video-quality-max":
        ("Highest quality",
         "Qualité maximale"),
    "threesea-la-video-quality-description":
        ("Both sides use the lower of their two settings. Calls that go only through a relay (Always relay calls, or with an unverified contact) use Standard at most.",
         "Les deux côtés utilisent le plus bas de leurs deux réglages. Les appels qui ne passent que par un relais (Toujours relayer les appels, ou avec un contact non vérifié) utilisent au plus Standard."),
    "threesea-me-switch-camera":
        ("Switch camera",
         "Changer de caméra"),
    "threesea-me-show-self":
        ("Show own picture",
         "Afficher ma propre image"),
    "threesea-me-hide-self":
        ("Hide own picture",
         "Masquer ma propre image"),
    "threesea-la-me":
        ("Me",
         "Moi"),
    "threesea-me-remove-profile-picture":
        ("Remove profile picture",
         "Supprimer la photo de profil"),
    "threesea-me-join-group-call":
        ("Join group call",
         "Rejoindre l’appel de groupe"),
    "threesea-la-group-call-joining":
        ("Joining…",
         "Connexion…"),
    "threesea-me-start-group-call":
        ("Start group call",
         "Démarrer un appel de groupe"),
    "threesea-la-group-call-waiting":
        ("Waiting for others",
         "En attente des autres"),
    "threesea-la-group-call":
        ("Group call",
         "Appel de groupe"),
    "threesea-la-group-call-participants":
        ("%n participant(s)",
         ("%n participant",
          "%n participants")),
    "threesea-la-group-call-return":
        ("Return",
         "Revenir"),
    "threesea-la-group-call-join":
        ("Join",
         "Rejoindre"),
    "threesea-la-error-web-version":
        ("This version of Threema Web can't be used. Reload the page in the browser.",
         "Cette version de Threema Web ne peut pas être utilisée. Rechargez la page dans le navigateur."),
    "threesea-la-error-web-session":
        ("This Threema Web session no longer exists.",
         "Cette session Threema Web n’existe plus."),
    "threesea-he-web-trust":
        ("Trust",
         "Faire confiance"),
    "threesea-he-web-allow":
        ("Allow",
         "Autoriser"),
    "threesea-la-web-certificate-changed":
        ("This Threema Web server shows a different certificate than last time. Trust it only if you know its certificate was renewed; otherwise go back.",
         "Ce serveur Threema Web présente un certificat différent de la dernière fois. Ne lui faites confiance que si vous savez que son certificat a été renouvelé ; sinon, revenez en arrière."),
    "threesea-la-web-self-hosted":
        ("This code is for a Threema Web server run by someone other than Threema. Your chats go through it end-to-end encrypted. Allow it only if you run it yourself or trust who does. Its certificate is remembered and you'll be asked if it changes.",
         "Ce code est celui d’un serveur Threema Web exploité par quelqu’un d’autre que Threema. Vos discussions y transitent chiffrées de bout en bout. Ne l’autorisez que si vous l’exploitez vous-même ou si vous faites confiance à celui qui l’exploite. Son certificat est mémorisé et Threesea vous demandera votre accord s’il change."),
    "threesea-la-web-connecting":
        ("Connecting…",
         "Connexion…"),
    "threesea-la-web-waiting":
        ("Waiting for the browser",
         "En attente du navigateur"),
    "threesea-la-web-connected":
        ("Connected",
         "Connecté"),
    "threesea-la-web-last-used":
        ("Last used %1",
         "Dernière utilisation %1"),
    "threesea-la-web-not-connected":
        ("Not connected",
         "Non connecté"),
    "threesea-me-scan-web-code":
        ("Scan Threema Web code",
         "Scanner le code Threema Web"),
    "threesea-he-threema-web":
        ("Threema Web",
         "Threema Web"),
    "threesea-la-web-none":
        ("No browsers",
         "Aucun navigateur"),
    "threesea-la-web-none-hint":
        ("Open web.threema.ch in a browser and pull down to scan its code",
         "Ouvrez web.threema.ch dans un navigateur et tirez vers le bas pour scanner son code"),
    "threesea-me-web-remove":
        ("Remove",
         "Retirer"),
    "threesea-la-web-browser":
        ("Browser",
         "Navigateur"),
    "threesea-la-no-recent-emoji":
        ("No emoji used yet",
         "Aucun émoji utilisé pour l’instant"),
    "threesea-la-poll-closed-preview":
        ("Poll closed: %1",
         "Sondage fermé : %1"),
    "threesea-la-poll-tap-results":
        ("Poll closed. Tap to view results",
         "Sondage fermé. Touchez pour voir les résultats"),
    "threesea-he-poll-results":
        ("Results",
         "Résultats"),
    "threesea-se-chat":
        ("Chat",
         "Discussion"),
    "threesea-la-scan-hint-any":
        ("Point the camera at a contact's Threema QR code, or at the one Threema Web or Threema for Desktop shows.",
         "Dirigez l’appareil photo vers le code QR Threema d’un contact, ou vers celui affiché par Threema Web ou Threema for Desktop."),
    "threesea-la-pin-paused":
        ("Too many wrong attempts. Try again in %n second(s).",
         ("Trop de tentatives erronées. Réessayez dans %n seconde.",
          "Trop de tentatives erronées. Réessayez dans %n secondes.")),
    "threesea-se-troubleshooting":
        ("Troubleshooting",
         "Dépannage"),
    "threesea-la-log-level":
        ("Log",
         "Journal"),
    "threesea-la-log-level-warn":
        ("Problems only",
         "Problèmes seulement"),
    "threesea-la-log-level-info":
        ("Normal",
         "Normal"),
    "threesea-la-log-level-debug":
        ("Detailed",
         "Détaillé"),
    "threesea-la-log-level-description":
        ("How much Threesea logs, to help find the cause of a problem.",
         "Quantité d’informations que Threesea consigne, pour aider à trouver la cause d’un problème."),
    "threesea-se-disclaimer":
        ("Disclaimer",
         "Avertissement"),
    "threesea-la-disclaimer-ai":
        ("Threesea was developed with Claude Code (Anthropic) and audited with OpenAI Codex, directed and tested by a human maintainer. It is a research and experimentation project.",
         "Threesea a été développé avec Claude Code (Anthropic) et audité avec OpenAI Codex, sous la direction d’un mainteneur humain qui l’a aussi testé. C’est un projet de recherche et d’expérimentation."),
    "threesea-la-disclaimer-warranty":
        ("AI makes mistakes. The code, including its security-relevant parts, can contain errors nobody noticed. The developers give no warranty of any kind; you use the app at your own risk.",
         "L’IA se trompe. Le code, y compris ses parties liées à la sécurité, peut contenir des erreurs que personne n’a remarquées. Les développeurs ne donnent aucune garantie, de quelque nature que ce soit ; vous utilisez l’application à vos propres risques."),
    "threesea-la-disclaimer-threema":
        ("Threesea is based on Threema Libre's open-source code and tries to play fair with Threema's APIs and licensing. Threema told us it doesn't actively block third-party apps that cause no stability issues or high server load, are clearly marked as unofficial without using its branding, and don't circumvent the licensing. Threesea aims to meet that. This is not a permission, and Threesea is not affiliated with, endorsed or supported by Threema AG. As with any Threema app, keep a backup of your ID.",
         "Threesea s’appuie sur le code open source de Threema Libre et s’efforce de jouer franc jeu avec les API et les licences de Threema. Threema nous a indiqué ne pas bloquer activement les applications tierces tant qu’elles ne causent ni problèmes de stabilité ni charge élevée sur les serveurs, qu’elles sont clairement présentées comme non officielles sans utiliser sa marque et qu’elles ne contournent pas les licences. Threesea s’efforce de respecter cela. Ce n’est pas une autorisation, et Threesea n’est ni affilié à Threema AG, ni approuvé ni soutenu par elle. Comme avec toute application Threema, conservez une sauvegarde de votre ID."),
    "threesea-se-credits":
        ("Credits",
         "Remerciements"),
    "threesea-la-credit-threema":
        ("Threema AG: threema-android and libthreema (AGPL-3.0), the protocols of Threema, Threema for Desktop and Threema Web.",
         "Threema AG : threema-android et libthreema (AGPL-3.0), les protocoles de Threema, Threema for Desktop et Threema Web."),
    "threesea-la-credit-opus":
        ("opus-pure by Stephen Berry, from Rust ports of libopus by the Xiph.Org Foundation and others (BSD-3-Clause): voice messages.",
         "opus-pure de Stephen Berry, d’après des portages Rust de libopus par la Xiph.Org Foundation et d’autres (BSD-3-Clause) : messages vocaux."),
    "threesea-la-credit-webrtc":
        ("webrtc-audio-processing, from Google's WebRTC (BSD-3-Clause): echo cancellation in calls.",
         "webrtc-audio-processing, issu du WebRTC de Google (BSD-3-Clause) : annulation d’écho dans les appels."),
    "threesea-la-credit-jolla":
        ("Jolla and Sailfish OS: Silica and its icons; Jolla's Messages, People, Settings and Gallery as the model for the design.",
         "Jolla et Sailfish OS : Silica et ses icônes ; les applications Messages, Contacts, Paramètres et Galerie de Jolla comme modèle de conception."),
    "threesea-la-credit-whisperfish":
        ("Whisperfish, the Signal client for Sailfish OS: a reference for building such an app.",
         "Whisperfish, le client Signal pour Sailfish OS : une référence pour construire une telle application."),
    "threesea-la-credit-libraries":
        ("SQLCipher, GStreamer, Qt and many Rust libraries (tokio, rustls, reqwest, rusqlite, RustCrypto and others).",
         "SQLCipher, GStreamer, Qt et de nombreuses bibliothèques Rust (tokio, rustls, reqwest, rusqlite, RustCrypto et d’autres)."),
    "threesea-la-credit-eu-icon":
        ("The “AI generated” label is the European Commission's icon for AI-generated content.",
         "Le label « AI generated » est l’icône de la Commission européenne pour les contenus générés par IA."),
    "threesea-la-update-notification":
        ("Threesea %1 is available",
         "Threesea %1 est disponible"),
    "threesea-la-update-notification-body":
        ("Tap for details and the download page.",
         "Touchez pour les détails et la page de téléchargement."),
    "threesea-me-check-updates-now":
        ("Check for updates now",
         "Rechercher les mises à jour maintenant"),
    "threesea-la-keep-running-description-full":
        ("Threesea starts when you turn on the phone and stays connected after you close it, so new messages arrive right away. Uses more battery.",
         "Threesea démarre à l’allumage du téléphone et reste connecté après sa fermeture, pour que les nouveaux messages arrivent tout de suite. Consomme davantage de batterie."),
    "threesea-se-updates":
        ("Updates",
         "Mises à jour"),
    "threesea-la-update-check":
        ("Check for updates",
         "Rechercher les mises à jour"),
    "threesea-la-update-check-description":
        ("Once a day, Threesea asks GitHub whether a newer version has been released, and tells you. GitHub sees your IP address; nothing else about you is sent.",
         "Une fois par jour, Threesea demande à GitHub si une version plus récente est parue, et vous le dit. GitHub voit votre adresse IP ; rien d’autre vous concernant n’est envoyé."),
    "threesea-la-update-open":
        ("Open download page",
         "Ouvrir la page de téléchargement"),
    "threesea-he-update-ask-accept":
        ("Check",
         "Rechercher"),
    "threesea-he-update-ask-cancel":
        ("Don't check",
         "Ne pas rechercher"),
    "threesea-la-update-ask-title":
        ("Check for updates?",
         "Rechercher les mises à jour ?"),
    "threesea-la-update-ask-text":
        ("This version of Threesea comes from GitHub, so it isn't updated by a store. Once a day, Threesea can ask GitHub whether a newer version has been released and tell you; you install it yourself. GitHub sees your IP address; nothing else about you is sent. You can change this in Settings, Advanced.",
         "Cette version de Threesea vient de GitHub : aucune boutique ne la met à jour. Une fois par jour, Threesea peut demander à GitHub si une version plus récente est parue et vous le dire ; vous l’installez vous-même. GitHub voit votre adresse IP ; rien d’autre vous concernant n’est envoyé. Vous pouvez changer cela dans Paramètres, Avancé."),
    "threesea-la-update-newest":
        ("Newest version",
         "Version la plus récente"),
    "threesea-la-update-installed":
        ("Installed",
         "Installée"),
    "threesea-la-update-checked":
        ("Last checked",
         "Dernière recherche"),
    "threesea-la-n-contacts":
        ("%n contact(s)",
         ("%n contact",
          "%n contacts")),
    "threesea-la-n-groups":
        ("%n group(s)",
         ("%n groupe",
          "%n groupes")),
    "threesea-la-n-lists":
        ("%n distribution list(s)",
         ("%n liste de distribution",
          "%n listes de distribution")),
    "threesea-me-safe-retry-import":
        ("Import the rest now",
         "Importer le reste maintenant"),
    "threesea-se-safe-restore":
        ("Restore",
         "Restauration"),
    "threesea-la-safe-to-import":
        ("Waiting to import",
         "En attente d’importation"),
    "threesea-la-safe-import-attempt":
        ("Import attempt",
         "Tentative d’importation"),
    "threesea-la-safe-skipped-deleted":
        ("Deleted groups",
         "Groupes supprimés"),
    "threesea-la-safe-skipped-creator":
        ("Creator not available",
         "Créateur indisponible"),
    "threesea-la-safe-skipped-invalid":
        ("Unreadable entries",
         "Entrées illisibles"),
    "threesea-la-safe-revoked":
        ("No longer registered",
         "Contacts plus enregistrés"),
    "threesea-la-error-backup-version":
        ("This backup's format is not supported.",
         "Le format de cette sauvegarde n’est pas pris en charge."),
    "threesea-la-error-backup-invalid":
        ("The file could not be read as a Threema backup.",
         "Le fichier n’a pas pu être lu comme une sauvegarde Threema."),
    "threesea-la-android-backup-file":
        ("Backup file",
         "Fichier de sauvegarde"),
    "threesea-la-select":
        ("Select",
         "Choisir"),
    "threesea-la-android-backup-password":
        ("Backup password",
         "Mot de passe de la sauvegarde"),
    "threesea-bu-android-backup-check":
        ("Check backup",
         "Vérifier la sauvegarde"),
    "threesea-se-android-backup-contents":
        ("Contents",
         "Contenu"),
    "threesea-la-android-backup-identity":
        ("Threema ID",
         "ID Threema"),
    "threesea-la-android-backup-id-match":
        ("This device's Threema ID",
         "L’ID Threema de cet appareil"),
    "threesea-la-android-backup-id-other":
        ("Another Threema ID",
         "Un autre ID Threema"),
    "threesea-la-android-backup-id-key":
        ("Same ID, different key",
         "Même ID, clé différente"),
    "threesea-la-android-backup-id-unreadable":
        ("Not readable with this password",
         "Illisible avec ce mot de passe"),
    "threesea-la-android-backup-id-missing":
        ("Not in the backup",
         "Absent de la sauvegarde"),
    "threesea-la-android-backup-chats":
        ("Chats",
         "Discussions"),
    "threesea-la-android-backup-group-chats":
        ("Group chats",
         "Discussions de groupe"),
    "threesea-la-android-backup-lists":
        ("Distribution lists",
         "Listes de distribution"),
    "threesea-la-android-backup-media":
        ("Media files",
         "Fichiers médias"),
    "threesea-la-android-backup-pictures":
        ("Pictures",
         "Images"),
    "threesea-la-error-backup-identity":
        ("This backup belongs to a different Threema ID or key.",
         "Cette sauvegarde appartient à un autre ID Threema ou à une autre clé."),
    "threesea-la-android-backup-imported-contacts":
        ("Contacts",
         "Contacts"),
    "threesea-la-android-backup-imported-groups":
        ("Groups",
         "Groupes"),
    "threesea-la-android-backup-existing":
        ("Already on this device",
         "Déjà sur cet appareil"),
    "threesea-la-android-import-chats":
        ("Chats %1 of %2",
         "Discussions %1 sur %2"),
    "threesea-bu-android-backup-stop":
        ("Stop",
         "Arrêter"),
    "threesea-se-android-backup-history":
        ("Message history",
         "Historique des messages"),
    "threesea-la-android-import-messages":
        ("Messages",
         "Messages"),
    "threesea-la-android-import-reactions":
        ("Reactions",
         "Réactions"),
    "threesea-se-android-backup-create":
        ("Create backups",
         "Créer des sauvegardes"),
    "threesea-la-android-export-folder":
        ("Backup folder",
         "Dossier des sauvegardes"),
    "threesea-la-android-export-password-saved":
        ("Password saved",
         "Mot de passe enregistré"),
    "threesea-la-android-export-auto":
        ("Automatic backup",
         "Sauvegarde automatique"),
    "threesea-la-android-export-off":
        ("Off",
         "Désactivée"),
    "threesea-la-android-export-daily":
        ("Daily",
         "Quotidienne"),
    "threesea-la-android-export-weekly":
        ("Weekly",
         "Hebdomadaire"),
    "threesea-la-android-export-keep":
        ("Keep",
         "Conserver"),
    "threesea-la-n-backups":
        ("%n backup(s)",
         ("%n sauvegarde",
          "%n sauvegardes")),
    "threesea-bu-android-export-now":
        ("Create backup now",
         "Créer une sauvegarde maintenant"),
    "threesea-la-android-export-written":
        ("Backup written: %1",
         "Sauvegarde écrite : %1"),
    "threesea-la-android-import-polls":
        ("Polls",
         "Sondages"),
    "threesea-he-import":
        ("Import",
         "Importer"),
    "threesea-la-android-backup-import-duration":
        ("A large backup can take a while. You can stop the import; importing again continues where it stopped.",
         "Une grande sauvegarde peut prendre du temps. Vous pouvez arrêter l’importation ; la relancer reprend là où elle s’est arrêtée."),
    "threesea-he-importing":
        ("Importing",
         "Importation"),
    "threesea-la-android-import-stopping":
        ("Stopping…",
         "Arrêt…"),
    "threesea-la-android-import-step-metadata":
        ("Contacts, groups and pictures",
         "Contacts, groupes et images"),
    "threesea-la-android-import-step-reading":
        ("Reading the backup",
         "Lecture de la sauvegarde"),
    "threesea-la-android-import-step-history":
        ("Messages and media",
         "Messages et médias"),
    "threesea-la-android-import-chat-media":
        ("Chat %1 of %2, media %3 of %4",
         "Discussion %1 sur %2, médias %3 sur %4"),
    "threesea-la-android-import-keep-open":
        ("Keep the app open until the import is done. What is imported stays; importing again continues.",
         "Laissez l’application ouverte jusqu’à la fin de l’importation. Ce qui est importé reste ; relancer l’importation la poursuit."),
    "threesea-he-data-backup":
        ("Data backup",
         "Sauvegarde des données"),
    "threesea-se-data-backup-import":
        ("Import a backup",
         "Importer une sauvegarde"),
    "threesea-la-data-backup-import-description":
        ("Checking a backup only reads the file and shows what it holds. The password never leaves this device.",
         "Vérifier une sauvegarde ne fait que lire le fichier et montrer ce qu’il contient. Le mot de passe ne quitte jamais cet appareil."),
    "threesea-la-data-backup-create-description":
        ("Writes a backup into the chosen folder, now or on a schedule. The password is kept in the encrypted storage so automatic backups can run.",
         "Écrit une sauvegarde dans le dossier choisi, maintenant ou à intervalles réguliers. Le mot de passe est conservé dans le stockage chiffré pour que les sauvegardes automatiques puissent se faire."),
    "threesea-he-import-failed":
        ("Import failed",
         "Échec de l’importation"),
    "threesea-he-import-stopped":
        ("Import stopped",
         "Importation arrêtée"),
    "threesea-he-import-finished":
        ("Import finished",
         "Importation terminée"),
    "threesea-la-data-backup-resume-hint":
        ("What is imported stays. Import the backup again to continue where it stopped.",
         "Ce qui est importé reste. Importez à nouveau la sauvegarde pour reprendre là où elle s’est arrêtée."),
    "threesea-la-data-backup-imported":
        ("The backup is imported. Everything already on this device stayed as it was.",
         "La sauvegarde est importée. Tout ce qui était déjà sur cet appareil est resté tel quel."),
    "threesea-se-import-contacts-groups":
        ("Contacts and groups",
         "Contacts et groupes"),
    "threesea-la-android-import-media-failed":
        ("Media not imported",
         "Médias non importés"),
    "threesea-la-forward-secrecy":
        ("Perfect Forward Secrecy",
         "Confidentialité persistante"),
    "threesea-la-forward-secrecy-description":
        ("Protects chats with keys that change continuously. Messages are end-to-end encrypted either way; contacts see a security notice when the protection starts, resets or is turned off.",
         "Protège les discussions avec des clés qui changent sans cesse. Les messages sont chiffrés de bout en bout dans les deux cas ; les contacts voient un avis de sécurité quand la protection démarre, se réinitialise ou est désactivée."),
    "threesea-la-view-log":
        ("View log",
         "Afficher le journal"),
    "threesea-me-log-clear":
        ("Clear log",
         "Effacer le journal"),
    "threesea-la-clearing-log":
        ("Clearing log",
         "Effacement du journal"),
    "threesea-me-log-save":
        ("Save support package",
         "Enregistrer le paquet d’assistance"),
    "threesea-me-log-refresh":
        ("Refresh",
         "Actualiser"),
    "threesea-he-log":
        ("Log",
         "Journal"),
    "threesea-la-log-empty":
        ("No entries",
         "Aucune entrée"),
    "threesea-la-log-empty-hint":
        ("What Threesea logs appears here. Settings → Advanced sets how much that is.",
         "Ce que Threesea consigne apparaît ici. Paramètres → Avancé règle la quantité."),
    "threesea-se-encryption":
        ("Encryption",
         "Chiffrement"),
    "threesea-la-device-cookie-title":
        ("Threema ID used on another device",
         "ID Threema utilisé sur un autre appareil"),
    "threesea-la-device-cookie-body":
        ("Threema's server noticed a connection from another device with your Threema ID. If you recently used your ID on another phone, you can ignore this. An ID works on one phone at a time: new messages go only to the phone that connected last. If you didn't, your private key may be compromised: protect this phone and its data, and create a new ID.",
         "Le serveur de Threema a remarqué une connexion depuis un autre appareil avec votre ID Threema. Si vous avez récemment utilisé votre ID sur un autre téléphone, vous pouvez ignorer ceci. Un ID fonctionne sur un seul téléphone à la fois : les nouveaux messages ne vont qu’au dernier téléphone connecté. Si ce n’était pas vous, votre clé privée est peut-être compromise : protégez ce téléphone et ses données, et créez un nouvel ID."),
    "threesea-me-delete-for-me":
        ("Delete for me",
         "Supprimer pour moi"),
    "threesea-me-delete-for-everyone":
        ("Delete for everyone",
         "Supprimer pour tous"),
    "threesea-la-playback-speed":
        ("%1×",
         "%1×"),
    "threesea-la-ear-to-play":
        ("Play voice messages at the ear",
         "Lire les messages vocaux à l’oreille"),
    "threesea-la-ear-to-play-description":
        ("Hold the phone to your ear and the voice message plays on the earpiece, like a call. Take it away to pause.",
         "Portez le téléphone à l’oreille et le message vocal est lu par l’écouteur, comme un appel. Éloignez-le pour mettre en pause."),
    "threesea-se-notifications-inapp":
        ("In the app",
         "Dans l’application"),
    "threesea-la-inapp-sound-description":
        ("A soft sound for a message arriving in the chat you are reading, and between two voice messages playing one after the other.",
         "Un son discret pour un message qui arrive dans la discussion que vous lisez, et entre deux messages vocaux lus l’un après l’autre."),
    "threesea-la-inapp-vibrate-description":
        ("A short vibration for a message arriving in the chat you are reading.",
         "Une brève vibration pour un message qui arrive dans la discussion que vous lisez."),
    "threesea-me-mark-unread":
        ("Mark as unread",
         "Marquer comme non lu"),
    "threesea-me-wallpaper-clear":
        ("Use the Ambience's background",
         "Utiliser le fond de l’ambiance"),
    "threesea-me-font-size-small":
        ("Small",
         "Petite"),
    "threesea-me-font-size-normal":
        ("Normal",
         "Normale"),
    "threesea-me-font-size-large":
        ("Large",
         "Grande"),
    "threesea-la-chat-font-size":
        ("Message text",
         "Taille du texte des messages"),
    "threesea-la-wallpaper":
        ("Chat background",
         "Fond des discussions"),
    "threesea-la-wallpaper-ambience":
        ("The Ambience's background",
         "Le fond de l’ambiance"),
    "threesea-la-receive-pictures":
        ("Show profile pictures",
         "Afficher les photos de profil"),
    "threesea-la-receive-pictures-description":
        ("Contacts who share their profile picture are shown with it. Pictures they send keep arriving while this is off, and appear when it is turned on again.",
         "Les contacts qui partagent leur photo de profil apparaissent avec celle-ci. Les photos qu’ils envoient continuent d’arriver pendant que ce réglage est désactivé, et apparaissent quand il est réactivé."),
    "threesea-la-group-calls":
        ("Group calls",
         "Appels de groupe"),
    "threesea-la-group-calls-description":
        ("You can start group calls and join the ones running in your groups.",
         "Vous pouvez démarrer des appels de groupe et rejoindre ceux en cours dans vos groupes."),
    "threesea-he-rename-device":
        ("Rename device",
         "Renommer l’appareil"),
    "threesea-la-device-name":
        ("Device name",
         "Nom de l’appareil"),
    "threesea-la-device-name-own-description":
        ("The other devices show this name for this phone.",
         "Les autres appareils affichent ce nom pour ce téléphone."),
    "threesea-la-device-name-alias-description":
        ("Only this phone shows this name; the device keeps its own name everywhere else.",
         "Seul ce téléphone affiche ce nom ; l’appareil garde le sien partout ailleurs."),
    "threesea-me-rename-device":
        ("Rename",
         "Renommer"),
    "threesea-me-keep-forever":
        ("For good",
         "Pour toujours"),
    "threesea-me-keep-1-year":
        ("1 year",
         "1 an"),
    "threesea-me-keep-6-months":
        ("6 months",
         "6 mois"),
    "threesea-me-keep-3-months":
        ("3 months",
         "3 mois"),
    "threesea-me-keep-1-month":
        ("1 month",
         "1 mois"),
    "threesea-me-keep-1-week":
        ("1 week",
         "1 semaine"),
    "threesea-me-video-small":
        ("Small (480 px)",
         "Petite (480 px)"),
    "threesea-me-video-medium":
        ("Medium (848 px)",
         "Moyenne (848 px)"),
    "threesea-la-video-size":
        ("Video size",
         "Taille des vidéos"),
    "threesea-la-video-size-description":
        ("Smaller videos are quicker to send and use less data. The sound is kept as it is.",
         "Les vidéos plus petites s’envoient plus vite et consomment moins de données. Le son est conservé tel quel."),
    "threesea-la-keep-messages":
        ("Keep messages",
         "Conserver les messages"),
    "threesea-la-keep-messages-description":
        ("Messages older than the chosen time are deleted by themselves, with their media. Starred messages, open polls, messages still on their way and groups with only you in them stay.",
         "Les messages plus anciens que la durée choisie sont supprimés d’eux-mêmes, avec leurs médias. Les messages marqués d’une étoile, les sondages ouverts, les messages encore en route et les groupes dont vous êtes le seul membre restent."),
    "threesea-la-exporting-chat":
        ("Exporting chat",
         "Exportation de la discussion"),
    "threesea-me-export-chat":
        ("Export chat",
         "Exporter la discussion"),
    "threesea-he-export":
        ("Export",
         "Exporter"),
    "threesea-he-export-chat":
        ("Export chat",
         "Exporter la discussion"),
    "threesea-la-export-chat-description":
        ("The messages of %1 are written as a text file into a ZIP archive in your Downloads folder.",
         "Les messages de %1 sont écrits dans un fichier texte, à l’intérieur d’une archive ZIP placée dans votre dossier Téléchargements."),
    "threesea-la-export-chat-media":
        ("Include pictures and files",
         "Inclure les images et les fichiers"),
    "threesea-la-export-chat-media-description":
        ("Everything that has been downloaded to this phone goes into the archive.",
         "Tout ce qui a été téléchargé sur ce téléphone entre dans l’archive."),
    "threesea-me-limit-any":
        ("Any size",
         "Toute taille"),
    "threesea-me-limit-1":
        ("Up to 1 MB",
         "Jusqu’à 1 Mo"),
    "threesea-me-limit-5":
        ("Up to 5 MB",
         "Jusqu’à 5 Mo"),
    "threesea-me-limit-20":
        ("Up to 20 MB",
         "Jusqu’à 20 Mo"),
    "threesea-me-limit-100":
        ("Up to 100 MB",
         "Jusqu’à 100 Mo"),
    "threesea-la-download-limit":
        ("File size",
         "Taille des fichiers"),
    "threesea-la-download-limit-description":
        ("Larger files wait in the chat until you tap them. Previews always come.",
         "Les fichiers plus grands attendent dans la discussion que vous les touchiez. Les aperçus arrivent toujours."),
    "threesea-me-photo-turn-right":
        ("Turn right",
         "Tourner à droite"),
    "threesea-me-photo-turn-left":
        ("Turn left",
         "Tourner à gauche"),
    "threesea-me-photo-whole":
        ("Whole picture",
         "Image entière"),
    "threesea-he-photo-use":
        ("Use",
         "Utiliser"),
    "threesea-he-edit-photo":
        ("Edit photo",
         "Modifier la photo"),
    "threesea-la-edit-photo-failed":
        ("The photo could not be edited.",
         "La photo n’a pas pu être modifiée."),
    "threesea-la-license-notification":
        ("Threesea is offline",
         "Threesea est hors ligne"),
    "threesea-la-license-notification-body":
        ("Threema no longer accepts this license key. Open Threesea to check it.",
         "Threema n’accepte plus cette clé de licence. Ouvrez Threesea pour la vérifier."),
    "threesea-he-license-key":
        ("License key",
         "Clé de licence"),
    "threesea-la-license-refused":
        ("Threesea stays offline until a license key is accepted. Your chats stay on this device.",
         "Threesea reste hors ligne jusqu’à ce qu’une clé de licence soit acceptée. Vos discussions restent sur cet appareil."),
    "threesea-la-spam-reported":
        ("Reported to Threema as spam.",
         "Signalé à Threema comme spam."),
    "threesea-la-spam-unknown-group":
        ("This group is from an unknown contact",
         "Ce groupe provient d’un contact inconnu"),
    "threesea-la-spam-unknown-contact":
        ("This chat is from an unknown contact",
         "Cette discussion provient d’un contact inconnu"),
    "threesea-la-spam-tap-to-report":
        ("Tap to report spam",
         "Touchez pour signaler un spam"),
    "threesea-he-spam-report":
        ("Report",
         "Signaler"),
    "threesea-he-spam-report-name":
        ("Report %1",
         "Signaler %1"),
    "threesea-la-spam-explain-group":
        ("This sends your Threema ID to Threema, with the group's ID and number of members and the ID and nickname of its creator. No messages and not the group's name.",
         "Ceci envoie votre ID Threema à Threema, avec l’ID et le nombre de membres du groupe ainsi que l’ID et le surnom de son créateur. Ni les messages ni le nom du groupe ne sont envoyés."),
    "threesea-la-spam-explain-contact":
        ("This sends your Threema ID to Threema, with the ID and nickname of this contact. No messages.",
         "Ceci envoie votre ID Threema à Threema, avec l’ID et le surnom de ce contact. Aucun message n’est envoyé."),
    "threesea-la-spam-block-group":
        ("Also block the creator, leave the group and delete the chat",
         "Bloquer aussi le créateur, quitter le groupe et supprimer la discussion"),
    "threesea-la-spam-block-contact":
        ("Also block this contact and delete the chat",
         "Bloquer aussi ce contact et supprimer la discussion"),
    "threesea-la-spam-report-failed":
        ("The report could not be sent.",
         "Le signalement n’a pas pu être envoyé."),
    "threesea-la-address-book-none-new":
        ("%n address book entries checked, no new contacts.",
         ("%n entrée du carnet d’adresses vérifiée, aucun nouveau contact.",
          "%n entrées du carnet d’adresses vérifiées, aucun nouveau contact.")),
    "threesea-la-address-book-empty":
        ("No phone numbers or e-mail addresses found in the address book.",
         "Aucun numéro de téléphone ni aucune adresse e-mail dans le carnet d’adresses."),
    "threesea-la-link-mobile-target":
        ("The code goes to +%1.",
         "Le code sera envoyé au +%1."),
    "threesea-la-email-already-linked":
        ("This address is already linked to your ID, so no e-mail is needed.",
         "Cette adresse est déjà liée à votre ID, aucun e-mail n’est nécessaire."),
    "threesea-la-phone-already-linked":
        ("This number is already linked to your ID, so no SMS is needed.",
         "Ce numéro est déjà lié à votre ID, aucun SMS n’est nécessaire."),
    "threesea-la-country-ae":
        ("United Arab Emirates",
         "Émirats arabes unis"),
    "threesea-la-country-at":
        ("Austria",
         "Autriche"),
    "threesea-la-country-au":
        ("Australia",
         "Australie"),
    "threesea-la-country-be":
        ("Belgium",
         "Belgique"),
    "threesea-la-country-bg":
        ("Bulgaria",
         "Bulgarie"),
    "threesea-la-country-br":
        ("Brazil",
         "Brésil"),
    "threesea-la-country-ca":
        ("Canada",
         "Canada"),
    "threesea-la-country-ch":
        ("Switzerland",
         "Suisse"),
    "threesea-la-country-cn":
        ("China",
         "Chine"),
    "threesea-la-country-cy":
        ("Cyprus",
         "Chypre"),
    "threesea-la-country-cz":
        ("Czechia",
         "Tchéquie"),
    "threesea-la-country-de":
        ("Germany",
         "Allemagne"),
    "threesea-la-country-dk":
        ("Denmark",
         "Danemark"),
    "threesea-la-country-ee":
        ("Estonia",
         "Estonie"),
    "threesea-la-country-es":
        ("Spain",
         "Espagne"),
    "threesea-la-country-fi":
        ("Finland",
         "Finlande"),
    "threesea-la-country-fr":
        ("France",
         "France"),
    "threesea-la-country-gb":
        ("United Kingdom",
         "Royaume-Uni"),
    "threesea-la-country-gr":
        ("Greece",
         "Grèce"),
    "threesea-la-country-hr":
        ("Croatia",
         "Croatie"),
    "threesea-la-country-hu":
        ("Hungary",
         "Hongrie"),
    "threesea-la-country-ie":
        ("Ireland",
         "Irlande"),
    "threesea-la-country-il":
        ("Israel",
         "Israël"),
    "threesea-la-country-in":
        ("India",
         "Inde"),
    "threesea-la-country-is":
        ("Iceland",
         "Islande"),
    "threesea-la-country-it":
        ("Italy",
         "Italie"),
    "threesea-la-country-jp":
        ("Japan",
         "Japon"),
    "threesea-la-country-kr":
        ("South Korea",
         "Corée du Sud"),
    "threesea-la-country-li":
        ("Liechtenstein",
         "Liechtenstein"),
    "threesea-la-country-lu":
        ("Luxembourg",
         "Luxembourg"),
    "threesea-la-country-lv":
        ("Latvia",
         "Lettonie"),
    "threesea-la-country-mt":
        ("Malta",
         "Malte"),
    "threesea-la-country-mx":
        ("Mexico",
         "Mexique"),
    "threesea-la-country-nl":
        ("Netherlands",
         "Pays-Bas"),
    "threesea-la-country-no":
        ("Norway",
         "Norvège"),
    "threesea-la-country-nz":
        ("New Zealand",
         "Nouvelle-Zélande"),
    "threesea-la-country-pl":
        ("Poland",
         "Pologne"),
    "threesea-la-country-pt":
        ("Portugal",
         "Portugal"),
    "threesea-la-country-ro":
        ("Romania",
         "Roumanie"),
    "threesea-la-country-rs":
        ("Serbia",
         "Serbie"),
    "threesea-la-country-ru":
        ("Russia",
         "Russie"),
    "threesea-la-country-se":
        ("Sweden",
         "Suède"),
    "threesea-la-country-sg":
        ("Singapore",
         "Singapour"),
    "threesea-la-country-si":
        ("Slovenia",
         "Slovénie"),
    "threesea-la-country-sk":
        ("Slovakia",
         "Slovaquie"),
    "threesea-la-country-tr":
        ("Turkey",
         "Turquie"),
    "threesea-la-country-ua":
        ("Ukraine",
         "Ukraine"),
    "threesea-la-country-us":
        ("United States",
         "États-Unis"),
    "threesea-la-country-za":
        ("South Africa",
         "Afrique du Sud"),
    "threesea-la-address-book-country-automatic":
        ("Automatic",
         "Automatique"),
    "threesea-la-address-book-country-automatic-code":
        ("Automatic (+%1)",
         "Automatique (+%1)"),
    "threesea-la-address-book-country":
        ("Numbers without a country code",
         "Numéros sans indicatif international"),
    "threesea-la-address-book-country-description":
        ("Address book numbers written without a country code count as from this country. Automatic goes by your linked phone number, else by the device's region. Numbers from other countries are found when written with + and their country code.",
         "Les numéros du carnet d’adresses écrits sans indicatif international sont considérés comme venant de ce pays. Automatique se règle sur votre numéro de téléphone lié, sinon sur la région de l’appareil. Les numéros d’autres pays sont trouvés s’ils sont écrits avec + et leur indicatif."),
    "threesea-la-no-emoji-found":
        ("No emoji found",
         "Aucun émoji trouvé"),
    "threesea-la-upgrading-hint":
        ("Updating the stored chats. This can take a moment; keep Threesea open.",
         "Mise à jour des discussions enregistrées. Cela peut prendre un moment ; laissez Threesea ouvert."),
    "threesea-la-error-text-too-long":
        ("The text is too long. Shorten it and try again.",
         "Le texte est trop long. Raccourcissez-le et réessayez."),
    "threesea-la-error-poll-results-too-long":
        ("The results of this poll are too long to send. The poll stays open.",
         "Les résultats de ce sondage sont trop longs pour être envoyés. Le sondage reste ouvert."),
    "threesea-la-error-backup-no-identity":
        ("This backup holds no Threema ID. Restore the ID from Threema Safe or an ID backup, then import this backup in Settings, Backup, Data backup.",
         "Cette sauvegarde ne contient pas d’ID Threema. Restaurez l’ID depuis Threema Safe ou une sauvegarde d’ID, puis importez cette sauvegarde dans Paramètres, Sauvegarde, Sauvegarde des données."),
    "threesea-la-error-backup-write":
        ("The backup could not be written. Check the backup folder.",
         "La sauvegarde n’a pas pu être écrite. Vérifiez le dossier de sauvegarde."),
    "threesea-la-error-backup-no-room":
        ("The backup folder's storage has too little free space for a backup. Free some space or choose another folder.",
         "Le stockage du dossier de sauvegarde n’a pas assez d’espace libre pour une sauvegarde. Libérez de l’espace ou choisissez un autre dossier."),
    "threesea-la-error-backup-running":
        ("A backup is being written right now.",
         "Une sauvegarde est en cours d’écriture."),
    "threesea-la-error-backup-no-folder":
        ("Choose a backup folder first.",
         "Choisissez d’abord un dossier de sauvegarde."),
    "threesea-la-error-backup-no-password":
        ("Enter a backup password first.",
         "Saisissez d’abord un mot de passe de sauvegarde."),
    "threesea-la-error-safe-damaged":
        ("This Threema Safe backup is damaged and can't be read.",
         "Cette sauvegarde Threema Safe est endommagée et illisible."),
    "threesea-la-error-contact-revoked":
        ("This Threema ID was revoked.",
         "Cet ID Threema a été révoqué."),
    "threesea-la-error-contact-unknown":
        ("Block unknown senders is on, and this person is not one of your contacts.",
         "Bloquer les expéditeurs inconnus est activé, et cette personne ne fait pas partie de vos contacts."),
    "threesea-la-error-qr-expired":
        ("This code has expired. Scan it directly from the other person's app.",
         "Ce code a expiré. Scannez-le directement dans l’application de l’autre personne."),
    "threesea-la-error-video-metadata":
        ("A video was not sent, because its metadata, which can hold where and when it was made, could not be removed. It can still be sent as a file.",
         "Une vidéo n’a pas été envoyée, car ses métadonnées, qui peuvent indiquer où et quand elle a été filmée, n’ont pas pu être supprimées. Elle peut encore être envoyée comme fichier."),
    "threesea-la-error-file-picture-metadata":
        ("A picture was not sent, because its metadata, which can hold where and when it was made, could not be removed. It can still be sent as a file.",
         "Une image n’a pas été envoyée, car ses métadonnées, qui peuvent indiquer où et quand elle a été prise, n’ont pas pu être supprimées. Elle peut encore être envoyée comme fichier."),
    "threesea-la-error-picture-metadata":
        ("A picture was not sent, because its metadata, which can hold where and when it was made, could not be removed. Save it, and it can be sent as a file.",
         "Une image n’a pas été envoyée, car ses métadonnées, qui peuvent indiquer où et quand elle a été prise, n’ont pas pu être supprimées. Enregistrez-la, et elle pourra être envoyée comme fichier."),
    "threesea-la-error-not-web-code":
        ("This is not a Threema Web code. Scan the code Threema Web shows in the browser.",
         "Ceci n’est pas un code Threema Web. Scannez le code que Threema Web affiche dans le navigateur."),
    "threesea-la-error-web-disabled":
        ("Threema Web is switched off. Turn it on in the settings under Threema Web.",
         "Threema Web est désactivé. Activez-le dans les paramètres, sous Threema Web."),
    "threesea-la-error-web-already-paired":
        ("This browser is already in the list under Threema Web. Remove it there to connect it with a new code.",
         "Ce navigateur figure déjà dans la liste sous Threema Web. Retirez-le là-bas pour le connecter avec un nouveau code."),
    "threesea-la-error-link-removed":
        ("Linking didn't finish, so the devices in the device group were removed again. The computer had already received your ID. Close Threema for Desktop, open it again and scan its new code.",
         "L’association n’est pas allée au bout : les appareils du groupe d’appareils ont donc été retirés. L’ordinateur avait déjà reçu votre ID. Fermez Threema for Desktop, rouvrez-le et scannez son nouveau code."),
    "threesea-la-error-link-uncertain":
        ("Linking didn't finish, but the computer received your ID and may be linked anyway. Linked devices shows what is linked once you are online, and you can remove it there.",
         "L’association n’est pas allée au bout, mais l’ordinateur a reçu votre ID et est peut-être associé malgré tout. Appareils associés montre ce qui est associé une fois que vous êtes en ligne, et vous pouvez l’y retirer."),
    "threesea-la-error-group-call-full":
        ("This group call is full.",
         "Cet appel de groupe est complet."),
    "threesea-la-error-group-call-not-running":
        ("This group call has ended.",
         "Cet appel de groupe est terminé."),
    "threesea-la-error-group-call-version":
        ("This group call needs a different version of the app.",
         "Cet appel de groupe nécessite une autre version de l’application."),
    "threesea-la-error-group-call-connection":
        ("The connection to the group call failed.",
         "La connexion à l’appel de groupe a échoué."),
    "threesea-la-error-group-call-audio":
        ("The microphone or the speaker stopped working, so the group call ended.",
         "Le microphone ou le haut-parleur a cessé de fonctionner : l’appel de groupe a pris fin."),
    "threesea-la-error-contact-not-found":
        ("Add this ID to your contacts to write to it.",
         "Ajoutez cet ID à vos contacts pour lui écrire."),
    "threesea-la-error-group-not-found":
        ("This group is no longer on this device.",
         "Ce groupe n’est plus sur cet appareil."),
    "threesea-la-error-no-identity":
        ("Create or restore your Threema ID first.",
         "Créez ou restaurez d’abord votre ID Threema."),
    "threesea-la-error-server-trouble":
        ("Threema's server didn't accept this. Try again later.",
         "Le serveur de Threema n’a pas accepté cela. Réessayez plus tard."),
    "threesea-la-error-rate-limited":
        ("Threema's servers are busy. Try again in a few minutes.",
         "Les serveurs de Threema sont surchargés. Réessayez dans quelques minutes."),
    "threesea-la-error-server-unexplained":
        ("Threema's servers turned this down and gave no reason. Try again later.",
         "Les serveurs de Threema ont refusé cela sans donner de raison. Réessayez plus tard."),
    "threesea-la-error-report-refused":
        ("Threema didn't accept the report. Try again later.",
         "Threema n’a pas accepté le signalement. Réessayez plus tard."),
    "threesea-la-error-id-search-input":
        ("Enter a phone number with its country code, or an e-mail address.",
         "Saisissez un numéro de téléphone avec son indicatif international, ou une adresse e-mail."),
    "threesea-la-error-identity-exists":
        ("This device already has a Threema ID.",
         "Cet appareil a déjà un ID Threema."),
    "threesea-la-error-disk-full":
        ("The phone's storage is full. Free some space.",
         "Le stockage du téléphone est plein. Libérez de l’espace."),
    "threesea-la-error-upgrade-no-room":
        ("Updating the stored chats needs more free space. Free some space; the chats stay as they were until then.",
         "La mise à jour des discussions enregistrées demande plus d’espace libre. Libérez de l’espace ; d’ici là, les discussions restent telles quelles."),
    "threesea-la-error-store-too-new":
        ("This version of Threesea is older than your stored data. Install the newest version.",
         "Cette version de Threesea est plus ancienne que vos données enregistrées. Installez la version la plus récente."),
    "threesea-la-error-store-key":
        ("The stored data could not be opened: the key does not fit, or the file is damaged. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Les données enregistrées n’ont pas pu être ouvertes : la clé ne correspond pas, ou le fichier est endommagé. Threesea laisse les données telles quelles. Essayez de fermer puis rouvrir Threesea, puis de redémarrer le téléphone."),
    "threesea-la-error-store-damaged":
        ("Part of the stored data could not be read. It may be damaged. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Une partie des données enregistrées n’a pas pu être lue. Elle est peut-être endommagée. Threesea laisse les données telles quelles. Essayez de fermer puis rouvrir Threesea, puis de redémarrer le téléphone."),
    "threesea-la-error-store-unreadable":
        ("The stored data could not be read. Check that the phone has free storage.",
         "Les données enregistrées n’ont pas pu être lues. Vérifiez que le téléphone a de l’espace libre."),
    "threesea-la-error-store":
        ("The stored data could not be opened.",
         "Les données enregistrées n’ont pas pu être ouvertes."),
    "threesea-la-error-key-missing":
        ("The key to the stored data was not found in the phone's key storage, so the data could not be opened. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "La clé des données enregistrées est introuvable dans le stockage des clés du téléphone : les données n’ont pas pu être ouvertes. Threesea laisse les données telles quelles. Essayez de fermer puis rouvrir Threesea, puis de redémarrer le téléphone."),
    "threesea-la-error-secrets":
        ("Threesea could not get the key to its data from the phone's key storage. Allow the access if the phone asks for it.",
         "Threesea n’a pas pu obtenir la clé de ses données depuis le stockage des clés du téléphone. Autorisez l’accès si le téléphone le demande."),
    "threesea-la-error-internal-plain":
        ("Something went wrong.",
         "Quelque chose s’est mal passé."),
    "threesea-la-gif":
        ("GIF",
         "GIF"),
    "threesea-la-lock-private-passphrase":
        ("Passphrase for private chats",
         "Phrase secrète des discussions privées"),
    "threesea-la-lock-private-pin":
        ("PIN for private chats",
         "Code PIN des discussions privées"),
    "threesea-la-status-group-no-longer-member":
        ("You are no longer a member of this group.",
         "Vous n’êtes plus membre de ce groupe."),
    "threesea-la-status-group-call-started-unnamed":
        ("A group call started.",
         "Un appel de groupe a démarré."),
    "threesea-la-status-group-call-started":
        ("%1 started a group call.",
         "%1 a démarré un appel de groupe."),
    "threesea-la-status-group-call-you-started":
        ("You started a group call.",
         "Vous avez démarré un appel de groupe."),
    "threesea-la-status-group-call-ended":
        ("The group call ended.",
         "L’appel de groupe a pris fin."),
    "threesea-la-status-fs-session-reset":
        ("Perfect Forward Secrecy was reset. This can happen when the contact changes devices or reinstalls the app.",
         "La confidentialité persistante a été réinitialisée. Cela peut arriver quand le contact change d’appareil ou réinstalle l’application."),
    "threesea-la-status-fs-message-rejected":
        ("Perfect Forward Secrecy was reset. Affected messages are sent again.",
         "La confidentialité persistante a été réinitialisée. Les messages concernés sont renvoyés."),
    "threesea-la-status-fs-message-without":
        ("A message without Perfect Forward Secrecy was received. Check with the contact that they turned it off on purpose.",
         "Un message sans confidentialité persistante a été reçu. Vérifiez avec le contact qu’il l’a désactivée volontairement."),
    "threesea-la-status-fs-unavailable":
        ("Perfect Forward Secrecy is off in this chat because this contact's app is not using it at the moment.",
         "La confidentialité persistante est désactivée dans cette discussion, car l’application de ce contact ne l’utilise pas pour le moment."),
    "threesea-la-status-fs-session-ended":
        ("The Perfect Forward Secrecy session with this contact ended.",
         "La session de confidentialité persistante avec ce contact a pris fin."),
    "threesea-me-copy-id":
        ("Copy ID",
         "Copier l’ID"),
    "threesea-la-copied":
        ("Copied",
         "Copié"),
    "threesea-me-share-id":
        ("Share ID",
         "Partager l’ID"),
    "threesea-he-share-id":
        ("Share ID",
         "Partager l’ID"),
    "threesea-me-send-again":
        ("Send again",
         "Envoyer à nouveau"),
    "threesea-me-stop-download":
        ("Stop download",
         "Arrêter le téléchargement"),
    "threesea-me-stop-upload":
        ("Stop sending",
         "Arrêter l’envoi"),
    "threesea-me-share":
        ("Share",
         "Partager"),
    "threesea-me-draw-on-picture":
        ("Draw on it",
         "Dessiner dessus"),
    "threesea-la-saved-n-downloads":
        ("Saved %n file(s) to Downloads",
         ("%n fichier enregistré dans Téléchargements",
          "%n fichiers enregistrés dans Téléchargements")),
    "threesea-la-download-failed":
        ("The file could not be downloaded. It may no longer be on the server.",
         "Le fichier n’a pas pu être téléchargé. Il n’est peut-être plus sur le serveur."),
    "threesea-la-send-in-turn-failed":
        ("Threesea did not send everything",
         "Threesea n’a pas tout envoyé"),
    "threesea-la-not-sent-to-chats":
        ("Not everything went to %n chat(s)",
         ("Tout n’est pas parti vers %n discussion",
          "Tout n’est pas parti vers %n discussions")),
    "threesea-la-sent-to-chats":
        ("On its way to %n chat(s)",
         ("En route vers %n discussion",
          "En route vers %n discussions")),
    "threesea-la-send-in-turn-lost":
        ("Some chats did not get what you sent them. Send it to them again.",
         "Certaines discussions n’ont pas reçu ce que vous leur avez envoyé. Envoyez-le-leur à nouveau."),
    "threesea-la-locked-notification-reply-body":
        ("Open it and enter your PIN or passphrase to send your reply and receive messages.",
         "Ouvrez Threesea et saisissez votre code PIN ou votre phrase secrète pour envoyer votre réponse et recevoir les messages."),
    "threesea-la-startup-failed-notification-body":
        ("Open it to see why. Messages arrive again once it runs.",
         "Ouvrez-le pour savoir pourquoi. Les messages arrivent à nouveau dès qu’il fonctionne."),
    "threesea-la-safe-failing-notification":
        ("Threema Safe backup failing",
         "La sauvegarde Threema Safe échoue"),
    "threesea-la-safe-failing-notification-body":
        ("Threema Safe hasn't been able to back up for %n day(s). Tap to check.",
         ("Threema Safe n’a pas pu sauvegarder depuis %n jour. Touchez pour vérifier.",
          "Threema Safe n’a pas pu sauvegarder depuis %n jours. Touchez pour vérifier.")),
    "threesea-la-reason-login-refused":
        ("Threema's server keeps closing the connection during login.",
         "Le serveur de Threema ferme sans cesse la connexion pendant l’identification."),
    "threesea-la-reason-another-connection":
        ("Another device keeps taking over this Threema ID. An ID works on one phone at a time, so Threesea tries again every 10 minutes.",
         "Un autre appareil reprend sans cesse cet ID Threema. Un ID fonctionne sur un seul téléphone à la fois : Threesea réessaie toutes les 10 minutes."),
    "threesea-la-reason-mediator-version":
        ("Threema's server for linked devices no longer accepts this version of Threesea.",
         "Le serveur de Threema pour les appareils associés n’accepte plus cette version de Threesea."),
    "threesea-la-reason-mediator-device-limit":
        ("Threema's server for linked devices turned this device away: this Threema ID has reached its limit of linked devices.",
         "Le serveur de Threema pour les appareils associés a refusé cet appareil : cet ID Threema a atteint sa limite d’appareils associés."),
    "threesea-la-reason-mediator-duplicate":
        ("Another connection under this device's login took over on Threema's server for linked devices.",
         "Une autre connexion avec l’identifiant de cet appareil a pris le relais sur le serveur de Threema pour les appareils associés."),
    "threesea-la-reason-mediator-queue-full":
        ("Too many messages are waiting for this device on Threema's server for linked devices, so it closed the connection.",
         "Trop de messages attendent cet appareil sur le serveur de Threema pour les appareils associés : il a fermé la connexion."),
    "threesea-la-reason-mediator-other":
        ("Threema's server for linked devices closed the connection (code %1).",
         "Le serveur de Threema pour les appareils associés a fermé la connexion (code %1)."),
    "threesea-la-server-message-notification":
        ("Message from Threema's server",
         "Message du serveur de Threema"),
    "threesea-la-opens-after-call":
        ("Opens when the call is over",
         "S’ouvre à la fin de l’appel"),
    "threesea-la-capture-discarded":
        ("The photo or video waiting to be sent was discarded",
         "La photo ou la vidéo en attente d’envoi a été abandonnée"),
    "threesea-la-mobile-verified":
        ("Phone number confirmed",
         "Numéro de téléphone confirmé"),
    "threesea-la-share-before-setup":
        ("Set up Threesea first, then share this again",
         "Configurez d’abord Threesea, puis partagez cela à nouveau"),
    "threesea-la-storage-problem-title":
        ("Saving messages failed",
         "L’enregistrement des messages a échoué"),
    "threesea-la-storage-problem-body":
        ("Threesea can't write to its data, so new messages wait on the server, for a limited time. Check that this device has free storage. The log in Settings, Advanced has the error code.",
         "Threesea ne peut pas écrire dans ses données : les nouveaux messages attendent sur le serveur, pour une durée limitée. Vérifiez que cet appareil a de l’espace libre. Le journal dans Paramètres, Avancé contient le code d’erreur."),
    "threesea-la-web-connected-notification":
        ("Connected to Threema Web",
         "Connecté à Threema Web"),
    "threesea-la-web-connected-notification-body":
        ("%n browser(s) can read and send your messages through this phone. Tap to manage.",
         ("%n navigateur peut lire et envoyer vos messages par ce téléphone. Touchez pour gérer.",
          "%n navigateurs peuvent lire et envoyer vos messages par ce téléphone. Touchez pour gérer.")),
    "threesea-la-web-certificate-notification":
        ("Threema Web server has a new certificate",
         "Le serveur Threema Web a un nouveau certificat"),
    "threesea-la-web-certificate-notification-body":
        ("Tap to decide whether to trust it.",
         "Touchez pour décider si vous lui faites confiance."),
    "threesea-me-call-decline":
        ("Decline",
         "Refuser"),
    "threesea-me-call-answer":
        ("Answer",
         "Répondre"),
    "threesea-la-app-locked-reply-body":
        ("Open it and enter your PIN or passphrase to send your reply.",
         "Ouvrez Threesea et saisissez votre code PIN ou votre phrase secrète pour envoyer votre réponse."),
    "threesea-la-updated":
        ("Threesea was updated",
         "Threesea a été mis à jour"),
    "threesea-la-removed":
        ("Threesea was removed from this device",
         "Threesea a été supprimé de cet appareil"),
    "threesea-la-reply-not-sent":
        ("Your reply was not sent",
         "Votre réponse n’a pas été envoyée"),
    "threesea-la-app-locked-body":
        ("Something new has arrived. Open it and enter your PIN or passphrase to see it.",
         "Du nouveau est arrivé. Ouvrez Threesea et saisissez votre code PIN ou votre phrase secrète pour le voir."),
    "threesea-la-removed-body":
        ("Close it to end it.",
         "Fermez-le pour le quitter."),
    "threesea-la-updated-body":
        ("Close it and open it again to use the new version.",
         "Fermez-le et rouvrez-le pour utiliser la nouvelle version."),
    "threesea-la-updated-open-body":
        ("Tap to open the new version.",
         "Touchez pour ouvrir la nouvelle version."),
    "threesea-la-updated-closed-body":
        ("Open it to receive messages again.",
         "Ouvrez-le pour recevoir à nouveau les messages."),
    "threesea-la-reply-not-sent-ended-kept":
        ("Threesea ended before sending it. Your text is kept as a draft in its chat.",
         "Threesea s’est arrêté avant de l’envoyer. Votre texte est conservé comme brouillon dans sa discussion."),
    "threesea-la-reply-not-sent-ended":
        ("Threesea ended before sending it. Write it again in the chat.",
         "Threesea s’est arrêté avant de l’envoyer. Écrivez-la à nouveau dans la discussion."),
    "threesea-la-delete-data-linked-remain":
        ("This phone could not reach the server, so your linked devices stay linked to this ID.",
         "Ce téléphone n’a pas pu joindre le serveur : vos appareils associés restent associés à cet ID."),
    "threesea-la-reply-not-sent-body":
        ("Threesea could not start. Open it to see why.",
         "Threesea n’a pas pu démarrer. Ouvrez-le pour savoir pourquoi."),
    "threesea-la-reply-not-sent-yet":
        ("It is sent when you next open Threesea.",
         "Elle sera envoyée à la prochaine ouverture de Threesea."),
    "threesea-la-reply-not-sent-kept":
        ("%1 Your text is kept as a draft in the chat.",
         "%1 Votre texte est conservé comme brouillon dans la discussion."),
    "threesea-la-reply-not-sent-late":
        ("It reached Threesea too late. Write it again in the chat.",
         "Elle est arrivée trop tard à Threesea. Écrivez-la à nouveau dans la discussion."),
    "threesea-la-share-empty":
        ("What was shared holds nothing to send",
         "Ce qui a été partagé ne contient rien à envoyer"),
    "threesea-la-share-own-files":
        ("Files of Threesea's own were left out of what was shared",
         "Des fichiers propres à Threesea ont été écartés de ce qui a été partagé"),
    "threesea-la-call-route-failed":
        ("The sound could not be switched",
         "Le son n’a pas pu être basculé"),
    "threesea-la-recording-discarded":
        ("Recording stopped and discarded",
         "Enregistrement arrêté et abandonné"),
    "threesea-la-delete-data-failed":
        ("The ID was not removed: %1",
         "L’ID n’a pas été supprimé : %1"),
    "threesea-la-backup-reminder":
        ("Back up your Threema ID",
         "Sauvegardez votre ID Threema"),
    "threesea-la-backup-reminder-body":
        ("A backup brings your ID, contacts and groups back on a new phone. Tap to set up Threema Safe.",
         "Une sauvegarde ramène votre ID, vos contacts et vos groupes sur un nouveau téléphone. Touchez pour configurer Threema Safe."),
    "threesea-la-safe-server-copy-left":
        ("Threema Safe is off. The backup could not be deleted from Threema's server, so it stays there until it expires.",
         "Threema Safe est désactivé. La sauvegarde n’a pas pu être supprimée du serveur de Threema : elle y reste jusqu’à son expiration."),
    "threesea-la-device-removal-failed":
        ("Removing linked devices did not finish. Linked devices shows what is still linked, and you can try again there.",
         "Le retrait des appareils associés n’est pas allé au bout. Appareils associés montre ce qui est encore associé, et vous pouvez y réessayer."),
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
         "Mots-clés de recherche des émojis issus de threema-android (AGPL-3.0)."),
    "threesea-la-license-copyright":
        ("Copyright © 2026 the Threesea developers. libthreema and the code taken from threema-android: copyright Threema AG.",
         "Copyright © 2026 les développeurs de Threesea. libthreema et le code repris de threema-android : copyright Threema AG."),
    "threesea-la-license-source":
        ("Source code and bug reports: %1",
         "Code source et signalement de bogues : %1"),
    "threesea-bt-license-agpl":
        ("License text",
         "Texte de la licence"),
    "threesea-bt-license-notices":
        ("Third-party licenses",
         "Licences tierces"),
    "threesea-la-add-scanned-contact":
        ("Adds this Threema ID to your contacts with the key from Threema's directory. To verify it, scan the QR code in the other person's app.",
         "Ajoute cet ID Threema à vos contacts avec la clé de l’annuaire de Threema. Pour le vérifier, scannez le code QR dans l’application de l’autre personne."),
    "threesea-la-app-lock-about":
        ("With a lock you can also make chats private: they are hidden from the chat list and are opened from its bottom menu. If you forget the app lock's PIN or passphrase, the way back in is to delete Threesea's data folder .local/share/org.threesea/harbour-threesea with the chats and media in it, restart the phone and restore from a backup. A passphrase for the stored data stays set: Threesea asks for it first.",
         "Avec un verrouillage, vous pouvez aussi rendre des discussions privées : elles sont masquées dans la liste des discussions et s’ouvrent depuis son menu du bas. Si vous oubliez le code PIN ou la phrase secrète du verrouillage, le seul moyen de revenir est de supprimer le dossier de données de Threesea, .local/share/org.threesea/harbour-threesea, avec les discussions et les médias qu’il contient, de redémarrer le téléphone et de restaurer depuis une sauvegarde. Une phrase secrète pour les données enregistrées reste définie : Threesea la demande d’abord."),
    "threesea-la-call-interrupted":
        ("Connection interrupted…",
         "Connexion interrompue…"),
    "threesea-la-call-unverified":
        ("The connection could not be verified. The call was ended.",
         "La connexion n’a pas pu être vérifiée. L’appel a été terminé."),
    "threesea-la-call-connection-failed":
        ("Connection failed",
         "Échec de la connexion"),
    "threesea-la-camera-photo-failed":
        ("The photo could not be taken.",
         "La photo n’a pas pu être prise."),
    "threesea-la-camera-video-failed":
        ("The video could not be recorded.",
         "La vidéo n’a pas pu être enregistrée."),
    "threesea-la-sending-drawing":
        ("Sending drawing",
         "Envoi du dessin"),
    "threesea-la-discarding-drawing":
        ("Discarding drawing",
         "Abandon du dessin"),
    "threesea-la-discarding-recording":
        ("Discarding recording",
         "Abandon de l’enregistrement"),
    "threesea-la-recording-held":
        ("Recording stopped. Send it or discard it.",
         "Enregistrement arrêté. Envoyez-le ou abandonnez-le."),
    "threesea-la-recording-not-playable":
        ("This recording could not be played. It can still be sent.",
         "Cet enregistrement n’a pas pu être lu. Il peut quand même être envoyé."),
    "threesea-la-spam-reporting-group":
        ("Reporting, blocking, leaving and deleting",
         "Signalement, blocage, sortie et suppression"),
    "threesea-la-spam-reporting-contact":
        ("Reporting, blocking and deleting",
         "Signalement, blocage et suppression"),
    "threesea-me-polls":
        ("Polls",
         "Sondages"),
    "threesea-la-polls-waiting":
        ("%n poll(s) waiting for your vote",
         ("%n sondage attend votre vote",
          "%n sondages attendent votre vote")),
    "threesea-la-revoked-input":
        ("This Threema ID was revoked",
         "Cet ID Threema a été révoqué"),
    "threesea-la-recording-paused":
        ("Paused %1",
         "En pause %1"),
    "threesea-he-choose-chats-accept":
        ("Choose",
         "Choisir"),
    "threesea-la-search-chats-contacts":
        ("Search chats and contacts",
         "Rechercher dans les discussions et les contacts"),
    "threesea-la-search-no-chats-contacts":
        ("No chats or contacts found",
         "Aucune discussion ni aucun contact trouvé"),
    "threesea-me-clean-up-scope":
        ("Groups with only you in them are cleaned up too, and private chats while they are unlocked.",
         "Les groupes dont vous êtes le seul membre sont aussi nettoyés, ainsi que les discussions privées tant qu’elles sont déverrouillées."),
    "threesea-he-clean-up-all-chats":
        ("Clean up all chats",
         "Nettoyer toutes les discussions"),
    "threesea-he-confirm-send":
        ("Send",
         "Envoyer"),
    "threesea-la-confirm-forward-to":
        ("Forward to %1",
         "Transférer à %1"),
    "threesea-la-confirm-send-to":
        ("Send to %1",
         "Envoyer à %1"),
    "threesea-la-confirm-caption":
        ("Caption",
         "Légende"),
    "threesea-la-confirm-message":
        ("Message",
         "Message"),
    "threesea-la-confirm-include-captions":
        ("Include captions",
         "Inclure les légendes"),
    "threesea-me-add-to-contacts":
        ("Add to contacts",
         "Ajouter aux contacts"),
    "threesea-me-write-message":
        ("Write message",
         "Écrire un message"),
    "threesea-la-contact-share-picture":
        ("Share profile picture",
         "Partager la photo de profil"),
    "threesea-la-search-contacts":
        ("Search contacts",
         "Rechercher des contacts"),
    "threesea-la-search-no-contacts":
        ("No contacts found",
         "Aucun contact trouvé"),
    "threesea-me-lock-now":
        ("Lock now",
         "Verrouiller maintenant"),
    "threesea-la-data-import-unfinished":
        ("Your Data backup isn't fully imported yet. Tap to import it again; it continues where it stopped.",
         "Votre sauvegarde des données n’est pas encore entièrement importée. Touchez pour l’importer à nouveau ; l’importation reprend là où elle s’est arrêtée."),
    "threesea-me-notifications-back-on":
        ("Notifications on",
         "Notifications activées"),
    "threesea-me-notifications-off-8-hours":
        ("Notifications off for 8 hours",
         "Notifications coupées pendant 8 heures"),
    "threesea-la-chats-not-loaded":
        ("The chats could not be loaded",
         "Les discussions n’ont pas pu être chargées"),
    "threesea-la-data-backup-import-intro":
        ("Adds what is missing: contacts, groups and pictures, then messages and media. Everything already on this device stays as it is. No messages are sent.",
         "Ajoute ce qui manque : contacts, groupes et images, puis messages et médias. Tout ce qui est déjà sur cet appareil reste tel quel. Aucun message n’est envoyé."),
    "threesea-la-data-backup-import-other":
        ("This backup belongs to another Threema ID or key. To import it, first restore that ID on this device.",
         "Cette sauvegarde appartient à un autre ID Threema ou à une autre clé. Pour l’importer, restaurez d’abord cet ID sur cet appareil."),
    "threesea-la-data-backup-import-unreadable":
        ("The Threema ID in this backup isn't readable with this password. The file may be damaged. Only a backup of this device's ID is imported.",
         "L’ID Threema de cette sauvegarde est illisible avec ce mot de passe. Le fichier est peut-être endommagé. Seule une sauvegarde de l’ID de cet appareil est importée."),
    "threesea-la-data-backup-import-no-id":
        ("This backup holds no Threema ID. Only a backup of this device's ID is imported.",
         "Cette sauvegarde ne contient pas d’ID Threema. Seule une sauvegarde de l’ID de cet appareil est importée."),
    "threesea-la-data-backup-contents":
        ("Encrypted backups of your Threema ID, chats, contacts, groups and media, in the format of Threema for Android.",
         "Sauvegardes chiffrées de votre ID Threema, de vos discussions, contacts, groupes et médias, au format de Threema pour Android."),
    "threesea-la-android-export-running":
        ("Writing the backup",
         "Écriture de la sauvegarde"),
    "threesea-la-android-export-left-out":
        ("%n chat(s) stayed out of the last backup: restoring them needs the contact's key, which could not be fetched. The next backup tries again.",
         ("%n discussion est restée hors de la dernière sauvegarde : la restaurer nécessite la clé du contact, qui n’a pas pu être récupérée. La prochaine sauvegarde réessaiera.",
          "%n discussions sont restées hors de la dernière sauvegarde : les restaurer nécessite la clé des contacts, qui n’a pas pu être récupérée. La prochaine sauvegarde réessaiera.")),
    "threesea-la-android-export-files-left-out":
        ("%n file(s) stayed out of the last backup: they could not be read on this phone.",
         ("%n fichier est resté hors de la dernière sauvegarde : il n’a pas pu être lu sur ce téléphone.",
          "%n fichiers sont restés hors de la dernière sauvegarde : ils n’ont pas pu être lus sur ce téléphone.")),
    "threesea-la-safe-last-attempt-failed":
        ("Last attempt failed",
         "La dernière tentative a échoué"),
    "threesea-he-restore-data-backup":
        ("Restore from Data backup",
         "Restaurer depuis une sauvegarde des données"),
    "threesea-la-restore-data-backup-intro":
        ("Choose a Data backup from Threema for Android or Threesea and enter its password. It brings your ID, contacts, groups, chats and media. The password stays on this device.",
         "Choisissez une sauvegarde des données de Threema pour Android ou de Threesea et saisissez son mot de passe. Elle ramène votre ID, vos contacts, vos groupes, vos discussions et vos médias. Le mot de passe reste sur cet appareil."),
    "threesea-la-restore-one-phone":
        ("An ID works on one phone at a time: new messages go only to the phone that connected last.",
         "Un ID fonctionne sur un seul téléphone à la fois : les nouveaux messages ne vont qu’au dernier téléphone connecté."),
    "threesea-la-data-backup-reason-too-large":
        ("too large to import",
         "trop grand pour être importé"),
    "threesea-la-data-backup-reason-damaged":
        ("damaged in the backup",
         "endommagé dans la sauvegarde"),
    "threesea-la-data-backup-reason-other-key":
        ("a different key in the backup",
         "autre clé dans la sauvegarde"),
    "threesea-la-data-backup-reason-not-here":
        ("not on this device",
         "absent de cet appareil"),
    "threesea-la-data-backup-imported-except":
        ("The backup is imported, except for what is listed below. Everything already on this device stayed as it was.",
         "La sauvegarde est importée, sauf ce qui figure ci-dessous. Tout ce qui était déjà sur cet appareil est resté tel quel."),
    "threesea-la-data-backup-restored-private":
        ("Chats that were marked private come back as ordinary, visible chats. To hide them again, set up an app lock in Settings and choose Make private in the chat list.",
         "Les discussions qui étaient privées reviennent comme des discussions ordinaires et visibles. Pour les masquer de nouveau, configurez un verrouillage de l’application dans les Paramètres et choisissez Rendre privée dans la liste des discussions."),
    "threesea-la-data-backup-received-list-missing":
        ("The list of messages already received isn't part of this import. Messages sent again are recognised from what this device receives from now on.",
         "La liste des messages déjà reçus ne fait pas partie de cette importation. Les messages envoyés à nouveau sont reconnus d’après ce que cet appareil reçoit à partir de maintenant."),
    "threesea-la-data-backup-key-conflicts":
        ("Contacts with a different key (kept as here)",
         "Contacts avec une clé différente (conservés tels quels)"),
    "threesea-la-data-backup-local-avatars":
        ("Pictures chosen for contacts on the other device, left out",
         "Images choisies pour des contacts sur l’autre appareil, laissées de côté"),
    "threesea-la-data-backup-groups-left-out":
        ("Groups left out",
         "Groupes laissés de côté"),
    "threesea-la-data-backup-unsent":
        ("Messages that were never sent (kept as failed)",
         "Messages jamais envoyés (conservés comme échoués)"),
    "threesea-la-data-backup-chats-left-out":
        ("Chats left out",
         "Discussions laissées de côté"),
    "threesea-la-data-backup-chats-too-large":
        ("Chats too large to import",
         "Discussions trop grandes pour être importées"),
    "threesea-la-data-backup-chats-damaged":
        ("Damaged chats left out",
         "Discussions endommagées, laissées de côté"),
    "threesea-la-data-backup-messages-left-out":
        ("Messages left out",
         "Messages laissés de côté"),
    "threesea-la-data-backup-collisions":
        ("Messages here from another sender (kept as here)",
         "Messages d’un autre expéditeur ici (conservés tels quels)"),
    "threesea-la-data-backup-reactions-left-out":
        ("Reactions left out",
         "Réactions laissées de côté"),
    "threesea-la-data-backup-reaction-files-left-out":
        ("Lists of reactions left out",
         "Listes de réactions laissées de côté"),
    "threesea-la-data-backup-entries-left-out":
        ("Entries left out",
         "Entrées laissées de côté"),
    "threesea-la-data-backup-media-missing":
        ("Media not in the backup",
         "Médias absents de la sauvegarde"),
    "threesea-se-data-backup-left-out":
        ("Chats that weren't imported",
         "Discussions non importées"),
    "threesea-la-data-backup-uid-conflicts":
        ("Messages also found in another chat",
         "Messages trouvés aussi dans une autre discussion"),
    "threesea-la-data-backup-foreign-group-chats":
        ("Group chats mostly from non-members",
         "Discussions de groupe venant surtout de non-membres"),
    "threesea-la-delete-data-goes":
        ("This removes your Threema ID, contacts, chats, media and settings from this device.",
         "Ceci supprime de cet appareil votre ID Threema, vos contacts, vos discussions, vos médias et vos paramètres."),
    "threesea-la-delete-data-comes-back-data-backup":
        ("Your ID itself is not revoked: you can restore it later from Threema Safe, an ID backup or a Data backup — without one of them, the ID is lost. Chats and media come back only from a Data backup.",
         "L’ID lui-même n’est pas révoqué : vous pourrez le restaurer depuis Threema Safe, une sauvegarde d’ID ou une sauvegarde des données — sans l’une d’elles, l’ID est perdu. Les discussions et les médias ne reviennent que depuis une sauvegarde des données."),
    "threesea-la-delete-data-stays":
        ("Pictures saved to the Threesea folder in Pictures, and backups and files you exported, stay on this device. A Threema Safe backup stays on the server.",
         "Les images enregistrées dans le dossier Threesea des images, ainsi que les sauvegardes et les fichiers que vous avez exportés, restent sur cet appareil. Une sauvegarde Threema Safe reste sur le serveur."),
    "threesea-la-delete-data-linked":
        ("Your linked devices are removed from this ID first, which takes a connection. If this phone can't reach the server then, they stay linked and keep the ID.",
         "Vos appareils associés sont d’abord retirés de cet ID, ce qui demande une connexion. Si ce téléphone ne joint pas le serveur à ce moment-là, ils restent associés et gardent l’ID."),
    "threesea-la-delete-data-have-a-backup":
        ("I have a Threema Safe, ID or Data backup, or no longer need this ID",
         "J’ai une sauvegarde Threema Safe, d’ID ou des données, ou je n’ai plus besoin de cet ID"),
    "threesea-he-share-id-backup":
        ("Share ID backup",
         "Partager la sauvegarde d’ID"),
    "threesea-bt-share":
        ("Share",
         "Partager"),
    "threesea-bt-done":
        ("Done",
         "Terminé"),
    "threesea-la-closing-id-backup":
        ("Closing ID backup",
         "Fermeture de la sauvegarde d’ID"),
    "threesea-la-export-chat-password-note":
        ("At least 8 characters. With a password the messages and files in the archive are encrypted with AES-256, and it opens in ZIP programs with AES support, such as 7-Zip; the names of the files stay readable. Empty: not encrypted, anyone who gets the file can read it.",
         "Au moins 8 caractères. Avec un mot de passe, les messages et les fichiers de l’archive sont chiffrés en AES-256, et elle s’ouvre avec les programmes ZIP qui prennent en charge AES, comme 7-Zip ; les noms des fichiers restent lisibles. Vide : non chiffré, quiconque obtient le fichier peut le lire."),
    "threesea-he-rename-list":
        ("Rename list",
         "Renommer la liste"),
    "threesea-me-choose-picture":
        ("Choose picture",
         "Choisir une image"),
    "threesea-me-take-picture":
        ("Take photo",
         "Prendre une photo"),
    "threesea-la-removing-picture":
        ("Removing picture",
         "Suppression de l’image"),
    "threesea-la-license-key-changed":
        ("License key changed",
         "Clé de licence modifiée"),
    "threesea-me-change-license-key":
        ("Change license key",
         "Changer la clé de licence"),
    "threesea-la-identity-intro-id-export":
        ("Your Threema ID identifies you to your contacts. Create a new one, or restore the ID you already use from Threema Safe, an ID backup (Threema for Android calls it an ID export) or a Data backup. A Data backup brings your chats as well.",
         "Votre ID Threema vous identifie auprès de vos contacts. Créez-en un nouveau, ou restaurez celui que vous utilisez déjà depuis Threema Safe, une sauvegarde d’ID (Threema pour Android l’appelle exportation de l’ID) ou une sauvegarde des données. Une sauvegarde des données ramène aussi vos discussions."),
    "threesea-bt-restore-data-backup":
        ("Restore from Data backup",
         "Restaurer depuis une sauvegarde des données"),
    "threesea-la-cannot-show-picture":
        ("This picture could not be shown",
         "Cette image ne peut pas être affichée"),
    "threesea-la-media-position":
        ("%1 of %2",
         "%1 sur %2"),
    "threesea-me-select-all":
        ("Select all",
         "Tout sélectionner"),
    "threesea-me-select":
        ("Select",
         "Sélectionner"),
    "threesea-la-select-media-hint":
        ("Tap to select",
         "Touchez pour sélectionner"),
    "threesea-la-all-contacts-on-list":
        ("All your contacts are on this list",
         "Tous vos contacts sont dans cette liste"),
    "threesea-la-all-contacts-in-group":
        ("All your contacts are in this group",
         "Tous vos contacts sont dans ce groupe"),
    "threesea-he-reactions":
        ("Reactions",
         "Réactions"),
    "threesea-la-nickname-ask-title":
        ("Choose a nickname",
         "Choisissez un surnom"),
    "threesea-la-nickname-ask-text":
        ("This name goes with your messages. Contacts who haven't given you a name of their own see it. You can change it in Settings, My profile.",
         "Ce nom accompagne vos messages. Les contacts qui ne vous ont pas donné de nom eux-mêmes le voient. Vous pouvez le changer dans Paramètres, Mon profil."),
    "threesea-he-photo-text":
        ("Text on the photo",
         "Texte sur la photo"),
    "threesea-la-photo-text":
        ("Text",
         "Texte"),
    "threesea-la-data-passphrase-note":
        ("If you forget it, everything Threesea has stored on this phone is lost.",
         "Si vous l’oubliez, tout ce que Threesea a enregistré sur ce téléphone est perdu."),
    "threesea-me-try-again":
        ("Try again",
         "Réessayer"),
    "threesea-he-choose-place":
        ("Choose a place",
         "Choisir un lieu"),
    "threesea-la-place-search":
        ("Search by name",
         "Rechercher par nom"),
    "threesea-la-place-use-own":
        ("Use your position",
         "Utiliser votre position"),
    "threesea-he-places-found":
        ("Places found",
         "Lieux trouvés"),
    "threesea-he-places-nearby":
        ("Places nearby",
         "Lieux à proximité"),
    "threesea-la-places-need-position":
        ("Places are found around your position",
         "Les lieux sont trouvés autour de votre position"),
    "threesea-la-places-failed":
        ("Threema's map server did not answer",
         "Le serveur de cartes de Threema n’a pas répondu"),
    "threesea-la-places-none":
        ("No places found",
         "Aucun lieu trouvé"),
    "threesea-la-places-failed-hint":
        ("Check your internet connection and pull down to try again.",
         "Vérifiez votre connexion Internet et tirez vers le bas pour réessayer."),
    "threesea-la-places-none-hint":
        ("Search by name to look further away.",
         "Recherchez par nom pour chercher plus loin."),
    "threesea-he-polls":
        ("Polls",
         "Sondages"),
    "threesea-la-no-polls":
        ("No polls in this chat",
         "Aucun sondage dans cette discussion"),
    "threesea-se-polls-closed":
        ("Closed",
         "Fermés"),
    "threesea-se-polls-open":
        ("Open",
         "Ouverts"),
    "threesea-la-poll-waiting":
        ("Waiting for your vote",
         "En attente de votre vote"),
    "threesea-la-restore-intro-scan":
        ("Scan the QR code of the ID backup or paste its text (80 characters in groups of four, like XXXX-XXXX-…), and enter the password you chose when you created it. Threema for Android calls it an ID export. It brings your ID. Chats come from a Data backup, which Settings, Backup can import afterwards.",
         "Scannez le code QR de la sauvegarde d’ID ou collez son texte (80 caractères par groupes de quatre, comme XXXX-XXXX-…), et saisissez le mot de passe choisi lors de sa création. Threema pour Android l’appelle exportation de l’ID. Elle ramène votre ID. Les discussions viennent d’une sauvegarde des données, que Paramètres, Sauvegarde peut importer ensuite."),
    "threesea-la-safe-find-id-none":
        ("No Threema ID is linked to this.",
         "Aucun ID Threema n’y est lié."),
    "threesea-he-safe-find-id":
        ("Find my ID",
         "Retrouver mon ID"),
    "threesea-la-safe-find-id-intro":
        ("Enter the phone number or e-mail address linked to your Threema ID. It is hashed on this device, and Threema's directory is asked with the hash.",
         "Saisissez le numéro de téléphone ou l’adresse e-mail lié à votre ID Threema. Il est haché sur cet appareil, et l’annuaire de Threema est interrogé avec l’empreinte."),
    "threesea-la-safe-find-id-phone":
        ("Phone number",
         "Numéro de téléphone"),
    "threesea-la-safe-find-id-email":
        ("E-mail address",
         "Adresse e-mail"),
    "threesea-bt-safe-find-id":
        ("Find",
         "Rechercher"),
    "threesea-se-safe-find-id-found":
        ("Linked IDs",
         "ID liés"),
    "threesea-la-safe-description-changes":
        ("Encrypted backup of your ID, profile, contacts, groups and settings, updated when something changes. For your chats, use Data backup.",
         "Sauvegarde chiffrée de votre ID, de votre profil, de vos contacts, de vos groupes et de vos paramètres, mise à jour à chaque changement. Pour vos discussions, utilisez la sauvegarde des données."),
    "threesea-la-safe-disabling-delete":
        ("Turning off and deleting the backup",
         "Désactivation et suppression de la sauvegarde"),
    "threesea-he-continue":
        ("Continue",
         "Continuer"),
    "threesea-he-change":
        ("Change",
         "Modifier"),
    "threesea-la-safe-password-change-intro":
        ("Choose a new password for Threema Safe. A new backup replaces the one made with the old password. Only your password (at least 8 characters) unlocks it: if you forget it, the backup cannot be recovered.",
         "Choisissez un nouveau mot de passe pour Threema Safe. Une nouvelle sauvegarde remplace celle faite avec l’ancien mot de passe. Seul votre mot de passe (au moins 8 caractères) la déverrouille : si vous l’oubliez, la sauvegarde est irrécupérable."),
    "threesea-la-safe-password-intro-changes":
        ("Threema Safe stores an encrypted backup of your ID, profile, contacts, groups and settings on Threema's servers, updated when something changes. For your chats, use Data backup. Only your password (at least 8 characters) unlocks it — if you forget it, the backup cannot be recovered.",
         "Threema Safe enregistre sur les serveurs de Threema une sauvegarde chiffrée de votre ID, de votre profil, de vos contacts, de vos groupes et de vos paramètres, mise à jour à chaque changement. Pour vos discussions, utilisez la sauvegarde des données. Seul votre mot de passe (au moins 8 caractères) la déverrouille — si vous l’oubliez, la sauvegarde est irrécupérable."),
    "threesea-la-safe-setup-skip":
        ("Continue without a backup",
         "Continuer sans sauvegarde"),
    "threesea-la-safe-setup-skip-description":
        ("Your ID, contacts and groups then exist on this phone alone, and go with it. Settings, Backup can turn Threema Safe on or export an ID backup at any time.",
         "Votre ID, vos contacts et vos groupes n’existent alors que sur ce téléphone, et disparaissent avec lui. Paramètres, Sauvegarde permet à tout moment d’activer Threema Safe ou d’exporter une sauvegarde d’ID."),
    "threesea-me-safe-find-id":
        ("Find my ID",
         "Retrouver mon ID"),
    "threesea-la-restore-safe-intro-data-backup":
        ("Enter your Threema ID and your Threema Safe password. Your profile, contacts, groups and settings are restored with it. Chats come from a Data backup, which Settings, Backup can import afterwards.",
         "Saisissez votre ID Threema et votre mot de passe Threema Safe. Votre profil, vos contacts, vos groupes et vos paramètres sont restaurés en même temps. Les discussions viennent d’une sauvegarde des données, que Paramètres, Sauvegarde peut importer ensuite."),
    "threesea-la-qr-not-id-backup":
        ("This is not an ID backup. Scan the QR code of your ID backup.",
         "Ceci n’est pas une sauvegarde d’ID. Scannez le code QR de votre sauvegarde d’ID."),
    "threesea-la-qr-not-web-code":
        ("This is a contact's code. Scan the code Threema Web shows.",
         "Ceci est le code d’un contact. Scannez le code affiché par Threema Web."),
    "threesea-la-qr-link-verify":
        ("This is a link to a Threema ID. To verify it, scan the QR code in the other person's app.",
         "Ceci est un lien vers un ID Threema. Pour le vérifier, scannez le code QR dans l’application de l’autre personne."),
    "threesea-la-qr-id-backup-elsewhere":
        ("This is an ID backup. It restores an ID while the app is being set up.",
         "Ceci est une sauvegarde d’ID. Elle restaure un ID pendant la configuration de l’application."),
    "threesea-la-qr-unknown":
        ("This is not a Threema QR code.",
         "Ceci n’est pas un code QR Threema."),
    "threesea-la-scan-hint-id-backup":
        ("Point the camera at the QR code of your ID backup.",
         "Dirigez l’appareil photo vers le code QR de votre sauvegarde d’ID."),
    "threesea-la-scan-hint-web":
        ("Point the camera at the QR code Threema Web shows.",
         "Dirigez l’appareil photo vers le code QR affiché par Threema Web."),
    "threesea-la-search-names-messages":
        ("Names and messages",
         "Noms et messages"),
    "threesea-he-messages":
        ("Messages",
         "Messages"),
    "threesea-me-remove-star":
        ("Remove star",
         "Retirer l’étoile"),
    "threesea-la-search-nothing-at-all":
        ("No chats, contacts or messages found",
         "Aucune discussion, aucun contact ni aucun message trouvé"),
    "threesea-la-location-waiting":
        ("No position yet. Check that location is on in Settings → Location. Outdoors this goes faster.",
         "Pas encore de position. Vérifiez que la localisation est activée dans Paramètres → Localisation. En extérieur, c’est plus rapide."),
    "threesea-la-location-own":
        ("Your position",
         "Votre position"),
    "threesea-la-location-place":
        ("Place",
         "Lieu"),
    "threesea-la-location-last-known":
        ("Your last known position, from %1",
         "Votre dernière position connue, de %1"),
    "threesea-la-place-server":
        ("To find places, Threema's map server gets your position and what you search for.",
         "Pour trouver des lieux, le serveur de cartes de Threema reçoit votre position et ce que vous recherchez."),
    "threesea-la-location-use-last-known":
        ("Use your last known position, from",
         "Utiliser votre dernière position connue, de"),
    "threesea-la-update-check-failed":
        ("Could not check for updates. Try again later.",
         "Impossible de rechercher les mises à jour. Réessayez plus tard."),
    "threesea-me-emoji-twemoji":
        ("Twemoji",
         "Twemoji"),
    "threesea-la-safe-on-failing":
        ("On, the last attempt failed",
         "Activé, la dernière tentative a échoué"),
    "threesea-he-leave-device-group":
        ("Leave",
         "Quitter"),
    "threesea-he-leave-device-group-title":
        ("Leave on this device only?",
         "Quitter le groupe d’appareils sur cet appareil seulement ?"),
    "threesea-la-leave-device-group-failed":
        ("Removing your linked devices did not finish, so they may still be linked. Go back to try again once you are online.",
         "Le retrait de vos appareils associés n’est pas allé au bout : ils sont peut-être encore associés. Revenez en arrière pour réessayer une fois en ligne."),
    "threesea-la-leave-device-group-text":
        ("If you leave, this device connects directly again. The other devices keep your ID and stay in their group, and only they can remove themselves from then on.",
         "Si vous quittez, cet appareil se connecte de nouveau directement. Les autres appareils gardent votre ID et restent dans leur groupe, et eux seuls peuvent ensuite s’en retirer."),
    "threesea-he-link-desktop":
        ("Link Threema for Desktop",
         "Associer Threema for Desktop"),
    "threesea-he-link-next":
        ("Next",
         "Suivant"),
    "threesea-la-link-grant":
        ("Threema for Desktop gets your Threema ID with its private key, your contacts and groups, and can read and send messages as you. Link only your own computer.",
         "Threema for Desktop reçoit votre ID Threema avec sa clé privée, vos contacts et vos groupes, et peut lire et envoyer des messages en votre nom. N’associez que votre propre ordinateur."),
    "threesea-la-link-pfs-if-on":
        ("While a device is linked, chats with your contacts use end-to-end encryption without Perfect Forward Secrecy. If it was on, it comes back when the last linked device is removed.",
         "Tant qu’un appareil est associé, les discussions avec vos contacts utilisent le chiffrement de bout en bout sans confidentialité persistante (Perfect Forward Secrecy). Si elle était activée, elle revient quand le dernier appareil associé est retiré."),
    "threesea-me-link-scan-again":
        ("Scan again",
         "Scanner à nouveau"),
    "threesea-me-clear-reflected-left-out":
        ("Clear count of messages not kept",
         "Remettre à zéro le compte des messages non conservés"),
    "threesea-la-reflected-left-out":
        ("%n message(s) received on your other devices weren't kept on this phone.",
         ("%n message reçu sur vos autres appareils n’a pas été conservé sur ce téléphone.",
          "%n messages reçus sur vos autres appareils n’ont pas été conservés sur ce téléphone.")),
    "threesea-la-linked-devices-offline":
        ("Waiting for the connection",
         "En attente de la connexion"),
    "threesea-la-linked-devices-failed":
        ("The list of linked devices did not load",
         "La liste des appareils associés n’a pas été chargée"),
    "threesea-la-linked-devices-offline-hint":
        ("The linked devices appear here once the server answers.",
         "Les appareils associés apparaissent ici dès que le serveur répond."),
    "threesea-la-linked-devices-failed-hint":
        ("Open this page again to load it.",
         "Rouvrez cette page pour la charger."),
    "threesea-la-log-save-failed":
        ("Could not save the support package to Downloads.",
         "Le paquet d’assistance n’a pas pu être enregistré dans Téléchargements."),
    "threesea-la-keep-messages-remorse":
        ("Deleting older messages",
         "Suppression des messages anciens"),
    "threesea-la-cleaning-up":
        ("Cleaning up",
         "Nettoyage"),
    "threesea-la-notifications-calls":
        ("Notify about new messages and missed calls",
         "Notifier les nouveaux messages et les appels manqués"),
    "threesea-la-notification-preview-who-description":
        ("Otherwise notifications show who wrote, but not what, e.g. on the lock screen.",
         "Sinon, les notifications indiquent qui a écrit, mais pas quoi, par exemple sur l’écran de verrouillage."),
    "threesea-se-notifications-calls":
        ("Calls",
         "Appels"),
    "threesea-la-call-tone":
        ("Ringtone",
         "Sonnerie"),
    "threesea-la-call-tone-system":
        ("System call tone",
         "Sonnerie du système"),
    "threesea-la-inapp-sound-voice-description":
        ("A soft sound between two voice messages playing one after the other.",
         "Un son discret entre deux messages vocaux lus l’un après l’autre."),
    "threesea-la-exclusion-list-description":
        ("Finding contacts from the address book will not add or rename these Threema IDs.",
         "La recherche de contacts dans le carnet d’adresses n’ajoutera ni ne renommera ces ID Threema."),
    "threesea-la-clear-contact-policies-description":
        ("The read receipt and typing indicator choices made on single contacts' pages go back to the settings above.",
         "Les choix d’accusés de lecture et d’indicateur de saisie faits sur la page de chaque contact reviennent aux réglages ci-dessus."),
    "threesea-la-contact-policies-cleared":
        ("Cleared for %n contact(s)",
         ("Effacé pour %n contact",
          "Effacé pour %n contacts")),
    "threesea-la-forward-secrecy-linked":
        ("Off while other devices are linked to this ID. Your choice here applies again when the last one is removed.",
         "Désactivée tant que d’autres appareils sont associés à cet ID. Votre choix ici s’applique de nouveau quand le dernier est retiré."),
    "threesea-la-removing-link":
        ("Removing link",
         "Suppression du lien"),
    "threesea-la-cancelling-link":
        ("Cancelling link",
         "Annulation de l’association"),
    "threesea-me-share-my-id":
        ("Share my ID",
         "Partager mon ID"),
    "threesea-la-revocation-not-checked":
        ("Not checked yet",
         "Pas encore vérifié"),
    "threesea-la-data-passphrase-about":
        ("Your messages are stored encrypted with a key that is kept encrypted with this passphrase. Threesea asks for it every time it starts, also after restarting the phone or deleting its data folder, and only this passphrase opens it. It receives messages once it is entered. If you forget it, everything Threesea has stored on this phone is lost. Threema Safe or an ID backup keeps your ID, a Data backup your chats.",
         "Vos messages sont enregistrés chiffrés avec une clé elle-même chiffrée par cette phrase secrète. Threesea la demande à chaque démarrage, y compris après un redémarrage du téléphone ou la suppression de son dossier de données, et seule cette phrase secrète l’ouvre. Il reçoit les messages une fois qu’elle est saisie. Si vous l’oubliez, tout ce que Threesea a enregistré sur ce téléphone est perdu. Threema Safe ou une sauvegarde d’ID conserve votre ID, une sauvegarde des données vos discussions."),
    "threesea-la-web-pair-grants":
        ("This code connects a browser to your Threema ID. That browser can read your contacts and chats and send messages in your name.",
         "Ce code connecte un navigateur à votre ID Threema. Ce navigateur peut lire vos contacts et vos discussions et envoyer des messages en votre nom."),
    "threesea-la-web-pair-only-yours":
        ("Allow it only if you opened Threema Web yourself and the code is on your own screen. If someone sent you this code, go back.",
         "Ne l’autorisez que si vous avez ouvert Threema Web vous-même et que le code est sur votre propre écran. Si quelqu’un vous a envoyé ce code, revenez en arrière."),
    "threesea-la-web-pair-remembered":
        ("If the browser asks to be remembered, it can connect again without a new code until you remove it from Threema Web in the settings.",
         "Si le navigateur demande à être mémorisé, il peut se reconnecter sans nouveau code jusqu’à ce que vous le retiriez de Threema Web dans les paramètres."),
    "threesea-he-rename-browser":
        ("Rename browser",
         "Renommer le navigateur"),
    "threesea-la-browser-name":
        ("Browser name",
         "Nom du navigateur"),
    "threesea-la-browser-name-description":
        ("Only this phone shows this name. Left empty, the browser's own name is shown.",
         "Seul ce téléphone affiche ce nom. S’il reste vide, le nom propre du navigateur est affiché."),
    "threesea-la-web-scan-again":
        ("The new certificate is trusted. Scan the code in the browser again to connect it.",
         "Le nouveau certificat est approuvé. Scannez à nouveau le code dans le navigateur pour le connecter."),
    "threesea-la-web-disconnected-kept":
        ("Threema Web disconnected. Tap the browser to connect it again.",
         "Threema Web déconnecté. Touchez le navigateur pour le reconnecter."),
    "threesea-la-web-disconnected":
        ("Threema Web disconnected.",
         "Threema Web déconnecté."),
    "threesea-la-web-removing":
        ("Removing",
         "Retrait"),
    "threesea-me-web-remove-all":
        ("Remove all browsers",
         "Retirer tous les navigateurs"),
    "threesea-la-web-removing-all":
        ("Removing all browsers",
         "Retrait de tous les navigateurs"),
    "threesea-la-web-enabled":
        ("Threema Web",
         "Threema Web"),
    "threesea-la-web-enabled-description":
        ("Lets the browsers listed here connect to this phone and use your chats. Turning it off disconnects them and keeps them in the list.",
         "Permet aux navigateurs listés ici de se connecter à ce téléphone et d’utiliser vos discussions. Le désactiver les déconnecte et les garde dans la liste."),
    "threesea-la-web-tap-hint":
        ("Tap a browser to connect or disconnect it. Threema Web must be open in the browser to connect. A disconnected browser can connect again when it is opened; removing it or turning off the switch above keeps it out.",
         "Touchez un navigateur pour le connecter ou le déconnecter. Pour se connecter, Threema Web doit être ouvert dans le navigateur. Un navigateur déconnecté peut se reconnecter quand il est ouvert ; le retirer ou désactiver l’interrupteur ci-dessus le tient à l’écart."),
    "threesea-me-web-rename":
        ("Rename",
         "Renommer"),
    "threesea-se-web-servers":
        ("Allowed servers",
         "Serveurs autorisés"),
    "threesea-la-web-servers-description":
        ("Self-hosted Threema Web servers you allowed. Removing one also removes its browsers, and the next code naming it asks again.",
         "Serveurs Threema Web auto-hébergés que vous avez autorisés. En retirer un retire aussi ses navigateurs, et le prochain code qui le désigne vous demande de nouveau votre accord."),
    "threesea-la-share-refused-several":
        ("Nothing was sent. Blocked or revoked, and taken off the choice: %1",
         "Rien n’a été envoyé. Bloqués ou révoqués, et retirés du choix : %1"),
    "threesea-me-choose-several-chats":
        ("Choose several chats",
         "Choisir plusieurs discussions"),
    "threesea-la-startup-try-again":
        ("Pull down to try again.",
         "Tirez vers le bas pour réessayer."),
    "threesea-la-startup-code":
        ("Code: %1",
         "Code : %1"),
    "threesea-la-removal-stuck":
        ("Removing the ID did not finish",
         "La suppression de l’ID n’est pas allée au bout"),
    "threesea-la-removal-stuck-hint":
        ("Pull down to try again, or close Threesea and open it again (%1).",
         "Tirez vers le bas pour réessayer, ou fermez Threesea et rouvrez-le (%1)."),
    "threesea-he-enter-data-passphrase":
        ("Enter the passphrase of your stored data",
         "Saisissez la phrase secrète de vos données enregistrées"),
    "threesea-he-video-keeps-metadata-send":
        ("Send as it is",
         "Envoyer tel quel"),
    "threesea-he-video-keeps-metadata-cancel":
        ("Don't send",
         "Ne pas envoyer"),
    "threesea-la-video-keeps-metadata":
        ("This video keeps what its file says about it",
         "Cette vidéo garde ce que son fichier dit d’elle"),
    "threesea-la-picture-keeps-metadata":
        ("This picture keeps what its file says about it",
         "Cette image garde ce que son fichier dit d’elle"),
    "threesea-la-video-keeps-metadata-description":
        ("Threesea can't remove the metadata of this file. It can hold where and when the video was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file.",
         "Threesea ne peut pas supprimer les métadonnées de ce fichier. Elles peuvent indiquer où, quand et avec quel appareil la vidéo a été filmée, et le destinataire les reçoit toutes, avec le nom du fichier : %1. Envoyée telle quelle, elle part comme simple fichier."),
    "threesea-la-picture-keeps-metadata-description":
        ("Threesea can't remove the metadata of this file. It can hold where and when the picture was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file.",
         "Threesea ne peut pas supprimer les métadonnées de ce fichier. Elles peuvent indiquer où, quand et avec quel appareil l’image a été prise, et le destinataire les reçoit toutes, avec le nom du fichier : %1. Envoyée telle quelle, elle part comme simple fichier."),
    "threesea-la-ended-early":
        ("Threesea ended before it was done",
         "Threesea s’est arrêté avant d’avoir fini"),
    "threesea-la-ended-early-body":
        ("Your last files or draft did not reach the chat. Add them again there.",
         "Vos derniers fichiers ou votre brouillon ne sont pas arrivés dans la discussion. Ajoutez-les à nouveau là-bas."),
}


def typography(text):
    """French spacing: a plain space before : ; ! ? » or after « would let a line break there."""
    return re.sub(r" ([:;!?»])", " \\1", text).replace("« ", "« ")


def fill(path, table):
    tree = ET.parse(path)
    missing, stale = [], []
    for message in tree.iter("message"):
        key = message.get("id")
        entry = table.get(key)
        translation = message.find("translation")
        if entry is None:
            if translation.get("type") == "unfinished":
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
                ET.SubElement(translation, "numerusform").text = typography(form)
        else:
            translation.text = typography(text)
        translation.attrib.pop("type", None)
    ET.indent(tree, "    ")
    with open(path, "w", encoding="utf-8") as out:
        out.write('<?xml version="1.0" encoding="utf-8"?>\n<!DOCTYPE TS>\n')
        out.write(ET.tostring(tree.getroot(), encoding="unicode"))
        out.write("\n")
    return missing, stale


missing, stale = fill("translations/harbour-threesea-fr.ts", FR)
for key in stale:
    print(f"English changed, translate again: {key}", file=sys.stderr)
if missing:
    print(f"{len(missing)} texts without French yet (left unfinished)", file=sys.stderr)
