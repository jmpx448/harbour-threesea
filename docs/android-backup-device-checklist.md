# Android data backup: device checklist

Two checks that need real devices and real data, so they cannot run here. Everything else
(file format, nonces, polls, round trip through our own importer, the emulator smoke test) is
covered; see docs/STATUS.md, "Restore fixes and Android backup import".

Note for each step: OK, or what you saw (a screenshot of the page and the numbers it shows help
most). Never share the backup password or the ZIP file.

## Before you start

1. Build fresh RPMs from the current `HEAD` (the ones in `RPMS/` can be older than the backup
   work) and install on the phone: `devel-su pkcon install-local harbour-threesea-full-….rpm`.
2. Back up the app's data on the phone, because an import merges into the running identity:
   close the app, then copy `~/.local/share/org.threesea/harbour-threesea/` (the database and
   `media/`) somewhere safe, e.g. `cp -a ~/.local/share/org.threesea/harbour-threesea
   ~/Documents/threesea-copy`. To go back: close the app and copy it back.
3. For a log while testing (optional, in a terminal on the phone or over SSH):
   `devel-su journalctl -f | grep -i threesea`. The log never contains message text or the
   password.

## A. Import a real Android archive on a Sailfish device

Put the ZIP from Threema for Android into `Documents` or `Downloads` (the app may only read
those folders, plus Pictures, Videos and Music).

1. Settings → Backup → "Android data backup". Backup file: pick the ZIP. Enter its password,
   "Check backup".
   - Expect: format version (Android's current backups: 27), "This device's Threema ID", the
     number of chats, group chats, media files and pictures.
   - These show on their own screen. Swiping back ("Cancel") imports nothing. With another
     Threema ID, "Import" cannot be accepted: the import only merges into the same ID.
   - This also shows the reader accepts how Android lays out its archive: it refuses entries
     that share bytes, and only synthetic archives have been through that so far. Do this with
     one ordinary backup and, if you can make one, one over 4 GiB (ZIP64, a phone with many
     media files). A refusal on such a file is a bug in the reader: note the size and the log
     line.
2. Swipe forward ("Import"). The progress screen first takes over contacts, groups, lists and
   pictures, then the message history and media with a progress bar ("Chat x of y", media,
   the number of messages so far). When done it goes back to the backup page by itself.
   - Watch the progress; note roughly how long it takes for how many messages and media.
   - Tap "Stop" halfway once, or close the app from the cover; then check the backup again
     and import again: it should continue and end with the same totals, nothing twice.
3. The results on the backup page:
   - "Imported": counts for added and "Already on this device"; "Contacts with a different
     key" should not show. The log's "Android backup metadata imported" line has the number
     of used nonces taken over (`nonces`): above 0 for any phone that received messages.
   - "Message history": imported messages, media files, reactions, polls; "Messages left out"
     only for status lines (e.g. group status lines), and the modified-poll lines.
   - Rows like "Chats left out", "Groups left out" or "Entries left out" show only above 0, and
     the headline then says the backup is imported except for what is listed. Note any you see.
   - A contact added by the import offers editing and deleting messages shortly after the
     import, when online, without pulling the Contacts page.
4. Look at the result:
   - The chat list: old chats appear, with their latest message. Your live messages from
     before the import are still there, in time order around the old ones.
   - One chat with photos and voice messages: open them; they should play/show without
     downloading.
   - One chat with a poll: the question, the choices, who voted what; a closed poll shows as
     closed. If a contact votes in an open old poll now, the vote should show in that poll.
   - Stars, quotes and reactions on a few messages.
5. Check and import a second time: everything counts as "already on this device", nothing
   new appears.
6. Battery and memory: if the phone got hot or the app was killed during step 2, say so (with
   the archive's size).

## B. Restore one of our backups in Threema for Android

Needs an Android device with Threema. **Restoring replaces everything in Threema on that
device**, so use a device (or an Android profile) without data you need, and keep the Sailfish
device as the main one.

1. On the Sailfish device: Settings → Backup → "Android data backup" → "Create backups". Backup folder:
   `Documents` or `Downloads`; a password (at least 8 characters, like Android asks); "Create
   backup now". Expect "Backup written: threema-backup-….zip".
2. Copy that ZIP to the Android device (USB, cloud, …) into a folder Threema can open.
3. Install Threema for Android, and on its first screen choose to restore a data backup
   ("Restore" → "Data backup"), pick the ZIP, enter the password.
   - If it refuses the file: the exact message, and the Threema for Android version.
4. After the restore, check on Android:
   - Your ID and nickname; contacts with their names and verification levels; groups with
     members; distribution lists.
   - The chat list holds the 1:1 chats, without a new message having arrived, and groups and lists
     sort by their newest message (the file's last update).
   - A few chats: texts in order, photos/videos/files open, voice messages play, quotes,
     stars, reactions, a poll with its votes.
   - Group and contact pictures.
5. Send yourself one message from another device right after the restore: it should arrive
   once (the used nonces went along). Older messages must not show up a second time.
6. Afterwards, on that Android device, the ID is now active there as well: unless you use
   multi-device, go back to the Sailfish device and check it still receives messages (if not, the
   Android device took the ID over; uninstall Threema there).

## What to note

- Step results (OK / what happened), the numbers from the import summaries, the Threema for
  Android version for B.
- Anything that looked wrong, with a screenshot; the journal lines around the time if you
  had the log running.
