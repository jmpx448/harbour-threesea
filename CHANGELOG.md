# Changelog

All notable changes. Versions follow `MAJOR.MINOR.PATCH`; each release is built from the
`release` branch and tagged `v<version>` (see [docs/release.md](docs/release.md)).

## [Unreleased]

Changed:

- Linking a phone number now needs its country code, and the dialog shows the number the code will
  go to. It used to take the country from the phone's language, which could send a linking request
  for the wrong number.

Fixed:

- Numbers written as `+49 (0) 171 …` keep no extra zero, when linking and in the address book.
  Hungarian national numbers lose their whole `06` prefix.
- Linking a phone number or an e-mail address that is already linked to your ID says so, rather than
  showing nothing.
- A manual address book sync (turning it on, or pulling to refresh the contacts) always tells how it
  went: new contacts, how many entries it checked, no phone numbers or addresses found, or the error.

## [0.2.0] - 2026-11-03

Changed:

- The app now tells Threema's servers what it is when it connects, as Threema suggested: Threesea, its
  version, the language and country, the device model and the Sailfish OS version. Your linked
  devices show it as "Threesea (device model)".
- The license key is checked again once a day, not only at setup, as Threema's own apps do. If
  Threema refuses it, Threesea goes offline and asks for a valid key; your chats stay. Having no
  connection, or a server with problems, never counts against you.
- The About page and the README now say what Threema told us about third-party clients, in place of
  the general warning.
- A new app icon in Sailfish's style: a green speech bubble over three waves. The old one looked too
  much like Threema's logo.

Added:

- Voice messages and videos show on the lock screen's media controls, and headset and Bluetooth
  buttons work on them. The chat's name is hidden there under "Hide content on the cover".
- Report spam, in 1:1 chats and groups: a chat from an unknown contact offers a report to Threema
  under its header, and to block the sender and delete the chat along with it. It sends your ID and
  the sender's ID and nickname (for a group also its ID and number of members), never messages.
  The requests are checked against Threema's server, but a complete report has not been sent yet.
- French: the whole interface is translated (Paramètres → Langue of Sailfish OS decides; a
  phone set to French shows Threesea in French).
- Edit a photo before sending it: tap the picked photo above the input to cut and turn it.
- Export a chat: the chat's pull-down menu writes the messages as a text file into a ZIP in
  Downloads, with the pictures and files of the chat if you want them, and with a password if
  you give one.
- Settings → Media and storage → File size: how large a file may be to be downloaded by
  itself. Previews always come, larger files wait in the chat until you tap them.
- Voice messages: playback speed 1×, 1.5× and 2× on the bubble.
- Settings → Chats: play voice messages at the ear (on or off).
- Settings → Notifications: sound and vibration for a message arriving in the chat you are
  reading.
- Settings → Calls: group calls on or off (your linked devices learn the choice).
- Settings → Appearance: message text size, a chat background, and whether contacts' profile
  pictures are accepted.
- Settings → Media & storage: keep messages for a week up to a year (older ones and their media
  are deleted by themselves), and a video size for sending — 480 or 848 px with the sound kept
  as it is (Full flavour, where the phone has an H.264 encoder).
- Linked devices: name this phone for the other devices, and give each linked device a name of
  your own.
- Chat list: mark a chat as unread, and as read again.

Fixed:

- A vote with several choices in a single-choice poll, which only a broken client can send, is now
  ignored, and votes for choices a poll doesn't have don't count.
- When Threema's servers asked the app not to reconnect by itself (for example because another
  device took over the ID), it came back after a second if the connection had been up for more
  than a minute. It now stays away as intended.
- With many favourite GIFs the emoji panel's GIF tab, and the whole app with it, slowed down
  badly. GIFs are now read and played away from the interface, only those on screen play, and
  nothing plays while you scroll. GIFs in chats and in the viewer benefit too.
