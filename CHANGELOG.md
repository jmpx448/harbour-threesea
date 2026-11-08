# Changelog

All notable changes. Versions follow `MAJOR.MINOR.PATCH`; each release is built from the
`release` branch and tagged `v<version>` (see [docs/release.md](docs/release.md)).

## [0.3.0] - 2026-11-08

A large round of fixes after a review of the whole app, and the features from Threema for Android
that were still missing for everyday use.

Privacy, please read:

- **Pictures sent from Threema Web kept their metadata.** Before this release, JPEG, PNG and WebP
  pictures sent from Threema Web went out with their EXIF data: where and when a picture was taken,
  and with which device. They are now made anew the way the phone's own pictures are, without any
  of it. Pictures already sent can't be called back.
- **Pictures sent from the phone could carry text from their file.** A JPEG's comment or a PNG's
  text could pass through when a photo was scaled, cut or turned before sending. Nothing of the kind
  goes along now; GIFs keep only their frames and timing, and forwarded pictures are cleaned too.
- **Older logs may hold place names.** Up to this release a place name you typed in Send location
  could end up in the log. New logs don't have it. To remove old ones before you share a support
  package: Settings → Advanced → View log → Clear log.

Changed:

- Linking a phone number now needs its country code, and the dialog shows the number the code will
  go to. It used to take the country from the phone's language, which could send a linking request
  for the wrong number.
- Videos sent as media leave without their metadata (place, time, device); only what a player
  needs stays. A video that can't be cleaned is asked about: send it as a file, as it is, or don't.
- Forward secrecy follows what the other side's app says it supports, as Threema's own apps do: a
  contact who links Threema for Desktop gets one line in the chat and messages without it, and
  when they unlink it comes back. Blocked contacts get no forward secrecy session and no answers.
- Calls with contacts you haven't verified go only through Threema's relay, so your IP address
  doesn't reach them. Joining a group call with four or more others starts with the microphone off.
- While the app is locked, notifications only say that something arrived: no name, no text and no
  Reply.
- Profile pictures: the setting is now called "Show profile pictures". As on Android, pictures that
  arrive are stored and the setting decides whether they are shown; with it off, nobody is asked
  for theirs. Contacts are asked for their picture once after a restore, not each time a chat opens.
- Deleting a contact deletes the chat with them too, as on Android. Their messages in groups stay.
- Removing the ID from this device stops calls and backups, removes linked devices first when
  online, and replaces the database file and its key, so no copy of the old file can be opened.
- Uploads and downloads that fail try again by themselves, after a minute and then up to eight
  hours apart, five times. A transfer gives up when nothing has moved for 100 seconds, not after a
  fixed time, so large files on slow networks go through.
- Keep messages and Clean up go by when a message arrived, not by the sender's clock, and leave
  notes groups and hidden private chats alone. A shorter Keep messages period counts down first.
  The chat list orders chats by when their messages arrived, too.
- Scanning a code asks before it pairs a browser, links a device or adds a contact, and a contact's
  code is refused once it has expired or names another key.
- Pull-down menus have at most four items: Export chat moved to the contact and group pages, and
  sharing the profile picture is a choice on the contact's page. "Verify by QR code" stays the
  contact page's first pulley item.
- Errors are plain sentences everywhere; the app no longer shows internal English.
- With IPv6 off, every connection uses IPv4, calls included.
- Contacts are sorted by their letter whatever its accents.

Added:

- Finnish: the whole interface is translated, by [jmpx448](https://github.com/jmpx448), a
  first-time contributor ([#1](https://github.com/penttimus/harbour-threesea/pull/1)).
- Draw on a photo before sending it (pen, text, colours, undo), also on a picture you received, and
  send it back.
- Send a place: a place nearby or one found by name, from Threema's own map server, or a point
  picked on Threema's map. A received location opens on that map when you tap it. The rows say what
  goes to the server before anything does.
- A chat's polls on a page of their own, and a reminder of open polls under the chat's header.
- Profile and group pictures: cut them to a square, or take one with the camera.
- Voice messages: pause the recording, listen to it before sending, and discard it with a countdown.
- Threema's text formatting (*bold*, _italic_, ~strikethrough~) shows as formatting.
- Search the emoji panel by a word.
- The picture viewer swipes through a chat's pictures and videos, with Save and Show in chat.
- Forward and share to several chats at once, after a look at what goes, with a caption you can
  change.
- Share a message's text with other apps; copy, save or share several selected messages; select in
  the media gallery.
- Send a failed message again, and stop an upload or a download, from its bubble.
- Hold a reaction to see who reacted.
- Calls (Full): a notification with Answer and Decline for an incoming call, a loudspeaker switch,
  the screen going dark at the ear, a ringtone and vibration of their own, and a line and a
  notification when someone starts a group call.
- First start: restore the ID and the chats from a data backup.
- ID backups as a QR code to show or share, and restored by scanning it.
- Threema Safe: find a forgotten ID by its linked number or address; a warning when backups keep
  failing; a new ID is asked about Threema Safe and reminded while it has no backup of any kind.
- Threema Web: names for sessions, Remove all, a switch to turn it off, a notification while a
  browser is connected, the self-hosted servers you allowed (and taking them back), and accepting
  a self-hosted server's new certificate. The browser now follows what changes on the phone:
  contacts, groups, pictures, deleted messages, mute and unread marks.
- Chat list: Lock now, mute from a chat's menu, and the search finds chats and contacts by name.
  Contacts, the share page and the member picker can be searched too.
- The cover opens the waiting chat directly.
- Settings → Privacy: which country numbers without a country code belong to, for the address book.
- Links shared from the Browser; memory cards in the file picker and for data backups.
- Copy and share Threema IDs.
- The link in Threema's verification SMS confirms the number.
- Why Threesea is offline (what Threema's servers said) shows under the chat list's header, and as
  a notification when the window is closed.
- The licence texts and a link to the source under Settings → About.

Fixed:

- Full: Threesea starts again in the background when the home screen restarts. It used to end with
  the home screen and stay away until opened, which could happen a minute after login.
- Full: after a restart the phone asks once whether Threesea may read the key to its data. Sailfish
  OS always asks this, and nothing arrives until it is answered, so a notification now says what the
  prompt is for.
- Numbers written as `+49 (0) 171 …` keep no extra zero, when linking and in the address book.
  Hungarian national numbers lose their whole `06` prefix.
- Linking a phone number or an e-mail address that is already linked to your ID says so, rather than
  showing nothing.
- A manual address book sync (turning it on, or pulling to refresh the contacts) always tells how it
  went: new contacts, how many entries it checked, no phone numbers or addresses found, or the error.
  It takes only one-to-one matches, as Android does, and a name from the address book follows later
  changes there.
- Photos and videos of private chats no longer land in the Gallery folder, private groups and lists
  stay out of Threema Web while hidden, and the settings file no longer names your chat partners.
- A received message is stored completely before the server is told, so a full disk or a failing
  store no longer loses one; it stays on the server and comes again. Read and delivery receipts,
  and forward secrecy answers, wait until they have really gone out.
- What you type stays in the input until the message is accepted; a draft keeps its mentions and
  its quote; a refused file comes back with its caption; a swipe no longer throws away picked files
  or a recording.
- A chat stays where you are reading when messages arrive or change, reopens where you left it,
  and a tap on a quote goes to the quoted message.
- Groups: being added by someone who isn't a contact, messages written while offline, leaving
  right after writing, being removed rather than leaving, new members getting the group's name,
  picture and running call, and changes made on Threema Desktop.
- Revoked IDs: the chat says so and nothing more goes to them.
- Notifications: a reply that couldn't be sent says so and keeps its words in the chat; an edited
  or deleted message updates its notification quietly; a backlog after being offline alerts once.
- Data backups: a damaged or too large chat is left out and named instead of stopping the import;
  the result page says what didn't come along; scheduled backups no longer fill the disk or run
  hourly after a failure; a running backup can be stopped.
- Threema Safe: turning it off deletes the backup on the server; a restore keeps the key of a
  contact you verified in person, and runs the same steps afterwards as Android.
- The connection comes back when the network changes and when you return to the app, tries IPv4
  after a few seconds where IPv6 doesn't work, and stops waiting for a write after 30 seconds.
  Another device taking over the ID no longer makes Threesea log in again and again.
- Linking and removing devices say what really happened.
- A start that fails says why (storage full, damaged, from a newer version) and offers a way on;
  upgrading a large store shows a hint.
- After an update the running app notices it and ends; a notification opens the new version.
- Send location sends only a recent position, or offers the last known one with its time.
- Videos play again after their end and the slider follows; a picture turned by its EXIF data
  shows in its own proportions; a damaged picture says so.
- Pictures from linked devices, read receipts and typing reach the other devices and come from them.
- Many texts now say what a setting really does.

Faster:

- Long work no longer holds up the app: Threema Safe, linking, exports and maintenance run beside
  everything else, and large files are read, encrypted and decrypted away from it, within a
  memory budget.
- Searching a long history, the media gallery and the chat list's counts use indexes and pages.
- The emoji panel opens faster, and starting the app reads the emoji tables only when needed.

Not tested yet: the headset and Bluetooth buttons on the lock screen's media controls, which the
0.2.0 notes below say work, have not been tried on a phone. Calls, the loudspeaker switch and
everything else in `docs/voice-video-device-checklist.md` still wait for a phone too.

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
