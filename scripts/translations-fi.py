#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Finnish texts for translations/harbour-threesea-fi.ts, in the same form as translations-de.py:
# each entry holds the English it was translated from, then the Finnish. A text whose English
# changed stays unfinished and is listed. Run as the last step, like the German and French.
# Style: Sailfish OS Finnish (sinuttelu, imperatiivi), Threema-tunnus, keskustelu, yhdistetty laite,
# jakelulista, sovelluslukko, tunnuslause, ambienssi.
import os
import sys
import xml.etree.ElementTree as ET

FI = {
    "threesea-la-error-network":
        ("Could not reach the Threema servers. Check your internet connection and try again.",
         "Threeman palvelimiin ei saatu yhteyttä. Tarkista internetyhteys ja yritä uudelleen."),
    "threesea-la-error-license-format":
        ("A license key looks like ABCDE-12345.",
         "Lisenssiavain on muotoa ABCDE-12345."),
    "threesea-la-error-backup-decrypt":
        ("Wrong password, or the backup is damaged.",
         "Väärä salasana, tai varmuuskopio on vioittunut."),
    "threesea-la-error-password-short":
        ("The password needs at least 8 characters.",
         "Salasanassa on oltava vähintään 8 merkkiä."),
    "threesea-la-error-password-long":
        ("The password is too long.",
         "Salasana on liian pitkä."),
    "threesea-la-error-safe-not-found":
        ("No Threema Safe backup found for this Threema ID and password.",
         "Tälle Threema-tunnukselle ja salasanalle ei löytynyt Threema Safe -varmuuskopiota."),
    "threesea-la-error-safe-too-large":
        ("The backup is larger than the Threema Safe server allows.",
         "Varmuuskopio on suurempi kuin Threema Safe -palvelin sallii."),
    "threesea-la-error-identity-format":
        ("A Threema ID has 8 characters, letters and digits.",
         "Threema-tunnuksessa on 8 merkkiä, kirjaimia ja numeroita."),
    "threesea-la-error-contact-invalid":
        ("This Threema ID does not exist or was revoked.",
         "Tätä Threema-tunnusta ei ole olemassa, tai se on mitätöity."),
    "threesea-la-error-contact-self":
        ("This is your own Threema ID.",
         "Tämä on oma Threema-tunnuksesi."),
    "threesea-la-error-no-license":
        ("Enter your license key first.",
         "Syötä ensin lisenssiavaimesi."),
    "threesea-la-verification-full":
        ("Fully verified",
         "Täysin vahvistettu"),
    "threesea-la-verification-none":
        ("Not verified",
         "Ei vahvistettu"),
    "threesea-la-cover-no-new":
        ("No new messages",
         "Ei uusia viestejä"),
    "threesea-he-about":
        ("About",
         "Tietoja"),
    "threesea-la-version":
        ("Version",
         "Versio"),
    "threesea-se-license":
        ("License",
         "Lisenssi"),
    "threesea-la-license-text":
        ("An unofficial Threema client for Sailfish OS, licensed under the GNU AGPL v3. Uses libthreema by Threema AG. Requires a Threema Libre license from the Threema Shop. Not affiliated with or supported by Threema AG.",
         "Epävirallinen Threema-sovellus Sailfish OS:lle, lisensoitu GNU AGPL v3 -lisenssillä. Käyttää Threema AG:n libthreema-kirjastoa. Vaatii Threema Libre -lisenssin Threema Shopista. Ei ole sidoksissa Threema AG:hen eikä sen tukema."),
    "threesea-he-add":
        ("Add",
         "Lisää"),
    "threesea-la-add-contact-intro":
        ("Enter the Threema ID of the person you want to add. Meet in person and scan each other's QR code later to fully verify the contact.",
         "Syötä lisättävän henkilön Threema-tunnus. Tapaa hänet myöhemmin kasvokkain ja skannatkaa toistenne QR-koodit, niin yhteystieto on täysin vahvistettu."),
    "threesea-la-threema-id":
        ("Threema ID",
         "Threema-tunnus"),
    "threesea-me-delete":
        ("Delete",
         "Poista"),
    "threesea-la-deleting-contact":
        ("Deleting contact and chat",
         "Poistetaan yhteystieto ja keskustelu"),
    "threesea-me-edit":
        ("Edit",
         "Muokkaa"),
    "threesea-la-nickname":
        ("Nickname",
         "Lempinimi"),
    "threesea-la-type":
        ("Type",
         "Tyyppi"),
    "threesea-la-threema-work":
        ("Threema Work",
         "Threema Work"),
    "threesea-la-status":
        ("Status",
         "Tila"),
    "threesea-la-revoked":
        ("Revoked",
         "Mitätöity"),
    "threesea-la-inactive":
        ("Inactive",
         "Ei aktiivinen"),
    "threesea-se-public-key":
        ("Public key",
         "Julkinen avain"),
    "threesea-me-add-contact":
        ("Add contact",
         "Lisää yhteystieto"),
    "threesea-he-contacts":
        ("Contacts",
         "Yhteystiedot"),
    "threesea-la-no-contacts":
        ("No contacts",
         "Ei yhteystietoja"),
    "threesea-la-no-contacts-hint":
        ("Pull down to add a contact by Threema ID",
         "Vedä alas lisätäksesi yhteystiedon Threema-tunnuksella"),
    "threesea-me-settings":
        ("Settings",
         "Asetukset"),
    "threesea-he-chats":
        ("Chats",
         "Keskustelut"),
    "threesea-la-no-chats":
        ("No chats yet",
         "Ei vielä keskusteluja"),
    "threesea-la-no-chats-hint":
        ("Swipe left to choose a contact",
         "Pyyhkäise vasemmalle valitaksesi yhteystiedon"),
    "threesea-he-remove":
        ("Remove",
         "Poista"),
    "threesea-he-save":
        ("Save",
         "Tallenna"),
    "threesea-la-first-name":
        ("First name",
         "Etunimi"),
    "threesea-la-last-name":
        ("Last name",
         "Sukunimi"),
    "threesea-he-export-backup":
        ("Export ID backup",
         "Vie tunnuksen varmuuskopio"),
    "threesea-la-export-intro":
        ("An ID backup lets you restore your Threema ID on another device. Keep it and its password safe: anyone with both can use your ID.",
         "Tunnuksen varmuuskopiolla voit palauttaa Threema-tunnuksesi toiseen laitteeseen. Säilytä varmuuskopio ja sen salasana turvassa: kuka tahansa, jolla on molemmat, voi käyttää tunnustasi."),
    "threesea-la-new-backup-password":
        ("Password (at least 8 characters)",
         "Salasana (vähintään 8 merkkiä)"),
    "threesea-la-repeat-password":
        ("Repeat password",
         "Toista salasana"),
    "threesea-bt-create-backup":
        ("Create backup",
         "Luo varmuuskopio"),
    "threesea-bt-copy":
        ("Copy to clipboard",
         "Kopioi leikepöydälle"),
    "threesea-he-identity-setup":
        ("Your Threema ID",
         "Threema-tunnuksesi"),
    "threesea-bt-create-identity":
        ("Create new ID",
         "Luo uusi tunnus"),
    "threesea-bt-restore-safe":
        ("Restore from Threema Safe",
         "Palauta Threema Safesta"),
    "threesea-bt-restore-backup":
        ("Restore ID backup",
         "Palauta tunnuksen varmuuskopio"),
    "threesea-he-welcome":
        ("Welcome to Threesea",
         "Tervetuloa Threeseaan"),
    "threesea-la-license-intro":
        ("Threesea is an unofficial Threema client for Sailfish OS. It needs a Threema Libre license key, which you can buy in the Threema Shop (shop.threema.ch).",
         "Threesea on epävirallinen Threema-sovellus Sailfish OS:lle. Se tarvitsee Threema Libre -lisenssiavaimen, jonka voit ostaa Threema Shopista (shop.threema.ch)."),
    "threesea-la-license-key":
        ("License key",
         "Lisenssiavain"),
    "threesea-bt-check-license":
        ("Check license",
         "Tarkista lisenssi"),
    "threesea-he-restore-backup":
        ("Restore ID backup",
         "Palauta tunnuksen varmuuskopio"),
    "threesea-la-id-backup":
        ("ID backup",
         "Tunnuksen varmuuskopio"),
    "threesea-la-backup-password":
        ("Backup password",
         "Varmuuskopion salasana"),
    "threesea-bt-restore":
        ("Restore",
         "Palauta"),
    "threesea-me-safe-change-password":
        ("Change password",
         "Vaihda salasana"),
    "threesea-me-safe-backup-now":
        ("Back up now",
         "Varmuuskopioi nyt"),
    "threesea-la-threema-safe":
        ("Threema Safe",
         "Threema Safe"),
    "threesea-se-status":
        ("Status",
         "Tila"),
    "threesea-la-safe-last-backup":
        ("Last backup",
         "Viimeisin varmuuskopio"),
    "threesea-la-safe-not-yet":
        ("Not yet",
         "Ei vielä"),
    "threesea-la-safe-size":
        ("Size",
         "Koko"),
    "threesea-la-safe-server":
        ("Server",
         "Palvelin"),
    "threesea-la-safe-retention":
        ("Kept on the server",
         "Säilytysaika palvelimella"),
    "threesea-la-days":
        ("%1 days",
         "%1 päivää"),
    "threesea-he-enable":
        ("Enable",
         "Ota käyttöön"),
    "threesea-la-password":
        ("Password",
         "Salasana"),
    "threesea-la-safe-password-weak":
        ("This password is easy to guess. Choose a stronger one, or accept to use it anyway.",
         "Tämä salasana on helppo arvata. Valitse vahvempi, tai hyväksy käyttääksesi sitä silti."),
    "threesea-he-restore-safe":
        ("Restore from Threema Safe",
         "Palauta Threema Safesta"),
    "threesea-la-safe-password":
        ("Threema Safe password",
         "Threema Safe -salasana"),
    "threesea-la-safe-deriving":
        ("Unlocking the backup takes a moment.",
         "Varmuuskopion avaaminen kestää hetken."),
    "threesea-me-about":
        ("About",
         "Tietoja"),
    "threesea-he-settings":
        ("Settings",
         "Asetukset"),
    "threesea-se-profile":
        ("My profile",
         "Oma profiili"),
    "threesea-se-privacy":
        ("Privacy",
         "Yksityisyys"),
    "threesea-la-send-read-receipts":
        ("Send read receipts",
         "Lähetä lukukuittaukset"),
    "threesea-la-send-read-receipts-description":
        ("Let people know when you have read their messages.",
         "Kerro muille, kun olet lukenut heidän viestinsä."),
    "threesea-la-send-typing":
        ("Send typing indicator",
         "Lähetä kirjoitusilmaisin"),
    "threesea-la-send-typing-description":
        ("Let people see when you are writing to them.",
         "Näytä muille, kun kirjoitat heille."),
    "threesea-la-block-unknown":
        ("Block unknown senders",
         "Estä tuntemattomat lähettäjät"),
    "threesea-la-block-unknown-description":
        ("Ignore messages from people who are not in your contacts.",
         "Ohita viestit henkilöiltä, jotka eivät ole yhteystiedoissasi."),
    "threesea-se-backup":
        ("Backup",
         "Varmuuskopio"),
    "threesea-la-on":
        ("On",
         "Päällä"),
    "threesea-la-off":
        ("Off",
         "Pois"),
    "threesea-la-export-backup":
        ("Export ID backup",
         "Vie tunnuksen varmuuskopio"),
    "threesea-la-startup-failed":
        ("Threesea could not start",
         "Threesean käynnistys epäonnistui"),
    "threesea-la-location":
        ("Location",
         "Sijainti"),
    "threesea-me-copy":
        ("Copy",
         "Kopioi"),
    "threesea-la-cover-unread-one":
        ("unread message",
         "lukematon viesti"),
    "threesea-la-cover-unread-many":
        ("unread messages",
         "lukematonta viestiä"),
    "threesea-la-typing":
        ("typing…",
         "kirjoittaa…"),
    "threesea-la-offline-queue":
        ("Offline – messages will be sent later",
         "Ei yhteyttä – viestit lähetetään myöhemmin"),
    "threesea-la-connecting":
        ("Connecting…",
         "Yhdistetään…"),
    "threesea-la-offline":
        ("Offline",
         "Ei yhteyttä"),
    "threesea-me-delete-chat":
        ("Delete chat",
         "Poista keskustelu"),
    "threesea-la-deleting-chat":
        ("Deleting chat",
         "Poistetaan keskustelu"),
    "threesea-la-keep-running":
        ("Keep running in the background",
         "Pidä käynnissä taustalla"),
    "threesea-la-keep-running-description":
        ("Threesea stays connected after you close it, so new messages arrive right away. After you turn on the phone, it connects once you open it. Uses more battery.",
         "Threesea pysyy yhteydessä, vaikka suljet sen, joten uudet viestit tulevat heti. Kun käynnistät puhelimen, yhteys muodostuu vasta, kun avaat sovelluksen. Kuluttaa enemmän akkua."),
    "threesea-la-status-fs-established":
        ("Perfect Forward Secrecy is active for this chat.",
         "Perfect Forward Secrecy (PFS) on käytössä tässä keskustelussa."),
    "threesea-la-today":
        ("Today",
         "Tänään"),
    "threesea-la-yesterday":
        ("Yesterday",
         "Eilen"),
    "threesea-la-type-message":
        ("Type message",
         "Kirjoita viesti"),
    "threesea-la-error-reaction-not-supported":
        ("This contact's app only understands thumbs up and thumbs down.",
         "Tämän yhteystiedon sovellus ymmärtää vain peukun ylös ja peukun alas."),
    "threesea-la-error-not-supported-by-contact":
        ("The app on the other side doesn't support this.",
         "Vastapuolen sovellus ei tue tätä."),
    "threesea-la-error-message-too-old":
        ("Messages can be edited or deleted for everyone for 6 hours after sending.",
         "Viestejä voi muokata tai poistaa kaikilta 6 tunnin ajan lähettämisen jälkeen."),
    "threesea-la-message-deleted":
        ("Message deleted",
         "Viesti poistettu"),
    "threesea-la-edited":
        ("Edited",
         "Muokattu"),
    "threesea-la-deleting-everyone":
        ("Deleting for everyone",
         "Poistetaan kaikilta"),
    "threesea-la-deleting-message":
        ("Deleting",
         "Poistetaan"),
    "threesea-la-editing-message":
        ("Editing message",
         "Muokataan viestiä"),
    "threesea-la-file":
        ("File",
         "Tiedosto"),
    "threesea-la-photo":
        ("Photo",
         "Kuva"),
    "threesea-la-error-file-too-large":
        ("Files can be at most 100 MB.",
         "Tiedostot voivat olla enintään 100 Mt."),
    "threesea-la-add-caption":
        ("Add a caption",
         "Lisää kuvateksti"),
    "threesea-me-send-photo":
        ("Send photo",
         "Lähetä kuva"),
    "threesea-me-send-file":
        ("Send file",
         "Lähetä tiedosto"),
    "threesea-me-save-downloads":
        ("Save to Downloads",
         "Tallenna Latauksiin"),
    "threesea-la-saved-downloads":
        ("Saved to Downloads",
         "Tallennettu Latauksiin"),
    "threesea-la-voice-message":
        ("Voice message",
         "Ääniviesti"),
    "threesea-la-recording-failed":
        ("Recording failed",
         "Nauhoitus epäonnistui"),
    "threesea-la-recording":
        ("Recording %1",
         "Nauhoitetaan %1"),
    "threesea-la-cannot-play":
        ("This voice message could not be played",
         "Ääniviestiä ei voitu toistaa"),
    "threesea-la-error-group-left":
        ("You are no longer in this group.",
         "Et ole enää tässä ryhmässä."),
    "threesea-la-error-not-group-creator":
        ("Only the group's creator can do this.",
         "Vain ryhmän luoja voi tehdä tämän."),
    "threesea-la-status-group-created":
        ("You created this group.",
         "Loit tämän ryhmän."),
    "threesea-la-status-group-joined":
        ("You were added to this group.",
         "Sinut lisättiin tähän ryhmään."),
    "threesea-la-status-group-removed":
        ("You were removed from this group.",
         "Sinut poistettiin tästä ryhmästä."),
    "threesea-la-status-group-disbanded":
        ("This group was dissolved.",
         "Tämä ryhmä lakkautettiin."),
    "threesea-la-status-group-left":
        ("You left this group.",
         "Poistuit tästä ryhmästä."),
    "threesea-la-status-group-renamed":
        ("Group renamed to %1.",
         "Ryhmän uusi nimi on %1."),
    "threesea-la-status-group-member-added":
        ("%1 was added.",
         "%1 lisättiin."),
    "threesea-la-status-group-member-removed":
        ("%1 was removed.",
         "%1 poistettiin."),
    "threesea-la-status-group-member-left":
        ("%1 left the group.",
         "%1 poistui ryhmästä."),
    "threesea-la-group-left-input":
        ("You are no longer in this group",
         "Et ole enää tässä ryhmässä"),
    "threesea-me-new-group":
        ("New group",
         "Uusi ryhmä"),
    "threesea-he-rename-group":
        ("Rename group",
         "Nimeä ryhmä uudelleen"),
    "threesea-la-group-name":
        ("Group name",
         "Ryhmän nimi"),
    "threesea-me-delete-group":
        ("Delete group",
         "Poista ryhmä"),
    "threesea-la-deleting-group":
        ("Deleting group",
         "Poistetaan ryhmä"),
    "threesea-me-dissolve-group":
        ("Dissolve group",
         "Lakkauta ryhmä"),
    "threesea-me-leave-group":
        ("Leave group",
         "Poistu ryhmästä"),
    "threesea-la-dissolving-group":
        ("Dissolving group",
         "Lakkautetaan ryhmä"),
    "threesea-la-leaving-group":
        ("Leaving group",
         "Poistutaan ryhmästä"),
    "threesea-me-rename":
        ("Rename",
         "Nimeä uudelleen"),
    "threesea-he-members":
        ("Members",
         "Jäsenet"),
    "threesea-he-group":
        ("Group",
         "Ryhmä"),
    "threesea-la-member-count":
        ("%n member(s)",
         ("%n jäsen",
          "%n jäsentä")),
    "threesea-la-you-creator":
        ("You (creator)",
         "Sinä (luoja)"),
    "threesea-la-you":
        ("You",
         "Sinä"),
    "threesea-la-member-creator":
        ("%1 (creator)",
         "%1 (luoja)"),
    "threesea-he-create":
        ("Create",
         "Luo"),
    "threesea-he-new-group":
        ("New group",
         "Uusi ryhmä"),
    "threesea-me-remove-picture":
        ("Remove picture",
         "Poista kuva"),
    "threesea-me-set-picture":
        ("Set picture",
         "Aseta kuva"),
    "threesea-me-add-members":
        ("Add members",
         "Lisää jäseniä"),
    "threesea-he-add-members":
        ("Add members",
         "Lisää jäseniä"),
    "threesea-la-removing-member":
        ("Removing %1",
         "Poistetaan %1"),
    "threesea-la-poll-preview":
        ("Poll: %1",
         "Kysely: %1"),
    "threesea-me-close-poll":
        ("Close poll",
         "Sulje kysely"),
    "threesea-la-poll-closed":
        ("Closed",
         "Suljettu"),
    "threesea-la-poll-multiple":
        ("Multiple choice",
         "Monivalinta"),
    "threesea-la-poll-single":
        ("Single choice",
         "Yksi valinta"),
    "threesea-la-poll-votes":
        ("%n vote(s)",
         ("%n ääni",
          "%n ääntä")),
    "threesea-me-create-poll":
        ("Create poll",
         "Luo kysely"),
    "threesea-he-send":
        ("Send",
         "Lähetä"),
    "threesea-he-new-poll":
        ("New poll",
         "Uusi kysely"),
    "threesea-la-poll-question":
        ("Question",
         "Kysymys"),
    "threesea-he-poll-choices":
        ("Choices",
         "Vaihtoehdot"),
    "threesea-la-poll-choice":
        ("Choice %1",
         "Vaihtoehto %1"),
    "threesea-la-poll-announce":
        ("Show votes before closing",
         "Näytä äänet ennen sulkemista"),
    "threesea-la-poll-announce-description":
        ("Otherwise only you see the votes until you close the poll.",
         "Muuten vain sinä näet äänet, kunnes suljet kyselyn."),
    "threesea-la-poll-you-voted":
        ("You voted",
         "Äänestit"),
    "threesea-la-poll-tap-to-vote":
        ("Tap to vote",
         "Napauta äänestääksesi"),
    "threesea-la-closing-poll":
        ("Closing poll",
         "Suljetaan kysely"),
    "threesea-he-poll-not-voted":
        ("Not voted yet",
         "Ei vielä äänestäneet"),
    "threesea-la-poll-only-you":
        ("Only you see the votes until you close the poll.",
         "Vain sinä näet äänet, kunnes suljet kyselyn."),
    "threesea-me-clear-choices":
        ("Clear choices",
         "Tyhjennä valinnat"),
    "threesea-he-vote":
        ("Vote",
         "Äänestä"),
    "threesea-me-reply":
        ("Reply",
         "Vastaa"),
    "threesea-me-mark-read":
        ("Mark as read",
         "Merkitse luetuksi"),
    "threesea-la-new-message":
        ("New message",
         "Uusi viesti"),
    "threesea-se-notifications":
        ("Notifications",
         "Ilmoitukset"),
    "threesea-la-notifications":
        ("Notify about new messages",
         "Ilmoita uusista viesteistä"),
    "threesea-la-notification-preview":
        ("Show message content",
         "Näytä viestin sisältö"),
    "threesea-me-quote":
        ("Quote",
         "Lainaa"),
    "threesea-la-quote-missing":
        ("Message not available",
         "Viesti ei ole saatavilla"),
    "threesea-la-quoting":
        ("Quoting %1",
         "Lainataan: %1"),
    "threesea-he-share":
        ("Share",
         "Jaa"),
    "threesea-la-share-files":
        ("%n file(s)",
         ("%n tiedosto",
          "%n tiedostoa")),
    "threesea-la-error-qr-invalid":
        ("This is not a Threema contact QR code.",
         "Tämä ei ole Threema-yhteystiedon QR-koodi."),
    "threesea-la-error-qr-mismatch":
        ("This code has the right Threema ID but a different key from the one known for it, so it may be forged. It was not accepted. Compare the codes with this person directly.",
         "Tässä koodissa on oikea Threema-tunnus, mutta eri avain kuin siihen liitetty tunnettu avain, joten koodi voi olla väärennetty. Sitä ei hyväksytty. Vertaa koodeja suoraan tämän henkilön kanssa."),
    "threesea-me-verify-qr":
        ("Verify by QR code",
         "Vahvista QR-koodilla"),
    "threesea-he-scan-qr":
        ("Scan QR code",
         "Skannaa QR-koodi"),
    "threesea-he-my-qr-code":
        ("My QR code",
         "Oma QR-koodi"),
    "threesea-la-my-qr-code-description":
        ("When someone scans this code with Threema, you are verified for them in person (three dots).",
         "Kun joku skannaa tämän koodin Threemalla, olet hänelle kasvokkain vahvistettu (kolme pistettä)."),
    "threesea-la-qr-other-contact":
        ("This is someone else's code.",
         "Tämä on jonkun toisen koodi."),
    "threesea-la-qr-verified":
        ("%1 is verified",
         "%1 on vahvistettu"),
    "threesea-me-scan-from-picture":
        ("Scan from picture",
         "Skannaa kuvasta"),
    "threesea-la-no-camera":
        ("No camera available. Pull down to scan a code from a picture.",
         "Kameraa ei ole käytettävissä. Vedä alas skannataksesi koodin kuvasta."),
    "threesea-la-scan-hint":
        ("Point the camera at the Threema QR code of the person next to you.",
         "Osoita kameralla vieressäsi olevan henkilön Threema-QR-koodia."),
    "threesea-la-qr-not-found":
        ("No Threema QR code found in this picture.",
         "Kuvasta ei löytynyt Threema-QR-koodia."),
    "threesea-la-address-book-unavailable":
        ("The address book is not available on this device.",
         "Osoitekirja ei ole käytettävissä tällä laitteella."),
    "threesea-la-address-book-added":
        ("%n new contact(s) from your address book",
         ("%n uusi yhteystieto osoitekirjasta",
          "%n uutta yhteystietoa osoitekirjasta")),
    "threesea-la-address-book-sync":
        ("Find contacts from the address book",
         "Etsi yhteystietoja osoitekirjasta"),
    "threesea-la-address-book-sync-description":
        ("Daily, only hashed e-mail addresses and phone numbers are sent to Threema to find who uses it. Found people become contacts with their address book names.",
         "Kerran päivässä Threemalle lähetetään vain tiivisteiksi muunnetut sähköpostiosoitteet ja puhelinnumerot, jotta nähdään, ketkä sitä käyttävät. Löydetyt henkilöt lisätään yhteystiedoiksi osoitekirjassa olevilla nimillään."),
    "threesea-la-mention-all":
        ("All",
         "Kaikki"),
    "threesea-la-mention-me":
        ("Me",
         "Minä"),
    "threesea-la-chat-notifications":
        ("Notifications",
         "Ilmoitukset"),
    "threesea-la-notifications-until":
        ("Until %1",
         "%1 asti"),
    "threesea-me-notifications-on":
        ("On",
         "Päällä"),
    "threesea-me-notifications-1-hour":
        ("Off for 1 hour",
         "Pois 1 tunniksi"),
    "threesea-me-notifications-8-hours":
        ("Off for 8 hours",
         "Pois 8 tunniksi"),
    "threesea-me-notifications-1-day":
        ("Off for 1 day",
         "Pois 1 päiväksi"),
    "threesea-me-notifications-1-week":
        ("Off for 1 week",
         "Pois 1 viikoksi"),
    "threesea-me-notifications-off":
        ("Off",
         "Pois"),
    "threesea-me-notifications-mentioned":
        ("Only when I am mentioned",
         "Vain kun minut mainitaan"),
    "threesea-la-error-file-not-downloaded":
        ("Download the file first.",
         "Lataa tiedosto ensin."),
    "threesea-la-error-not-forwardable":
        ("This message cannot be forwarded.",
         "Tätä viestiä ei voi välittää eteenpäin."),
    "threesea-me-forward":
        ("Forward",
         "Välitä"),
    "threesea-he-forward":
        ("Forward",
         "Välitä eteenpäin"),
    "threesea-la-error-contact-blocked":
        ("You have blocked this contact.",
         "Olet estänyt tämän yhteystiedon."),
    "threesea-me-block-id":
        ("Block a Threema ID",
         "Estä Threema-tunnus"),
    "threesea-he-block":
        ("Block",
         "Estä"),
    "threesea-la-block-intro":
        ("Messages from this Threema ID will be discarded, and you cannot write to it.",
         "Tästä Threema-tunnuksesta tulevat viestit hylätään, etkä voi kirjoittaa sille."),
    "threesea-he-blocked-contacts":
        ("Blocked contacts",
         "Estetyt yhteystiedot"),
    "threesea-me-unblock":
        ("Unblock",
         "Poista esto"),
    "threesea-la-no-blocked":
        ("No blocked contacts",
         "Ei estettyjä yhteystietoja"),
    "threesea-la-blocked-input":
        ("You have blocked this contact",
         "Olet estänyt tämän yhteystiedon"),
    "threesea-me-block":
        ("Block",
         "Estä"),
    "threesea-la-blocked":
        ("Blocked",
         "Estetty"),
    "threesea-me-send-location":
        ("Send location",
         "Lähetä sijainti"),
    "threesea-la-location-unavailable":
        ("Positioning is not available. Turn on location in Settings → Location.",
         "Paikannus ei ole käytettävissä. Ota sijainti käyttöön kohdassa Asetukset → Sijainti."),
    "threesea-la-location-finding":
        ("Finding your location…",
         "Etsitään sijaintiasi…"),
    "threesea-la-location-name":
        ("Name of the place (optional)",
         "Paikan nimi (valinnainen)"),
    "threesea-me-search":
        ("Search",
         "Hae"),
    "threesea-he-search":
        ("Search",
         "Haku"),
    "threesea-la-search-messages":
        ("Search messages",
         "Hae viestejä"),
    "threesea-la-search-you-to":
        ("You → %1",
         "Sinä → %1"),
    "threesea-la-search-nothing":
        ("No messages found",
         "Viestejä ei löytynyt"),
    "threesea-se-pinned":
        ("Pinned",
         "Kiinnitetyt"),
    "threesea-me-unpin":
        ("Unpin",
         "Poista kiinnitys"),
    "threesea-me-pin":
        ("Pin to top",
         "Kiinnitä ylös"),
    "threesea-me-move-up":
        ("Move up",
         "Siirrä ylös"),
    "threesea-me-move-down":
        ("Move down",
         "Siirrä alas"),
    "threesea-me-send-video":
        ("Send video",
         "Lähetä video"),
    "threesea-la-cannot-play-video":
        ("This video could not be played",
         "Videota ei voitu toistaa"),
    "threesea-me-select-messages":
        ("Select messages",
         "Valitse viestejä"),
    "threesea-he-n-selected":
        ("%n selected",
         ("%n valittu",
          "%n valittu")),
    "threesea-la-deleting-messages":
        ("Deleting %n message(s)",
         ("Poistetaan %n viesti",
          "Poistetaan %n viestiä")),
    "threesea-la-n-messages":
        ("%n message(s)",
         ("%n viesti",
          "%n viestiä")),
    "threesea-la-video":
        ("Video",
         "Video"),
    "threesea-me-cancel-selection":
        ("Cancel selection",
         "Peru valinta"),
    "threesea-la-select-hint":
        ("Tap messages to select them",
         "Valitse viestejä napauttamalla"),
    "threesea-la-status-list-created":
        ("You created this distribution list.",
         "Loit tämän jakelulistan."),
    "threesea-he-new-list":
        ("New distribution list",
         "Uusi jakelulista"),
    "threesea-me-new-list":
        ("New distribution list",
         "Uusi jakelulista"),
    "threesea-me-delete-list":
        ("Delete list",
         "Poista lista"),
    "threesea-la-deleting-list":
        ("Deleting list",
         "Poistetaan listaa"),
    "threesea-he-distribution-list":
        ("Distribution list",
         "Jakelulista"),
    "threesea-la-recipient-count":
        ("%n recipient(s)",
         ("%n vastaanottaja",
          "%n vastaanottajaa")),
    "threesea-la-distribution-list-info":
        ("Each recipient gets the messages as a normal message from you; they don't see who else got them.",
         "Jokainen vastaanottaja saa viestit tavallisina viesteinä sinulta; he eivät näe, kuka muu ne sai."),
    "threesea-he-recipients":
        ("Recipients",
         "Vastaanottajat"),
    "threesea-la-list-name":
        ("List name",
         "Listan nimi"),
    "threesea-me-star":
        ("Star",
         "Merkitse tähdellä"),
    "threesea-he-starred":
        ("Starred messages",
         "Tähdellä merkityt viestit"),
    "threesea-la-no-starred":
        ("No starred messages",
         "Ei tähdellä merkittyjä viestejä"),
    "threesea-la-no-starred-hint":
        ("Long press a message and choose Star to keep it here.",
         "Paina viestiä pitkään ja valitse Merkitse tähdellä, niin se säilyy täällä."),
    "threesea-la-draft":
        ("Draft: %1",
         "Luonnos: %1"),
    "threesea-he-archived-chats":
        ("Archived chats",
         "Arkistoidut keskustelut"),
    "threesea-me-unarchive":
        ("Move back to chats",
         "Siirrä takaisin keskusteluihin"),
    "threesea-me-archive":
        ("Archive",
         "Arkistoi"),
    "threesea-la-no-archived":
        ("No archived chats",
         "Ei arkistoituja keskusteluja"),
    "threesea-me-details":
        ("Details",
         "Tiedot"),
    "threesea-he-message-details":
        ("Message details",
         "Viestin tiedot"),
    "threesea-la-details-created":
        ("Created",
         "Luotu"),
    "threesea-la-details-sent":
        ("Sent",
         "Lähetetty"),
    "threesea-la-details-received":
        ("Received",
         "Vastaanotettu"),
    "threesea-la-details-delivered":
        ("Delivered",
         "Toimitettu"),
    "threesea-la-details-read":
        ("Read",
         "Luettu"),
    "threesea-la-details-deleted":
        ("Deleted",
         "Poistettu"),
    "threesea-la-details-edited":
        ("Edited",
         "Muokattu"),
    "threesea-la-details-file-type":
        ("File type",
         "Tiedostotyyppi"),
    "threesea-la-details-file-size":
        ("File size",
         "Tiedostokoko"),
    "threesea-la-details-message-id":
        ("Message ID",
         "Viestin tunniste"),
    "threesea-me-show-in-chat":
        ("Show in chat",
         "Näytä keskustelussa"),
    "threesea-la-no-media":
        ("No media",
         "Ei mediaa"),
    "threesea-la-wrong-pin":
        ("Wrong PIN",
         "Väärä PIN-koodi"),
    "threesea-la-error-pin-short":
        ("The PIN needs at least 4 digits.",
         "PIN-koodissa on oltava vähintään 4 numeroa."),
    "threesea-la-error-locked":
        ("Enter your PIN first.",
         "Syötä ensin PIN-koodisi."),
    "threesea-la-private-needs-lock":
        ("Private chats need an app lock. Set one up in Settings, Security, App lock.",
         "Yksityiset keskustelut vaativat sovelluslukon. Ota se käyttöön kohdassa Asetukset, Turvallisuus, Sovelluslukko."),
    "threesea-la-error-private-chats-exist":
        ("Remove all chats from the private chats first.",
         "Poista ensin kaikki keskustelut yksityisistä keskusteluista."),
    "threesea-la-private-chat":
        ("Private chat",
         "Yksityinen keskustelu"),
    "threesea-me-lock-immediately":
        ("Immediately",
         "Heti"),
    "threesea-me-lock-30-seconds":
        ("After 30 seconds",
         "30 sekunnin kuluttua"),
    "threesea-me-lock-1-minute":
        ("After 1 minute",
         "1 minuutin kuluttua"),
    "threesea-me-lock-2-minutes":
        ("After 2 minutes",
         "2 minuutin kuluttua"),
    "threesea-me-lock-5-minutes":
        ("After 5 minutes",
         "5 minuutin kuluttua"),
    "threesea-me-lock-10-minutes":
        ("After 10 minutes",
         "10 minuutin kuluttua"),
    "threesea-me-lock-30-minutes":
        ("After 30 minutes",
         "30 minuutin kuluttua"),
    "threesea-he-app-lock":
        ("App lock",
         "Sovelluslukko"),
    "threesea-me-lock-off":
        ("Off",
         "Pois"),
    "threesea-me-lock-pin":
        ("PIN or passphrase",
         "PIN-koodi tai tunnuslause"),
    "threesea-la-change-pin":
        ("Change PIN",
         "Vaihda PIN-koodi"),
    "threesea-la-lock-app":
        ("Lock the app",
         "Lukitse sovellus"),
    "threesea-la-lock-app-description":
        ("Ask when Threesea starts, and when you come back to it after the time you choose. Private chats always ask.",
         "Kysy, kun Threesea käynnistyy ja kun palaat siihen valitsemasi ajan jälkeen. Yksityiset keskustelut kysyvät aina."),
    "threesea-la-lock-after":
        ("Lock",
         "Lukitse"),
    "threesea-he-private-chats":
        ("Private chats",
         "Yksityiset keskustelut"),
    "threesea-me-unmark-private":
        ("Remove from private chats",
         "Poista yksityisistä keskusteluista"),
    "threesea-me-mark-private":
        ("Make private",
         "Tee yksityiseksi"),
    "threesea-la-no-private":
        ("No private chats",
         "Ei yksityisiä keskusteluja"),
    "threesea-he-repeat-pin":
        ("Enter the PIN again",
         "Syötä PIN-koodi uudelleen"),
    "threesea-la-pin-hint":
        ("At least 4 digits",
         "Vähintään 4 numeroa"),
    "threesea-la-pin-mismatch":
        ("They did not match. Try again.",
         "Ne eivät täsmänneet. Yritä uudelleen."),
    "threesea-he-enter-pin":
        ("Enter your PIN",
         "Syötä PIN-koodisi"),
    "threesea-la-locked-notification":
        ("Threesea is locked",
         "Threesea on lukittu"),
    "threesea-la-locked-notification-body":
        ("Open it and enter your PIN or passphrase to receive messages.",
         "Avaa se ja syötä PIN-koodisi tai tunnuslauseesi, jotta voit vastaanottaa viestejä."),
    "threesea-la-key-encryption-failed":
        ("Could not change the encryption of the stored data.",
         "Tallennettujen tietojen salausta ei voitu muuttaa."),
    "threesea-la-lock-passphrase":
        ("Passphrase",
         "Tunnuslause"),
    "threesea-la-lock-pin":
        ("PIN",
         "PIN-koodi"),
    "threesea-la-change-passphrase":
        ("Change passphrase",
         "Vaihda tunnuslause"),
    "threesea-he-new-passphrase":
        ("Enter new passphrase",
         "Syötä uusi tunnuslause"),
    "threesea-he-new-pin":
        ("Enter new PIN",
         "Syötä uusi PIN-koodi"),
    "threesea-he-repeat-passphrase":
        ("Enter the passphrase again",
         "Syötä tunnuslause uudelleen"),
    "threesea-la-passphrase-hint":
        ("At least %1 characters",
         "Vähintään %1 merkkiä"),
    "threesea-he-enter-passphrase":
        ("Enter your passphrase",
         "Syötä tunnuslauseesi"),
    "threesea-la-wrong-passphrase":
        ("Wrong passphrase",
         "Väärä tunnuslause"),
    "threesea-la-revocation-intro":
        ("With this password you can revoke your Threema ID at myid.threema.ch/revoke in case you lose your phone or it gets stolen. Use at least 8 characters.",
         "Tällä salasanalla voit mitätöidä Threema-tunnuksesi osoitteessa myid.threema.ch/revoke, jos puhelimesi katoaa tai varastetaan. Käytä vähintään 8 merkkiä."),
    "threesea-la-error-phone-invalid":
        ("Enter the number with its country code, e.g. +41 79 123 45 67.",
         "Syötä numero maatunnuksen kanssa, esim. +41 79 123 45 67."),
    "threesea-la-error-email-invalid":
        ("This is not an e-mail address.",
         "Tämä ei ole sähköpostiosoite."),
    "threesea-la-error-no-verification":
        ("No phone number is waiting for a code. Link it again.",
         "Mikään puhelinnumero ei odota koodia. Yhdistä se uudelleen."),
    "threesea-me-unlink":
        ("Remove link",
         "Poista yhdistäminen"),
    "threesea-he-link":
        ("Link",
         "Yhdistä"),
    "threesea-la-link-email-info":
        ("People who have this e-mail address in their address book can find you on Threema. It is sent to Threema, which e-mails you a link to confirm it.",
         "Henkilöt, joiden osoitekirjassa tämä sähköpostiosoite on, voivat löytää sinut Threemasta. Osoite lähetetään Threemalle, joka lähettää sinulle sähköpostilla vahvistuslinkin."),
    "threesea-la-link-mobile-info":
        ("People who have this number in their address book can find you on Threema. It is sent to Threema, which texts you a code to confirm it.",
         "Henkilöt, joiden osoitekirjassa tämä numero on, voivat löytää sinut Threemasta. Numero lähetetään Threemalle, joka lähettää sinulle tekstiviestillä vahvistuskoodin."),
    "threesea-la-email":
        ("E-mail address",
         "Sähköpostiosoite"),
    "threesea-la-phone":
        ("Phone number",
         "Puhelinnumero"),
    "threesea-me-cancel-link":
        ("Cancel linking",
         "Peru yhdistäminen"),
    "threesea-me-call-me":
        ("Call me instead",
         "Soita minulle sen sijaan"),
    "threesea-la-call-coming":
        ("You will get a call with the code.",
         "Saat koodin puhelulla."),
    "threesea-he-confirm":
        ("Confirm",
         "Vahvista"),
    "threesea-la-enter-sms-code":
        ("Enter the code Threema sent to +%1.",
         "Syötä koodi, jonka Threema lähetti numeroon +%1."),
    "threesea-la-sms-code":
        ("Code",
         "Koodi"),
    "threesea-he-media":
        ("Media",
         "Media"),
    "threesea-me-media-show":
        ("Show",
         "Näytä"),
    "threesea-la-kind-photos":
        ("Photos",
         "Kuvat"),
    "threesea-la-kind-gifs":
        ("GIFs",
         "GIFit"),
    "threesea-la-kind-videos":
        ("Videos",
         "Videot"),
    "threesea-la-kind-voice":
        ("Voice messages",
         "Ääniviestit"),
    "threesea-la-kind-audio":
        ("Audio",
         "Äänet"),
    "threesea-la-kind-files":
        ("Files",
         "Tiedostot"),
    "threesea-la-no-media-filtered":
        ("Nothing of the kinds shown",
         "Ei näytettäviä tyyppejä"),
    "threesea-la-no-media-filtered-hint":
        ("Pull down to choose what to show",
         "Vedä alas valitaksesi, mitä näytetään"),
    "threesea-la-enter-to-send":
        ("Enter sends",
         "Enter lähettää"),
    "threesea-la-enter-to-send-description":
        ("Enter sends the message. Off: Enter starts a new line and the send button sends. On a hardware keyboard, Shift+Enter does the opposite.",
         "Enter lähettää viestin. Pois: Enter aloittaa uuden rivin ja lähetyspainike lähettää. Fyysisellä näppäimistöllä Vaihto+Enter toimii päinvastoin."),
    "threesea-se-advanced":
        ("Advanced",
         "Lisäasetukset"),
    "threesea-se-security":
        ("Security",
         "Turvallisuus"),
    "threesea-se-account":
        ("Account",
         "Tili"),
    "threesea-se-privacy-security":
        ("Privacy and security",
         "Tietosuoja ja turvallisuus"),
    "threesea-la-app-lock-on":
        ("App lock on",
         "Sovelluslukko päällä"),
    "threesea-la-app-lock-off":
        ("App lock off",
         "Sovelluslukko pois"),
    "threesea-se-app":
        ("App",
         "Sovellus"),
    "threesea-se-info":
        ("Info",
         "Tietoja"),
    "threesea-me-delete-data":
        ("Remove ID from this device",
         "Poista tunnus tästä laitteesta"),
    "threesea-la-removing-data":
        ("Removing ID and data",
         "Poistetaan tunnus ja tiedot"),
    "threesea-la-revocation":
        ("ID revocation",
         "Tunnuksen mitätöinti"),
    "threesea-la-saving":
        ("Saving…",
         "Tallennetaan…"),
    "threesea-la-revocation-set-at":
        ("Password set %1",
         "Salasana asetettu %1"),
    "threesea-la-revocation-set":
        ("Password set",
         "Salasana asetettu"),
    "threesea-la-revocation-not-set":
        ("No password set",
         "Salasanaa ei ole asetettu"),
    "threesea-la-link-email":
        ("E-mail",
         "Sähköposti"),
    "threesea-la-not-linked":
        ("Not linked",
         "Ei yhdistetty"),
    "threesea-la-email-pending":
        ("Waiting for you to confirm the link in the e-mail.",
         "Odotetaan, että vahvistat sähköpostissa olevan linkin."),
    "threesea-la-link-phone":
        ("Phone number",
         "Puhelinnumero"),
    "threesea-la-phone-pending":
        ("+%1 (not confirmed)",
         "+%1 (ei vahvistettu)"),
    "threesea-la-nickname-placeholder":
        ("Shown to the people you write to",
         "Näkyy henkilöille, joille kirjoitat"),
    "threesea-la-change-profile-picture":
        ("Change profile picture",
         "Vaihda profiilikuva"),
    "threesea-la-set-profile-picture":
        ("Set profile picture",
         "Aseta profiilikuva"),
    "threesea-me-share-everyone":
        ("Everyone I write to",
         "Kaikki, joille kirjoitan"),
    "threesea-me-share-nobody":
        ("Nobody",
         "Ei kukaan"),
    "threesea-me-share-selected":
        ("Selected contacts",
         "Valitut yhteystiedot"),
    "threesea-la-picture-share":
        ("Share profile picture with",
         "Jaa profiilikuva"),
    "threesea-la-share-selected-count":
        ("%n selected contact(s)",
         ("%n valittu yhteystieto",
          "%n valittua yhteystietoa")),
    "threesea-me-exclude-id":
        ("Add a Threema ID",
         "Lisää Threema-tunnus"),
    "threesea-la-exclusion-intro":
        ("Finding contacts from the address book will not add or rename this Threema ID.",
         "Yhteystietojen etsiminen osoitekirjasta ei lisää tätä Threema-tunnusta eikä muuta sen nimeä."),
    "threesea-he-exclusion-list":
        ("Exclusion list",
         "Poissulkemislista"),
    "threesea-me-remove-excluded":
        ("Remove",
         "Poista"),
    "threesea-la-no-excluded":
        ("Nobody is excluded",
         "Ketään ei ole poissuljettu"),
    "threesea-me-refresh":
        ("Refresh",
         "Päivitä"),
    "threesea-me-empty-chat":
        ("Empty chat",
         "Tyhjennä keskustelu"),
    "threesea-la-emptying-chat":
        ("Emptying chat",
         "Tyhjennetään keskustelua"),
    "threesea-se-other":
        ("Other",
         "Muut"),
    "threesea-la-incognito-keyboard":
        ("Incognito keyboard",
         "Incognito-näppäimistö"),
    "threesea-la-incognito-keyboard-description":
        ("Ask the keyboard not to learn or suggest from what you type in chats.",
         "Pyydä näppäimistöä olemaan oppimatta ja ehdottamatta keskusteluissa kirjoittamastasi tekstistä."),
    "threesea-la-hide-cover":
        ("Hide content on the cover",
         "Piilota sisältö kannessa"),
    "threesea-la-hide-cover-description":
        ("The cover only shows the number of new messages, not who wrote.",
         "Kansi näyttää vain uusien viestien määrän, ei kirjoittajaa."),
    "threesea-se-appearance":
        ("Appearance",
         "Ulkoasu"),
    "threesea-la-bigger-emojis":
        ("Bigger single emojis",
         "Suuremmat yksittäiset emojit"),
    "threesea-la-bigger-emojis-description":
        ("Messages of up to three emojis are shown larger.",
         "Enintään kolmen emojin viestit näytetään suurempina."),
    "threesea-se-contacts":
        ("Contacts",
         "Yhteystiedot"),
    "threesea-se-receipts":
        ("Receipts",
         "Kuittaukset"),
    "threesea-me-policy-default-on":
        ("Same as in Settings (on)",
         "Sama kuin asetuksissa (päällä)"),
    "threesea-me-policy-default-off":
        ("Same as in Settings (off)",
         "Sama kuin asetuksissa (pois)"),
    "threesea-me-policy-send":
        ("Always",
         "Aina"),
    "threesea-me-policy-dont-send":
        ("Never",
         "Ei koskaan"),
    "threesea-la-contact-read-receipts":
        ("Read receipts",
         "Lukukuittaukset"),
    "threesea-la-contact-typing":
        ("Typing indicator",
         "Kirjoitusilmaisin"),
    "threesea-me-clear-contact-policies":
        ("Clear individual settings",
         "Tyhjennä yksilölliset asetukset"),
    "threesea-la-clearing-contact-policies":
        ("Clearing individual settings",
         "Tyhjennetään yksilöllisiä asetuksia"),
    "threesea-la-no-contact-policies":
        ("No contact has its own setting.",
         "Millään yhteystiedolla ei ole omaa asetusta."),
    "threesea-se-data-encryption":
        ("Encryption of stored data",
         "Tallennettujen tietojen salaus"),
    "threesea-me-older-1-year":
        ("Older than 1 year",
         "Yli 1 vuoden vanhat"),
    "threesea-me-older-6-months":
        ("Older than 6 months",
         "Yli 6 kuukauden vanhat"),
    "threesea-me-older-3-months":
        ("Older than 3 months",
         "Yli 3 kuukauden vanhat"),
    "threesea-me-older-1-month":
        ("Older than 1 month",
         "Yli 1 kuukauden vanhat"),
    "threesea-me-older-1-week":
        ("Older than 1 week",
         "Yli 1 viikon vanhat"),
    "threesea-me-older-all":
        ("All",
         "Kaikki"),
    "threesea-he-delete":
        ("Delete",
         "Poista"),
    "threesea-se-clean-up-what":
        ("What",
         "Mitä"),
    "threesea-me-clean-up-media":
        ("Media files",
         "Mediatiedostot"),
    "threesea-me-clean-up-media-description":
        ("Photos, videos, voice messages and files are deleted from this device, usually for good; the messages and their previews stay. Starred messages and messages still on their way keep their files.",
         "Kuvat, videot, ääniviestit ja tiedostot poistetaan tältä laitteelta, yleensä lopullisesti; viestit ja niiden esikatselut säilyvät. Tähdellä merkityt viestit ja vielä matkalla olevat viestit säilyttävät tiedostonsa."),
    "threesea-me-clean-up-messages":
        ("Messages and media",
         "Viestit ja media"),
    "threesea-me-clean-up-messages-description":
        ("Starred messages, open polls and messages still on their way stay.",
         "Tähdellä merkityt viestit, avoimet kyselyt ja vielä matkalla olevat viestit säilyvät."),
    "threesea-se-clean-up-age":
        ("Age",
         "Ikä"),
    "threesea-me-image-small":
        ("Small (640 × 640)",
         "Pieni (640 × 640)"),
    "threesea-me-image-medium":
        ("Medium (1024 × 1024)",
         "Keskikokoinen (1024 × 1024)"),
    "threesea-me-image-large":
        ("Large (1600 × 1600)",
         "Suuri (1600 × 1600)"),
    "threesea-me-image-xlarge":
        ("Extra large (2592 × 2592)",
         "Erittäin suuri (2592 × 2592)"),
    "threesea-me-image-original":
        ("Original size",
         "Alkuperäinen koko"),
    "threesea-la-download-nothing":
        ("Nothing",
         "Ei mitään"),
    "threesea-se-media-storage":
        ("Media and storage",
         "Media ja tallennustila"),
    "threesea-se-sending":
        ("Sending",
         "Lähettäminen"),
    "threesea-la-image-size":
        ("Image size",
         "Kuvan koko"),
    "threesea-se-receiving":
        ("Receiving",
         "Vastaanottaminen"),
    "threesea-la-download-wifi":
        ("Download on Wi-Fi",
         "Lataa Wi-Fi-yhteydellä"),
    "threesea-la-download-mobile":
        ("Download on mobile data",
         "Lataa mobiilidatalla"),
    "threesea-la-auto-save":
        ("Save to Gallery",
         "Tallenna Galleriaan"),
    "threesea-la-auto-save-description":
        ("Received photos and videos are also saved, unencrypted, to the Threesea folder in Pictures.",
         "Vastaanotetut kuvat ja videot tallennetaan lisäksi salaamattomina Kuvat-kansion Threesea-kansioon."),
    "threesea-se-storage":
        ("Storage",
         "Tallennustila"),
    "threesea-la-storage-messages":
        ("Messages",
         "Viestit"),
    "threesea-me-clean-up":
        ("Clean up",
         "Siivoa"),
    "threesea-la-twemoji-credit":
        ("Emoji graphics by Twemoji (github.com/jdecked/twemoji), licensed under CC-BY 4.0.",
         "Emojigrafiikat: Twemoji (github.com/jdecked/twemoji), lisenssi CC-BY 4.0."),
    "threesea-me-emoji-system":
        ("System",
         "Järjestelmä"),
    "threesea-la-emoji-style":
        ("Emoji style",
         "Emojityyli"),
    "threesea-se-blocking":
        ("Blocking",
         "Estäminen"),
    "threesea-me-first-name-first":
        ("First name first",
         "Etunimi ensin"),
    "threesea-me-last-name-first":
        ("Last name first",
         "Sukunimi ensin"),
    "threesea-la-name-order":
        ("Names",
         "Nimet"),
    "threesea-me-sort-first-name":
        ("By first name",
         "Etunimen mukaan"),
    "threesea-me-sort-last-name":
        ("By last name",
         "Sukunimen mukaan"),
    "threesea-la-sorting":
        ("Sorting",
         "Lajittelu"),
    "threesea-la-show-inactive":
        ("Show inactive IDs",
         "Näytä passiiviset tunnukset"),
    "threesea-la-show-inactive-description":
        ("Contacts whose Threema ID has not been used for a long time stay in the list.",
         "Yhteystiedot, joiden Threema-tunnusta ei ole käytetty pitkään aikaan, pysyvät luettelossa."),
    "threesea-se-notifications-chats":
        ("Chats",
         "Keskustelut"),
    "threesea-la-notification-sound":
        ("Sound",
         "Ääni"),
    "threesea-la-notification-sound-description":
        ("Played at the volume of the current sound profile.",
         "Toistetaan nykyisen ääniprofiilin äänenvoimakkuudella."),
    "threesea-la-notification-vibrate":
        ("Vibrate",
         "Värinä"),
    "threesea-se-notifications-groups":
        ("Groups",
         "Ryhmät"),
    "threesea-se-network":
        ("Network",
         "Verkko"),
    "threesea-la-ipv6":
        ("IPv6",
         "IPv6"),
    "threesea-la-ipv6-description":
        ("Messages, files and calls use IPv6 where the network has it. Off: Threesea uses IPv4 for them.",
         "Viestit, tiedostot ja puhelut käyttävät IPv6:ta, jos verkossa on se. Pois: Threesea käyttää niille IPv4:ää."),
    "threesea-la-error-not-gif":
        ("This picture is not a GIF.",
         "Tämä kuva ei ole GIF."),
    "threesea-la-gif-favourite-added":
        ("Added to favourite GIFs",
         "Lisätty GIF-suosikkeihin"),
    "threesea-he-add-gif":
        ("Add GIF",
         "Lisää GIF"),
    "threesea-me-remove-gif":
        ("Remove",
         "Poista"),
    "threesea-la-no-gif-favourites":
        ("No favourite GIFs",
         "Ei GIF-suosikkeja"),
    "threesea-la-no-gif-favourites-hint":
        ("Long press a GIF in a chat to add it, or add one from this device.",
         "Lisää GIF painamalla sitä pitkään keskustelussa tai lisää se tältä laitteelta."),
    "threesea-la-autoplay-gifs":
        ("Play GIFs automatically",
         "Toista GIFit automaattisesti"),
    "threesea-la-autoplay-gifs-description":
        ("Animated images play in the chat. Off: they play when you open them.",
         "Animoidut kuvat toistetaan keskustelussa. Pois: ne toistetaan, kun avaat ne."),
    "threesea-la-notification-tone":
        ("Notification tone",
         "Ilmoitusääni"),
    "threesea-la-tone-all-chats":
        ("Same as all chats",
         "Sama kuin kaikissa keskusteluissa"),
    "threesea-la-tone-all-groups":
        ("Same as all groups",
         "Sama kuin kaikissa ryhmissä"),
    "threesea-la-tone-profile":
        ("Instant message tone",
         "Pikaviestin ääni"),
    "threesea-la-tone-from-music":
        ("Select from music",
         "Valitse musiikista"),
    "threesea-se-tones":
        ("Tones",
         "Äänet"),
    "threesea-la-error-link-unknown-code":
        ("This is not a code of Threema for Desktop. Scan the code it shows when you open it.",
         "Tämä ei ole Threema Desktopin koodi. Skannaa koodi, jonka se näyttää, kun avaat sen."),
    "threesea-la-error-link-web-code":
        ("This code is for Threema Web. Scan the code of Threema for Desktop instead.",
         "Tämä koodi on Threema Webille. Skannaa sen sijaan Threema Desktopin koodi."),
    "threesea-la-error-link-version":
        ("This version of Threema for Desktop can't be linked. Please update both apps.",
         "Tätä Threema Desktopin versiota ei voi yhdistää. Päivitä molemmat sovellukset."),
    "threesea-la-error-link-failed":
        ("Linking didn't work. Check the connection on both devices and try again.",
         "Yhdistäminen ei onnistunut. Tarkista yhteys molemmissa laitteissa ja yritä uudelleen."),
    "threesea-la-qr-not-device":
        ("This is a contact's code. Scan the code on the computer.",
         "Tämä on yhteystiedon koodi. Skannaa tietokoneella näkyvä koodi."),
    "threesea-la-scan-hint-device":
        ("Point the camera at the QR code Threema for Desktop shows.",
         "Osoita kameralla QR-koodia, jonka Threema Desktop näyttää."),
    "threesea-la-link-intro":
        ("Open Threema for Desktop on your computer. It shows a QR code; scan it on the next page.",
         "Avaa Threema Desktop tietokoneellasi. Se näyttää QR-koodin; skannaa se seuraavalla sivulla."),
    "threesea-la-link-emoji-mismatch":
        ("The emojis don't match. Close Threema for Desktop, open it again and scan its new code.",
         "Emojit eivät täsmää. Sulje Threema Desktop, avaa se uudelleen ja skannaa sen uusi koodi."),
    "threesea-he-link-device":
        ("Link device",
         "Yhdistä laite"),
    "threesea-la-link-connecting":
        ("Connecting to the computer…",
         "Yhdistetään tietokoneeseen…"),
    "threesea-la-link-verify":
        ("Which emojis does the computer show?",
         "Mitkä emojit tietokone näyttää?"),
    "threesea-la-link-linking":
        ("Linking. Keep Threesea open until the computer is ready.",
         "Yhdistetään. Pidä Threesea auki, kunnes tietokone on valmis."),
    "threesea-la-link-done":
        ("Threema for Desktop is linked.",
         "Threema Desktop on yhdistetty."),
    "threesea-me-link-no-match":
        ("None of them",
         "Ei mikään niistä"),
    "threesea-me-done":
        ("Done",
         "Valmis"),
    "threesea-la-removing-device":
        ("Removing",
         "Poistetaan"),
    "threesea-me-remove-all-devices":
        ("Remove all linked devices",
         "Poista kaikki yhdistetyt laitteet"),
    "threesea-la-removing-all-devices":
        ("Removing all linked devices",
         "Poistetaan kaikkia yhdistettyjä laitteita"),
    "threesea-me-link-device":
        ("Link device",
         "Yhdistä laite"),
    "threesea-se-linked-devices":
        ("Linked devices",
         "Yhdistetyt laitteet"),
    "threesea-la-device":
        ("Device",
         "Laite"),
    "threesea-la-this-device":
        ("This device",
         "Tämä laite"),
    "threesea-la-device-connected":
        ("Connected",
         "Yhdistetty"),
    "threesea-la-device-last-active":
        ("Last active %1",
         "Viimeksi aktiivinen %1"),
    "threesea-me-remove-device":
        ("Remove",
         "Poista"),
    "threesea-la-no-linked-devices":
        ("No linked devices",
         "Ei yhdistettyjä laitteita"),
    "threesea-la-no-linked-devices-hint":
        ("With Threema for Desktop you can read and write your chats on a computer. Open it there and pull down here to link it.",
         "Threema Desktopilla voit lukea ja kirjoittaa keskustelujasi tietokoneella. Avaa se siellä ja yhdistä se vetämällä tätä sivua alas."),
    "threesea-me-connect-now":
        ("Connect now",
         "Yhdistä nyt"),
    "threesea-la-removing-lock":
        ("Removing the app lock",
         "Poistetaan sovelluslukkoa"),
    "threesea-me-unstar-short":
        ("Unstar",
         "Poista tähti"),
    "threesea-la-camera-unavailable":
        ("The camera is not available.",
         "Kamera ei ole käytettävissä."),
    "threesea-me-take-photo":
        ("Take photo or video",
         "Ota kuva tai video"),
    "threesea-me-add-gif-favourite":
        ("Add to favourite GIFs",
         "Lisää GIF-suosikkeihin"),
    "threesea-me-add-recipients":
        ("Add recipients",
         "Lisää vastaanottajia"),
    "threesea-la-status-call-outgoing":
        ("Outgoing call, %1",
         "Lähtevä puhelu, %1"),
    "threesea-la-status-call-incoming":
        ("Incoming call, %1",
         "Saapuva puhelu, %1"),
    "threesea-la-status-call-missed":
        ("Missed call",
         "Vastaamaton puhelu"),
    "threesea-la-status-call-declined":
        ("Call declined",
         "Puhelu hylätty"),
    "threesea-la-status-call-you-declined":
        ("You declined the call",
         "Hylkäsit puhelun"),
    "threesea-la-status-call-busy":
        ("Call not answered: busy",
         "Puheluun ei vastattu: varattu"),
    "threesea-la-status-call-unanswered":
        ("Call not answered",
         "Puheluun ei vastattu"),
    "threesea-la-status-call-disabled":
        ("Call not answered: calls are off",
         "Puheluun ei vastattu: puhelut ovat pois päältä"),
    "threesea-la-status-call-cancelled":
        ("Call cancelled",
         "Puhelu peruttu"),
    "threesea-la-status-call-failed":
        ("Call failed",
         "Puhelu epäonnistui"),
    "threesea-la-call-calling":
        ("Calling…",
         "Soitetaan…"),
    "threesea-la-call-ringing":
        ("Ringing…",
         "Hälyttää…"),
    "threesea-la-call-incoming":
        ("Incoming call",
         "Saapuva puhelu"),
    "threesea-la-call-connecting":
        ("Connecting…",
         "Yhdistetään…"),
    "threesea-la-call-ended":
        ("Call ended",
         "Puhelu päättyi"),
    "threesea-me-call":
        ("Call",
         "Soita"),
    "threesea-se-calls":
        ("Calls",
         "Puhelut"),
    "threesea-la-threema-calls":
        ("Threema calls",
         "Threema-puhelut"),
    "threesea-la-threema-calls-description":
        ("Contacts can call you, and you can call them from the chat's pull-down menu.",
         "Yhteystietosi voivat soittaa sinulle, ja voit soittaa heille keskustelun alasvetovalikosta."),
    "threesea-la-relay-calls":
        ("Always relay calls",
         "Välitä puhelut aina palvelimen kautta"),
    "threesea-la-relay-calls-description":
        ("Calls go through a Threema server, which keeps your IP address hidden from the other side. Call quality may be lower.",
         "Puhelut kulkevat Threeman palvelimen kautta, jolloin IP-osoitteesi pysyy piilossa toiselta osapuolelta. Puhelun laatu voi olla heikompi."),
    "threesea-me-video-call":
        ("Video call",
         "Videopuhelu"),
    "threesea-se-video":
        ("Video",
         "Video"),
    "threesea-la-video-calls":
        ("Video in one-to-one calls",
         "Video kahdenkeskisissä puheluissa"),
    "threesea-la-video-calls-description":
        ("One-to-one calls can show the camera of both sides; each side turns its camera on when it wants.",
         "Kahdenkeskisissä puheluissa voidaan näyttää kummankin osapuolen kamera; kumpikin osapuoli laittaa kameransa päälle, kun haluaa."),
    "threesea-la-video-quality":
        ("Video quality",
         "Videon laatu"),
    "threesea-la-video-quality-auto":
        ("Standard",
         "Normaali"),
    "threesea-la-video-quality-low":
        ("Low data use",
         "Vähäinen datankäyttö"),
    "threesea-la-video-quality-max":
        ("Highest quality",
         "Paras laatu"),
    "threesea-la-video-quality-description":
        ("Both sides use the lower of their two settings. Calls that go only through a relay (Always relay calls, or with an unverified contact) use Standard at most.",
         "Molemmat osapuolet käyttävät kahdesta asetuksesta alempaa. Pelkästään välityspalvelimen kautta kulkevat puhelut (Välitä puhelut aina palvelimen kautta, tai vahvistamattoman yhteystiedon kanssa) käyttävät enintään Normaali-laatua."),
    "threesea-me-switch-camera":
        ("Switch camera",
         "Vaihda kameraa"),
    "threesea-me-show-self":
        ("Show own picture",
         "Näytä oma kuva"),
    "threesea-me-hide-self":
        ("Hide own picture",
         "Piilota oma kuva"),
    "threesea-la-me":
        ("Me",
         "Minä"),
    "threesea-me-remove-profile-picture":
        ("Remove profile picture",
         "Poista profiilikuva"),
    "threesea-me-join-group-call":
        ("Join group call",
         "Liity ryhmäpuheluun"),
    "threesea-la-group-call-joining":
        ("Joining…",
         "Liitytään…"),
    "threesea-me-start-group-call":
        ("Start group call",
         "Aloita ryhmäpuhelu"),
    "threesea-la-group-call-waiting":
        ("Waiting for others",
         "Odotetaan muita"),
    "threesea-la-group-call":
        ("Group call",
         "Ryhmäpuhelu"),
    "threesea-la-group-call-participants":
        ("%n participant(s)",
         ("%n osallistuja",
          "%n osallistujaa")),
    "threesea-la-group-call-return":
        ("Return",
         "Palaa"),
    "threesea-la-group-call-join":
        ("Join",
         "Liity"),
    "threesea-la-error-web-version":
        ("This version of Threema Web can't be used. Reload the page in the browser.",
         "Tätä Threema Webin versiota ei voi käyttää. Lataa sivu uudelleen selaimessa."),
    "threesea-la-error-web-session":
        ("This Threema Web session no longer exists.",
         "Tätä Threema Web -istuntoa ei ole enää olemassa."),
    "threesea-he-web-trust":
        ("Trust",
         "Luota"),
    "threesea-he-web-allow":
        ("Allow",
         "Salli"),
    "threesea-la-web-certificate-changed":
        ("This Threema Web server shows a different certificate than last time. Trust it only if you know its certificate was renewed; otherwise go back.",
         "Tämä Threema Web -palvelin näyttää eri varmenteen kuin viime kerralla. Luota siihen vain, jos tiedät, että sen varmenne on uusittu; muuten palaa takaisin."),
    "threesea-la-web-self-hosted":
        ("This code is for a Threema Web server run by someone other than Threema. Your chats go through it end-to-end encrypted. Allow it only if you run it yourself or trust who does. Its certificate is remembered and you'll be asked if it changes.",
         "Tämä koodi on Threema Web -palvelimelle, jota ylläpitää joku muu kuin Threema. Keskustelusi kulkevat sen kautta päästä päähän salattuina. Salli se vain, jos ylläpidät sitä itse tai luotat ylläpitäjään. Sen varmenne tallennetaan, ja sinulta kysytään, jos se muuttuu."),
    "threesea-la-web-connecting":
        ("Connecting…",
         "Yhdistetään…"),
    "threesea-la-web-waiting":
        ("Waiting for the browser",
         "Odotetaan selainta"),
    "threesea-la-web-connected":
        ("Connected",
         "Yhdistetty"),
    "threesea-la-web-last-used":
        ("Last used %1",
         "Viimeksi käytetty %1"),
    "threesea-la-web-not-connected":
        ("Not connected",
         "Ei yhdistetty"),
    "threesea-me-scan-web-code":
        ("Scan Threema Web code",
         "Skannaa Threema Web -koodi"),
    "threesea-he-threema-web":
        ("Threema Web",
         "Threema Web"),
    "threesea-la-web-none":
        ("No browsers",
         "Ei selaimia"),
    "threesea-la-web-none-hint":
        ("Open web.threema.ch in a browser and pull down to scan its code",
         "Avaa web.threema.ch selaimessa ja skannaa sen koodi vetämällä alas"),
    "threesea-me-web-remove":
        ("Remove",
         "Poista"),
    "threesea-la-web-browser":
        ("Browser",
         "Selain"),
    "threesea-la-no-recent-emoji":
        ("No emoji used yet",
         "Emojeja ei ole vielä käytetty"),
    "threesea-la-poll-closed-preview":
        ("Poll closed: %1",
         "Kysely suljettu: %1"),
    "threesea-la-poll-tap-results":
        ("Poll closed. Tap to view results",
         "Kysely suljettu. Näytä tulokset napauttamalla"),
    "threesea-he-poll-results":
        ("Results",
         "Tulokset"),
    "threesea-se-chat":
        ("Chat",
         "Keskustelu"),
    "threesea-la-scan-hint-any":
        ("Point the camera at a contact's Threema QR code, or at the one Threema Web or Threema for Desktop shows.",
         "Osoita kameralla yhteystiedon Threema-QR-koodia tai koodia, jonka Threema Web tai Threema Desktop näyttää."),
    "threesea-la-pin-paused":
        ("Too many wrong attempts. Try again in %n second(s).",
         ("Liian monta väärää yritystä. Yritä uudelleen %n sekunnin kuluttua.",
          "Liian monta väärää yritystä. Yritä uudelleen %n sekunnin kuluttua.")),
    "threesea-se-troubleshooting":
        ("Troubleshooting",
         "Vianmääritys"),
    "threesea-la-log-level":
        ("Log",
         "Loki"),
    "threesea-la-log-level-warn":
        ("Problems only",
         "Vain ongelmat"),
    "threesea-la-log-level-info":
        ("Normal",
         "Normaali"),
    "threesea-la-log-level-debug":
        ("Detailed",
         "Yksityiskohtainen"),
    "threesea-la-log-level-description":
        ("How much Threesea logs, to help find the cause of a problem.",
         "Kuinka paljon Threesea kirjaa lokiin ongelman syyn selvittämiseksi."),
    "threesea-se-disclaimer":
        ("Disclaimer",
         "Vastuuvapauslauseke"),
    "threesea-la-disclaimer-ai":
        ("Threesea was developed with Claude Code (Anthropic) and audited with OpenAI Codex, directed and tested by a human maintainer. It is a research and experimentation project.",
         "Threesea on kehitetty Claude Codella (Anthropic) ja auditoitu OpenAI Codexilla ihmisylläpitäjän ohjauksessa ja testaamana. Se on tutkimus- ja kokeiluprojekti."),
    "threesea-la-disclaimer-warranty":
        ("AI makes mistakes. The code, including its security-relevant parts, can contain errors nobody noticed. The developers give no warranty of any kind; you use the app at your own risk.",
         "Tekoäly tekee virheitä. Koodi, myös sen tietoturvan kannalta olennaiset osat, voi sisältää virheitä, joita kukaan ei ole huomannut. Kehittäjät eivät anna minkäänlaista takuuta; käytät sovellusta omalla vastuullasi."),
    "threesea-la-disclaimer-threema":
        ("Threesea is based on Threema Libre's open-source code and tries to play fair with Threema's APIs and licensing. Threema told us it doesn't actively block third-party apps that cause no stability issues or high server load, are clearly marked as unofficial without using its branding, and don't circumvent the licensing. Threesea aims to meet that. This is not a permission, and Threesea is not affiliated with, endorsed or supported by Threema AG. As with any Threema app, keep a backup of your ID.",
         "Threesea perustuu Threema Libren avoimeen lähdekoodiin ja pyrkii toimimaan reilusti Threeman rajapintojen ja lisensoinnin suhteen. Threema on kertonut meille, ettei se aktiivisesti estä kolmannen osapuolen sovelluksia, jotka eivät aiheuta vakausongelmia tai suurta palvelinkuormaa, on selvästi merkitty epävirallisiksi käyttämättä sen tuotemerkkiä eivätkä kierrä lisensointia. Threesea pyrkii täyttämään nämä ehdot. Tämä ei ole lupa, eikä Threesea ole Threema AG:n kanssa sidoksissa eikä sen hyväksymä tai tukema. Kuten minkä tahansa Threema-sovelluksen kanssa, pidä tunnuksestasi varmuuskopio."),
    "threesea-se-credits":
        ("Credits",
         "Kiitokset"),
    "threesea-la-credit-threema":
        ("Threema AG: threema-android and libthreema (AGPL-3.0), the protocols of Threema, Threema for Desktop and Threema Web.",
         "Threema AG: threema-android ja libthreema (AGPL-3.0), Threeman, Threema Desktopin ja Threema Webin protokollat."),
    "threesea-la-credit-opus":
        ("opus-pure by Stephen Berry, from Rust ports of libopus by the Xiph.Org Foundation and others (BSD-3-Clause): voice messages.",
         "opus-pure, tekijä Stephen Berry, pohjana Xiph.Org Foundationin ja muiden libopus-kirjaston Rust-käännökset (BSD-3-Clause): ääniviestit."),
    "threesea-la-credit-webrtc":
        ("webrtc-audio-processing, from Google's WebRTC (BSD-3-Clause): echo cancellation in calls.",
         "webrtc-audio-processing, pohjana Googlen WebRTC (BSD-3-Clause): kaiunpoisto puheluissa."),
    "threesea-la-credit-jolla":
        ("Jolla and Sailfish OS: Silica and its icons; Jolla's Messages, People, Settings and Gallery as the model for the design.",
         "Jolla ja Sailfish OS: Silica ja sen kuvakkeet; Jollan Viestit, Henkilöt, Asetukset ja Galleria ulkoasun esikuvana."),
    "threesea-la-credit-whisperfish":
        ("Whisperfish, the Signal client for Sailfish OS: a reference for building such an app.",
         "Whisperfish, Sailfish OS:n Signal-sovellus: esimerkki tällaisen sovelluksen rakentamisesta."),
    "threesea-la-credit-libraries":
        ("SQLCipher, GStreamer, Qt and many Rust libraries (tokio, rustls, reqwest, rusqlite, RustCrypto and others).",
         "SQLCipher, GStreamer, Qt ja monet Rust-kirjastot (tokio, rustls, reqwest, rusqlite, RustCrypto ja muut)."),
    "threesea-la-credit-eu-icon":
        ("The “AI generated” label is the European Commission's icon for AI-generated content.",
         "”AI generated” -merkintä on Euroopan komission kuvake tekoälyn tuottamalle sisällölle."),
    "threesea-la-update-notification":
        ("Threesea %1 is available",
         "Threesea %1 on saatavilla"),
    "threesea-la-update-notification-body":
        ("Tap for details and the download page.",
         "Napauta nähdäksesi tiedot ja lataussivun."),
    "threesea-me-check-updates-now":
        ("Check for updates now",
         "Tarkista päivitykset nyt"),
    "threesea-la-keep-running-description-full":
        ("Threesea starts when you turn on the phone and stays connected after you close it, so new messages arrive right away. Uses more battery.",
         "Threesea käynnistyy, kun käynnistät puhelimen, ja pysyy yhteydessä sen sulkemisen jälkeen, joten uudet viestit saapuvat heti. Kuluttaa enemmän akkua."),
    "threesea-se-updates":
        ("Updates",
         "Päivitykset"),
    "threesea-la-update-check":
        ("Check for updates",
         "Päivitystarkistus"),
    "threesea-la-update-check-description":
        ("Once a day, Threesea asks GitHub whether a newer version has been released, and tells you. GitHub sees your IP address; nothing else about you is sent.",
         "Kerran päivässä Threesea kysyy GitHubilta, onko uudempi versio julkaistu, ja kertoo sinulle. GitHub näkee IP-osoitteesi; muuta tietoa sinusta ei lähetetä."),
    "threesea-la-update-open":
        ("Open download page",
         "Avaa lataussivu"),
    "threesea-he-update-ask-accept":
        ("Check",
         "Tarkista"),
    "threesea-he-update-ask-cancel":
        ("Don't check",
         "Älä tarkista"),
    "threesea-la-update-ask-title":
        ("Check for updates?",
         "Tarkistetaanko päivitykset?"),
    "threesea-la-update-ask-text":
        ("This version of Threesea comes from GitHub, so it isn't updated by a store. Once a day, Threesea can ask GitHub whether a newer version has been released and tell you; you install it yourself. GitHub sees your IP address; nothing else about you is sent. You can change this in Settings, Advanced.",
         "Tämä Threesea-versio on peräisin GitHubista, joten mikään sovelluskauppa ei päivitä sitä. Threesea voi kerran päivässä kysyä GitHubista, onko uudempi versio julkaistu, ja kertoa siitä sinulle; asennat sen itse. GitHub näkee IP-osoitteesi; muuta sinusta ei lähetetä. Voit muuttaa tätä kohdassa Asetukset, Lisäasetukset."),
    "threesea-la-update-newest":
        ("Newest version",
         "Uusin versio"),
    "threesea-la-update-installed":
        ("Installed",
         "Asennettu"),
    "threesea-la-update-checked":
        ("Last checked",
         "Tarkistettu viimeksi"),
    "threesea-la-n-contacts":
        ("%n contact(s)",
         ("%n yhteystieto",
          "%n yhteystietoa")),
    "threesea-la-n-groups":
        ("%n group(s)",
         ("%n ryhmä",
          "%n ryhmää")),
    "threesea-la-n-lists":
        ("%n distribution list(s)",
         ("%n jakelulista",
          "%n jakelulistaa")),
    "threesea-me-safe-retry-import":
        ("Import the rest now",
         "Tuo loput nyt"),
    "threesea-se-safe-restore":
        ("Restore",
         "Palautus"),
    "threesea-la-safe-to-import":
        ("Waiting to import",
         "Odottaa tuontia"),
    "threesea-la-safe-import-attempt":
        ("Import attempt",
         "Tuontiyritys"),
    "threesea-la-safe-skipped-deleted":
        ("Deleted groups",
         "Poistetut ryhmät"),
    "threesea-la-safe-skipped-creator":
        ("Creator not available",
         "Luoja ei saatavilla"),
    "threesea-la-safe-skipped-invalid":
        ("Unreadable entries",
         "Lukukelvottomat merkinnät"),
    "threesea-la-safe-revoked":
        ("No longer registered",
         "Ei enää rekisteröity"),
    "threesea-la-error-backup-version":
        ("This backup's format is not supported.",
         "Tämän varmuuskopion muotoa ei tueta."),
    "threesea-la-error-backup-invalid":
        ("The file could not be read as a Threema backup.",
         "Tiedostoa ei voitu lukea Threema-varmuuskopiona."),
    "threesea-la-android-backup-file":
        ("Backup file",
         "Varmuuskopiotiedosto"),
    "threesea-la-select":
        ("Select",
         "Valitse"),
    "threesea-la-android-backup-password":
        ("Backup password",
         "Varmuuskopion salasana"),
    "threesea-bu-android-backup-check":
        ("Check backup",
         "Tarkista varmuuskopio"),
    "threesea-se-android-backup-contents":
        ("Contents",
         "Sisältö"),
    "threesea-la-android-backup-identity":
        ("Threema ID",
         "Threema-tunnus"),
    "threesea-la-android-backup-id-match":
        ("This device's Threema ID",
         "Tämän laitteen Threema-tunnus"),
    "threesea-la-android-backup-id-other":
        ("Another Threema ID",
         "Toinen Threema-tunnus"),
    "threesea-la-android-backup-id-key":
        ("Same ID, different key",
         "Sama tunnus, eri avain"),
    "threesea-la-android-backup-id-unreadable":
        ("Not readable with this password",
         "Ei luettavissa tällä salasanalla"),
    "threesea-la-android-backup-id-missing":
        ("Not in the backup",
         "Ei varmuuskopiossa"),
    "threesea-la-android-backup-chats":
        ("Chats",
         "Keskustelut"),
    "threesea-la-android-backup-group-chats":
        ("Group chats",
         "Ryhmäkeskustelut"),
    "threesea-la-android-backup-lists":
        ("Distribution lists",
         "Jakelulistat"),
    "threesea-la-android-backup-media":
        ("Media files",
         "Mediatiedostot"),
    "threesea-la-android-backup-pictures":
        ("Pictures",
         "Kuvat"),
    "threesea-la-error-backup-identity":
        ("This backup belongs to a different Threema ID or key.",
         "Tämä varmuuskopio kuuluu eri Threema-tunnukselle tai avaimelle."),
    "threesea-la-android-backup-imported-contacts":
        ("Contacts",
         "Yhteystiedot"),
    "threesea-la-android-backup-imported-groups":
        ("Groups",
         "Ryhmät"),
    "threesea-la-android-backup-existing":
        ("Already on this device",
         "Jo tällä laitteella"),
    "threesea-la-android-import-chats":
        ("Chats %1 of %2",
         "Keskustelut %1/%2"),
    "threesea-bu-android-backup-stop":
        ("Stop",
         "Pysäytä"),
    "threesea-se-android-backup-history":
        ("Message history",
         "Viestihistoria"),
    "threesea-la-android-import-messages":
        ("Messages",
         "Viestit"),
    "threesea-la-android-import-reactions":
        ("Reactions",
         "Reaktiot"),
    "threesea-se-android-backup-create":
        ("Create backups",
         "Luo varmuuskopioita"),
    "threesea-la-android-export-folder":
        ("Backup folder",
         "Varmuuskopiokansio"),
    "threesea-la-android-export-password-saved":
        ("Password saved",
         "Salasana tallennettu"),
    "threesea-la-android-export-auto":
        ("Automatic backup",
         "Automaattinen varmuuskopio"),
    "threesea-la-android-export-off":
        ("Off",
         "Pois"),
    "threesea-la-android-export-daily":
        ("Daily",
         "Päivittäin"),
    "threesea-la-android-export-weekly":
        ("Weekly",
         "Viikoittain"),
    "threesea-la-android-export-keep":
        ("Keep",
         "Säilytä"),
    "threesea-la-n-backups":
        ("%n backup(s)",
         ("%n varmuuskopio",
          "%n varmuuskopiota")),
    "threesea-bu-android-export-now":
        ("Create backup now",
         "Luo varmuuskopio nyt"),
    "threesea-la-android-export-written":
        ("Backup written: %1",
         "Varmuuskopio kirjoitettu: %1"),
    "threesea-la-android-import-polls":
        ("Polls",
         "Kyselyt"),
    "threesea-he-import":
        ("Import",
         "Tuo"),
    "threesea-la-android-backup-import-duration":
        ("A large backup can take a while. You can stop the import; importing again continues where it stopped.",
         "Suuren varmuuskopion tuonti voi kestää jonkin aikaa. Voit pysäyttää tuonnin; kun tuot uudelleen, tuonti jatkuu siitä, mihin se jäi."),
    "threesea-he-importing":
        ("Importing",
         "Tuodaan"),
    "threesea-la-android-import-stopping":
        ("Stopping…",
         "Pysäytetään…"),
    "threesea-la-android-import-step-metadata":
        ("Contacts, groups and pictures",
         "Yhteystiedot, ryhmät ja kuvat"),
    "threesea-la-android-import-step-reading":
        ("Reading the backup",
         "Luetaan varmuuskopiota"),
    "threesea-la-android-import-step-history":
        ("Messages and media",
         "Viestit ja media"),
    "threesea-la-android-import-chat-media":
        ("Chat %1 of %2, media %3 of %4",
         "Keskustelu %1/%2, media %3/%4"),
    "threesea-la-android-import-keep-open":
        ("Keep the app open until the import is done. What is imported stays; importing again continues.",
         "Pidä sovellus auki, kunnes tuonti on valmis. Jo tuotu säilyy; uusi tuonti jatkaa siitä."),
    "threesea-he-data-backup":
        ("Data backup",
         "Tietojen varmuuskopio"),
    "threesea-se-data-backup-import":
        ("Import a backup",
         "Tuo varmuuskopio"),
    "threesea-la-data-backup-import-description":
        ("Checking a backup only reads the file and shows what it holds. The password never leaves this device.",
         "Varmuuskopion tarkistus vain lukee tiedoston ja näyttää, mitä se sisältää. Salasana ei koskaan poistu tältä laitteelta."),
    "threesea-la-data-backup-create-description":
        ("Writes a backup into the chosen folder, now or on a schedule. The password is kept in the encrypted storage so automatic backups can run.",
         "Kirjoittaa varmuuskopion valittuun kansioon nyt tai ajastetusti. Salasana säilytetään salatussa tallennustilassa, jotta automaattiset varmuuskopiot voivat toimia."),
    "threesea-he-import-failed":
        ("Import failed",
         "Tuonti epäonnistui"),
    "threesea-he-import-stopped":
        ("Import stopped",
         "Tuonti pysäytetty"),
    "threesea-he-import-finished":
        ("Import finished",
         "Tuonti valmis"),
    "threesea-la-data-backup-resume-hint":
        ("What is imported stays. Import the backup again to continue where it stopped.",
         "Jo tuotu säilyy. Tuo varmuuskopio uudelleen, niin tuonti jatkuu siitä, mihin se jäi."),
    "threesea-la-data-backup-imported":
        ("The backup is imported. Everything already on this device stayed as it was.",
         "Varmuuskopio on tuotu. Kaikki, mitä tällä laitteella jo oli, säilyi ennallaan."),
    "threesea-se-import-contacts-groups":
        ("Contacts and groups",
         "Yhteystiedot ja ryhmät"),
    "threesea-la-android-import-media-failed":
        ("Media not imported",
         "Mediaa ei tuotu"),
    "threesea-la-forward-secrecy":
        ("Perfect Forward Secrecy",
         "Perfect Forward Secrecy"),
    "threesea-la-forward-secrecy-description":
        ("Protects chats with keys that change continuously. Messages are end-to-end encrypted either way; contacts see a security notice when the protection starts, resets or is turned off.",
         "Suojaa keskustelut jatkuvasti vaihtuvilla avaimilla. Viestit ovat päästä päähän salattuja joka tapauksessa; yhteystiedot näkevät turvallisuusilmoituksen, kun suojaus alkaa, nollautuu tai poistetaan käytöstä."),
    "threesea-la-view-log":
        ("View log",
         "Näytä loki"),
    "threesea-me-log-clear":
        ("Clear log",
         "Tyhjennä loki"),
    "threesea-la-clearing-log":
        ("Clearing log",
         "Tyhjennetään lokia"),
    "threesea-me-log-save":
        ("Save support package",
         "Tallenna tukipaketti"),
    "threesea-me-log-refresh":
        ("Refresh",
         "Päivitä"),
    "threesea-he-log":
        ("Log",
         "Loki"),
    "threesea-la-log-empty":
        ("No entries",
         "Ei merkintöjä"),
    "threesea-la-log-empty-hint":
        ("What Threesea logs appears here. Settings → Advanced sets how much that is.",
         "Threesean lokiin kirjaamat tiedot näkyvät tässä. Kohdassa Asetukset → Lisäasetukset määritetään, kuinka paljon niitä on."),
    "threesea-se-encryption":
        ("Encryption",
         "Salaus"),
    "threesea-la-device-cookie-title":
        ("Threema ID used on another device",
         "Threema-tunnusta käytetty toisella laitteella"),
    "threesea-la-device-cookie-body":
        ("Threema's server noticed a connection from another device with your Threema ID. If you recently used your ID on another phone, you can ignore this. An ID works on one phone at a time: new messages go only to the phone that connected last. If you didn't, your private key may be compromised: protect this phone and its data, and create a new ID.",
         "Threeman palvelin havaitsi yhteyden toisesta laitteesta Threema-tunnuksellasi. Jos käytit tunnustasi äskettäin toisessa puhelimessa, voit jättää tämän huomiotta. Tunnus toimii yhdessä puhelimessa kerrallaan: uudet viestit menevät vain siihen puhelimeen, joka muodosti yhteyden viimeksi. Jos et käyttänyt, yksityinen avaimesi voi olla vaarantunut: suojaa tämä puhelin ja sen tiedot ja luo uusi tunnus."),
    "threesea-me-delete-for-me":
        ("Delete for me",
         "Poista minulta"),
    "threesea-me-delete-for-everyone":
        ("Delete for everyone",
         "Poista kaikilta"),
    "threesea-la-playback-speed":
        ("%1×",
         "%1×"),
    "threesea-la-ear-to-play":
        ("Play voice messages at the ear",
         "Toista ääniviestit korvalla"),
    "threesea-la-ear-to-play-description":
        ("Hold the phone to your ear and the voice message plays on the earpiece, like a call. Take it away to pause.",
         "Nosta puhelin korvallesi, niin ääniviesti toistuu kuulokkeesta kuten puhelussa. Siirrä puhelin pois, niin toisto keskeytyy."),
    "threesea-se-notifications-inapp":
        ("In the app",
         "Sovelluksessa"),
    "threesea-la-inapp-sound-description":
        ("A soft sound for a message arriving in the chat you are reading, and between two voice messages playing one after the other.",
         "Pehmeä ääni, kun luettavaan keskusteluun saapuu viesti, sekä peräkkäin toistettavien ääniviestien välissä."),
    "threesea-la-inapp-vibrate-description":
        ("A short vibration for a message arriving in the chat you are reading.",
         "Lyhyt värinä, kun luettavaan keskusteluun saapuu viesti."),
    "threesea-me-mark-unread":
        ("Mark as unread",
         "Merkitse lukemattomaksi"),
    "threesea-me-wallpaper-clear":
        ("Use the Ambience's background",
         "Käytä ambienssin taustakuvaa"),
    "threesea-me-font-size-small":
        ("Small",
         "Pieni"),
    "threesea-me-font-size-normal":
        ("Normal",
         "Normaali"),
    "threesea-me-font-size-large":
        ("Large",
         "Suuri"),
    "threesea-la-chat-font-size":
        ("Message text",
         "Viestin teksti"),
    "threesea-la-wallpaper":
        ("Chat background",
         "Keskustelun tausta"),
    "threesea-la-wallpaper-ambience":
        ("The Ambience's background",
         "Ambienssin taustakuva"),
    "threesea-la-receive-pictures":
        ("Show profile pictures",
         "Näytä profiilikuvat"),
    "threesea-la-receive-pictures-description":
        ("Contacts who share their profile picture are shown with it. Pictures they send keep arriving while this is off, and appear when it is turned on again.",
         "Profiilikuvansa jakavat yhteystiedot näytetään kuvan kanssa. Heidän lähettämänsä kuvat saapuvat edelleen, vaikka tämä on pois päältä, ja ne tulevat näkyviin, kun asetus otetaan taas käyttöön."),
    "threesea-la-group-calls":
        ("Group calls",
         "Ryhmäpuhelut"),
    "threesea-la-group-calls-description":
        ("You can start group calls and join the ones running in your groups.",
         "Voit aloittaa ryhmäpuheluita ja liittyä ryhmissäsi käynnissä oleviin puheluihin."),
    "threesea-he-rename-device":
        ("Rename device",
         "Nimeä laite uudelleen"),
    "threesea-la-device-name":
        ("Device name",
         "Laitteen nimi"),
    "threesea-la-device-name-own-description":
        ("The other devices show this name for this phone.",
         "Muut laitteet näyttävät tämän nimen tälle puhelimelle."),
    "threesea-la-device-name-alias-description":
        ("Only this phone shows this name; the device keeps its own name everywhere else.",
         "Vain tämä puhelin näyttää tämän nimen; muualla laite säilyttää oman nimensä."),
    "threesea-me-rename-device":
        ("Rename",
         "Nimeä uudelleen"),
    "threesea-me-keep-forever":
        ("For good",
         "Pysyvästi"),
    "threesea-me-keep-1-year":
        ("1 year",
         "1 vuosi"),
    "threesea-me-keep-6-months":
        ("6 months",
         "6 kuukautta"),
    "threesea-me-keep-3-months":
        ("3 months",
         "3 kuukautta"),
    "threesea-me-keep-1-month":
        ("1 month",
         "1 kuukausi"),
    "threesea-me-keep-1-week":
        ("1 week",
         "1 viikko"),
    "threesea-me-video-small":
        ("Small (480 px)",
         "Pieni (480 px)"),
    "threesea-me-video-medium":
        ("Medium (848 px)",
         "Keskikokoinen (848 px)"),
    "threesea-la-video-size":
        ("Video size",
         "Videon koko"),
    "threesea-la-video-size-description":
        ("Smaller videos are quicker to send and use less data. The sound is kept as it is.",
         "Pienemmät videot lähtevät nopeammin ja kuluttavat vähemmän dataa. Ääni säilyy ennallaan."),
    "threesea-la-keep-messages":
        ("Keep messages",
         "Säilytä viestit"),
    "threesea-la-keep-messages-description":
        ("Messages older than the chosen time are deleted by themselves, with their media. Starred messages, open polls, messages still on their way and groups with only you in them stay.",
         "Valittua aikaa vanhemmat viestit poistetaan itsestään medioineen. Tähdellä merkityt viestit, avoimet kyselyt, vielä matkalla olevat viestit ja ryhmät, joissa olet ainoa jäsen, säilyvät."),
    "threesea-la-exporting-chat":
        ("Exporting chat",
         "Viedään keskustelua"),
    "threesea-me-export-chat":
        ("Export chat",
         "Vie keskustelu"),
    "threesea-he-export":
        ("Export",
         "Vie"),
    "threesea-he-export-chat":
        ("Export chat",
         "Vie keskustelu"),
    "threesea-la-export-chat-description":
        ("The messages of %1 are written as a text file into a ZIP archive in your Downloads folder.",
         "Keskustelun %1 viestit kirjoitetaan tekstitiedostona ZIP-arkistoon Lataukset-kansioosi."),
    "threesea-la-export-chat-media":
        ("Include pictures and files",
         "Sisällytä kuvat ja tiedostot"),
    "threesea-la-export-chat-media-description":
        ("Everything that has been downloaded to this phone goes into the archive.",
         "Kaikki tähän puhelimeen ladattu tallentuu arkistoon."),
    "threesea-me-limit-any":
        ("Any size",
         "Mikä tahansa koko"),
    "threesea-me-limit-1":
        ("Up to 1 MB",
         "Enintään 1 Mt"),
    "threesea-me-limit-5":
        ("Up to 5 MB",
         "Enintään 5 Mt"),
    "threesea-me-limit-20":
        ("Up to 20 MB",
         "Enintään 20 Mt"),
    "threesea-me-limit-100":
        ("Up to 100 MB",
         "Enintään 100 Mt"),
    "threesea-la-download-limit":
        ("File size",
         "Tiedostokoko"),
    "threesea-la-download-limit-description":
        ("Larger files wait in the chat until you tap them. Previews always come.",
         "Suuremmat tiedostot odottavat keskustelussa, kunnes napautat niitä. Esikatselut tulevat aina."),
    "threesea-me-photo-turn-right":
        ("Turn right",
         "Käännä oikealle"),
    "threesea-me-photo-turn-left":
        ("Turn left",
         "Käännä vasemmalle"),
    "threesea-me-photo-whole":
        ("Whole picture",
         "Koko kuva"),
    "threesea-he-photo-use":
        ("Use",
         "Käytä"),
    "threesea-he-edit-photo":
        ("Edit photo",
         "Muokkaa kuvaa"),
    "threesea-la-edit-photo-failed":
        ("The photo could not be edited.",
         "Kuvaa ei voitu muokata."),
    "threesea-la-license-notification":
        ("Threesea is offline",
         "Threesea on offline-tilassa"),
    "threesea-la-license-notification-body":
        ("Threema no longer accepts this license key. Open Threesea to check it.",
         "Threema ei enää hyväksy tätä lisenssiavainta. Avaa Threesea tarkistaaksesi sen."),
    "threesea-he-license-key":
        ("License key",
         "Lisenssiavain"),
    "threesea-la-license-refused":
        ("Threesea stays offline until a license key is accepted. Your chats stay on this device.",
         "Threesea pysyy offline-tilassa, kunnes lisenssiavain hyväksytään. Keskustelusi säilyvät tällä laitteella."),
    "threesea-la-spam-reported":
        ("Reported to Threema as spam.",
         "Ilmoitettu Threemalle roskaviestiksi."),
    "threesea-la-spam-unknown-group":
        ("This group is from an unknown contact",
         "Tämä ryhmä on tuntemattomalta yhteystiedolta"),
    "threesea-la-spam-unknown-contact":
        ("This chat is from an unknown contact",
         "Tämä keskustelu on tuntemattomalta yhteystiedolta"),
    "threesea-la-spam-tap-to-report":
        ("Tap to report spam",
         "Napauta ilmoittaaksesi roskaviestistä"),
    "threesea-he-spam-report":
        ("Report",
         "Ilmoita"),
    "threesea-he-spam-report-name":
        ("Report %1",
         "Ilmoita %1"),
    "threesea-la-spam-explain-group":
        ("This sends your Threema ID to Threema, with the group's ID and number of members and the ID and nickname of its creator. No messages and not the group's name.",
         "Tämä lähettää Threemalle Threema-tunnuksesi sekä ryhmän tunnuksen ja jäsenmäärän ja sen luojan tunnuksen ja lempinimen. Ei viestejä eikä ryhmän nimeä."),
    "threesea-la-spam-explain-contact":
        ("This sends your Threema ID to Threema, with the ID and nickname of this contact. No messages.",
         "Tämä lähettää Threemalle Threema-tunnuksesi sekä tämän yhteystiedon tunnuksen ja lempinimen. Ei viestejä."),
    "threesea-la-spam-block-group":
        ("Also block the creator, leave the group and delete the chat",
         "Estä myös luoja, poistu ryhmästä ja poista keskustelu"),
    "threesea-la-spam-block-contact":
        ("Also block this contact and delete the chat",
         "Estä myös tämä yhteystieto ja poista keskustelu"),
    "threesea-la-spam-report-failed":
        ("The report could not be sent.",
         "Ilmoitusta ei voitu lähettää."),
    "threesea-la-address-book-none-new":
        ("%n address book entries checked, no new contacts.",
         ("%n osoitekirjan merkintä tarkistettu, ei uusia yhteystietoja.",
          "%n osoitekirjan merkintää tarkistettu, ei uusia yhteystietoja.")),
    "threesea-la-address-book-empty":
        ("No phone numbers or e-mail addresses found in the address book.",
         "Osoitekirjasta ei löytynyt puhelinnumeroita eikä sähköpostiosoitteita."),
    "threesea-la-link-mobile-target":
        ("The code goes to +%1.",
         "Koodi lähetetään numeroon +%1."),
    "threesea-la-email-already-linked":
        ("This address is already linked to your ID, so no e-mail is needed.",
         "Tämä osoite on jo liitetty tunnukseesi, joten sähköpostia ei tarvita."),
    "threesea-la-phone-already-linked":
        ("This number is already linked to your ID, so no SMS is needed.",
         "Tämä numero on jo liitetty tunnukseesi, joten tekstiviestiä ei tarvita."),
    "threesea-la-country-ae":
        ("United Arab Emirates",
         "Arabiemiirikunnat"),
    "threesea-la-country-at":
        ("Austria",
         "Itävalta"),
    "threesea-la-country-au":
        ("Australia",
         "Australia"),
    "threesea-la-country-be":
        ("Belgium",
         "Belgia"),
    "threesea-la-country-bg":
        ("Bulgaria",
         "Bulgaria"),
    "threesea-la-country-br":
        ("Brazil",
         "Brasilia"),
    "threesea-la-country-ca":
        ("Canada",
         "Kanada"),
    "threesea-la-country-ch":
        ("Switzerland",
         "Sveitsi"),
    "threesea-la-country-cn":
        ("China",
         "Kiina"),
    "threesea-la-country-cy":
        ("Cyprus",
         "Kypros"),
    "threesea-la-country-cz":
        ("Czechia",
         "Tšekki"),
    "threesea-la-country-de":
        ("Germany",
         "Saksa"),
    "threesea-la-country-dk":
        ("Denmark",
         "Tanska"),
    "threesea-la-country-ee":
        ("Estonia",
         "Viro"),
    "threesea-la-country-es":
        ("Spain",
         "Espanja"),
    "threesea-la-country-fi":
        ("Finland",
         "Suomi"),
    "threesea-la-country-fr":
        ("France",
         "Ranska"),
    "threesea-la-country-gb":
        ("United Kingdom",
         "Yhdistynyt kuningaskunta"),
    "threesea-la-country-gr":
        ("Greece",
         "Kreikka"),
    "threesea-la-country-hr":
        ("Croatia",
         "Kroatia"),
    "threesea-la-country-hu":
        ("Hungary",
         "Unkari"),
    "threesea-la-country-ie":
        ("Ireland",
         "Irlanti"),
    "threesea-la-country-il":
        ("Israel",
         "Israel"),
    "threesea-la-country-in":
        ("India",
         "Intia"),
    "threesea-la-country-is":
        ("Iceland",
         "Islanti"),
    "threesea-la-country-it":
        ("Italy",
         "Italia"),
    "threesea-la-country-jp":
        ("Japan",
         "Japani"),
    "threesea-la-country-kr":
        ("South Korea",
         "Etelä-Korea"),
    "threesea-la-country-li":
        ("Liechtenstein",
         "Liechtenstein"),
    "threesea-la-country-lu":
        ("Luxembourg",
         "Luxemburg"),
    "threesea-la-country-lv":
        ("Latvia",
         "Latvia"),
    "threesea-la-country-mt":
        ("Malta",
         "Malta"),
    "threesea-la-country-mx":
        ("Mexico",
         "Meksiko"),
    "threesea-la-country-nl":
        ("Netherlands",
         "Alankomaat"),
    "threesea-la-country-no":
        ("Norway",
         "Norja"),
    "threesea-la-country-nz":
        ("New Zealand",
         "Uusi-Seelanti"),
    "threesea-la-country-pl":
        ("Poland",
         "Puola"),
    "threesea-la-country-pt":
        ("Portugal",
         "Portugali"),
    "threesea-la-country-ro":
        ("Romania",
         "Romania"),
    "threesea-la-country-rs":
        ("Serbia",
         "Serbia"),
    "threesea-la-country-ru":
        ("Russia",
         "Venäjä"),
    "threesea-la-country-se":
        ("Sweden",
         "Ruotsi"),
    "threesea-la-country-sg":
        ("Singapore",
         "Singapore"),
    "threesea-la-country-si":
        ("Slovenia",
         "Slovenia"),
    "threesea-la-country-sk":
        ("Slovakia",
         "Slovakia"),
    "threesea-la-country-tr":
        ("Turkey",
         "Turkki"),
    "threesea-la-country-ua":
        ("Ukraine",
         "Ukraina"),
    "threesea-la-country-us":
        ("United States",
         "Yhdysvallat"),
    "threesea-la-country-za":
        ("South Africa",
         "Etelä-Afrikka"),
    "threesea-la-address-book-country-automatic":
        ("Automatic",
         "Automaattinen"),
    "threesea-la-address-book-country-automatic-code":
        ("Automatic (+%1)",
         "Automaattinen (+%1)"),
    "threesea-la-address-book-country":
        ("Numbers without a country code",
         "Numerot ilman maatunnusta"),
    "threesea-la-address-book-country-description":
        ("Address book numbers written without a country code count as from this country. Automatic goes by your linked phone number, else by the device's region. Numbers from other countries are found when written with + and their country code.",
         "Osoitekirjan numerot, joista puuttuu maatunnus, tulkitaan tämän maan numeroiksi. Automaattinen valinta määräytyy yhdistetyn puhelinnumerosi mukaan, muuten laitteen alueen mukaan. Muiden maiden numerot löytyvät, kun ne on kirjoitettu +-merkin ja maatunnuksen kanssa."),
    "threesea-la-no-emoji-found":
        ("No emoji found",
         "Emojeja ei löytynyt"),
    "threesea-la-upgrading-hint":
        ("Updating the stored chats. This can take a moment; keep Threesea open.",
         "Tallennettuja keskusteluja päivitetään. Tämä voi kestää hetken; pidä Threesea auki."),
    "threesea-la-error-text-too-long":
        ("The text is too long. Shorten it and try again.",
         "Teksti on liian pitkä. Lyhennä sitä ja yritä uudelleen."),
    "threesea-la-error-poll-results-too-long":
        ("The results of this poll are too long to send. The poll stays open.",
         "Tämän kyselyn tulokset ovat liian pitkät lähetettäviksi. Kysely pysyy avoinna."),
    "threesea-la-error-backup-no-identity":
        ("This backup holds no Threema ID. Restore the ID from Threema Safe or an ID backup, then import this backup in Settings, Backup, Data backup.",
         "Tässä varmuuskopiossa ei ole Threema-tunnusta. Palauta tunnus Threema Safesta tai tunnuksen varmuuskopiosta ja tuo sitten tämä varmuuskopio kohdassa Asetukset, Varmuuskopio, Tietojen varmuuskopio."),
    "threesea-la-error-backup-write":
        ("The backup could not be written. Check the backup folder.",
         "Varmuuskopiota ei voitu kirjoittaa. Tarkista varmuuskopiokansio."),
    "threesea-la-error-backup-no-room":
        ("The backup folder's storage has too little free space for a backup. Free some space or choose another folder.",
         "Varmuuskopiokansion tallennustilassa ei ole tarpeeksi vapaata tilaa varmuuskopiolle. Vapauta tilaa tai valitse toinen kansio."),
    "threesea-la-error-backup-running":
        ("A backup is being written right now.",
         "Varmuuskopiota kirjoitetaan parhaillaan."),
    "threesea-la-error-backup-no-folder":
        ("Choose a backup folder first.",
         "Valitse ensin varmuuskopiokansio."),
    "threesea-la-error-backup-no-password":
        ("Enter a backup password first.",
         "Syötä ensin varmuuskopion salasana."),
    "threesea-la-error-safe-damaged":
        ("This Threema Safe backup is damaged and can't be read.",
         "Tämä Threema Safe -varmuuskopio on vioittunut, eikä sitä voi lukea."),
    "threesea-la-error-contact-revoked":
        ("This Threema ID was revoked.",
         "Tämä Threema-tunnus on mitätöity."),
    "threesea-la-error-contact-unknown":
        ("Block unknown senders is on, and this person is not one of your contacts.",
         "Tuntemattomien lähettäjien esto on käytössä, eikä tämä henkilö ole yhteystiedoissasi."),
    "threesea-la-error-qr-expired":
        ("This code has expired. Scan it directly from the other person's app.",
         "Tämä koodi on vanhentunut. Skannaa se suoraan toisen henkilön sovelluksesta."),
    "threesea-la-error-video-metadata":
        ("A video was not sent, because its metadata, which can hold where and when it was made, could not be removed. It can still be sent as a file.",
         "Videota ei lähetetty, koska sen metatietoja, joista voi selvitä missä ja milloin se on tehty, ei voitu poistaa. Sen voi silti lähettää tiedostona."),
    "threesea-la-error-file-picture-metadata":
        ("A picture was not sent, because its metadata, which can hold where and when it was made, could not be removed. It can still be sent as a file.",
         "Kuvaa ei lähetetty, koska sen metatietoja, joista voi selvitä missä ja milloin se on otettu, ei voitu poistaa. Sen voi silti lähettää tiedostona."),
    "threesea-la-error-picture-metadata":
        ("A picture was not sent, because its metadata, which can hold where and when it was made, could not be removed. Save it, and it can be sent as a file.",
         "Kuvaa ei lähetetty, koska sen metatietoja, joista voi selvitä missä ja milloin se on otettu, ei voitu poistaa. Tallenna se, niin sen voi lähettää tiedostona."),
    "threesea-la-error-not-web-code":
        ("This is not a Threema Web code. Scan the code Threema Web shows in the browser.",
         "Tämä ei ole Threema Web -koodi. Skannaa koodi, jonka Threema Web näyttää selaimessa."),
    "threesea-la-error-web-disabled":
        ("Threema Web is switched off. Turn it on in the settings under Threema Web.",
         "Threema Web on pois päältä. Ota se käyttöön asetuksista kohdasta Threema Web."),
    "threesea-la-error-web-already-paired":
        ("This browser is already in the list under Threema Web. Remove it there to connect it with a new code.",
         "Tämä selain on jo luettelossa kohdassa Threema Web. Poista se sieltä, jotta voit yhdistää sen uudella koodilla."),
    "threesea-la-error-link-removed":
        ("Linking didn't finish, so the devices in the device group were removed again. The computer had already received your ID. Close Threema for Desktop, open it again and scan its new code.",
         "Yhdistäminen ei valmistunut, joten laiteryhmän laitteet poistettiin taas. Tietokone oli jo saanut tunnuksesi. Sulje Threema Desktop, avaa se uudelleen ja skannaa sen uusi koodi."),
    "threesea-la-error-link-uncertain":
        ("Linking didn't finish, but the computer received your ID and may be linked anyway. Linked devices shows what is linked once you are online, and you can remove it there.",
         "Yhdistäminen ei valmistunut, mutta tietokone sai tunnuksesi ja saattaa silti olla yhdistetty. Yhdistetyt laitteet -kohta näyttää, mitä on yhdistetty, kun olet verkossa, ja voit poistaa laitteen sieltä."),
    "threesea-la-error-group-call-full":
        ("This group call is full.",
         "Tämä ryhmäpuhelu on täynnä."),
    "threesea-la-error-group-call-not-running":
        ("This group call has ended.",
         "Tämä ryhmäpuhelu on päättynyt."),
    "threesea-la-error-group-call-version":
        ("This group call needs a different version of the app.",
         "Tähän ryhmäpuheluun tarvitaan sovelluksen eri versio."),
    "threesea-la-error-group-call-connection":
        ("The connection to the group call failed.",
         "Yhteys ryhmäpuheluun epäonnistui."),
    "threesea-la-error-group-call-audio":
        ("The microphone or the speaker stopped working, so the group call ended.",
         "Mikrofoni tai kaiutin lakkasi toimimasta, joten ryhmäpuhelu päättyi."),
    "threesea-la-error-contact-not-found":
        ("Add this ID to your contacts to write to it.",
         "Lisää tämä tunnus yhteystietoihisi, jotta voit kirjoittaa sille."),
    "threesea-la-error-group-not-found":
        ("This group is no longer on this device.",
         "Tätä ryhmää ei ole enää tällä laitteella."),
    "threesea-la-error-no-identity":
        ("Create or restore your Threema ID first.",
         "Luo tai palauta ensin Threema-tunnuksesi."),
    "threesea-la-error-server-trouble":
        ("Threema's server didn't accept this. Try again later.",
         "Threeman palvelin ei hyväksynyt tätä. Yritä myöhemmin uudelleen."),
    "threesea-la-error-rate-limited":
        ("Threema's servers are busy. Try again in a few minutes.",
         "Threeman palvelimet ovat ruuhkautuneet. Yritä muutaman minuutin kuluttua uudelleen."),
    "threesea-la-error-server-unexplained":
        ("Threema's servers turned this down and gave no reason. Try again later.",
         "Threeman palvelimet hylkäsivät tämän ilmoittamatta syytä. Yritä myöhemmin uudelleen."),
    "threesea-la-error-report-refused":
        ("Threema didn't accept the report. Try again later.",
         "Threema ei hyväksynyt ilmoitusta. Yritä myöhemmin uudelleen."),
    "threesea-la-error-id-search-input":
        ("Enter a phone number with its country code, or an e-mail address.",
         "Syötä puhelinnumero maatunnuksineen tai sähköpostiosoite."),
    "threesea-la-error-identity-exists":
        ("This device already has a Threema ID.",
         "Tällä laitteella on jo Threema-tunnus."),
    "threesea-la-error-disk-full":
        ("The phone's storage is full. Free some space.",
         "Puhelimen tallennustila on täynnä. Vapauta tilaa."),
    "threesea-la-error-upgrade-no-room":
        ("Updating the stored chats needs more free space. Free some space; the chats stay as they were until then.",
         "Tallennettujen keskustelujen päivitys tarvitsee enemmän vapaata tilaa. Vapauta tilaa; keskustelut säilyvät ennallaan siihen asti."),
    "threesea-la-error-store-too-new":
        ("This version of Threesea is older than your stored data. Install the newest version.",
         "Tämä Threesean versio on tallennettuja tietojasi vanhempi. Asenna uusin versio."),
    "threesea-la-error-store-key":
        ("The stored data could not be opened: the key does not fit, or the file is damaged. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Tallennettuja tietoja ei voitu avata: avain ei sovi tai tiedosto on vioittunut. Threesea jättää tiedot ennalleen. Kannattaa sulkea Threesea ja avata se uudelleen ja sitten käynnistää puhelin uudelleen."),
    "threesea-la-error-store-damaged":
        ("Part of the stored data could not be read. It may be damaged. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Osaa tallennetuista tiedoista ei voitu lukea. Ne voivat olla vioittuneet. Threesea jättää tiedot ennalleen. Kannattaa sulkea Threesea ja avata se uudelleen ja sitten käynnistää puhelin uudelleen."),
    "threesea-la-error-store-unreadable":
        ("The stored data could not be read. Check that the phone has free storage.",
         "Tallennettuja tietoja ei voitu lukea. Tarkista, että puhelimessa on vapaata tallennustilaa."),
    "threesea-la-error-store":
        ("The stored data could not be opened.",
         "Tallennettuja tietoja ei voitu avata."),
    "threesea-la-error-key-missing":
        ("The key to the stored data was not found in the phone's key storage, so the data could not be opened. Threesea leaves the data as it is. It is worth closing Threesea and opening it again, then restarting the phone.",
         "Tallennettujen tietojen avainta ei löytynyt puhelimen avainsäilöstä, joten tietoja ei voitu avata. Threesea jättää tiedot ennalleen. Kannattaa sulkea Threesea ja avata se uudelleen ja sitten käynnistää puhelin uudelleen."),
    "threesea-la-error-secrets":
        ("Threesea could not get the key to its data from the phone's key storage. Allow the access if the phone asks for it.",
         "Threesea ei saanut tietojensa avainta puhelimen avainsäilöstä. Salli käyttö, jos puhelin kysyy sitä."),
    "threesea-la-error-internal-plain":
        ("Something went wrong.",
         "Jokin meni vikaan."),
    "threesea-la-gif":
        ("GIF",
         "GIF"),
    "threesea-la-lock-private-passphrase":
        ("Passphrase for private chats",
         "Yksityisten keskustelujen tunnuslause"),
    "threesea-la-lock-private-pin":
        ("PIN for private chats",
         "Yksityisten keskustelujen PIN-koodi"),
    "threesea-la-status-group-no-longer-member":
        ("You are no longer a member of this group.",
         "Et ole enää tämän ryhmän jäsen."),
    "threesea-la-status-group-call-started-unnamed":
        ("A group call started.",
         "Ryhmäpuhelu alkoi."),
    "threesea-la-status-group-call-started":
        ("%1 started a group call.",
         "%1 aloitti ryhmäpuhelun."),
    "threesea-la-status-group-call-you-started":
        ("You started a group call.",
         "Aloitit ryhmäpuhelun."),
    "threesea-la-status-group-call-ended":
        ("The group call ended.",
         "Ryhmäpuhelu päättyi."),
    "threesea-la-status-fs-session-reset":
        ("Perfect Forward Secrecy was reset. This can happen when the contact changes devices or reinstalls the app.",
         "Perfect Forward Secrecy nollattiin. Näin voi käydä, kun yhteystieto vaihtaa laitetta tai asentaa sovelluksen uudelleen."),
    "threesea-la-status-fs-message-rejected":
        ("Perfect Forward Secrecy was reset. Affected messages are sent again.",
         "Perfect Forward Secrecy nollattiin. Asianomaiset viestit lähetetään uudelleen."),
    "threesea-la-status-fs-message-without":
        ("A message without Perfect Forward Secrecy was received. Check with the contact that they turned it off on purpose.",
         "Vastaanotettiin viesti ilman Perfect Forward Secrecyä. Varmista yhteystiedolta, että hän on poistanut sen käytöstä tarkoituksella."),
    "threesea-la-status-fs-unavailable":
        ("Perfect Forward Secrecy is off in this chat because this contact's app is not using it at the moment.",
         "Perfect Forward Secrecy on pois käytöstä tässä keskustelussa, koska tämän yhteystiedon sovellus ei käytä sitä tällä hetkellä."),
    "threesea-la-status-fs-session-ended":
        ("The Perfect Forward Secrecy session with this contact ended.",
         "Perfect Forward Secrecy -istunto tämän yhteystiedon kanssa päättyi."),
    "threesea-me-copy-id":
        ("Copy ID",
         "Kopioi tunnus"),
    "threesea-la-copied":
        ("Copied",
         "Kopioitu"),
    "threesea-me-share-id":
        ("Share ID",
         "Jaa tunnus"),
    "threesea-he-share-id":
        ("Share ID",
         "Jaa tunnus"),
    "threesea-me-send-again":
        ("Send again",
         "Lähetä uudelleen"),
    "threesea-me-stop-download":
        ("Stop download",
         "Keskeytä lataus"),
    "threesea-me-stop-upload":
        ("Stop sending",
         "Keskeytä lähetys"),
    "threesea-me-share":
        ("Share",
         "Jaa"),
    "threesea-me-draw-on-picture":
        ("Draw on it",
         "Piirrä kuvaan"),
    "threesea-la-saved-n-downloads":
        ("Saved %n file(s) to Downloads",
         ("Tallennettiin %n tiedosto Latauksiin",
          "Tallennettiin %n tiedostoa Latauksiin")),
    "threesea-la-download-failed":
        ("The file could not be downloaded. It may no longer be on the server.",
         "Tiedostoa ei voitu ladata. Se ei ehkä ole enää palvelimella."),
    "threesea-la-send-in-turn-failed":
        ("Threesea did not send everything",
         "Threesea ei lähettänyt kaikkea"),
    "threesea-la-not-sent-to-chats":
        ("Not everything went to %n chat(s)",
         ("Kaikki ei mennyt perille %n keskusteluun",
          "Kaikki ei mennyt perille %n keskusteluun")),
    "threesea-la-sent-to-chats":
        ("On its way to %n chat(s)",
         ("Matkalla %n keskusteluun",
          "Matkalla %n keskusteluun")),
    "threesea-la-send-in-turn-lost":
        ("Some chats did not get what you sent them. Send it to them again.",
         "Osa keskusteluista ei saanut sitä, mitä lähetit. Lähetä se niihin uudelleen."),
    "threesea-la-locked-notification-reply-body":
        ("Open it and enter your PIN or passphrase to send your reply and receive messages.",
         "Avaa sovellus ja syötä PIN-koodisi tai tunnuslauseesi, jotta vastauksesi lähetetään ja viestit saapuvat."),
    "threesea-la-startup-failed-notification-body":
        ("Open it to see why. Messages arrive again once it runs.",
         "Avaa sovellus nähdäksesi syyn. Viestit saapuvat taas, kun se on käynnissä."),
    "threesea-la-safe-failing-notification":
        ("Threema Safe backup failing",
         "Threema Safe -varmuuskopiointi epäonnistuu"),
    "threesea-la-safe-failing-notification-body":
        ("Threema Safe hasn't been able to back up for %n day(s). Tap to check.",
         ("Threema Safe ei ole voinut varmuuskopioida %n päivään. Tarkista napauttamalla.",
          "Threema Safe ei ole voinut varmuuskopioida %n päivään. Tarkista napauttamalla.")),
    "threesea-la-reason-login-refused":
        ("Threema's server keeps closing the connection during login.",
         "Threeman palvelin sulkee yhteyden toistuvasti kirjautumisen aikana."),
    "threesea-la-reason-another-connection":
        ("Another device keeps taking over this Threema ID. An ID works on one phone at a time, so Threesea tries again every 10 minutes.",
         "Toinen laite ottaa toistuvasti tämän Threema-tunnuksen haltuunsa. Tunnus toimii yhdessä puhelimessa kerrallaan, joten Threesea yrittää uudelleen 10 minuutin välein."),
    "threesea-la-reason-mediator-version":
        ("Threema's server for linked devices no longer accepts this version of Threesea.",
         "Threeman yhdistettyjen laitteiden palvelin ei enää hyväksy tätä Threesean versiota."),
    "threesea-la-reason-mediator-device-limit":
        ("Threema's server for linked devices turned this device away: this Threema ID has reached its limit of linked devices.",
         "Threeman yhdistettyjen laitteiden palvelin torjui tämän laitteen: tämä Threema-tunnus on saavuttanut yhdistettyjen laitteiden enimmäismäärän."),
    "threesea-la-reason-mediator-duplicate":
        ("Another connection under this device's login took over on Threema's server for linked devices.",
         "Toinen yhteys tämän laitteen kirjautumistiedoilla otti vallan Threeman yhdistettyjen laitteiden palvelimella."),
    "threesea-la-reason-mediator-queue-full":
        ("Too many messages are waiting for this device on Threema's server for linked devices, so it closed the connection.",
         "Threeman yhdistettyjen laitteiden palvelimella odottaa liian monta viestiä tälle laitteelle, joten palvelin sulki yhteyden."),
    "threesea-la-reason-mediator-other":
        ("Threema's server for linked devices closed the connection (code %1).",
         "Threeman yhdistettyjen laitteiden palvelin sulki yhteyden (koodi %1)."),
    "threesea-la-server-message-notification":
        ("Message from Threema's server",
         "Viesti Threeman palvelimelta"),
    "threesea-la-opens-after-call":
        ("Opens when the call is over",
         "Avautuu, kun puhelu on päättynyt"),
    "threesea-la-capture-discarded":
        ("The photo or video waiting to be sent was discarded",
         "Lähetystä odottanut kuva tai video hylättiin"),
    "threesea-la-mobile-verified":
        ("Phone number confirmed",
         "Puhelinnumero vahvistettu"),
    "threesea-la-share-before-setup":
        ("Set up Threesea first, then share this again",
         "Ota Threesea ensin käyttöön ja jaa tämä sitten uudelleen"),
    "threesea-la-storage-problem-title":
        ("Saving messages failed",
         "Viestien tallennus epäonnistui"),
    "threesea-la-storage-problem-body":
        ("Threesea can't write to its data, so new messages wait on the server, for a limited time. Check that this device has free storage. The log in Settings, Advanced has the error code.",
         "Threesea ei pysty kirjoittamaan tietoihinsa, joten uudet viestit odottavat palvelimella rajoitetun ajan. Tarkista, että tällä laitteella on vapaata tallennustilaa. Virhekoodi löytyy lokista kohdasta Asetukset, Lisäasetukset."),
    "threesea-la-web-connected-notification":
        ("Connected to Threema Web",
         "Yhdistetty Threema Webiin"),
    "threesea-la-web-connected-notification-body":
        ("%n browser(s) can read and send your messages through this phone. Tap to manage.",
         ("%n selain voi lukea ja lähettää viestejäsi tämän puhelimen kautta. Hallitse napauttamalla.",
          "%n selainta voi lukea ja lähettää viestejäsi tämän puhelimen kautta. Hallitse napauttamalla.")),
    "threesea-la-web-certificate-notification":
        ("Threema Web server has a new certificate",
         "Threema Web -palvelimella on uusi varmenne"),
    "threesea-la-web-certificate-notification-body":
        ("Tap to decide whether to trust it.",
         "Napauta ja päätä, luotatko siihen."),
    "threesea-me-call-decline":
        ("Decline",
         "Hylkää"),
    "threesea-me-call-answer":
        ("Answer",
         "Vastaa"),
    "threesea-la-app-locked-reply-body":
        ("Open it and enter your PIN or passphrase to send your reply.",
         "Avaa sovellus ja syötä PIN-koodisi tai tunnuslauseesi, jotta vastauksesi lähetetään."),
    "threesea-la-updated":
        ("Threesea was updated",
         "Threesea päivitettiin"),
    "threesea-la-removed":
        ("Threesea was removed from this device",
         "Threesea poistettiin tältä laitteelta"),
    "threesea-la-reply-not-sent":
        ("Your reply was not sent",
         "Vastaustasi ei lähetetty"),
    "threesea-la-app-locked-body":
        ("Something new has arrived. Open it and enter your PIN or passphrase to see it.",
         "Jotain uutta on saapunut. Avaa sovellus ja syötä PIN-koodisi tai tunnuslauseesi nähdäksesi sen."),
    "threesea-la-removed-body":
        ("Close it to end it.",
         "Sulje sovellus lopettaaksesi sen."),
    "threesea-la-updated-body":
        ("Close it and open it again to use the new version.",
         "Sulje sovellus ja avaa se uudelleen ottaaksesi uuden version käyttöön."),
    "threesea-la-updated-open-body":
        ("Tap to open the new version.",
         "Avaa uusi versio napauttamalla."),
    "threesea-la-updated-closed-body":
        ("Open it to receive messages again.",
         "Avaa sovellus, niin viestit saapuvat taas."),
    "threesea-la-reply-not-sent-ended-kept":
        ("Threesea ended before sending it. Your text is kept as a draft in its chat.",
         "Threesea sulkeutui ennen lähettämistä. Tekstisi on tallessa luonnoksena keskustelussaan."),
    "threesea-la-reply-not-sent-ended":
        ("Threesea ended before sending it. Write it again in the chat.",
         "Threesea sulkeutui ennen lähettämistä. Kirjoita se uudelleen keskusteluun."),
    "threesea-la-delete-data-linked-remain":
        ("This phone could not reach the server, so your linked devices stay linked to this ID.",
         "Tämä puhelin ei saanut yhteyttä palvelimeen, joten yhdistetyt laitteesi pysyvät yhdistettyinä tähän tunnukseen."),
    "threesea-la-reply-not-sent-body":
        ("Threesea could not start. Open it to see why.",
         "Threesea ei käynnistynyt. Avaa se nähdäksesi syyn."),
    "threesea-la-reply-not-sent-yet":
        ("It is sent when you next open Threesea.",
         "Se lähetetään, kun avaat Threesean seuraavan kerran."),
    "threesea-la-reply-not-sent-kept":
        ("%1 Your text is kept as a draft in the chat.",
         "%1 Tekstisi on tallessa luonnoksena keskustelussa."),
    "threesea-la-reply-not-sent-late":
        ("It reached Threesea too late. Write it again in the chat.",
         "Se saapui Threesealle liian myöhään. Kirjoita se uudelleen keskusteluun."),
    "threesea-la-share-empty":
        ("What was shared holds nothing to send",
         "Jaetussa sisällössä ei ole mitään lähetettävää"),
    "threesea-la-share-own-files":
        ("Files of Threesea's own were left out of what was shared",
         "Threesean omat tiedostot jätettiin pois jaetusta sisällöstä"),
    "threesea-la-call-route-failed":
        ("The sound could not be switched",
         "Ääntä ei voitu vaihtaa"),
    "threesea-la-recording-discarded":
        ("Recording stopped and discarded",
         "Tallennus pysäytettiin ja hylättiin"),
    "threesea-la-delete-data-failed":
        ("The ID was not removed: %1",
         "Tunnusta ei poistettu: %1"),
    "threesea-la-backup-reminder":
        ("Back up your Threema ID",
         "Varmuuskopioi Threema-tunnuksesi"),
    "threesea-la-backup-reminder-body":
        ("A backup brings your ID, contacts and groups back on a new phone. Tap to set up Threema Safe.",
         "Varmuuskopion avulla saat tunnuksesi, yhteystietosi ja ryhmäsi takaisin uudessa puhelimessa. Ota Threema Safe käyttöön napauttamalla."),
    "threesea-la-safe-server-copy-left":
        ("Threema Safe is off. The backup could not be deleted from Threema's server, so it stays there until it expires.",
         "Threema Safe on pois käytöstä. Varmuuskopiota ei voitu poistaa Threeman palvelimelta, joten se säilyy siellä, kunnes se vanhenee."),
    "threesea-la-device-removal-failed":
        ("Removing linked devices did not finish. Linked devices shows what is still linked, and you can try again there.",
         "Yhdistettyjen laitteiden poisto ei valmistunut. Yhdistetyt laitteet -kohta näyttää, mitä on yhä yhdistetty, ja voit yrittää sieltä uudelleen."),
    "threesea-la-flavour":
        ("Flavour",
         "Versio"),
    "threesea-la-flavour-full":
        ("Full",
         "Full"),
    "threesea-la-flavour-lite":
        ("Lite",
         "Lite"),
    "threesea-la-credit-emoji-search":
        ("Emoji search terms from threema-android (AGPL-3.0).",
         "Emojien hakusanat threema-androidista (AGPL-3.0)."),
    "threesea-la-license-copyright":
        ("Copyright © 2026 the Threesea developers. libthreema and the code taken from threema-android: copyright Threema AG.",
         "Copyright © 2026 Threesean kehittäjät. libthreema ja threema-androidista otettu koodi: copyright Threema AG."),
    "threesea-la-license-source":
        ("Source code and bug reports: %1",
         "Lähdekoodi ja vikailmoitukset: %1"),
    "threesea-bt-license-agpl":
        ("License text",
         "Lisenssiteksti"),
    "threesea-bt-license-notices":
        ("Third-party licenses",
         "Kolmansien osapuolten lisenssit"),
    "threesea-la-add-scanned-contact":
        ("Adds this Threema ID to your contacts with the key from Threema's directory. To verify it, scan the QR code in the other person's app.",
         "Lisää tämän Threema-tunnuksen yhteystietoihisi Threeman hakemiston avaimella. Vahvista se skannaamalla QR-koodi toisen henkilön sovelluksesta."),
    "threesea-la-app-lock-about":
        ("With a lock you can also make chats private: they are hidden from the chat list and are opened from its bottom menu. If you forget the app lock's PIN or passphrase, the way back in is to delete Threesea's data folder .local/share/org.threesea/harbour-threesea with the chats and media in it, restart the phone and restore from a backup. A passphrase for the stored data stays set: Threesea asks for it first.",
         "Lukon avulla voit myös tehdä keskusteluista yksityisiä: ne piilotetaan keskusteluluettelosta ja avataan sen alavalikosta. Jos unohdat sovelluslukon PIN-koodin tai tunnuslauseen, ainoa keino päästä takaisin sisään on poistaa Threesean tietokansio .local/share/org.threesea/harbour-threesea siinä olevine keskusteluineen ja mediatiedostoineen, käynnistää puhelin uudelleen ja palauttaa tiedot varmuuskopiosta. Tallennettujen tietojen tunnuslause pysyy asetettuna: Threesea kysyy sitä ensin."),
    "threesea-la-call-interrupted":
        ("Connection interrupted…",
         "Yhteys katkesi…"),
    "threesea-la-call-unverified":
        ("The connection could not be verified. The call was ended.",
         "Yhteyttä ei voitu vahvistaa. Puhelu lopetettiin."),
    "threesea-la-call-connection-failed":
        ("Connection failed",
         "Yhteys epäonnistui"),
    "threesea-la-camera-photo-failed":
        ("The photo could not be taken.",
         "Kuvaa ei voitu ottaa."),
    "threesea-la-camera-video-failed":
        ("The video could not be recorded.",
         "Videota ei voitu tallentaa."),
    "threesea-la-sending-drawing":
        ("Sending drawing",
         "Lähetetään piirrosta"),
    "threesea-la-discarding-drawing":
        ("Discarding drawing",
         "Hylätään piirrosta"),
    "threesea-la-discarding-recording":
        ("Discarding recording",
         "Hylätään tallennetta"),
    "threesea-la-recording-held":
        ("Recording stopped. Send it or discard it.",
         "Tallennus pysäytetty. Lähetä tai hylkää se."),
    "threesea-la-recording-not-playable":
        ("This recording could not be played. It can still be sent.",
         "Tätä tallennetta ei voitu toistaa. Sen voi silti lähettää."),
    "threesea-la-spam-reporting-group":
        ("Reporting, blocking, leaving and deleting",
         "Ilmoittaminen, estäminen, poistuminen ja poistaminen"),
    "threesea-la-spam-reporting-contact":
        ("Reporting, blocking and deleting",
         "Ilmoittaminen, estäminen ja poistaminen"),
    "threesea-me-polls":
        ("Polls",
         "Kyselyt"),
    "threesea-la-polls-waiting":
        ("%n poll(s) waiting for your vote",
         ("%n kysely odottaa ääntäsi",
          "%n kyselyä odottaa ääntäsi")),
    "threesea-la-revoked-input":
        ("This Threema ID was revoked",
         "Tämä Threema-tunnus on mitätöity"),
    "threesea-la-recording-paused":
        ("Paused %1",
         "Keskeytetty %1"),
    "threesea-he-choose-chats-accept":
        ("Choose",
         "Valitse"),
    "threesea-la-search-chats-contacts":
        ("Search chats and contacts",
         "Hae keskusteluja ja yhteystietoja"),
    "threesea-la-search-no-chats-contacts":
        ("No chats or contacts found",
         "Keskusteluja tai yhteystietoja ei löytynyt"),
    "threesea-me-clean-up-scope":
        ("Groups with only you in them are cleaned up too, and private chats while they are unlocked.",
         "Myös ryhmät, joissa olet vain sinä, siivotaan, samoin yksityiset keskustelut, kun ne on avattu."),
    "threesea-he-clean-up-all-chats":
        ("Clean up all chats",
         "Siivoa kaikki keskustelut"),
    "threesea-he-confirm-send":
        ("Send",
         "Lähetä"),
    "threesea-la-confirm-forward-to":
        ("Forward to %1",
         "Välitä: %1"),
    "threesea-la-confirm-send-to":
        ("Send to %1",
         "Lähetä: %1"),
    "threesea-la-confirm-caption":
        ("Caption",
         "Kuvateksti"),
    "threesea-la-confirm-message":
        ("Message",
         "Viesti"),
    "threesea-la-confirm-include-captions":
        ("Include captions",
         "Liitä kuvatekstit"),
    "threesea-me-add-to-contacts":
        ("Add to contacts",
         "Lisää yhteystietoihin"),
    "threesea-la-contact-share-picture":
        ("Share profile picture",
         "Jaa profiilikuva"),
    "threesea-la-search-contacts":
        ("Search contacts",
         "Hae yhteystietoja"),
    "threesea-la-search-no-contacts":
        ("No contacts found",
         "Yhteystietoja ei löytynyt"),
    "threesea-me-lock-now":
        ("Lock now",
         "Lukitse nyt"),
    "threesea-la-data-import-unfinished":
        ("Your Data backup isn't fully imported yet. Tap to import it again; it continues where it stopped.",
         "Tietojen varmuuskopiota ei ole vielä tuotu kokonaan. Tuo se uudelleen napauttamalla; tuonti jatkuu siitä, mihin se jäi."),
    "threesea-me-notifications-back-on":
        ("Notifications on",
         "Ilmoitukset käyttöön"),
    "threesea-me-notifications-off-8-hours":
        ("Notifications off for 8 hours",
         "Ilmoitukset pois 8 tunniksi"),
    "threesea-la-chats-not-loaded":
        ("The chats could not be loaded",
         "Keskusteluja ei voitu ladata"),
    "threesea-la-data-backup-import-intro":
        ("Adds what is missing: contacts, groups and pictures, then messages and media. Everything already on this device stays as it is. No messages are sent.",
         "Lisää puuttuvat tiedot: yhteystiedot, ryhmät ja kuvat, sitten viestit ja mediatiedostot. Kaikki tällä laitteella jo oleva säilyy ennallaan. Viestejä ei lähetetä."),
    "threesea-la-data-backup-import-other":
        ("This backup belongs to another Threema ID or key. To import it, first restore that ID on this device.",
         "Tämä varmuuskopio kuuluu toiselle Threema-tunnukselle tai avaimelle. Jos haluat tuoda sen, palauta ensin kyseinen tunnus tälle laitteelle."),
    "threesea-la-data-backup-import-unreadable":
        ("The Threema ID in this backup isn't readable with this password. The file may be damaged. Only a backup of this device's ID is imported.",
         "Tämän varmuuskopion Threema-tunnusta ei voi lukea tällä salasanalla. Tiedosto voi olla vioittunut. Vain tämän laitteen tunnuksen varmuuskopio tuodaan."),
    "threesea-la-data-backup-import-no-id":
        ("This backup holds no Threema ID. Only a backup of this device's ID is imported.",
         "Tässä varmuuskopiossa ei ole Threema-tunnusta. Vain tämän laitteen tunnuksen varmuuskopio tuodaan."),
    "threesea-la-data-backup-contents":
        ("Encrypted backups of your Threema ID, chats, contacts, groups and media, in the format of Threema for Android.",
         "Salatut varmuuskopiot Threema-tunnuksestasi, keskusteluistasi, yhteystiedoistasi, ryhmistäsi ja mediatiedostoistasi Threema for Androidin muodossa."),
    "threesea-la-android-export-running":
        ("Writing the backup",
         "Varmuuskopiota kirjoitetaan"),
    "threesea-la-android-export-left-out":
        ("%n chat(s) stayed out of the last backup: restoring them needs the contact's key, which could not be fetched. The next backup tries again.",
         ("%n keskustelu jäi pois viimeisimmästä varmuuskopiosta: sen palauttamiseen tarvitaan yhteystiedon avain, jota ei voitu hakea. Seuraava varmuuskopio yrittää uudelleen.",
          "%n keskustelua jäi pois viimeisimmästä varmuuskopiosta: niiden palauttamiseen tarvitaan yhteystiedon avain, jota ei voitu hakea. Seuraava varmuuskopio yrittää uudelleen.")),
    "threesea-la-android-export-files-left-out":
        ("%n file(s) stayed out of the last backup: they could not be read on this phone.",
         ("%n tiedosto jäi pois viimeisimmästä varmuuskopiosta: sitä ei voitu lukea tässä puhelimessa.",
          "%n tiedostoa jäi pois viimeisimmästä varmuuskopiosta: niitä ei voitu lukea tässä puhelimessa.")),
    "threesea-la-safe-last-attempt-failed":
        ("Last attempt failed",
         "Viimeisin yritys epäonnistui"),
    "threesea-he-restore-data-backup":
        ("Restore from Data backup",
         "Palauta tietojen varmuuskopiosta"),
    "threesea-la-restore-data-backup-intro":
        ("Choose a Data backup from Threema for Android or Threesea and enter its password. It brings your ID, contacts, groups, chats and media. The password stays on this device.",
         "Valitse Threema for Androidin tai Threesean tietojen varmuuskopio ja syötä sen salasana. Se palauttaa tunnuksesi, yhteystietosi, ryhmäsi, keskustelusi ja mediatiedostosi. Salasana pysyy tällä laitteella."),
    "threesea-la-restore-one-phone":
        ("An ID works on one phone at a time: new messages go only to the phone that connected last.",
         "Tunnus toimii yhdessä puhelimessa kerrallaan: uudet viestit menevät vain puhelimeen, joka yhdisti viimeksi."),
    "threesea-la-data-backup-reason-too-large":
        ("too large to import",
         "liian suuri tuotavaksi"),
    "threesea-la-data-backup-reason-damaged":
        ("damaged in the backup",
         "vioittunut varmuuskopiossa"),
    "threesea-la-data-backup-reason-other-key":
        ("a different key in the backup",
         "varmuuskopiossa eri avain"),
    "threesea-la-data-backup-reason-not-here":
        ("not on this device",
         "ei tällä laitteella"),
    "threesea-la-data-backup-imported-except":
        ("The backup is imported, except for what is listed below. Everything already on this device stayed as it was.",
         "Varmuuskopio on tuotu lukuun ottamatta alla lueteltuja kohteita. Kaikki tällä laitteella jo ollut säilyi ennallaan."),
    "threesea-la-data-backup-restored-private":
        ("Chats that were marked private come back as ordinary, visible chats. To hide them again, set up an app lock in Settings and choose Make private in the chat list.",
         "Yksityisiksi merkityt keskustelut palautuvat tavallisina, näkyvinä keskusteluina. Jos haluat piilottaa ne taas, ota sovelluslukko käyttöön Asetuksissa ja valitse keskusteluluettelossa Tee yksityiseksi."),
    "threesea-la-data-backup-received-list-missing":
        ("The list of messages already received isn't part of this import. Messages sent again are recognised from what this device receives from now on.",
         "Luetteloa jo vastaanotetuista viesteistä ei tuoda mukana. Uudelleen lähetetyt viestit tunnistetaan siitä, mitä tämä laite vastaanottaa tästä eteenpäin."),
    "threesea-la-data-backup-key-conflicts":
        ("Contacts with a different key (kept as here)",
         "Yhteystiedot, joilla on eri avain (säilytetty tämän laitteen mukaisina)"),
    "threesea-la-data-backup-local-avatars":
        ("Pictures chosen for contacts on the other device, left out",
         "Toisella laitteella yhteystiedoille valitut kuvat, jätetty pois"),
    "threesea-la-data-backup-groups-left-out":
        ("Groups left out",
         "Pois jätetyt ryhmät"),
    "threesea-la-data-backup-unsent":
        ("Messages that were never sent (kept as failed)",
         "Viestit, joita ei koskaan lähetetty (säilytetty epäonnistuneina)"),
    "threesea-la-data-backup-chats-left-out":
        ("Chats left out",
         "Pois jätetyt keskustelut"),
    "threesea-la-data-backup-chats-too-large":
        ("Chats too large to import",
         "Liian suuret keskustelut tuotaviksi"),
    "threesea-la-data-backup-chats-damaged":
        ("Damaged chats left out",
         "Pois jätetyt vioittuneet keskustelut"),
    "threesea-la-data-backup-messages-left-out":
        ("Messages left out",
         "Pois jätetyt viestit"),
    "threesea-la-data-backup-collisions":
        ("Messages here from another sender (kept as here)",
         "Tämän laitteen viestit toiselta lähettäjältä (säilytetty tämän laitteen mukaisina)"),
    "threesea-la-data-backup-reactions-left-out":
        ("Reactions left out",
         "Pois jätetyt reaktiot"),
    "threesea-la-data-backup-reaction-files-left-out":
        ("Lists of reactions left out",
         "Pois jätetyt reaktioluettelot"),
    "threesea-la-data-backup-entries-left-out":
        ("Entries left out",
         "Pois jätetyt merkinnät"),
    "threesea-la-data-backup-media-missing":
        ("Media not in the backup",
         "Media, joka puuttuu varmuuskopiosta"),
    "threesea-se-data-backup-left-out":
        ("Chats that weren't imported",
         "Keskustelut, joita ei tuotu"),
    "threesea-la-data-backup-uid-conflicts":
        ("Messages also found in another chat",
         "Viestit, jotka löytyivät myös toisesta keskustelusta"),
    "threesea-la-data-backup-foreign-group-chats":
        ("Group chats mostly from non-members",
         "Ryhmäkeskustelut, joissa enimmäkseen muiden kuin jäsenten viestejä"),
    "threesea-la-delete-data-goes":
        ("This removes your Threema ID, contacts, chats, media and settings from this device.",
         "Tämä poistaa Threema-tunnuksesi, yhteystietosi, keskustelusi, mediasi ja asetuksesi tältä laitteelta."),
    "threesea-la-delete-data-comes-back-data-backup":
        ("Your ID itself is not revoked: you can restore it later from Threema Safe, an ID backup or a Data backup — without one of them, the ID is lost. Chats and media come back only from a Data backup.",
         "Itse tunnustasi ei mitätöidä: voit palauttaa sen myöhemmin Threema Safesta, tunnuksen varmuuskopiosta tai tietojen varmuuskopiosta – ilman jotakin niistä tunnus menetetään. Keskustelut ja media palautuvat vain tietojen varmuuskopiosta."),
    "threesea-la-delete-data-stays":
        ("Pictures saved to the Threesea folder in Pictures, and backups and files you exported, stay on this device. A Threema Safe backup stays on the server.",
         "Kuvat-kansion Threesea-kansioon tallennetut kuvat sekä viemäsi varmuuskopiot ja tiedostot jäävät tälle laitteelle. Threema Safe -varmuuskopio jää palvelimelle."),
    "threesea-la-delete-data-linked":
        ("Your linked devices are removed from this ID first, which takes a connection. If this phone can't reach the server then, they stay linked and keep the ID.",
         "Yhdistetyt laitteesi poistetaan ensin tästä tunnuksesta, mihin tarvitaan yhteys. Jos tämä puhelin ei silloin saa yhteyttä palvelimeen, laitteet jäävät yhdistetyiksi ja säilyttävät tunnuksen."),
    "threesea-la-delete-data-have-a-backup":
        ("I have a Threema Safe, ID or Data backup, or no longer need this ID",
         "Minulla on Threema Safe -varmuuskopio, tunnuksen varmuuskopio tai tietojen varmuuskopio, tai en enää tarvitse tätä tunnusta"),
    "threesea-he-share-id-backup":
        ("Share ID backup",
         "Jaa tunnuksen varmuuskopio"),
    "threesea-bt-share":
        ("Share",
         "Jaa"),
    "threesea-bt-done":
        ("Done",
         "Valmis"),
    "threesea-la-closing-id-backup":
        ("Closing ID backup",
         "Suljetaan tunnuksen varmuuskopio"),
    "threesea-la-export-chat-password-note":
        ("At least 8 characters. With a password the messages and files in the archive are encrypted with AES-256, and it opens in ZIP programs with AES support, such as 7-Zip; the names of the files stay readable. Empty: not encrypted, anyone who gets the file can read it.",
         "Vähintään 8 merkkiä. Salasanaa käytettäessä arkiston viestit ja tiedostot salataan AES-256:lla, ja arkisto aukeaa AES-tuetuilla ZIP-ohjelmilla, kuten 7-Zipillä; tiedostojen nimet jäävät luettaviksi. Tyhjä: ei salausta, kuka tahansa tiedoston saava voi lukea sen."),
    "threesea-he-rename-list":
        ("Rename list",
         "Nimeä lista uudelleen"),
    "threesea-me-choose-picture":
        ("Choose picture",
         "Valitse kuva"),
    "threesea-me-take-picture":
        ("Take photo",
         "Ota kuva"),
    "threesea-la-removing-picture":
        ("Removing picture",
         "Poistetaan kuva"),
    "threesea-la-license-key-changed":
        ("License key changed",
         "Lisenssiavain vaihdettu"),
    "threesea-me-change-license-key":
        ("Change license key",
         "Vaihda lisenssiavain"),
    "threesea-la-identity-intro-id-export":
        ("Your Threema ID identifies you to your contacts. Create a new one, or restore the ID you already use from Threema Safe, an ID backup (Threema for Android calls it an ID export) or a Data backup. A Data backup brings your chats as well.",
         "Threema-tunnuksesi yksilöi sinut yhteystiedoillesi. Luo uusi tunnus tai palauta jo käyttämäsi tunnus Threema Safesta, tunnuksen varmuuskopiosta (Threema for Android käyttää siitä nimeä ID-vienti) tai tietojen varmuuskopiosta. Tietojen varmuuskopio tuo mukanaan myös keskustelusi."),
    "threesea-bt-restore-data-backup":
        ("Restore from Data backup",
         "Palauta tietojen varmuuskopiosta"),
    "threesea-la-cannot-show-picture":
        ("This picture could not be shown",
         "Tätä kuvaa ei voitu näyttää"),
    "threesea-la-media-position":
        ("%1 of %2",
         "%1/%2"),
    "threesea-me-select-all":
        ("Select all",
         "Valitse kaikki"),
    "threesea-me-select":
        ("Select",
         "Valitse"),
    "threesea-la-select-media-hint":
        ("Tap to select",
         "Valitse napauttamalla"),
    "threesea-la-all-contacts-on-list":
        ("All your contacts are on this list",
         "Kaikki yhteystietosi ovat tällä listalla"),
    "threesea-la-all-contacts-in-group":
        ("All your contacts are in this group",
         "Kaikki yhteystietosi ovat tässä ryhmässä"),
    "threesea-he-reactions":
        ("Reactions",
         "Reaktiot"),
    "threesea-la-nickname-ask-title":
        ("Choose a nickname",
         "Valitse lempinimi"),
    "threesea-la-nickname-ask-text":
        ("This name goes with your messages. Contacts who haven't given you a name of their own see it. You can change it in Settings, My profile.",
         "Tämä nimi kulkee viestiesi mukana. Sen näkevät yhteystiedot, jotka eivät ole antaneet sinulle omaa nimeä. Voit vaihtaa sen kohdassa Asetukset, Oma profiili."),
    "threesea-he-photo-text":
        ("Text on the photo",
         "Teksti kuvaan"),
    "threesea-la-photo-text":
        ("Text",
         "Teksti"),
    "threesea-la-data-passphrase-note":
        ("If you forget it, Threesea can start fresh only without what it has stored on this phone. Your ID and chats can then come back from a backup, if you have one.",
         "Jos unohdat sen, Threesea voi aloittaa alusta vain ilman tähän puhelimeen tallentamiaan tietoja. Tunnuksesi ja keskustelusi voidaan silloin palauttaa varmuuskopiosta, jos sinulla on sellainen."),
    "threesea-me-try-again":
        ("Try again",
         "Yritä uudelleen"),
    "threesea-he-choose-place":
        ("Choose a place",
         "Valitse paikka"),
    "threesea-la-place-search":
        ("Search by name",
         "Hae nimellä"),
    "threesea-la-place-use-own":
        ("Use your position",
         "Käytä sijaintiasi"),
    "threesea-he-places-found":
        ("Places found",
         "Löydetyt paikat"),
    "threesea-he-places-nearby":
        ("Places nearby",
         "Lähellä olevat paikat"),
    "threesea-la-places-need-position":
        ("Places are found around your position",
         "Paikat haetaan sijaintisi ympäriltä"),
    "threesea-la-places-failed":
        ("Threema's map server did not answer",
         "Threeman karttapalvelin ei vastannut"),
    "threesea-la-places-none":
        ("No places found",
         "Paikkoja ei löytynyt"),
    "threesea-la-places-failed-hint":
        ("Check your internet connection and pull down to try again.",
         "Tarkista internetyhteys ja yritä uudelleen vetämällä alas."),
    "threesea-la-places-none-hint":
        ("Search by name to look further away.",
         "Hae nimellä löytääksesi kauempana olevia paikkoja."),
    "threesea-he-polls":
        ("Polls",
         "Kyselyt"),
    "threesea-la-no-polls":
        ("No polls in this chat",
         "Tässä keskustelussa ei ole kyselyjä"),
    "threesea-se-polls-closed":
        ("Closed",
         "Suljetut"),
    "threesea-se-polls-open":
        ("Open",
         "Avoimet"),
    "threesea-la-poll-waiting":
        ("Waiting for your vote",
         "Odottaa ääntäsi"),
    "threesea-la-restore-intro-scan":
        ("Scan the QR code of the ID backup or paste its text (80 characters in groups of four, like XXXX-XXXX-…), and enter the password you chose when you created it. Threema for Android calls it an ID export. It brings your ID. Chats come from a Data backup, which Settings, Backup can import afterwards.",
         "Skannaa tunnuksen varmuuskopion QR-koodi tai liitä sen teksti (80 merkkiä neljän ryhmissä, kuten XXXX-XXXX-…) ja syötä salasana, jonka valitsit varmuuskopiota luodessasi. Threema for Android käyttää siitä nimeä ID-vienti. Se palauttaa tunnuksesi. Keskustelut saadaan tietojen varmuuskopiosta, jonka voi tuoda jälkikäteen kohdassa Asetukset, Varmuuskopio."),
    "threesea-la-safe-find-id-none":
        ("No Threema ID is linked to this.",
         "Tähän ei ole liitetty Threema-tunnusta."),
    "threesea-he-safe-find-id":
        ("Find my ID",
         "Etsi tunnukseni"),
    "threesea-la-safe-find-id-intro":
        ("Enter the phone number or e-mail address linked to your Threema ID. It is hashed on this device, and Threema's directory is asked with the hash.",
         "Syötä Threema-tunnukseesi liitetty puhelinnumero tai sähköpostiosoite. Siitä lasketaan tiiviste tällä laitteella, ja Threeman hakemistolta kysytään tiivisteellä."),
    "threesea-la-safe-find-id-phone":
        ("Phone number",
         "Puhelinnumero"),
    "threesea-la-safe-find-id-email":
        ("E-mail address",
         "Sähköpostiosoite"),
    "threesea-bt-safe-find-id":
        ("Find",
         "Etsi"),
    "threesea-se-safe-find-id-found":
        ("Linked IDs",
         "Liitetyt tunnukset"),
    "threesea-la-safe-description-changes":
        ("Encrypted backup of your ID, profile, contacts, groups and settings, updated when something changes. For your chats, use Data backup.",
         "Salattu varmuuskopio tunnuksestasi, profiilistasi, yhteystiedoistasi, ryhmistäsi ja asetuksistasi, päivitetään kun jokin muuttuu. Keskusteluille käytä tietojen varmuuskopiota."),
    "threesea-la-safe-disabling-delete":
        ("Turning off and deleting the backup",
         "Poistetaan käytöstä ja poistetaan varmuuskopio"),
    "threesea-he-continue":
        ("Continue",
         "Jatka"),
    "threesea-he-change":
        ("Change",
         "Vaihda"),
    "threesea-la-safe-password-change-intro":
        ("Choose a new password for Threema Safe. A new backup replaces the one made with the old password. Only your password (at least 8 characters) unlocks it: if you forget it, the backup cannot be recovered.",
         "Valitse Threema Safelle uusi salasana. Uusi varmuuskopio korvaa vanhalla salasanalla tehdyn. Vain salasanasi (vähintään 8 merkkiä) avaa sen: jos unohdat sen, varmuuskopiota ei voi palauttaa."),
    "threesea-la-safe-password-intro-changes":
        ("Threema Safe stores an encrypted backup of your ID, profile, contacts, groups and settings on Threema's servers, updated when something changes. For your chats, use Data backup. Only your password (at least 8 characters) unlocks it — if you forget it, the backup cannot be recovered.",
         "Threema Safe tallentaa Threeman palvelimille salatun varmuuskopion tunnuksestasi, profiilistasi, yhteystiedoistasi, ryhmistäsi ja asetuksistasi, ja päivittää sen kun jokin muuttuu. Keskusteluille käytä tietojen varmuuskopiota. Vain salasanasi (vähintään 8 merkkiä) avaa sen – jos unohdat sen, varmuuskopiota ei voi palauttaa."),
    "threesea-la-safe-setup-skip":
        ("Continue without a backup",
         "Jatka ilman varmuuskopiota"),
    "threesea-la-safe-setup-skip-description":
        ("Your ID, contacts and groups then exist on this phone alone, and go with it. Settings, Backup can turn Threema Safe on or export an ID backup at any time.",
         "Tunnuksesi, yhteystietosi ja ryhmäsi ovat silloin vain tässä puhelimessa ja katoavat sen mukana. Kohdassa Asetukset, Varmuuskopio voit milloin tahansa ottaa Threema Safen käyttöön tai viedä tunnuksen varmuuskopion."),
    "threesea-me-safe-find-id":
        ("Find my ID",
         "Etsi tunnukseni"),
    "threesea-la-restore-safe-intro-data-backup":
        ("Enter your Threema ID and your Threema Safe password. Your profile, contacts, groups and settings are restored with it. Chats come from a Data backup, which Settings, Backup can import afterwards.",
         "Syötä Threema-tunnuksesi ja Threema Safe -salasanasi. Profiilisi, yhteystietosi, ryhmäsi ja asetuksesi palautetaan samalla. Keskustelut saadaan tietojen varmuuskopiosta, jonka voi tuoda jälkikäteen kohdassa Asetukset, Varmuuskopio."),
    "threesea-la-qr-not-id-backup":
        ("This is not an ID backup. Scan the QR code of your ID backup.",
         "Tämä ei ole tunnuksen varmuuskopio. Skannaa tunnuksesi varmuuskopion QR-koodi."),
    "threesea-la-qr-not-web-code":
        ("This is a contact's code. Scan the code Threema Web shows.",
         "Tämä on yhteystiedon koodi. Skannaa koodi, jonka Threema Web näyttää."),
    "threesea-la-qr-link-verify":
        ("This is a link to a Threema ID. To verify it, scan the QR code in the other person's app.",
         "Tämä on linkki Threema-tunnukseen. Vahvista se skannaamalla QR-koodi toisen henkilön sovelluksesta."),
    "threesea-la-qr-id-backup-elsewhere":
        ("This is an ID backup. It restores an ID while the app is being set up.",
         "Tämä on tunnuksen varmuuskopio. Sillä palautetaan tunnus sovelluksen käyttöönoton aikana."),
    "threesea-la-qr-unknown":
        ("This is not a Threema QR code.",
         "Tämä ei ole Threeman QR-koodi."),
    "threesea-la-scan-hint-id-backup":
        ("Point the camera at the QR code of your ID backup.",
         "Osoita kameralla tunnuksesi varmuuskopion QR-koodia."),
    "threesea-la-scan-hint-web":
        ("Point the camera at the QR code Threema Web shows.",
         "Osoita kameralla QR-koodia, jonka Threema Web näyttää."),
    "threesea-la-search-names-messages":
        ("Names and messages",
         "Nimet ja viestit"),
    "threesea-he-messages":
        ("Messages",
         "Viestit"),
    "threesea-me-remove-star":
        ("Remove star",
         "Poista tähti"),
    "threesea-la-search-nothing-at-all":
        ("No chats, contacts or messages found",
         "Keskusteluja, yhteystietoja tai viestejä ei löytynyt"),
    "threesea-la-location-waiting":
        ("No position yet. Check that location is on in Settings → Location. Outdoors this goes faster.",
         "Sijaintia ei vielä ole. Tarkista, että sijainti on päällä kohdassa Asetukset → Sijainti. Ulkona tämä käy nopeammin."),
    "threesea-la-location-own":
        ("Your position",
         "Sijaintisi"),
    "threesea-la-location-place":
        ("Place",
         "Paikka"),
    "threesea-la-location-last-known":
        ("Your last known position, from %1",
         "Viimeisin tunnettu sijaintisi, ajalta %1"),
    "threesea-la-place-server":
        ("To find places, Threema's map server gets your position and what you search for.",
         "Paikkojen hakua varten Threeman karttapalvelin saa sijaintisi ja hakusanasi."),
    "threesea-la-location-use-last-known":
        ("Use your last known position, from",
         "Käytä viimeisintä tunnettua sijaintiasi, ajalta"),
    "threesea-la-update-check-failed":
        ("Could not check for updates. Try again later.",
         "Päivityksiä ei voitu tarkistaa. Yritä myöhemmin uudelleen."),
    "threesea-me-emoji-twemoji":
        ("Twemoji",
         "Twemoji"),
    "threesea-la-safe-on-failing":
        ("On, the last attempt failed",
         "Päällä, viimeisin yritys epäonnistui"),
    "threesea-he-leave-device-group":
        ("Leave",
         "Poistu"),
    "threesea-la-leave-device-group-text":
        ("This device connects directly again. The other devices keep your ID and stay in their group, and only they can remove themselves from then on.",
         "Tämä laite muodostaa taas yhteyden suoraan. Muut laitteet säilyttävät tunnuksesi ja pysyvät ryhmässään, ja siitä eteenpäin vain ne voivat poistaa itsensä."),
    "threesea-he-link-desktop":
        ("Link Threema for Desktop",
         "Yhdistä Threema Desktop"),
    "threesea-he-link-next":
        ("Next",
         "Seuraava"),
    "threesea-la-link-grant":
        ("Threema for Desktop gets your Threema ID with its private key, your contacts and groups, and can read and send messages as you. Link only your own computer.",
         "Threema Desktop saa Threema-tunnuksesi yksityisine avaimineen, yhteystietosi ja ryhmäsi, ja voi lukea ja lähettää viestejä sinun nimissäsi. Yhdistä vain oma tietokoneesi."),
    "threesea-la-link-pfs-if-on":
        ("While a device is linked, chats with your contacts use end-to-end encryption without Perfect Forward Secrecy. If it was on, it comes back when the last linked device is removed.",
         "Kun laite on yhdistetty, keskustelut yhteystietojesi kanssa käyttävät päästä päähän -salausta ilman Perfect Forward Secrecyä (PFS). Jos se oli päällä, se palaa käyttöön, kun viimeinen yhdistetty laite poistetaan."),
    "threesea-me-link-scan-again":
        ("Scan again",
         "Skannaa uudelleen"),
    "threesea-me-clear-reflected-left-out":
        ("Clear count of messages not kept",
         "Nollaa säilyttämättä jääneiden viestien määrä"),
    "threesea-la-reflected-left-out":
        ("%n message(s) received on your other devices weren't kept on this phone.",
         ("%n muilla laitteillasi vastaanotettu viesti jäi säilyttämättä tässä puhelimessa.",
          "%n muilla laitteillasi vastaanotettua viestiä jäi säilyttämättä tässä puhelimessa.")),
    "threesea-la-linked-devices-offline":
        ("Waiting for the connection",
         "Odotetaan yhteyttä"),
    "threesea-la-linked-devices-failed":
        ("The list of linked devices did not load",
         "Yhdistettyjen laitteiden luettelo ei latautunut"),
    "threesea-la-linked-devices-offline-hint":
        ("The linked devices appear here once the server answers.",
         "Yhdistetyt laitteet näkyvät tässä, kun palvelin vastaa."),
    "threesea-la-linked-devices-failed-hint":
        ("Open this page again to load it.",
         "Lataa luettelo avaamalla tämä sivu uudelleen."),
    "threesea-la-log-save-failed":
        ("Could not save the support package to Downloads.",
         "Tukipakettia ei voitu tallentaa Lataukset-kansioon."),
    "threesea-la-keep-messages-remorse":
        ("Deleting older messages",
         "Poistetaan vanhempia viestejä"),
    "threesea-la-cleaning-up":
        ("Cleaning up",
         "Siivotaan"),
    "threesea-la-notifications-calls":
        ("Notify about new messages and missed calls",
         "Ilmoita uusista viesteistä ja vastaamattomista puheluista"),
    "threesea-la-notification-preview-who-description":
        ("Otherwise notifications show who wrote, but not what, e.g. on the lock screen.",
         "Muuten ilmoitukset näyttävät, kuka kirjoitti, mutta eivät mitä, esim. lukitusnäytöllä."),
    "threesea-se-notifications-calls":
        ("Calls",
         "Puhelut"),
    "threesea-la-call-tone":
        ("Ringtone",
         "Soittoääni"),
    "threesea-la-call-tone-system":
        ("System call tone",
         "Järjestelmän soittoääni"),
    "threesea-la-inapp-sound-voice-description":
        ("A soft sound between two voice messages playing one after the other.",
         "Pehmeä ääni kahden peräkkäin toistettavan ääniviestin välissä."),
    "threesea-la-exclusion-list-description":
        ("Finding contacts from the address book will not add or rename these Threema IDs.",
         "Yhteystietojen haku osoitekirjasta ei lisää eikä nimeä uudelleen näitä Threema-tunnuksia."),
    "threesea-la-clear-contact-policies-description":
        ("The read receipt and typing indicator choices made on single contacts' pages go back to the settings above.",
         "Yksittäisten yhteystietojen sivuilla tehdyt lukukuittaus- ja kirjoitusilmaisinvalinnat palautetaan yllä oleviin asetuksiin."),
    "threesea-la-contact-policies-cleared":
        ("Cleared for %n contact(s)",
         ("Nollattu %n yhteystiedolta",
          "Nollattu %n yhteystiedolta")),
    "threesea-la-forward-secrecy-linked":
        ("Off while other devices are linked to this ID. Your choice here applies again when the last one is removed.",
         "Pois päältä, kun tähän tunnukseen on yhdistetty muita laitteita. Tässä tekemäsi valinta on taas voimassa, kun viimeinen niistä poistetaan."),
    "threesea-la-removing-link":
        ("Removing link",
         "Poistetaan yhdistys"),
    "threesea-la-cancelling-link":
        ("Cancelling link",
         "Perutaan yhdistäminen"),
    "threesea-me-share-my-id":
        ("Share my ID",
         "Jaa tunnukseni"),
    "threesea-la-revocation-not-checked":
        ("Not checked yet",
         "Ei vielä tarkistettu"),
    "threesea-la-data-passphrase-about":
        ("Your messages are stored encrypted with a key that is kept encrypted with this passphrase. Threesea asks for it every time it starts, also after restarting the phone or deleting its data folder, and only this passphrase opens it. It receives messages once it is entered. If you forget it, pull down where it is asked for to start fresh, without what Threesea has stored on this phone: Threema Safe or an ID backup brings your ID back, a Data backup your chats.",
         "Viestisi tallennetaan salattuina avaimella, joka on salattu tällä tunnuslauseella. Threesea kysyy sitä joka kerta käynnistyessään, myös puhelimen uudelleenkäynnistyksen tai datakansionsa poistamisen jälkeen, ja vain tämä tunnuslause avaa sen. Viestejä vastaanotetaan, kun se on syötetty. Jos unohdat sen, vedä alas kohdassa, jossa sitä kysytään, aloittaaksesi alusta ilman tietoja, jotka Threesea on tallentanut tähän puhelimeen: Threema Safe tai tunnuksen varmuuskopio palauttaa tunnuksesi, tietojen varmuuskopio keskustelusi."),
    "threesea-la-web-pair-grants":
        ("This code connects a browser to your Threema ID. That browser can read your contacts and chats and send messages in your name.",
         "Tämä koodi yhdistää selaimen Threema-tunnukseesi. Selain voi lukea yhteystietojasi ja keskustelujasi sekä lähettää viestejä sinun nimissäsi."),
    "threesea-la-web-pair-only-yours":
        ("Allow it only if you opened Threema Web yourself and the code is on your own screen. If someone sent you this code, go back.",
         "Salli tämä vain, jos avasit Threema Webin itse ja koodi on omalla näytölläsi. Jos joku lähetti sinulle tämän koodin, palaa takaisin."),
    "threesea-la-web-pair-remembered":
        ("If the browser asks to be remembered, it can connect again without a new code until you remove it from Threema Web in the settings.",
         "Jos selain pyytää tulla muistetuksi, se voi muodostaa yhteyden uudelleen ilman uutta koodia, kunnes poistat sen Threema Webistä asetuksissa."),
    "threesea-he-rename-browser":
        ("Rename browser",
         "Nimeä selain uudelleen"),
    "threesea-la-browser-name":
        ("Browser name",
         "Selaimen nimi"),
    "threesea-la-browser-name-description":
        ("Only this phone shows this name. Left empty, the browser's own name is shown.",
         "Vain tämä puhelin näyttää tämän nimen. Jos kenttä jätetään tyhjäksi, näytetään selaimen oma nimi."),
    "threesea-la-web-scan-again":
        ("The new certificate is trusted. Scan the code in the browser again to connect it.",
         "Uuteen varmenteeseen luotetaan. Yhdistä selain skannaamalla sen koodi uudelleen."),
    "threesea-la-web-disconnected-kept":
        ("Threema Web disconnected. Tap the browser to connect it again.",
         "Threema Webin yhteys katkesi. Yhdistä selain uudelleen napauttamalla sitä."),
    "threesea-la-web-disconnected":
        ("Threema Web disconnected.",
         "Threema Webin yhteys katkesi."),
    "threesea-la-web-removing":
        ("Removing",
         "Poistetaan"),
    "threesea-me-web-remove-all":
        ("Remove all browsers",
         "Poista kaikki selaimet"),
    "threesea-la-web-removing-all":
        ("Removing all browsers",
         "Poistetaan kaikki selaimet"),
    "threesea-la-web-enabled":
        ("Threema Web",
         "Threema Web"),
    "threesea-la-web-enabled-description":
        ("Lets the browsers listed here connect to this phone and use your chats. Turning it off disconnects them and keeps them in the list.",
         "Sallii tässä lueteltujen selainten muodostaa yhteyden tähän puhelimeen ja käyttää keskustelujasi. Kun tämä kytketään pois päältä, yhteydet katkaistaan, mutta selaimet säilyvät luettelossa."),
    "threesea-la-web-tap-hint":
        ("Tap a browser to connect or disconnect it. Threema Web must be open in the browser to connect. A disconnected browser can connect again when it is opened; removing it or turning off the switch above keeps it out.",
         "Napauta selainta muodostaaksesi tai katkaistaksesi sen yhteyden. Threema Webin on oltava auki selaimessa yhteyttä varten. Selain, jonka yhteys on katkaistu, voi muodostaa yhteyden uudelleen, kun se avataan; poistaminen tai yllä olevan kytkimen kääntäminen pois päältä estää sen."),
    "threesea-me-web-rename":
        ("Rename",
         "Nimeä uudelleen"),
    "threesea-se-web-servers":
        ("Allowed servers",
         "Sallitut palvelimet"),
    "threesea-la-web-servers-description":
        ("Self-hosted Threema Web servers you allowed. Removing one also removes its browsers, and the next code naming it asks again.",
         "Sallimasi itse ylläpidetyt Threema Web -palvelimet. Palvelimen poistaminen poistaa myös sen selaimet, ja seuraava sitä käyttävä koodi kysyy uudelleen."),
    "threesea-la-share-refused-several":
        ("Nothing was sent. Blocked or revoked, and taken off the choice: %1",
         "Mitään ei lähetetty. Estetty tai mitätöity, ja poistettu valinnasta: %1"),
    "threesea-me-choose-several-chats":
        ("Choose several chats",
         "Valitse useita keskusteluja"),
    "threesea-la-startup-try-again":
        ("Pull down to try again.",
         "Yritä uudelleen vetämällä alas."),
    "threesea-la-startup-code":
        ("Code: %1",
         "Koodi: %1"),
    "threesea-la-removal-stuck":
        ("Removing the ID did not finish",
         "Tunnuksen poisto ei valmistunut"),
    "threesea-la-removal-stuck-hint":
        ("Pull down to try again, or close Threesea and open it again (%1).",
         "Yritä uudelleen vetämällä alas tai sulje Threesea ja avaa se uudelleen (%1)."),
    "threesea-he-enter-data-passphrase":
        ("Enter the passphrase of your stored data",
         "Syötä tallennettujen tietojesi tunnuslause"),
    "threesea-he-video-keeps-metadata-send":
        ("Send as it is",
         "Lähetä sellaisenaan"),
    "threesea-he-video-keeps-metadata-cancel":
        ("Don't send",
         "Älä lähetä"),
    "threesea-la-video-keeps-metadata":
        ("This video keeps what its file says about it",
         "Tämä video säilyttää tiedostonsa sisältämät tiedot itsestään"),
    "threesea-la-picture-keeps-metadata":
        ("This picture keeps what its file says about it",
         "Tämä kuva säilyttää tiedostonsa sisältämät tiedot itsestään"),
    "threesea-la-video-keeps-metadata-description":
        ("Threesea can't remove the metadata of this file. It can hold where and when the video was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file.",
         "Threesea ei voi poistaa tämän tiedoston metatietoja. Ne voivat kertoa, missä, milloin ja millä laitteella video on tehty, ja vastaanottaja saa ne kaikki sekä tiedoston nimen: %1. Sellaisenaan lähetettynä se lähtee tavallisena tiedostona."),
    "threesea-la-picture-keeps-metadata-description":
        ("Threesea can't remove the metadata of this file. It can hold where and when the picture was made and on what device, and the recipient gets all of it, along with the file's name: %1. Sent as it is, it goes out as a plain file.",
         "Threesea ei voi poistaa tämän tiedoston metatietoja. Ne voivat kertoa, missä, milloin ja millä laitteella kuva on otettu, ja vastaanottaja saa ne kaikki sekä tiedoston nimen: %1. Sellaisenaan lähetettynä se lähtee tavallisena tiedostona."),
    "threesea-la-ended-early":
        ("Threesea ended before it was done",
         "Threesea sulkeutui ennen kuin oli valmis"),
    "threesea-la-ended-early-body":
        ("Your last files or draft did not reach the chat. Add them again there.",
         "Viimeisimmät tiedostosi tai luonnoksesi eivät päätyneet keskusteluun. Lisää ne sinne uudelleen."),
    "threesea-la-error-share-too-large":
        ("That is too much to share at once. Select fewer files or smaller ones, or try again in a few minutes.",
         "Tätä on liikaa jaettavaksi kerralla. Valitse vähemmän tai pienempiä tiedostoja, tai yritä uudelleen muutaman minuutin kuluttua."),
    "threesea-la-verification-partly":
        ("Partly verified",
         "Osittain vahvistettu"),
    "threesea-la-credit-cldr":
        ("Unicode CLDR: those search terms are likely derived from its emoji annotations (Unicode License v3).",
         "Unicode CLDR: nämä hakusanat on todennäköisesti johdettu sen emojimerkinnöistä (Unicode License v3)."),
    "threesea-la-verification-partly-explained":
        ("Partly verified: found in your address book, or checked with their code from a picture. Scanning their code with the camera, in person, verifies them fully.",
         "Osittain vahvistettu: löytyi osoitekirjastasi tai tarkistettiin kuvassa olevalla koodilla. Henkilön koodin skannaaminen kameralla kasvokkain vahvistaa hänet täysin."),
    "threesea-la-qr-verified-picture":
        ("The code in the picture matches %1's key. For full verification, scan their code with the camera, in person.",
         "Kuvassa oleva koodi vastaa yhteystiedon %1 avainta. Täyttä vahvistusta varten skannaa hänen koodinsa kameralla kasvokkain."),
    "threesea-he-remove-devices-again":
        ("Remove",
         "Poista"),
    "threesea-he-leave-device-group-choice":
        ("Your linked devices",
         "Yhdistetyt laitteesi"),
    "threesea-la-leave-device-group-not-finished":
        ("Removing your linked devices did not finish, so they may still be linked.",
         "Yhdistettyjen laitteidesi poisto ei valmistunut, joten ne saattavat olla yhä yhdistettyinä."),
    "threesea-la-remove-devices-first":
        ("Remove the linked devices first",
         "Poista ensin yhdistetyt laitteet"),
    "threesea-la-remove-devices-first-description":
        ("Tries again now, best once you are online. This device stays in the group until they are removed, so it can still remove a lost or stolen computer.",
         "Yrittää nyt uudelleen, mieluiten kun olet verkossa. Tämä laite pysyy ryhmässä, kunnes ne on poistettu, joten sillä voi yhä poistaa kadonneen tai varastetun tietokoneen."),
    "threesea-la-leave-device-group-only":
        ("Leave on this device only",
         "Poistu vain tällä laitteella"),
    "threesea-la-linked-devices-read-failed":
        ("The list of linked devices could not be read",
         "Yhdistettyjen laitteiden luetteloa ei voitu lukea"),
    "threesea-la-image-cache":
        ("Keep pictures in memory",
         "Pidä kuvat muistissa"),
    "threesea-la-image-cache-description":
        ("Chat pictures scroll back in faster: they stay decoded and unencrypted in the app's memory while it runs, until the app locks or a chat is made private. This uses more memory. Private chats load their pictures anew each time.",
         "Keskustelujen kuvat tulevat nopeammin näkyviin takaisin vieritettäessä: ne pysyvät purettuina ja salaamattomina sovelluksen muistissa sen ollessa käynnissä, kunnes sovellus lukittuu tai keskustelu muutetaan yksityiseksi. Tämä käyttää enemmän muistia. Yksityiset keskustelut lataavat kuvansa joka kerta uudelleen."),
    "threesea-he-start-fresh":
        ("Start fresh",
         "Aloita alusta"),
    "threesea-la-start-fresh-goes":
        ("This deletes Threesea's own data on this phone: chats, contacts, media, settings and your Threema ID. Backups and pictures saved to your folders stay.",
         "Tämä poistaa Threesean omat tiedot tästä puhelimesta: keskustelut, yhteystiedot, median, asetukset ja Threema-tunnuksesi. Varmuuskopiot ja kansioihisi tallennetut kuvat säilyvät."),
    "threesea-la-start-fresh-comes-back":
        ("You can then restore your ID from Threema Safe, an ID backup or a Data backup, and your chats from a Data backup. Without one of them, the ID is lost.",
         "Voit sen jälkeen palauttaa tunnuksesi Threema Safesta, tunnuksen varmuuskopiosta tai tietojen varmuuskopiosta, ja keskustelusi tietojen varmuuskopiosta. Ilman jotakin niistä tunnus menetetään."),
    "threesea-la-start-fresh-linked":
        ("Devices linked to your ID stay linked to it.",
         "Tunnukseesi yhdistetyt laitteet pysyvät yhdistettyinä siihen."),
    "threesea-la-start-fresh-understood":
        ("Delete the stored data",
         "Poista tallennetut tiedot"),
    "threesea-la-starting-fresh":
        ("Deleting the stored data",
         "Poistetaan tallennettuja tietoja"),
    "threesea-la-startup-try-again-or-fresh":
        ("Pull down to try again, or to start fresh without the stored data.",
         "Vedä alas yrittääksesi uudelleen tai aloittaaksesi alusta ilman tallennettuja tietoja."),
    "threesea-me-start-fresh":
        ("Start fresh",
         "Aloita alusta"),
    "threesea-la-data-passphrase-forgotten":
        ("Forgotten it? Pull down to start fresh without the stored data.",
         "Unohditko sen? Vedä alas aloittaaksesi alusta ilman tallennettuja tietoja."),
    "threesea-la-credit-maplibre":
        ("The map: MapLibre Native and its Qt bindings (BSD-2-Clause), mapbox-gl-qml by rinigus (LGPL-3.0) and his Sailfish packaging pkg-maplibre-gl-native (MIT). Their exact sources: github.com/maplibre/maplibre-native-qt 3.0.0, github.com/rinigus/pkg-maplibre-gl-native 3.0.0+git2 and github.com/rinigus/mapbox-gl-qml 3.2.1, with this app's patches in its source under scripts/maplibre.",
         "Kartta: MapLibre Native ja sen Qt-sidonnat (BSD-2-Clause), rinigusin mapbox-gl-qml (LGPL-3.0) ja hänen Sailfish-paketointinsa pkg-maplibre-gl-native (MIT). Niiden tarkat lähteet: github.com/maplibre/maplibre-native-qt 3.0.0, github.com/rinigus/pkg-maplibre-gl-native 3.0.0+git2 ja github.com/rinigus/mapbox-gl-qml 3.2.1, ja tämän sovelluksen omat muutokset sen lähdekoodissa hakemistossa scripts/maplibre."),
    "threesea-la-credit-map-data":
        ("Map data © OpenMapTiles © OpenStreetMap contributors (ODbL), from Threema's map server.",
         "Karttatiedot © OpenMapTiles © OpenStreetMap-tekijät (ODbL), Threeman karttapalvelimelta."),
    "threesea-la-key-confirmation-notification":
        ("Threesea is waiting for your OK",
         "Threesea odottaa vahvistustasi"),
    "threesea-la-key-confirmation-notification-body":
        ("After a restart the phone asks once whether Threesea may read the key to its data. Confirm that to receive messages.",
         "Uudelleenkäynnistyksen jälkeen puhelin kysyy kerran, saako Threesea lukea tietojensa avaimen. Vahvista se, jotta voit vastaanottaa viestejä."),
    "threesea-la-credit-finnish":
        ("The Finnish translation: jmpx448 (github.com/jmpx448), a first-time contributor.",
         "Suomennos: jmpx448 (github.com/jmpx448), ensikertalainen avustaja."),
    "threesea-la-starting-video-call":
        ("Starting video call",
         "Aloitetaan videopuhelu"),
    "threesea-la-starting-call":
        ("Calling",
         "Soitetaan"),
    "threesea-me-open-in-maps":
        ("Open in maps app",
         "Avaa karttasovelluksessa"),
    "threesea-la-no-maps-app-installed":
        ("No maps app installed",
         "Karttasovellusta ei ole asennettu"),
    "threesea-me-copy-coordinates":
        ("Copy coordinates",
         "Kopioi koordinaatit"),
    "threesea-he-choose":
        ("Choose",
         "Valitse"),
    "threesea-la-location-map-point":
        ("A point on the map",
         "Piste kartalla"),
    "threesea-la-location-on-map":
        ("Choose on the map",
         "Valitse kartalta"),
    "threesea-la-location-map-server":
        ("The map comes from Threema's map server, which learns the area shown. The places on it come from Threema's places server, which gets the middle of the map.",
         "Kartta tulee Threeman karttapalvelimelta, joka saa tietää näytetyn alueen. Kartan paikat tulevat Threeman paikkapalvelimelta, joka saa kartan keskipisteen."),
}



# fill() as in translations-de.py.
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


PATH = "translations/harbour-threesea-fi.ts"
if not os.path.exists(PATH):
    # First run: start from the base file (lupdate keeps it up to date after that).
    base = ET.parse("translations/harbour-threesea.ts")
    base.getroot().set("language", "fi")
    base.write(PATH, encoding="utf-8")
missing, stale = fill(PATH, FI)
for key in stale:
    print(f"English changed, translate again: {key}", file=sys.stderr)
if missing:
    print(f"{len(missing)} texts without Finnish yet (left unfinished)", file=sys.stderr)