- Removing a favourite GIF keeps your place in the list.
- Videos stayed black while their sound and time ran.
- A voice message was hard to get rid of: closing the app now stops it, leaving a chat drops a
  paused one, and one left paused for 10 minutes goes by itself. It no longer starts again by itself
  when the audio comes back, or when something covers the proximity sensor after you paused it.
- Renaming a linked device said "Something went wrong: invalid request" and did nothing.
- In the chat list, an emoji of a longer message could appear a line below where it belongs,
  over the text: Qt cannot elide a text that has pictures in it. The preview is now cut to
  length before it is drawn, and it shows the message as one flowing line, as Jolla's lists do.
- Notes (a group with only yourself): what you write there now reaches your other devices, and
  its pictures stay inside the device group instead of going to the public blob server.
- Pictures and files sent from another device (Threema Desktop, the Web client) stayed as a
  placeholder on the phone: they are now fetched like received ones — the preview always, the
  file itself as "Download automatically" says. Older ones arrive on the next start, so if you
  have sent a lot from Desktop, the first start after this update is best done on Wi-Fi (or set
  Settings → Media and storage → File size first).
- The cover named the wrong person: it showed whoever wrote last, even after that chat had been
  read. It now names the newest chat that still has something unread.
- Voice messages held to the ear played scrambled silence: they now go through the same audio
  route as calls (Full flavour).
- The sound between two voice messages played one after the other was loud and sharp; it is a
  soft, quiet bell now.

Faster:

- Scrolling back through a long chat no longer stops while the next block of messages is
  loaded: the block arrives in one piece, earlier, and the day band above the messages does
  less work while you scroll. Pictures are decoded away from the interface instead of in it,
  and a GIF scrolled out of sight stops playing.
- Scrolling back no longer jumps to the newest message when a block of older ones arrives.
- A chat keeps about 250 messages around what you are reading instead of everything you have
  scrolled past: scrolling the other way loads them back, and the quick scroll to the bottom
  fetches the newest ones again.
- The chat list is built from a per-chat summary the database keeps up to date, with the rest
  read in one go instead of once per chat: about five times quicker with many chats.

## [0.1.0] - 2026-10-27

First version. **Experimental:** a research project developed with AI assistance; read
the disclaimer in the [README](README.md) before using it with your Threema ID, and keep a
backup of the ID.

Two flavours for Sailfish OS 5.1 or later (aarch64, armv7hl, i486); install one of them:

- **Full** (`harbour-threesea-full-…rpm`): connects at login in the background; 1:1 and group
  calls with audio and video.
- **Lite** (`harbour-threesea-lite-…rpm`): follows the Jolla Store's rules; stays connected once
  opened; no calls.

What it does:

- Threema Libre license check, ID creation, ID backup and Threema Safe backup/restore.
- Data backups in the format of Threema for Android: import one from Android (contacts,
  groups, lists, pictures, message history with media, polls and reactions, and the used
  nonces for replay protection) and create your own, by hand or daily/weekly, into Documents
  or Downloads. Checking a backup shows what it holds on its own screen: swipe to import or
  to cancel, follow the progress, and a results page sums up what arrived. Not yet restored
  on a real Android device.
- 1:1 chats with forward secrecy, groups, distribution lists, delivery and read receipts,
  typing indicators, mentions.
- Photos, videos, files, voice messages, GIFs, locations, polls, quotes, reactions, editing and
  deleting messages, starred messages, chat search, media gallery.
- Profile pictures, contact verification by QR code, address book matching (opt-in, hashed).
- App lock and private chats; the database is encrypted (SQLCipher).
- Notifications with quick reply; sharing from other apps; Threema links.
- Multi-device with Threema for Desktop; Threema Web.
- English and German.
- An optional check for new versions on GitHub (asked once; off unless you agree).

Not tested on phones yet: calls, video playback, camera capture and some other hardware-bound
features were tested only in the Sailfish OS emulator.

Known open points (see the [audits](docs/deep-audit-2026-10-27.md)):

- Threema Safe backups and linking a device still hold up other work in the core while they
  wait for the network.
- With very long histories (100 000 messages and more), the chat list and search get slower.
