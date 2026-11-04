# Status

## Phase 0 — Toolchain and skeleton (2026-10-21)

Done:
- Upstream submodule pinned to `6.5.3-1216`; libthreema builds as a dependency of `core`.
- `core` exposes a C ABI (`threesea_core_version`, `threesea_libthreema_selftest`,
  `threesea_string_free`); host `cargo test` and `cargo clippy` pass.
- Cross-build for aarch64, armv7hl, i486 with Rust 1.95 in a Docker-only Sailfish SDK
  (`scripts/sdk-build.sh`); RPMs for all three pass `mb2 check` (Harbour validator; only warning:
  binary not stripped).
- Minimal Silica app: conversation list placeholder, About page showing the libthreema self-test,
  cover, id-based translations scaffold, placeholder icon, Sailjail `.desktop`.

Not done yet (Phase 0 exit criteria):
- ~~Run on emulator/device~~ → runs in the official 5.1.0.11 emulator (docs/emulator.md).
- `cargo audit` in CI, C++ unit-test target, CI pipeline itself.
- ~~Decide minimum SFOS version~~ → 5.1 (ADR 0004).
- Strip the binary in the spec.

## Phase 1 — Identity (in progress, 2026-10-21)

Done (tested on host and in the emulator):
- Build target lowered to SFOS 5.1.0.11 (ADR 0004).
- libthreema patch series (ADR 0005): serial license on identity creation; public directory
  challenge solver. Both with upstream-style unit tests; libthreema's 3650 tests still pass.
- Core: single core thread + JSON FFI (docs/ffi.md); pinned HTTPS client restricted to Threema
  hosts; SQLCipher store with migrations; license check; identity creation with license; ID
  backup export; ID backup restore incl. `identity/fetch_priv`.
- Bridge: database key in Sailfish Secrets (device-lock collection, owner-only), never
  regenerated when a database exists; backend with JS callbacks.
- UI: startup, license, ID setup (create/restore), restore backup, settings (Threema ID),
  export ID backup. Id-based translations with engineering-English fallback.
- Emulator, end to end: license page → bogus key → Threema server answers "This license key is
  invalid or expired." over pinned TLS; encrypted DB reopens after restart with the key from
  Secrets.
- Harbour validator: 18/18 passed (with Secrets permission, libsailfishsecrets, libcrypto).

Verified in the emulator with a real Threema Shop license (portrait-native screen):
- license accepted → ID created (a test ID) → settings show the ID
- ID backup exported → app data wiped → license re-entered → ID restored from the backup
  (`identity/fetch_priv` with the challenge solver) → same ID.

Open finding: after a reboot, Sailfish Secrets shows its own "Authorize" prompt once before the
app gets the database key (emulator has no device lock code). Verify on a phone with a lock code
that the collection unlocks silently with the device; important for the Chum background service.

Threema Safe (2026-10-21), verified in the emulator with the test ID:
- KDF matches upstream's test vector; live `config` over pinned TLS.
- Enable with password (weak-password warning like Android) → first backup uploaded
  (479 bytes, `safe-xx.threema.ch`) → app data wiped → wrong password gives "no backup found" →
  correct password restores the ID with Safe still enabled.
- Known gaps: profile picture not backed up yet (no profile pictures yet); "last backup" time
  shows 12 h format although the device is set to 24 h (Silica `Format` in the sandbox?);
  automatic backup only runs while the app runs (background service comes in Phase 4).

Contacts (2026-10-21), verified in the emulator:
- Add by Threema ID via `identity/fetch_bulk` (ECHOECHO found; made-up ID → "does not exist or
  was revoked" notice), alphabetic sections, verification dots, contact page with fingerprint,
  edit names, delete with remorse; nickname in settings.
- Safe round trip with contacts: "Back up now" (0.7 kB) → wipe → Safe restore → "Echo Test"
  (ECHOECHO) and nickname are back.
- Stored model is libthreema's own `Contact` (so Phase 2 can implement its `ContactProvider`).

Phase 1 wrap-up (2026-10-21):
- Privacy settings (read receipts, typing indicator, block unknown) stored as the Safe `settings`
  object, so every field round-trips through Threema Safe (verified: wipe → restore → switch
  state back).
- "Remove ID from this device": confirmation dialog + remorse, wipes local data (ID not revoked).
- `scripts/update-translations.sh` refreshes `translations/*.ts` (109 strings; German not
  translated yet).

**Phase 1 done**, with these items moved (all done now): QR verification → Phase 3/4 (camera), hashed
phone/e-mail lookup and address-book sync → Phase 4 (opt-in), ID revocation → later.

## Phase 2 — Messaging core (started 2026-10-21)


Done (verified in the emulator and with a live host test against the real chat server):
- Chat server connection with libthreema's CSP + CSP-E2E state machines on the core thread;
  keepalive, reconnect with backoff, "Connecting…/Offline" in the UI.
- 1:1 text messages both ways with ECHOECHO; delivery states (pending clock → sent/delivered/read
  check mark), delivery and read receipts (respecting the global setting and per-contact
  overrides); incoming typing indicators shown in the chat header.
- Offline queue: message written while the chat server was blocked was sent after reconnect.
- SQLCipher messages + nonce tables (migration 3); libthreema provider traits on our store.
- UI: chat list (last message, unread count, time, delivery state, delete with remorse), new
  message picker, chat page in the Jolla Messages layout (bubbles on Theme colours, input at the
  bottom, push-up/pull-down contact details), notifications per chat (im.received category)
  that open the chat via a D-Bus remote action, cover with unread count + last sender and a
  "new message" cover action.
- `cargo test -- --ignored echo_roundtrip` with `THREESEA_TEST_BACKUP(_PASSWORD)` runs the full
  send/echo round trip from the host (never commit the backup).

Phase 2, continued (2026-10-21):
- Chat page in the Jolla Messages layout the user asked for: name on top with the avatar
  (initials placeholder) to its right; contact details as an attached page (swipe left).
- Harbour-compatible background running: the process keeps the connection after the window is
  closed (setting "Keep running in the background", default on). The .desktop launches via
  D-Bus (X-Maemo-Service + Sailjail ExecDBus) so reopening shows the existing process' window;
  Nemo.KeepAlive BackgroundJob wakes every 5 min and verifies the connection with an echo.
  Verified in the emulator: close → message arrives → notification → tap → same process opens
  the chat. Harbour validator 18/18.
- Our nickname is already sent: it travels in the encrypted message metadata (incoming contact
  nicknames are applied by libthreema).

Moved to Phase 3: typing indicators (done: decoded/encoded in vendored libthreema, sent from the
chat input per the spec's 10 s / stop rules, global setting and per-contact override). Forward
secrecy (LIB-42) is done too, see Phase 3. Still open: Chum systemd service (Phase 4), measuring battery impact on a device (Phase 4).

**Phase 2 done** for the Harbour flavour.

## Phase 3 — Groups and media (started 2026-10-21)

Groups (1st pass): all group control and content messages in vendored libthreema, receive steps,
fan-out sending with per-member acks, create/rename/members/leave/dissolve, sender names in
bubbles, group page attached to the chat, groups in the chat list. Tested in the emulator:
created a group with the user's ID and sent to it. Group pictures and polls (1:1 and groups:
create, vote, close; verified with a vote from the official app).

Typing indicators (both directions, verified with the official app on a phone) and 1:1
reactions done: `Reaction` 0x82 in vendored libthreema, legacy 👍/👎 fallback, `REACTION_SUPPORT`
announced, reactions table + outbox (migration 5), quick-reaction row in the message context
menu and chips under the bubble (tap toggles ours). Edit and delete (1:1): `EditMessage` /
`DeleteMessage` in vendored libthreema, features announced, edit in the chat's own input box
(user request: no separate screen), "Edited" marker, delete for everyone with remorse →
"Message deleted" placeholder, delete on this device.
Files and images (1:1): `file` message in vendored libthreema; blob server client; encrypted
media store (key derived from the database key); receiving (thumbnails, images ≤ 20 MB
automatically, files on tap), picture bubbles with concentric corners, full-screen viewer; sending
via Sailfish pickers from a push-up menu under the input (photos re-encoded ≤ 2048 px without
EXIF, thumbnail), attachment bar above the input with the text as caption, upload retry.
Verified with the official app (receiving). Saving to Downloads (context menu; tapping a non-image file saves and opens it with the system
app). Voice messages: record with the send button (empty input), play in the bubble; platform AAC,
else Opus fallback (tested in the emulator: recorded, sent, delivered and read on the user's
phone, played back).

Forward secrecy (1:1) done:
- libthreema (vendored, ADR 0006): ratchet + session + processor + incoming-task integration +
  outgoing encoders; 3664 libthreema tests incl. Android's KDF vectors and an Alice/Bob
  conversation; app-level test through libthreema's real incoming task with our SQLCipher store.
- Feature mask 0x40 published; ECHOECHO (no FS) keeps working in plain mode (verified in the
  emulator); status lines in the chat for established/reset/terminated.
- **Verified against the official Threema Android app** (2026-10-21, a test ID after
  unlinking its multi-device client — Android disables FS while multi-device is active): the
  official app's Init was accepted, its 4DH-encapsulated delivery receipts decrypted ("first 4DH
  message"), our encapsulated replies delivered and receipted, no Reject/Terminate.
- Lessons: (1) feature masks go stale → contacts are refreshed from the directory daily and an
  existing FS session counts as FS support; (2) "Block unknown senders" silently drops first
  messages from strangers (works as designed, but tests must keep it off).

Notifications (done early, Phase 4 item): one per chat, group title + "Sender: text", quick
reply and Mark as read from the notification, taken over after restarts, settings for on/off
and hiding the content (lock screen privacy); status lines don't notify. Tested in the emulator
via ECHOECHO.

Quotes (1:1 and groups): long-press → Quote, "Quoting …" bar above the input, quote block in
the bubble (tap: jump to the original). Verified with the official app.

Profile pictures: own (Settings, share with everyone/nobody), contacts' shown in chat header and
contact page. Chat header: Threema ID for unsaved contacts, verification dots for saved ones.

Share target: "Share → Threesea" from other apps (files, photos, text); pick a chat, then it
opens ready to send. Contacts attached to the chat list (swipe left).

Contact verification by QR code (camera or picture; "My QR code" in Settings) and opt-in
address book matching (daily, hashed) are done. Tested: QR from a picture in the emulator,
matching against the live directory with a fake entry; camera and real address books need a device.

Threema links (Phase 4 item): `threema://` and `https://threema.id/…` / `go.threema.ch/…` open
in Threesea (compose, add contact, contact, license key); links, e-mail addresses and phone numbers
in messages are tappable. Tested in the emulator via libcontentaction and the D-Bus call.
Hardening on the way (ADR 0008): text from others was rendered as rich text in labels (chat list
preview); now plain everywhere, and the QML engine cannot reach the network.

Mentions (groups): "@Name" / "@Me" / "@All" (bold, highlight colour in bubbles), chat list and notifications; typing
"@" offers the members. Tapping a notification while Threesea is not running now opens that chat.

Per-chat notifications: "Notifications" on the contact and group pages (on, off for 1 h / 8 h /
1 day / 1 week, off, groups: only when mentioned); muted chats show the silent icon in the chat
list. Tested in the emulator: muted Echo chat did not notify, unmuted did.

Forwarding (message menu → Forward → pick a chat): texts and files (photos, voice messages,
documents; files are uploaded again with a new key). Tested in the emulator: a group photo
forwarded to ECHOECHO was delivered, read and echoed back. Contacts and chat list: the row opens
the chat, the picture the details (user request).

Blocking: contact page pulley Block/Unblock, Settings → Privacy → Blocked contacts (unblock by
long press, block any Threema ID from the pulley). Blocked chats show "You have blocked this
contact" instead of the input; the core refuses sending (also from notifications). Incoming
messages are discarded by libthreema's receive steps (its tests). Tested in the emulator with
ECHOECHO (block, refused reply, list, unblock).

German translation complete (266 texts, Sailfish conventions "du" and ß, Threema's own terms),
plural forms for English and German. Checked in the emulator with the user locale set to de_DE.

Locations: send the current position (push-up menu → Send location; optional place name),
received locations as bubbles with name/address/coordinates, tap opens a maps app (geo:),
long press copies the coordinates, forwarding works. No map tiles or address lookup (§7.6).
Tested in the emulator with a fixed test position (the emulator has no positioning): sent to
ECHOECHO and echoed back.

Search: Chats pulley → Search (all chats), contact/group page pulley → Search in chat. Texts,
captions, file and place names, poll questions; mentions match by name; results with the chat or
sender, time and the highlighted match; tapping opens the chat at the message (loading older
messages if needed). Runs in the core on the SQLCipher store (no separate index).

Pinned chats: long press → Pin to top / Unpin / Move up / Move down; a "Pinned" section on top of
the chat list in the user's order (migration 13, local only). UI feedback round: square pictures,
People-style contact/group header (tap picture: full screen), notification choice as a dialog
confirmed by swipe, chat search in the chat's push-up menu.

Videos: push-up menu → Send video (Sailfish video picker); sent as media with a thumbnail from
the system thumbnailer (square: its image provider only gives centred squares; Nemo's Thumbnail
item hangs on SFOS 5.1, see ChatPage) and the length if the media service knows it. Video
bubbles show a play button and the length; tapping plays in the app, streamed from memory
(VideoPlayer, nothing decrypted on disk); without a media service (emulator) the video is saved
to Downloads and opened in the system player as before. Tested in the emulator: sent a video
to ECHOECHO, delivered, read and echoed back with thumbnail. In-app playback needs a device.

Selecting messages: chat pulley → Select messages; in the chat itself, tapping bubbles selects them (lit like pressed), docked
panel with Delete (this device, remorse) and Forward (all in order, core-side `forwardMessages`;
only when every selected message can be forwarded). Tested in the emulator: deleted four, forwarded
three (two texts, a location) to ECHOECHO in order.

Distribution lists: chat list pulley → New distribution list (name optional, recipients); own
chat with the theme's "message forward" icon; texts, photos, files, voice messages and locations
go to every recipient as normal 1:1 messages; details page (attached) to rename, add, long-press
remove, pin, delete. Tested in the emulator: list with ECHOECHO, a text and a photo arrived as
1:1 messages and were echoed back.

Starred messages: long press → Star / Remove star (local, like Android), a small star next to the
time, Search page → Starred messages (newest first, tap opens the chat at it). Member/recipient
pickers redone: square pictures and Silica's glowing switch instead of check marks (user).

Drafts: unsent text stays in a chat's input when leaving it (stored encrypted with the settings)
and comes back; the chat list shows "Draft: …" instead of the last message. Tested in the emulator.

Archive: long press → Archive (unpins); a push-up menu "Archived chats", only when something is archived, opens the archive
(same page, long press → Move back to chats); a new incoming message brings a chat back, like
Android. Local only (migration 16). Tested in the emulator.

Message details: long press → Details shows created/sent/delivered/read for our messages,
sent (the sender's time)/received/read for incoming ones, edit or deletion time, the message id
and, for files, type and size (Android's message details). The times are recorded from now on
(migration 17); older messages and states a peer skipped (a "read" receipt without "received")
leave theirs out. Updates live on receipts. Tested in the emulator with ECHOECHO (outgoing:
created, sent, read; incoming: sent, received, read).

Media gallery: contact, group and list pages have "Photos and videos (n)" (hidden when there are
none), opening a grid like Jolla Gallery's (Sailfish.Gallery isn't Harbour-allowed, so rebuilt
with Silica: as many columns as fit Theme.itemSizeHuge, newest first, videos with a camera icon
and their length). Tap views a photo or plays a video (downloading first if needed); long press:
Show in chat, Save to Downloads. Photos and videos sent as files count too (by media type).
Android also filters files, voice messages and GIFs, and deletes from the gallery; not done.
Tested in the emulator with ECHOECHO (6 items, viewer, Show in chat) and the test group.

Profile picture for selected contacts: Settings → "Share profile picture with" is now a selection
dialog confirmed by swipe (it was a ComboBox that applied at once): everyone I write to, nobody,
or selected contacts, which continues to the contact picker. While sharing with selected
contacts, a contact page's pulley has Share / Stop sharing profile picture (like Android's
contact menu). Contacts off the list get a removal with the next message. Threema Safe backups
now carry the setting (`profilePicRelease`) and restores apply it. Tested in the emulator
(choose selected → pick → "1 selected contact"; contact pulley toggles; back to everyone);
nothing was sent. The picker still shows an empty section header for an excluded contact
(also when adding group members).

Threema Safe groups and lists: backups now contain our groups (also left ones, like Android) and
distribution lists, in Android's format; before, they only carried what an earlier restore had
brought along, so groups and lists created here were lost on a new device. Restores import them
after the contacts (Android's rules: skip deleted / unknown creator, drop members that don't
exist, left/kicked state). Unit test: backup → restore round trip between two app instances.
Live: "Back up now" in the emulator, then the backup downloaded and decrypted from the server
(ignored test `live_safe_backup_parses`, env `THREESEA_TEST_SAFE_ID`/`_PASSWORD`) holds the test
group, the Echo list and `profilePicRelease`. A full restore in the emulator was not run (it
would wipe the emulator's chats).

Our profile picture in Threema Safe: backups carry it in the spec's format (400 px, q=60, made by
the bridge when the picture is set; older pictures as their 512 px JPEG) and restores apply it.
Unit test in the Safe round trip; live: both cases backed up from the emulator and read back from
the server (512 px fallback, then 400×400 / 5.8 KB after setting the picture again).

App lock and private chats (ADR 0009), first part: Settings → App lock (behind the PIN once set):
a PIN entered twice on Silica's keypad like the device lock, "Lock the app" with a grace time
(immediately … 30 minutes; default 30 s). The app asks for the PIN on start and after being away
longer than the grace time; chats opened from notifications or D-Bus wait until then. Chats are
made private from the chat list's context menu; they leave the list, the archive, search, starred
messages, media and the unread count, and are reached through the push-up menu "Private chats",
which asks for the PIN every time. Opening a private chat from elsewhere (contacts, links) asks
for it too. Leaving the private chats hides them again. Their notifications say only "Private
chat: New message" and just open the app. The core enforces all of it (the PIN is only stored as
an scrypt hash). Threema Safe carries the private flags. Tested in the emulator: set up, wrong and
right PIN, private list, grace time, restart, private notification (read back from lipstick),
private chat from D-Bus, removing the lock. Not yet: device lock code as unlock method, the
optional encryption of the database key with the passphrase (both in ADR 0009).

App lock, second part: PIN or passphrase (keyboard button top left), screens rebuilt after the
system's code entry (user's screenshots); "Encrypt stored data" keeps the database key in Sailfish
Secrets only encrypted with the PIN/passphrase (≥ 8 characters), asked for on every start; until
then nothing is received and, if the app stays in the background, a notification says so. Tested
in the emulator: passphrase lock, encryption on, restart (notification, wrong passphrase, right
one opens the waiting chat without a second prompt), changing to an 8-digit PIN while encrypted
(re-encrypted, keypad after restart), removing the lock (plain key again, no prompt after
restart). Unlocking with the device code is not possible for apps (probed; ADR 0009).
Fixed: the last commit's chat page had a duplicated `Component.onDestruction`, so no chat could
be opened in that build (the emulator test had run on files from before the change).

Chum flavour (ADR 0010): a systemd user service starts Threesea without a window at login (D-Bus
activation, so the same Sailjail sandbox), so messages arrive after a reboot before the app is
opened. `FLAVOUR=chum scripts/sdk-build.sh`; the Harbour RPM has no systemd files and still passes
`mb2 check`. The window now only appears when the D-Bus call that started the app asks for it.
Tested in the emulator (login via user-session restart): background process started and connected,
an ECHOECHO echo arrived as a notification without opening the app; launcher shows the window of
that process; with "Keep running" off nothing stays running.
Fixed on the way: device-local settings (keep running, notifications, address book) were never
saved under Sailjail (QSettings wrote outside the allowed directory), so they reset on every start.
Still to check on a phone: Secrets "Authorize" prompt at boot, battery use.

ID revocation password: Settings → My profile → "ID revocation" shows whether one is set (and
when, from the directory) and sets it (8–256 characters; only a 4-byte hash goes to Threema, like
Android). Tested in the emulator: the state is read from the server ("No password set" for the
test ID), the dialog only accepts 8+ matching characters. Setting one on the test ID was not done
(it changes the ID on the directory); unit tests cover the key and the date parsing.

Identity links: Settings → My profile → E-mail / Phone number link them to the ID (Android's
identity links), so people who have them in their address book find you. E-mail: confirmed by the
link Threema mails (state "waiting", re-checked when the profile is shown and hourly). Phone: the
SMS code dialog follows; its pulley has "Call me instead" and "Cancel linking". Unlink from the
link dialog's pulley. The user allowed the plain address/number to go to Threema for this (§7.4).
Revocation password tested live on the test ID (set, date read back). Not tested: actually linking,
since the emulator has no address or number that can receive the mail or SMS; dialogs checked and
cancelled. Needs a test on the phone.

Media gallery filters (Android's): the gallery ("Media" on contact/group/list pages) now holds every
file message, classified like Android (photos, GIFs, videos, voice messages, audio, files);
pull-down "Show" opens a multi-select dialog (glowing switches, accepted by swipe) with the kinds
the chat has; the choice is saved for all chats. Files show their icon and name, voice messages
play in place. Tested in the emulator with ECHOECHO (a shared text file and its echo; hiding and
showing files). Fixed: tiles without a picture showed no icon (theme prefix missing). Deleting from
the gallery is not done (Android has it; the chat's own delete covers it).

Enter sends (Settings → Chats, on by default, device-local): Enter sends, Shift+Enter makes a new
line; off: the other way round. Silica's TextArea hands keys straight to its inner editor, so a
small C++ event filter (`bridge/enterkeyfilter.*`) catches Enter there. The on-screen keyboard has
no Shift: with the setting on, its Enter always sends. Enter on an empty input does nothing (never
starts a voice recording). Tested in the emulator: hardware Enter, on-screen Enter, Shift+Enter
(new line, then sent as two lines). The "off" direction is the same filter with the condition
flipped and was not clicked through.
Fixed before: the chat list's push-up menu (archive, private chats) opened empty.

Security audit, first pass (Phase 5): docs/security-audit.md. No §7 rule found broken in this first pass (later audits found more); `cargo audit`
had never run and found RUSTSEC-2026-0384 in tokio 1.54.0 (updated to 1.54.1). Open points listed
there (D-Bus notification actions callable by any session-bus process, voice recording temp file,
fuzzing, CI).

Settings split into subpages (user, 2026-10-23), laid out like Sailfish's Settings app: section
headers (Account, Privacy and security, App, Info) with a row and icon per category, each opening
a page in qml/pages/settings/ (My profile, Backup, Privacy, Security, Chats, Notifications,
Advanced, About). "Remove ID from this device" moved to My profile's pulley. Device-setting
switches no longer report a binding loop. Tested in the emulator: every page opens, switches work.

Small items (2026-10-23): contacts page pulley "Refresh" (directory lookup now, and the address book
if matching is on); exclusion list (Settings → Privacy; address book matching skips those IDs; part
of the app settings, so in Threema Safe like Android); "Empty chat" in the chat list's context menu
(messages deleted, chat stays listed); incognito keyboard (no prediction in chat, search and
dialog fields); "Hide content on the cover" (part of Android's "No thumbnails and screenshots"; whether
screenshots can be blocked too is still to be checked). Fixed: the chat
list's delete/empty remorse callbacks ran after the context menu was gone and failed.
Tested in the emulator: refresh, exclusion list page, empty chat on the archived Echo list.

Settings pattern from Sailfish's Settings → Top menu (user): a switch and below it a centred button
to that feature's own list, enabled only while the switch is on (components/SettingsButton.qml).
Privacy now reads Contacts (address book switch + Exclusion list; Block unknown + Blocked contacts),
Receipts, Other. New Appearance page with "Bigger single emojis" (on by default like Android: 1–3
emojis alone at double size). Tested in the emulator with ECHOECHO.
Seen for the UI round: in a chat, the sticky day header ("Yesterday") overlaps the next day's
section header ("Today") at the top.

Per-contact read receipts and typing indicator (Android's contact page): two choices on the contact
page (like in Settings / always / never, dialog accepted by swipe); Settings → Privacy → "Clear
individual settings" resets all (remorse). The core already honoured these (they come from Threema
Safe too). Tested in the emulator with ECHOECHO.

Separate passphrase for the stored data (user decision, like Android): Settings → Security has
"Encryption of stored data" with its own passphrase switch and "Change passphrase" (asks for the
current one first); the app lock is independent (ADR 0009 update). Tested in the emulator: on
(new passphrase on the keyboard), restart (wrong, then right passphrase), off (current one asked).
The emulator's existing app lock stayed untouched.

Settings → Media and storage (Android's): image size for sending (640/1024/1600/2592/original,
default 1600 like Android; was a fixed 2048), auto-download per network and kind (Wi-Fi: all; mobile
data: photos; the bridge reports Wi-Fi/Ethernet vs. mobile data and downloads start when it
changes), Save to Gallery (received photos/videos also to Pictures/Threesea, off by default), the
space used, and a clean-up (media files or messages, by age; starred messages kept).
Behaviour change: on mobile data, voice messages no longer download by themselves (Android's
default); on Wi-Fi everything does. Tested in the emulator: page, sizes, dialogs (clean-up
cancelled). Not tested: auto-save and the network switch (the emulator only has Ethernet).

Threema emojis (user: standard): emoji in chats and the chat list are shown as Twemoji pictures,
the set Threema Libre ships (CC-BY 4.0, credited on the About page), default on; Settings →
Appearance → Emoji style switches to the system's. scripts/make-emoji-table.py takes the 5213
sequences and the sprite sheets (3.5 MB) from upstream's EmojiParser.kt and foss_based assets;
image://emoji cuts the cells. Fixed on the way: bubbles at fractional positions showed a thin
seam (now on whole pixels). Privacy: "Blocked contacts" is its own row again, not under "Block
unknown senders" (user). Not yet: Twemoji in the input field, notifications and reactions.

Appearance → Contacts (Android's): names "First Last" or "Last First" (everywhere names are shown),
sorting by first or last name (also the section letters), show inactive IDs. Tested in the
emulator with a temporary name on ECHOECHO ("Tester Echo" under T), then undone.

Settings → Notifications (Android's sound and vibration, for chats and for groups; Android's "Do
not disturb" hours are Work-only and the sound profile covers it here): notifications use Jolla
Messages' categories (from commhistory-daemon): the chat's notification in x-nemo.messaging.im
plays the profile's instant message tone and the LED (ngfd `chat_exists`), an empty transient
x-nemo.messaging.im.preview vibrates (`chat`); with sound off the chat's notification stays in
im.received (LED only). Harbour allows no Nemo.Ngf, so categories are the way. Tested in the
emulator (categories written as in docs/emulator.md, feedback events read on D-Bus): tone + vibration,
silent, one banner. Phone: hear and feel it, that the categories exist there and that the lock screen shows our icon
(the categories name icon-lock-sms; we set appIcon).

Settings → Advanced → Network: Android's "IPv6 for messages". We always used Threema's dual-stack
hosts (libthreema's defaults: `ds-apip`, `ds-blobp-…`, `ds.g-…`), which is Android's "on"; off
drops the `ds-`/`ds.` prefix for the IPv4-only hosts (`https::server_url`, used by every HTTPS
request and the chat server connection) and connects again at once. Tested in the emulator: the
log names the host reached (`g-xx` off, `ds.g-xx` on), messages and directory calls work either way.
Chat settings: Android's proximity sensor (voice messages at the ear) comes with Calls, which need
the same earpiece routing; GIF autoplay with GIF support. Android's Threema Push, error reports,
power/hibernation fixes and web client options have no counterpart here.

GIFs (user: send, receive, favourites, re-use; no search): a GIF picked as a photo goes out
unchanged like Android's animated images (file message rendered as media, metadata `a: true`,
first frame as thumbnail; it used to go out as a plain file). GIFs play in the bubble and in the
viewer (bridge/animatedimage.cpp: QMovie on the decrypted bytes in memory, no plaintext file;
QML's AnimatedImage can't use image providers), with Android's "Auto-play animated images"
(Settings → Chats; off: a play button, they play in the viewer). Favourite GIFs: "Add to
favourite GIFs" on a GIF message, chat push-up "Send GIF" opens them (grid; tap sends, long press
removes, pull-down adds one from the device); encrypted in the media store, kept once by content
hash (core/src/gifs.rs, migration 21). The OS ships Qt's GIF plugin
(qt5-plugin-imageformat-gif). Tested in the emulator with ECHOECHO: send, echo, playback in bubble
and viewer, add from chat and device (duplicate kept once, JPEG refused), send from favourites,
remove with remorse.

Notification tones (user: "only when supported and allowed"): a tone for all chats, all groups
(Settings → Notifications) and per contact/group (their page, "Same as all chats/groups" by
default); ToneDialog like Jolla's sound dialog (system tones from
/usr/share/sounds/jolla-ringtones/stereo, "Select from music", preview), glowing switches per our
single-choice rule. Device-local (QSettings: toneChats, toneGroups, chatTones/<id>), as file
paths are. Mechanism (checked on D-Bus in the emulator): Nemo Notification `sound` = lipstick's
`sound-file` hint; lipstick then mutes the category's tone (`chat_exists` with sound.enabled
false, LED stays) and plays ngfd's `default` event with the file at the profile's ringing volume
(`x-ringtone-volume`; 0 in the silent profile). Choosing the default again removes it. Private
chats use the tone of all chats (no per-chat hint). Not tested: music picker (no music in the
emulator) and actually hearing it (phone).
LED colour: not offered. Lipstick passes no colour to the feedback daemon (its hints are
sound-file, sound-name, suppress-sound, x-nemo-vibrate; to ngfd only media.audio/leds/vibra); the
LED patterns are system configuration in mce, which apps can't change.

Multi-device, step 1 (ADR 0011; Threema Web decided with Calls): the connection can run through
the mediator. With multi-device active (store: `md_properties`, DGK and device ids) it connects
by WebSocket (`websocket.rs`, our pinned rustls config) to `wss://mediator-…`, runs libthreema's
D2M handshake (`transport.rs`), proxies CSP through it with the CSP device id login extension,
takes the leader role (unblock incoming), reflects incoming messages and runs libthreema's
contact transactions, lists devices (device info decrypted) and drops devices in a DROP_DEVICE
transaction; mediator close codes 4113/4115 end multi-device, other 41xx wait. Forward security
off while active: feature mask without FS, no FS sessions, a pending Terminate (DisabledByLocal)
to every FS peer after turning it on (libthreema patch). libthreema patches: join/url protobufs,
terminate with cause, DGK operations (envelope, device info, scope). Tested with the test ID on
the real mediator (temporary test buttons, removed): activate (3 device slots), ECHOECHO
messages both ways exactly once, device list, deactivate (dropped itself, 4113) and straight back
to the chat server with the FS feature mask. The test skipped the Terminate messages (the test
ID has other contacts). Next: linking (QR scan, rendezvous, emoji check, essential data),
reflecting outgoing messages and handling reflected ones, the Linked devices page.
Linking tested with Threema Beta for Desktop (2026-10-23, test ID): QR from a
picture, rendezvous, essential data (3 pictures), "new device registered", both devices listed
(Desktop "Connected"). Found on the way: a tap meant for the picture picker landed on the emoji
rows and confirmed the check; the rows now take taps only after 0.8 s.
Reflected envelopes (`reflected.rs`): outgoing messages from Desktop (text, location, file, edit,
delete, reaction, read receipts) stored as sent, incoming messages while Desktop leads, sent/read
updates; sync envelopes acknowledged, not applied yet. Fixed a libthreema decoder bug (envelope
started a byte late). Tested with the user's Desktop: "Desktop 2" shows as ours on the phone,
the echo arrived reflected while Desktop was leader. Still to do: reflecting what the phone sends
(Desktop doesn't see it yet), contact/group/profile/settings sync, typing over reflection.
Outgoing reflection: every message the phone sends is reflected once before it goes out (waiting
for the mediator's ack, like Android), "sent" is reflected after the server ack (groups: once all
members have it), read receipts reflect read updates; our reflect ids start at 0x80000000 to stay
apart from libthreema's. Tested with the user's Desktop: "From Sailfish to Desktop" appears there
as sent.
Sync from other devices (`sync.rs`): contact create/update (names, levels, feature mask, policy
overrides, notifications, pinned/archived/private), group create/update/delete (name, members,
left, notifications, visibility), settings, user profile (nickname, picture sharing), pictures
(blob mirror, below). Tested with the user's Desktop: renaming ECHOECHO to "Echo" and pinning
the chat arrived on the phone.
Sync to other devices: after a successful command the core reflects what changed (`sync_change`
in app.rs): contacts (add, name, policies), chat state (pin, archive, private, notifications;
contacts and groups), groups (create, rename, members, leave), settings (incl. blocked, sync
exclusion), profile (nickname, picture sharing), each in a transaction of its scope like
Android's Reflect*SyncTasks; pictures through the blob mirror (below). Tested with the user's
Desktop: unpinning on the phone unpinned it there. Offline changes: queued (below).
Blob mirror (multi-device): with a device group every blob goes through
`blob-mirror-…threema.ch` with Android's query parameters and scopes (media.rs `set_mirror`,
`BlobScope`); sync pictures both ways (incoming: downloaded in local scope, decrypted with the
message's key/nonce; outgoing: uploaded in local scope with the sync envelope). libthreema fix:
two-digit hex prefixes in server URLs (one device group in 16 got a wrong mediator URL). Tested
with the user's Desktop: photo through the mirror, the test ID's new profile picture shown there.
Sync changes made offline are queued in the store (migration 23, `d2d_sync_queue`) and
reflected after the next login, each in its transaction, removed once committed; the queue goes
when multi-device ends. Tested: pinned offline (443 blocked), reflected after reconnecting.
Chat list (user's todo): the pulley glows (busy) while connecting, "Connect now" while offline,
header description Offline/Connecting… (Jolla Mail).
In-app camera (user's todo): chat push-up "Take photo or video" opens CameraPage (Jolla Camera
style: viewfinder, shutter, photo/video and front/back switch); the capture goes to the app's
cache, is attached like a picked photo/video and removed once sent, discarded, or at the next
start. Not testable in the emulator (no camera service): phone test.
Member pages (user): "Add member" row with the theme's plus and a × per member (Jolla Settings →
Lock screen → shortcuts), no pulley detour.

Calls, step 1 — 1:1 audio (ADR 0012, Chum only): the core runs the call (`calls.rs` state,
`sdp.rs` Android's SdpPatcher, `app_calls.rs` commands/events, call messages 0x60–0x64 over the
connection, not reflected), TURN credentials from `identity/turn_cred` (IPv4 list, dual-stack
only for "relay only", like Android), the call recorded in the chat ("Missed call",
"Outgoing call, 1:23", …). The Harbour build rejects incoming offers with "disabled". The bridge's
`CallEngine` runs GStreamer webrtcbin like Android's PeerConnectionClient (max-bundle, Opus mono
CBR with in-band FEC, negotiated data channel `3MACALLdc0` id 0, no TCP candidates, relay-only
option), `CallManager` (`calls` in QML) joins both and rings with ngfd's `voip_ringtone`;
`CallPage` like Jolla's phone app (picture, name, state/time; answer/decline, mute, hang up),
"Call" in the chat's pulley for contacts that announce call support.
Tested: two engines connect and exchange audio in the emulator (`scripts/test-callengine.sh`);
webrtcbin's SDP passes the patcher (unit test with the captured offer); a call to ECHOECHO from
the emulator fetched TURN, sent the offer and ended as "Call cancelled" in the chat.
Interop (2026-10-23, Threema for Android calling the emulator): the call rang (call page, missed-call notification when not answered), connected on both
sides, audio flowed both ways (880 frames sent, 778 received in 8.8 s; heard on the PC with the
VM's audio on), hang-up from the phone ended it as "Incoming call, 0:08". Call support is
announced in the feature mask since. Outgoing calls to it first failed in DTLS ("no shared
cipher": GStreamer's default RSA certificate vs. libwebrtc's ECDSA-only client; then "certificate
unknown" until the SDP fingerprint matched our own certificate, ADR 0012 notes); fixed, the
emulator called the phone: 23 s, 2540 frames sent / 2297 received. The engine now checks the
peer's DTLS certificate against its SDP fingerprint (webrtcbin doesn't). Not yet: speaker/earpiece
switch and proximity sensor (phone tests); ringback tone; the in-call data channel messages
(capture state).
Settings → Calls (Chum): "Threema calls" and "Always relay calls", Android's settings, kept in the
app settings (Threema Safe, synced to linked devices); off: offers get "disabled" back, the chat
has no Call entry. Missed calls notify like messages (status lines otherwise stay silent). The
cover shows the call (name unless "Hide content on the cover", state/time) with decline/answer
or mute/hang up. TURN host names are resolved by the core: inside Sailjail libnice/GLib can't
resolve them ("No valid addresses were found"; the loopback test outside it shows no such
error). Tested in the emulator: relay-only
gathers a relay candidate from Threema's TURN server, normal mode host + srflx + relay.
Echo cancellation: bundled webrtc-audio-processing v2.1 (AEC3, noise suppression, high-pass,
AGC2) from pad probes around the call's PulseAudio streams; the loopback test runs through it.
How well it cancels on a real phone's speaker: phone test. Unanswered calls end after 60 s
("Call not answered" / missed).
Interop check without a peer: webrtcbin answers a hand-made libwebrtc-style Android offer
(`tests/callengine/android-offer.sdp`: audio, video with RTX and encrypted header extensions,
data channel) with audio sendrecv on the offer's Opus payload type, video recvonly (dropped
until video calls), the data channel, one BUNDLE group; the patcher accepts the offer.

Calls, step 2 — 1:1 video (in progress): core `call_video.rs` (Android's VoipVideoParams
profiles and common profile, o2o-call.proto capture state and quality profile on the data
channel), settings "Video calls" (synced like Android's video policy, not in Threema Safe) and
"Video quality"; the engine's data channel carries them (mute indicator both ways), VP8 send
from a camera branch that exists only while the camera is on (droidcamsrc on a phone, a test
pattern in the emulator), capped at 640×480 for software VP8; VP8 receive; `CallVideo` QML item
(frames as textures, front camera preview mirrored); call page with the peer's video full-screen,
our preview top right, camera on/off and front/back; "Video call" in the chat's pull-down menu.
Tested in the emulator: loopback video both ways (~21 fps at 640×480), camera off again cleanly;
webrtcbin answers the Android-style offer with VP8 sendrecv. With Threema for Android
(2026-10-23): the emulator video-called a phone; its camera showed full-screen in the
emulator, the phone showed our test pattern, mute shown both ways, 46 s. (Android refuses video
from callers without O2O_VIDEO_CALL_SUPPORT, so video is announced since; the first try crashed
in CallVideo: Qt 5.6's setTexture() deletes the old texture itself.) Not yet: droidcamsrc on a
phone, hardware encoding, rotation (phone tests).
Call tones: ngfd on the system bus (like libngf-qt; Sailjail's Base permission allows it): the
VoIP ringtone for incoming calls (it never played before: the calls went to the session bus,
where ngfd isn't) and the tone generator's ringback (`indicator`, pattern `ring`) while the other
side rings.

Group calls, step 1 (2026-10-26, docs/research/group-calls.md): libthreema GroupCallStart (0x4f);
core: key and call ID derivation (Android's test vector), SFU credentials, URL check, peek, join,
running calls per group (migration 24) with the refresh steps, the locally built remote offer
(MIDs, simulcast lines, inactive lines kept), the SFU's data channel (Hello, joined/left).
Bridge: GroupCallEngine (webrtcbin + data channel "p2s", our ECDSA certificate), GroupCallManager
(`groupCalls`), "Join group call" in the group chat's pull-down menu, a first GroupCallPage.
Tested with the user's Threema for Android starting a call in a group: the announcement arrived,
the peek found it, joining gave participant 1, ICE and DTLS came up and the SFU sent Hello with
the phone's participant. The SFU accepts SRTP_AES128_CM_SHA1_80, so stock GStreamer 1.26 works
(no DTLS backport).
Step 2: the participant handshake (`group_call_handshake.rs`, sans-I/O, Android's P2PHandshake;
group members only, replay and reflection checks), our media key (PCMK, ratcheted when someone
joins) in Auth, the others' keys and rekeys kept, their microphones subscribed after the
handshake; the call page lists the participants by name. libthreema exposes
ClientKey::derive_shared_secret for it. Tested with a phone: authenticated each other
(each listed the other by its nickname). Not yet: frame encryption,
audio, video, rekeying when someone leaves, the call state update, creating calls, the UI.
Step 3: audio. Frame encryption (`group_call_frames.rs`: AES-256-GCM with the per-participant
frame keys, Opus unencrypted header 0) behind two FFI calls used by pad probes; Opus send with
our MID header extension; receive per incoming stream into one mixer, the participant taken from
each packet's MID; ICE consent checks; microphone mute (CaptureState to the others). Tested with
a phone: heard both ways, 3702 frames decrypted and 0 dropped, the emulator stayed in
the call until the phone left. Group calls stay unannounced until video and creating calls work.
Step 4: starting calls and the in-call timers. "Start group call" in the group chat's pull-down
menu creates a call on our SFU (random GCK) and, once the SFU said Hello and 2 s passed, sends
`GroupCallStart` through the group outbox to the members whose feature mask has group calls.
When someone leaves we rekey (new random key one epoch up to everyone authenticated, applied
2 s later, again if another leave came meanwhile). The participant with the lowest ID keeps the
call state at the SFU current (secretbox under GCSK, every 10 s and on changes). Alone for
3 minutes we leave. A one-second tick from the bridge drives these. Tested with the user's
phone: it saw the started call (one participant from our call state), joined, audio both ways;
leaving and rejoining after our rekey still worked, 0 frames dropped.
Step 5: the call page and the chat banner. Tiles per participant (picture, name, microphone off
from their CaptureState; bubble-like corners via a small shader), the running time; back keeps
the call and the group chat's banner (names from the call state the SFU keeps, decrypted with
GCSK; participants and time) joins or returns to it. The chat refreshes a running call every
10 s (the refresh steps' timer). Tested with a phone in the call, muted and unmuted.
Step 6: video. The camera button (middle) sends VP8 from the camera (test pattern in the
emulator); the core encrypts each frame and packetizes it (`group_call_vp8.rs`: RFC 7741
descriptor with picture ID, TL0PICIDX, TID 0; GStreamer's payloader parses the first partition,
which encryption hides), RTP with our camera MID and RID "h" goes to webrtcbin through an appsrc;
key frame requests reach the encoder. Others' cameras: subscribed while their CaptureState says
on (640x360, 30 fps), decrypted after the depayloader, decoded, turned upright by the video
orientation extension (Threema sends unrotated pictures), shown in their tile. One simulcast
layer only for now. Tested with a phone: video both ways, upright; 9330 frames sent,
5577 received, 0 dropped.
Group call support is announced in our feature mask since (Chum only, like 1:1 calls), so
other Threema apps send us their GroupCallStart. Open: the phone tests (camera, CPU, speaker),
Threema Web (since done). The Jolla call UI was dropped (2026-10-26).
Simulcast: three layers like Android's (l 160x90 100 kbit/s, m 320x180 250 kbit/s, h 640x360
600 kbit/s), one encoder each, own RTP streams and picture IDs, one MFSN; key frame requests go
to all three. Tested with a phone: video arrives, 16161 frames sent, 0 dropped.

Threema Web, step 1 (2026-10-26, ADR 0013, docs/research/threema-web.md): relayed data only, no
WebRTC, both flavours. Core: QR parsing, a sans-I/O SaltyRTC responder (server handshake with
signed keys, token/key/auth, relayed-data task; tested against a simulated server and browser),
chunking, the ARP connection layer (connectionInfo with the derived connection ID, acks,
connectionAck, connectionDisconnect), sessions in the store (migration 25) with allowed
self-hosted servers pinned on first use; answers clientInfo (`os` "android", which selects the browser's parser for the threema-gateway push
token; `appVersion` names Threesea), profile, empty receivers and conversations, keyPersisted. UI: Settings → Threema Web
(sessions, scan, connect/disconnect, remove; dialogs for self-hosted servers and changed
certificates). Tested with web.threema.ch in Chrome: paired from a picture of its code, the
browser showed its main screen. Next: receivers, conversations, messages, updates, sending.
Step 2: read-only. Contacts, groups and lists, the chat list (private chats never), messages in
pages of 50, live updates (new and changed messages, typing, chat list, receivers; also for
what the phone sends), mark as read; status and call lines as text (like Android). Stored
sessions (keyPersisted) reconnect with the trusted keys without a QR code: tested by tapping the
session in Settings. The wake-up push (`web-session-resume` from `*3MAPUSH`) is handled, but in
the first test no such message reached us: Threema Desktop was the device group's leader, and
only the leader receives from the chat server. Confirmed 2026-10-26: with Desktop closed the
mediator promoted the app, both `*3MAPUSH` wake-ups arrived and the stopped session connected by
itself. The leader is the mediator's choice (no request in md-d2m); Android allows Threema Web
with multi-device too, so it has the same limit.
User decision: push only, no waiting connection of our own.
Step 3: sending from the browser through our own commands (same rules, sending, multi-device
sync): text with quotes, typing, delete, empty a chat, star = pin, agree/disagree = thumbs
reactions (our own thumbs show as user-ack/user-dec). Each action also updates our UI (chat
list, open chat) and the browsers. Tested with ECHOECHO.
Step 4: pictures and files. Profile pictures (48 px or 512 px JPEG), message thumbnails
(350 px) with a 50 px preview in each file message, files on request (downloaded from the blob
server first if needed, answered when done), files from the browser sent through our sendFile
(images as media with our 512 px thumbnail). Tested: pictures shown, an image sent to ECHOECHO
and back, downloads.
Step 5: the rest of the protocol. Contacts (add, first/last name), groups (create, rename,
members, picture, leave, delete, sync), distribution lists (create, change, delete), the profile
(nickname, picture), contact details (empty: we keep no address book details), emoji reactions
in messages (Android's buckets), quotesV2/emojiReactions capabilities; access flags like
Android (the creator manages a group; others' groups only deletable after leaving). Status lines
in groups name us as sender (the browser shows a sender in groups), emptied chats have no latest
message. Tested with web.threema.ch.
Calls appear in the browser as its call messages (voipStatus: missed, duration, reason, like
Android); update/voipStatus isn't sent, Threema Web has no handler for it. The battery
(/sys/class/power_supply, type Battery) goes out as update/batteryStatus every 30 s when it
changed: phone test (the emulator has none). A reloaded browser reconnects to the still open
session (new-initiator); after the session really ended, the push wake-up brings it back
(confirmed, see above).

## Phase 5 — Hardening (started 2026-10-27)

Security audit and rules audit of 2026-10-27 (reports in
`docs/security-audit-2026-10-27.md`, `docs/rules-audit-2026-10-27.md`). Fixed, each with a test
where it can run offline:
- V01: no call media or data channel messages before the peer's DTLS certificate matched its
  end-to-end signalled fingerprint; group calls now check the SFU's certificate at all.
  tests/callengine shows the window was real (220 ms in, 780 ms out with a delayed check).
- S01–S04: browser sessions end with "Delete ID and data", stale sessions' requests are
  ignored; browser management and subscriptions respect private chats; draft, star, forward
  and save check private chats (core/src/app_web_security_tests.rs).
- S05: group-call frames change key state only once authentic; replay window.
- S06: resource limits (HTTPS answers, concurrent downloads, Threema Web chunks, picture and
  GIF dimensions, voice decoding); S08: leftover voice recordings removed at start.
- Hardening: PIN entry pauses after wrong attempts (like Android); hosts checked before
  connecting; Threema Safe decompression bounded. R09: Web pages' orientation/decorator.
Open: S07 (notification Reply over D-Bus: caller check, needs the phone); the FFI media readers
have no private-chat check (no database on their threads; same-process caller only); R05
(central log redaction: rule wording or a layer, to decide); CI (R07); docs/ffi.md misses the
Web and group-call commands; the brief's wording vs ADR 0012/0013 boundaries.

Re-audit of the same day (`docs/deep-audit-2026-10-27.md`), fixed: Threema Web session events
tied to their task (D01); every content command and the UI's media readers behind the
private-chat check (D02); a decoding budget and a bounded event queue for Threema Web (D03);
no browser text in logs and a stricter log check (D05); the update check at most one attempt
a day, in the background (R01, P01); releases gated on the checks and the release branch (R02);
no Threema Web refresh without changes (P02, partly); list models move rows (P03); check.sh
runs the tests once (P04). Open: D04 (phone), P01 for backup/linking, P02's per-chat queries
and search index. Threema Web itself was not reconnected with a browser after D03 (the
decoding budget): to check on the next real session.


## Restore fixes and Android backup import (started 2026-10-27)

From a report on a real device (design: `docs/android-backup-design.md`). Done:
- Restored groups and distribution lists without messages show in the chat list (stand-in rows,
  kind `none`; Threema Web sends them without a latestMessage).
- Safe restore status: pending/skipped/revoked counts with reasons on the Threema Safe page,
  "Import the rest now", import retried on reconnect (rate-limited), errors kept
  (`safeRetryImport`, `safeStatus.restore`).
- Display cutouts: custom top layouts (chat header, profile card, PIN entry, call video,
  camera clock) pad by `components/CutoutInset.qml`; media viewers use
  `CutoutMode.FullScreen`; see docs/ui-guidelines.md. Needs the real phone for the final check
  (the emulator has no cutout).
- Profile picture recovery (store migration 27): request backoff instead of once-ever markers
  (old markers migrate), requests also from group messages / chat open / bounded batches on
  connect, persisted downloads with retries, generation guard against set/set/delete races,
  monotonic picture versions, `profileChanged` for the own picture, Avatar placeholder until
  the image decodes.

Android full-backup (ZIP) import, done so far:
- `core::android_backup`: container reader (zip crate, AES-256/WinZip AE like Zip4j, strict
  validation, authenticated reads, bounds) and an OpenCSV-exact CSV dialect; version gate
  15..=27; synthetic pyzipper fixtures (`scripts/make-backup-fixtures.py`) cover unicode
  passwords, ZIP64, stored/deflate, tampering, wrong password.
- `androidBackupInspect` + Settings → Backup → "Android data backup": pick the ZIP, check it
  locally (format version, identity match against the running ID and key, inventory).
  Verified end to end in the emulator incl. the Sailjail picker.
- `androidBackupImport` (scope `metadata`): offline merge of contacts, groups, distribution
  lists and pictures into the matching identity; existing local data wins, key conflicts
  counted, no sending side effects, idempotent. Local address-book avatars are counted, not
  imported (one picture slot per contact).

Steps 5 and 7, done:
- Store migration 28: history position (`sort_key`) separate from row ids; live arrival order
  untouched (insert trigger), imported history interleaved deterministically
  (`android_backup::place`); chat pages/cursors/list/media follow positions, search and
  starred by time, change detection stays id-based.
- `androidBackupImport` scope `history`: messages (TEXT/LOCATION/FILE + deprecated
  IMAGE/VIDEO/VOICEMESSAGE via the legacy transformation, VOIP_STATUS as our call line,
  CONTACT as text), media/thumbnails authenticated and only then marked local, quotes/edits/
  tombstones/stars/read states, USERACK/DEC and `g_msg_states` plus the format-26 reaction
  files as reactions; dedup where the existing row always wins, stable synthetic ids
  (migration 29), background job with progress/cancel/resume. Status lines other than calls
  are counted as unsupported, not dropped silently.
- Creating backups (user request): `androidBackupExport` writes a format-27 archive (AES-256,
  same CSV dialect) with identity, contacts/groups/lists, messages, local media, pictures and
  reaction files — proven by an export→import round trip in tests; `androidBackupSchedule`
  runs it daily/weekly from maintenance with rotation (`threema-backup-*.zip`, newest `keep`
  kept; `.part` until complete).
- Nonces (`android_backup::nonces`): Android's encoding read from its source, not a device:
  `nonces.csv`/`nonces_d2d.csv` hold one hex string per row, HMAC-SHA256(key: the Threema ID
  string) over the 24-byte nonce (`BackupService.writeNonces`, `NonceFactory`); older rows
  may be raw 24 bytes, a cell may hold several split by `;`. Import stores the hashes beside
  our raw nonces; `has_nonce`/`has_d2d_nonce` check the raw value and its hash (like
  Android's `nonce=? OR nonce=?`). Export writes hashes plus `nonce_counts.csv`. Test vector
  from Android's `DatabaseNonceStoreTest`; a replayed nonce is refused after the import.
- Polls (`android_backup::polls`): `ballot*.csv` ↔ our poll messages. The BALLOT row's body
  is `[type, ballot id]` (1 created, 2 modified (not shown), 3 closed); choices and votes
  link by `"<aid>-<creator>"`, the aid being the poll id's wire-order hex. Imported open
  polls are registered, so live votes and the closing land in them. Not carried: drafts
  (`TEMPORARY`), `last_viewed_at`; a closed poll with totals and no voter rows becomes a
  summary poll. Polls in distribution lists are not exported (Android links polls to a
  contact or group only; counted as skipped).
- Emulator smoke test (2026-10-27): folder picker (Downloads), password, "Create backup
  now" → `threema-backup-20261027-162055.zip`, 7.4 MB, no `.part` left. Checked
  independently with pyzipper: 81 entries all decrypt and authenticate, version 27, 807 CSP
  and 284 D2D nonces (all 32-byte hashes, counts file matching), 4 polls/10 choices/16 vote
  rows, group file names = `group_uid` = poll `ref_id` (Android's `groupUidMap` key). The
  app's own "Check backup" reads it back (this ID, 2 chats, 1 group chat, 35 media files).
  On the way, the folder choice's reply now lands in the backup page itself, not in the
  picker page that is destroyed while the call runs (it happened to work, by luck of timing).

- UI redesign (user feedback 2026-10-28, the screens looked unfinished): the feature is
  called **Data backup** everywhere a user sees it. `DataBackupPage` (import section first,
  then creating/schedule), the preview dialog, the progress page, and a new
  `DataBackupResultPage` that replaces the progress page when the import ends: headline
  (finished/stopped/failed), lead sentence, then the counts in two sections (contacts and
  groups; message history). Results no longer appear inline on the backup page. All texts
  have their German. Tested in the emulator: create (rotation to 3 files), check, preview,
  import, results.
- Re-import dedup fix (found by that test): our export wrote `VOIP_STATUS` rows with an
  empty api id, so every import of a *different* backup of the same store duplicated the
  call lines (synthetic ids are keyed by the archive fingerprint, by design). Local-only
  rows now export their stable random local message id as `aid`; Android ignores it for
  status rows. The existing-row repair pass also skips media whose bytes are already local,
  so importing again moves (and counts) no media. Regression: the round-trip test now
  re-imports A's file into A and asserts nothing is added.

Open: own-group picture redistribution when only local bytes (no blob descriptor) exist;
restore-into-Android of an exported file checked only against our own importer and Android's
reader rules (a real Android restore needs a device); real-archive import on a device (password
stays in the app). Both are in the checklist
`docs/android-backup-device-checklist.md`. Minor: the backup file name uses UTC, the page
shows local time.

Phone feedback round (user, 2026-10-28) — fixes and features, each tested in the emulator
where that is possible:
- Chat: an incoming message no longer closes the keyboard. Inserting a row steals the
  input's focus; the page now recognises that steal (close + focus gone right after an
  insert) and undoes it, while a keyboard the user closed stays closed (KeepFocus keeps the
  focus on the input through a user's dismissal — that tells them apart). Reproduced and
  verified with ECHOECHO.
- Chat: scrolling back pages the whole history in (`messages` has a `before` cursor on the
  history order); the first page shrank to 60 rows, so long chats open faster. Verified to
  the first message of the Echo chat. Silica's quick scroll is enabled again and behaves in
  the bottom-to-top list.
- Chat: the core reports `firstUnread` (page widened to reach it, bound 1000); the chat
  opens there and shows a highlight-colour Separator above that message for the visit.
  Rendering and position verified; the live race (echo in under a second) was faked by
  forcing the marker.
- Voice: the playback bar is Silica's Slider (drag seeks); a failed upload shows the warning
  delivery icon instead of hiding behind the clock (tap retries, as before) and every
  reconnect retries failed uploads itself (`on_connected`). Likely cause of a long voice
  message that never left the clock: a one-time upload failure used to
  stay invisible and untried forever.
- Voice, new (phone tests needed; the emulator has no audio or proximity sensor):
  `setVoiceListened` (body.listened; migration 30 marks old read voice rows), unplayed
  incoming voice messages glow (play icon and length in the highlight colour); when one
  voice message ends and the directly following message is a voice message too, a soft
  generated two-tone chime sounds and it plays on (any other kind breaks the queue); the
  proximity sensor (QtSensors; the Sailjail permission `Sensors` is not Harbour-allowed, so
  the spec adds it to the Full flavour only — in Lite the sensor stays silent and the
  feature simply never triggers) plays at the ear: covered
  → earpiece at call volume (PulseAudio role "phone" via QAudioOutput category /
  QMediaPlayer audio role), the running message reroutes in place, or the newest run of
  unheard voice messages starts at its first unheard one (only when the chat ends in voice
  messages); uncovered → pause and back to the loudspeaker; the screen shows black and
  takes no taps while at the ear (apps cannot switch the panel off; same as Android's
  proximity lock).
- Multi-device: a failed first linking rolls the activation back and `deactivateMultiDevice`
  always leaves the group locally even when the mediator is unreachable — the state the
  user could only escape by setting the ID up anew cannot stick any more. `confirmLinking`
  outlasts the connection's own bounds (200 s), and the join path logs where it fails.
  The actual first failure ("Rendezvous connection stream ended before device join was
  complete" on the Desktop) is not reproduced yet; the next attempt will leave usable logs.

Phone tests for this round: keyboard during a real conversation; long-history scrolling;
the unread line after real incoming messages; voice: slider look, seek, new-glow, chime and
queue, at-the-ear playback/pause/black screen and earpiece volume; a long voice message to
an iOS client (watch for the warning icon if the upload fails); linking Threema Desktop
again — if it fails, send the journal lines around "device join failed".
- Threema Web: a reply now carries its quote (who wrote the quoted message, its text or a
  pictogram stand-in for media, and the id for the jump); the browser used to show only the
  reply's own text (user report 2026-10-28). A quote whose message is not stored stays away,
  as before, since the wire quote carries only the id.
- In-app log and support package (user request 2026-10-28, after the second Desktop-join
  failure): the core's tracing also writes a rotated file in the data directory (512 KiB ×2;
  §7.3's content rules hold at every level, so it never carries message text or addresses),
  and the bridge forwards Qt/QML warnings into it (`threesea_core_log`). Settings →
  Advanced → View log shows it (pulley: Refresh, Save support package → Downloads as
  `threesea-log-<stamp>.txt` with device facts, Clear with remorse). For the join failure:
  reproduce once more and the package will carry the "device join failed" lines.
- Forward secrecy switch (user decision 2026-10-28): Settings → Privacy → Encryption.
  Off behaves exactly like multi-device activation (SE-199): feature mask without the FS
  bit, every session terminated with DisabledByLocal, no session store on the connection —
  messages stay end-to-end encrypted. Background: contacts could see repeated
  security notices (session churn); the switch is the escape while the churn's cause is
  hunted with the new log. Related guard: a message the peer rejects twice over forward
  security now stays failed (warning icon, manual resend) instead of bouncing without end.
- Updates restart the app (user request 2026-10-28; Full only — Lite quits when closed and
  Harbour restricts scriptlets): `%posttrans` finds running instances by executable path
  (`/proc/*/exe`, also "(deleted)" after the replace; `comm` is truncated), sends SIGTERM,
  waits up to 15 s (then SIGKILL), and starts the new version with `background()` on each
  affected user's session bus via `runuser` — only for users who had it running. `%preun`
  stops it on removal. The app now quits cleanly on SIGTERM through the event loop
  (self-pipe), so the core stops and the store closes as on a normal quit. Tested in the
  emulator: `rpm -U` over the running app swapped the process with no manual steps, the new
  instance runs as the user inside Sailjail, a SIGTERM quits in ~100 ms with the log line,
  and nothing is started when the app was not running. Updating from builds before this one
  still kills the old process abruptly (it has no handler yet); harmless, the store is
  transactional.
- **Desktop linking root cause found and fixed (2026-10-28, from the user's support
  package):** rendezvous and emoji check worked; the phone's *first mediator connection*
  was dropped ("Connection reset without closing handshake") before the server-hello, so
  the join never started. libthreema encoded `ClientUrlInfo.server_group` as the byte's
  decimal (`csp_server_group.0.to_string()`, group `e0` → "224"); Android sends the group
  string ("e0"). The mediator silently drops malformed groups. Comparing with Android's URL, and
  connection tests in a few server groups (no login), showed only groups whose decimal happened to
  be two characters greeted (that is why the test ID linked on 10-23); after the fix every group,
  `e0` included, is greeted. Fixed in the vendored libthreema with a regression test.
- Threema Desktop "Connection from another device detected" on every start (user report
  2026-10-28, right after linking finally worked): the chat server raises a device-cookie
  change flag when an ID logs in with a new cookie (for example after moving an ID from
  another device) and repeats the indication on every login until a client clears it. We ignored the
  indication, so the flag never cleared and every device in the group — now including the
  Desktop — saw it. Like Android's TaskManager the connection now answers with
  ClearDeviceCookieChangeIndication (libthreema already had the payload) and shows Android's rogue-device warning as a notification — skipped
  once right after the app generated its own cookie (Android's `skipNextIndication`).
- Notifications stayed up for chats read or answered on Threema Desktop (user report
  2026-10-28): the reflected read update only cleared them when it changed a row, but the
  Desktop's reflected read receipt often marked the messages read a moment earlier; the
  receipt path itself, and a reply sent from the Desktop, never cleared them. All three
  now clear the chat's notifications (`reflected::seen_elsewhere`; private chats stay
  unnamed while hidden). Regression test included.
- Repeated security notices (report 2026-10-28): each failed linking attempt
  activated multi-device, which requests "end every forward-security session" for the next
  login; the rollback never dropped that request, so the next *direct* login — with
  forward security meant to be on — still ended all sessions, and every contact saw a
  notice when the next message started a new one. Now the connection only ends sessions
  while forward security is really off (multi-device or the switch), and the rollback and
  the switch turned back on drop a stale request. With linking fixed, the churn's source is
  gone; the Privacy switch stays as the user's choice.
- Desktop sync gaps (reflected outgoing messages): polls created, voted on and closed on the
  Desktop (1:1 and groups) were dropped — now stored as our sent poll and registered
  (`polls::reflect_setup`), our votes land in it, the closing adds the results message.
  Legacy thumbs reactions from the Desktop become our reaction. Reviewed and deliberately
  not stored: group control messages (the group sync carries the change), typing, profile
  picture and call signalling messages.
- P02 (deep audit), long histories: search no longer scans every message. Migration 31 adds
  a trigram FTS5 index (contentless, inside the encrypted database, kept in step by
  triggers: text body; file caption/name; place name/address; poll question) plus a partial
  index for messages with mentions; queries of 3+ characters take their candidates from it,
  the exact check in Rust stays the final filter, so results are unchanged (shorter queries
  scan as before). `wipe` empties the index through FTS5 ('delete-all'), never its shadow
  tables. Host release build, 1,000 chats × 100 messages: search without hits 20 ms →
  0.04 ms, 50 hits 1 ms. The chat list's DISTINCT became a loose index scan (one seek per
  chat): 7.2 → 5.6 ms for that part, ~23 ms in total; the rest is genuine per-chat work
  (newest row, unread count, reactions, decoding) — the audit's maintained per-chat summary
  remains the structural follow-up. Benchmark: `messages::bench::bench_large_history`
  (ignored). The index costs disk space (trigram indexes are a few times the indexed text)
  and is built once at the upgrade.
- Fuzzing (open item from the audits): `core/fuzz/`, seven cargo-fuzz targets over the
  decoders fed from outside (voice, call SDP, Threema Web chunks and images, QR/join codes,
  backup CSV readers, quotes/mentions), seeds for the container formats. First runs
  (AddressSanitizer, 2–3 min per target, ~54 million inputs): no crash, panic or memory
  error. Seeding lifted voice coverage from 203 to 4,607 points. How to run: docs/build.md.
- Vibration for messages already read on the Desktop (user report 2026-10-28): while the
  Desktop leads, the phone gets each incoming message forwarded and notified at once; the
  Desktop's "read" came seconds later and only removed the banner. After waking, the
  mediator's backlog replayed messages and their read updates in one burst, vibrating for
  each. Now forwarded incoming messages appear in the lists at once but their notification
  is held (`Store::held_notifications`, `messageNotify` event): backlog ones until the
  mediator reports its queue dry, live ones for 8 s; only messages still unread then notify.
  A connection ending with held ones releases them, so none is lost. Messages the phone
  receives itself (it leads) notify at once, as before — checked in the emulator.

## Feature round 2026-10-29 (phone feedback and the gaps against Android)

Two phone bugs from the user's report, and the features that were still missing against
Android's settings (the list came out of a comparison with `res/xml/preference_*.xml`).

Phone fixes:
- **At the ear, playback was scrambled silence.** QMediaPlayer's `VoiceCommunicationRole` does
  not reach the earpiece on Sailfish; the call engines do, with PulseAudio's media role
  `phone`. `bridge/earpieceplayer.cpp` (Full only, like the engines) plays the loaded message
  through such a pipeline — appsrc over the bytes in memory (random access, so MP4/AAC can
  seek), decodebin, `scaletempo` where the system has it, `pulsesink` with that role. The
  loudspeaker keeps Qt's player. A second cause was removed on the way: rerouting handed the
  new pipeline the same `QBuffer` the old one was still reading.
- **The chime between two voice messages was too loud and aggressive.** It is now a soft bell
  (C5 with its fifth, 25 ms attack, 450 ms ring-out) at about a quarter of the old level, half
  of that at the ear, generated in `bridge/softsound.cpp` — which also plays the new in-app
  sound.

Features:
- Voice playback speed like Android's: the bubble of the message in the player shows 1× / 1.5× /
  2× and the choice holds for the next message and the next start. The platform route keeps the
  pitch where `scaletempo` exists; the PCM fallback (emulator) plays faster and higher.
- Settings → Chats: "Play voice messages at the ear" (Android's proximity setting; the switch
  only shows where the sensor exists, so Full). Settings → Notifications → In the app: sound and
  vibration for a message arriving in the chat the user is reading — that case was silent
  before. A held notification (multi-device) gives no late feedback.
- Linked devices: this device's label is a setting now instead of the constant "Sailfish OS"
  (`setDeviceLabel`, sent in the D2M client-hello, so it reconnects), and every other device can
  get a name for our own lists (`setDeviceAlias`). A device's own label is encrypted with the
  device group key, so renaming another device for everyone is not possible — not in Android
  either.
- Chat list: "Mark as unread" / "Mark as read" in the context menu (Android's MARKED_AS_UNREAD,
  store migration 32): local, not reflected, gone as soon as the chat is read or a message
  arrives. The row gets the unread look without a count.
- Settings → Calls: "Group calls" (Android's group call policy, synced to linked devices). Off
  means this device stays out: no peek at the SFU, no banner, no join, and a call it is in is
  left. The feature mask keeps the bit, as on Android.
- Settings → Appearance: message text size (small/normal/large, from the theme's own sizes), a
  chat background (Android's wallpaper; one picture behind the messages, under the theme's
  dimmer so the bubbles stay readable) and "Accept profile pictures" (Android's
  `receive_profilepics`; off: contacts' pictures are neither asked for nor downloaded).
- Settings → Media & storage: "Keep messages" (Android's auto-delete, 1 week … 1 year), run by
  maintenance twice a day with the clean-up's rules — which now keep open polls and messages
  still on their way, besides starred ones. And "Video size" (Full only): a video's longest edge
  becomes 480 or 848 px at Android's bitrates (`bridge/videotranscoder.cpp`), the AAC audio
  track is remuxed unchanged. It probes for an H.264 encoder (droidvenc, x264enc, openh264enc,
  avenc_h264) and a container it can read; without one the video is sent as it is and the log
  says so. The scaled copy lives in the capture cache and goes once the core has read it.

Fixed on the way (found while reading the code against Android's):
- `is_member_of_active_group` always said no, so with "Block unknown senders" on, a 1:1 message
  from someone who shares a group with us was dropped — Android delivers it.
  `Store::is_group_member` answers it now, with a test.
- Deleting a contact who is in one of our groups demotes it to the group acquaintance level
  (Android's `AcquaintanceLevel.GROUP`) instead of removing the row, which the group needs for
  the name and the public key.

Phone tests for this round: at-the-ear playback and its volume, the chime, the in-app sound,
speed 1.5×/2× on AAC messages, video scaling (which encoder the phone has — the log line names
it), the chat background on a real screen, and renaming this device while Threema Desktop is
linked.

## Chat list summary and smoother scrolling back (2026-10-29)

Both on the user's request, after the feature round.

**Per-chat summary (store migration 33).** `chat_summaries` holds, per chat, the newest message
in history order and the unread count. SQL triggers keep it in step — like the search index, so
no write path has to know about it, which was exactly the risk that had the table deferred on
2026-10-28. The insert trigger cannot see the `sort_key` yet (the `sort_key` trigger assigns it
right after the insert), so the summary may point at the previous message for the blink of an
eye inside the same statement, and the `sort_key` trigger corrects it; the delete trigger
recomputes with one indexed seek when the newest message goes, and drops the row with the last
message of a chat. A test runs 1500 random operations — live messages, history placed like an
import, reading, deleting single messages, emptying and deleting chats — and compares the table
against a full recomputation after every 37 of them.

With it, `conversations()` is one join instead of a loose index scan plus a count per chat.
The chat list's per-chat lookups (contacts, groups, lists, notifications, picture versions,
drafts) are now one query each as well, and group and list titles share one helper that takes a
name map. For 1000 chats, in release on the host:

| | before | after |
|---|---|---|
| `conversations()` (1000 × 100 messages) | 23.1 ms | 7.5 ms |
| the chat list the UI waits for (1000 × 20) | 33.9 ms | 6.9 ms |

Benchmarks: `messages::bench::bench_large_history` and `app::tests::bench_chat_list` (both
`#[ignore]`d; the command is in their doc comments).

**Second round on the same two things (report from the phone, 2026-10-29: still stalling when
scrolling up, and now also jumping back to the bottom).**

The jump came from our own change: the list's `countChanged` **handler** runs after the callback that caused
it (traced in the emulator — insert done, `loadingOlder = false`, and only then the handler),
so the "not while a page is coming in" guard was already false and `positionViewAtBeginning()`
ran for every page. Nothing scrolls on a count change any more: `ChatPage.addMessage` adds a
message and positions the view only when a row was really added, so an update of a message
that is already there (a receipt, an edit, a reaction) and a page of older messages leave the
view where it is. The initial load positions itself, where it already chose between the newest
message, a search hit and the unread line.

For the stalls, two things that are GUI-thread work by construction, neither of which the
emulator's renderer could have shown:
- **The image providers ran on the GUI thread.** Qt only lets the image loader thread call a
  provider when it declares `ForceAsynchronousImageLoading`; without it, `asynchronous: true`
  on the `Image` buys nothing. So every picture in a chat was decrypted and decoded while the
  interface waited. Both providers declare it now: the core's media reads are documented
  thread-safe (`core/src/ffi.rs`) and the emoji provider's sheet and cache are behind its
  mutex.
- **GIFs played while scrolled past.** `playing` only depended on the app being active, so
  every GIF whose delegate existed decoded frames; it now also asks whether the bubble is in
  the view.

And a way for the phone to say what the emulator cannot: Settings → Advanced → Log →
**Detailed** now notes when the interface stopped for more than 150 ms while a chat was open,
with the number of messages loaded and whether a page was being fetched (counts only, §7.3).
That level also stopped turning on Qt's own categories — the scene graph wrote a line per
frame, which drowned the log, the support package and the app itself.

**Scrolling back through a chat (report: loading the next block of messages visibly blocked the
whole UI).** The page of a hundred messages was put into the
model row by row, and the view laid itself out after every single row. `JsonListModel::insertAll`
takes the whole page as one insert (keys checked against a hash built once, overlapping rows
updated in place): 5.9 ms → 0.26 ms for the model part alone in the SDK test
(`scripts/test-models.sh` prints it), and one layout pass instead of a hundred. With it: pages
of 50 instead of 100 so there is less to lay out at once, fetched two screens ahead instead of
one so they are there before the reader arrives, and the day band no longer reads the topmost
message on every scrolled pixel — only when the message under it changes.

What the emulator could and could not say: the core answers a page in 5–27 ms, the reverse
takes 0 ms and the insert 1–3 ms (instrumented run), so the data path is not where the time
goes. Its stall detector (a 16 ms timer; gaps over 120 ms logged) counts 11–15 stalls of
150–280 ms while flicking back either way — but with a counter in the delegate, **no delegate
is built during those stalls**, and they also appear with a one-rectangle stand-in for the
bubbles. They are VirtualBox's software renderer drawing the flick, not the app, so the
emulator cannot measure what the user feels on the phone. A `cacheBuffer` on the list was
tried and dropped again for the same reason — nothing measurable, and Silica's default is the
smaller deviation. The phone has to say whether the rest is gone.

## A bounded window of messages in a chat (2026-10-30)

From the phone: does scrolling up take a lot of memory and never free it? Measured in the emulator with a chat of about 340 messages: 162 MB at start,
188 MB with the chat open, 216 MB after scrolling to the beginning, and 215 MB after
scrolling back down. Leaving it there, the app held every message it had ever scrolled past.

It is a high-water mark, not a leak: destroying the chat page (opening another chat over
D-Bus), coming back and scrolling the whole way again ends at the same 217 MB instead of
climbing, and putting the app aside gives back about 42 MB (Qt drops the window's scene
graph). What was unbounded was what the chat *kept*, and in the emulator's small chat the
memory is mostly the drawing side — decoded pictures, the scene graph, the JavaScript heap —
so the numbers there barely move with the window. It is the long chats on a phone the window
is for, and with it the list's work per change no longer grows with how far back someone has
scrolled ("the more you scroll, the more sluggish").

The chat now holds a window of about 250 messages around what the reader is looking at:

- Scrolling back pages older messages in (50 at a time, two screens ahead) and drops the
  newest end; scrolling forward pages them back in (`messages` takes an `after` cursor now,
  `Store::conversation_messages_newer`) and drops the oldest end. Nothing in view, or within
  a page of it, is ever dropped — the list is bottom to top, so the newest visible message
  has the smallest index.
- Landing at the newest end while the newest messages are not loaded (Silica's quick scroll,
  or outrunning the forward paging) fetches the newest page again — the same one opening a
  chat loads.
- A message of our own sent while the window is back in the history brings the reader to the
  newest page; someone else's arriving message no longer pulls them out of their reading.
- Leaving the chat clears the window at once; putting the app aside collects what the
  JavaScript side still holds.

Found by testing it: jumping to the newest end while a page was still on its way put that
stale page into the new window, so Tuesday's messages sat where today's belong. Every page
request now carries the window's generation and is thrown away if the window was replaced
meanwhile; no page is asked for while a full reload is in flight.

All of it driven in the emulator with a window of 80 messages so it triggers in a 340-message
chat: `dropNewest` while scrolling back, `loadNewer got 50` … `got 0` coming forward, and the
jump to the newest end landing on today's messages.

## The cover names who is waiting (2026-10-30)

From the phone: with one chat left unread, the cover correctly showed one unread message, but with
the name of the last sender, not of the unread chat.

The name came from `window.lastSender`, set in `notify()` for every message a notification was
published for and never corrected afterwards. Leaving one chat unread and reading the next one
left the read chat's name standing under the count.

The name now comes from the same place as the count. `conversations_json` counts the chat list
and names its newest unread chat (`unreadTotal`, `unreadFrom`) — for every list it answers, so
the archive's or the private chats' own reply can no longer change either number (the archive's
reply used to overwrite the count with the archive's own total). The bridge keeps the two as
`unreadCount` and `unreadFrom`, the cover binds to them, and `lastSender` is gone. Everything
that marks a chat read already asks for the list again (`refreshUnread`, the `conversationRead`
event), so the name moves on as soon as a chat is read.

Unchanged: archived and private chats stay out of both numbers, and the name is hidden while
"Hide content on the cover" is on — now also while the app lock is up, which is what clearing
`lastSender` did there before.

## Media of our own messages from another device (2026-10-30)

From the phone: images sent from Threema for Desktop did not download automatically on the phone,
despite the auto-download setting.

`Store::media_to_download` only ever looked at `outgoing = 0`. A picture sent from Threema
Desktop reaches the phone as a *reflected outgoing* message (`reflected.rs`): the body carries
the blob ids and the file key, both parts in state "remote", but nothing ever asked for them —
not even the thumbnail, so the bubble stayed a placeholder until it was tapped.

Android does fetch them (`ReflectedOutgoingFileTask`: thumbnail always, the file if
`shouldAutoDownload`), and the download side here was already right for them — our own blobs
are taken from the device group's mirror in local scope (`app_media.rs`), because the public
blob may be gone once the recipient has it. Only the list was wrong. It now covers every file
message whose parts are still remote; what this device sent itself has them locally and is
never listed. A reflected message arrives as `MessagesChanged`, not `MessageAdded`, so that
event starts a download pass too when one of its messages is an outgoing file with a remote
part. Media missed before this fix arrive on the next start, when the pass runs anyway.

Not changed, and a difference to Android worth knowing: Android auto-downloads only files up
to 5 MB (`FILE_AUTO_DOWNLOAD_MAX_SIZE_M`), we have no such cap.

## Demo data for the emulator (2026-10-30)

Fixing the cover showed what was missing: the emulator's install carries the project's test ID,
whose only talkative contact is ECHOECHO, so two unread chats — the very thing the bug was
about — could not be staged there. Verification had to lean on unit tests alone.

The i486 build now carries made-up chats (`core/src/demo.rs`, behind the Cargo feature `demo`,
which `scripts/build-core.sh` switches on for i486 and nothing else; the UI entry sits under
Settings → Advanced and only appears when `backend.emulator`). Contents and the reasoning are
in `docs/emulator.md`. Everything is written straight into the store — no network, no blobs,
no key material — and `{"remove": true}` takes it away again.

With it, the cover bug reproduces and the fix shows: seven unread, "Carla Mendes" on the cover;
open and read her chat; four unread, "Farid Nazari". Before the fix the cover kept the name of
the chat that had just been read. `scripts/check.sh` now runs clippy and the tests with the
feature on as well, so the demo code cannot rot unnoticed.

## Export a chat, a size for automatic downloads, notes groups (2026-10-30)

Three gaps against Android, found while going through its features.

**Export a chat** (`core/src/app_chat_export.rs`, Android's `ExportConversationServiceImpl`):
the chat's pull-down menu writes `messages.txt` — `[date] Who: text <file>`, one line per
message, status lines left out as on Android — into a ZIP in Downloads, with the media that are
on the device if they are wanted, and with AES-256 if a password is given (the same zip crate
the data backup uses). The archive is written entry by entry on the blocking pool, so the core
keeps answering while a chat full of pictures is packed. The words in the file come from the
UI, like every other text: "Me", and the times in the device's own offset, which the header
states.

**File size for automatic downloads** (Settings → Media and storage): Android has a fixed 5 MB
limit, we had none. It is a setting here — any size (the default, so nothing changes for anyone
without asking), 1, 5, 20 or 100 MB — and it never holds back a preview, only the file itself.

**Notes groups** (a group whose only member is you, which Threema uses as a notebook): messages
written there were never reflected, so they stayed on the phone and never reached Threema
Desktop — `send_to_group` returned as soon as it saw that nobody was left to send to, before
reflecting. It now reflects first and leaves afterwards. Their blobs also go to the device
group's mirror in local scope instead of the public blob server, as Android does
(`isNotesGroup ? BlobScope.Local : BlobScope.Public`): nobody outside our own devices reads
them.

Checked and found already there, against the same list: per-chat notification tones
(`backend.chatTone`), mute with "until" and "only when mentioned", starred messages, the
archive, the per-chat media gallery and auto-delete.

## Cutting and turning a photo before it is sent (2026-10-30)

Android lets a picked photo be edited before it goes out; we sent it as it was. Tapping the
photo above the input (where it sits waiting with its caption) now opens `EditPhotoPage`: a
frame with four grips over the picture, dimmed outside it, and "Turn right / Turn left / Whole
picture" in the pull-down menu. Accepting writes a new JPEG into the app's capture folder
(`ThreeseBackend::editPhoto`, Qt's `QImage`) and puts it in the attachment's place; the
original file is never touched, and the EXIF data — the camera's GPS — stays behind with it, as
it does for everything we send. Android's drawing, text and sticker tools are not in this.

## What the start costs (2026-10-30)

`bench_start` (`cargo test -p threesea-core --release --lib bench_start -- --ignored
--nocapture`) measures what the core does between tapping the icon and the chat list, with a
store of 200 chats and 10 000 messages: opening it (SQLCipher's key derivation and the
migrations) 0.5 ms, `status` 0.01 ms, the first chat list 1.4 ms, the contacts 0.1 ms, opening
a chat 0.24 ms — on a desktop, so a phone is some multiple of that, but nowhere near what the
eye sees. Whatever the start costs on the phone is the QML, Sailfish Secrets and the first
connection, not the store; measuring that needs the device.

## The Jolla Store: the Lite flavour passes Jolla's validator (2026-10-30)

`scripts/validate-rpm.sh` runs Jolla's own Harbour validator (`rpmvalidation`, the check the
store applies) inside the SDK image. The Lite RPM passes every part of it — file and package
name, the `.desktop` file, icons, installed paths, linked libraries, RPATH, Sailjail
sandboxing, dependencies, scriptlets, architecture — once the file carries the plain name the
store insists on, which `FLAVOUR_SUFFIX=0 scripts/sdk-build.sh` now produces (releases on
GitHub keep `…-lite-…` so both flavours can sit side by side). CI validates every Lite build.

What is left for the store is not code: an account, the submission form with texts and
screenshots, and their manual review. Chum stays blocked on Rust 1.95 in OBS (CLAUDE.md §11).
Written down in `docs/release.md`.

## Two Sailfish integrations that are not within reach (2026-10-30)

- **Jolla's Backup** (Settings → Backup, the `vault`): nothing of it is in the emulator image
  (`rpm -qa | grep vault` finds nothing), a unit would have to be installed into a system path
  — which Harbour forbids, so the Lite flavour could never carry it — and there is no way to
  test it here. The app's own "Data backup" (ZIP in and out) and Threema Safe cover the same
  ground. If the phone has `sailfish-vault`, this can be looked at again with the device.
- **A Threema channel in the People app**: Sailfish gets those from telepathy, and telepathy
  is not in the emulator image either (`rpm -qa | grep telepathy`: nothing). A connection
  manager is a system service: Harbour forbids it, and even in the Full flavour it would be a
  piece of work of its own with no way to try it out here. The app is reachable from outside
  through its share plugin and `threema://` links instead.

## French (2026-10-30)

All 792 texts are translated, plurals included: `scripts/translations-fr.py` beside the German
one, `translations/harbour-threesea-fr.ts` in the `.pro` and in
`scripts/update-translations.sh`, and the built `harbour-threesea-fr.qm` is in the package
(checked in the RPM's file list). Style: vouvoiement, French typography (narrow space before
`: ; ! ?`, « quotes »), Threema's own terms (ID Threema, Threema Safe, confirmations de
lecture, sondage, transférer, débloquer); "discussion" for a chat, as Threema's French
interface has it.

The language cannot be seen in the emulator: its image carries only the C and POSIX locales
(`locale -a`), so Qt's system locale can never become French — nor German, which is why that
one was never seen there either. A phone set to French shows it; that check is in the device
checklist.

The order of the translation steps matters and is now written down in CLAUDE.md:
`update-translations.sh` (lupdate finds the new ids), then `translations-de.py` and
`translations-fr.py`, which fill the texts and write both files in the repository's layout.
Running lupdate afterwards reformats every line.

## Audit of this round (2026-10-30)

`docs/audit-2026-10-30.md`: the whole diff of the day read against CLAUDE.md §7, the
repository's checks run (clippy with and without the `demo` feature, 221 tests, `cargo audit`
over 306 crates, the logging rule), every feature driven in the emulator, and two things
verified that only a build can answer — that the demo code is absent from a device build
(searched for in the built library), and how the photo editor treats a picture with an EXIF
orientation flag.

Eight findings, all fixed in the same pass — the eighth raised by the user on reading the
audit: the demo entry was *hidden* in every build's QML rather than absent from it, with its
texts in the shipped translation catalogues. It is gated at build time in every layer now (the
QML lives in `emulator/` and is installed only by `qmake CONFIG+=demo`, the texts are plain
English and out of the catalogues), and CLAUDE.md §10 carries the rule: nothing for testing
ships, and the package is checked rather than assumed. The photo editor's and the export's would show on
a phone; the two demo ones cannot (that code is not in a device build), but the first of them
still produced real traffic from the project's test identity:

- **The demo data made the app send.** A demo message sat in state "pending", and the
  connection sends every outgoing message in that state after each connect — so the emulator,
  carrying the project's test ID, repeatedly sent a few test messages to a made-up ID until fixed. Opening
  a demo chat sent a read receipt to one as well. The demo data is inert now (nothing
  sendable, and "never send" for receipts and typing on every demo contact), with a test that
  keeps it that way.
- **The photo editor ignored the EXIF rotation** while everything else applies it, so a camera
  photo would have been cut in the wrong place and sent sideways. Both sides turn the picture
  now; checked with a photo stored rotated with orientation 6.

- **"Remove demo chats" froze its own settings page.** A `RemorseItem` anchors itself over the
  item it counts down on; inside a `Column` that stops the layout working at all, so the page
  would not scroll any more (the app was fine underneath). It uses the page-level
  `RemorsePopup` now, like LogPage's "Clear log".

The rest: the dialog's swipe stole the crop drag (and the grips measured themselves), the
export held media and long chats twice in memory, a sender's time could overflow the export's
clock, and the editor's result could land on the wrong attachment and failed silently.

## The chat list's emoji landed a line too low (2026-10-30)

From the phone: in the chat list's preview the
first emoji of a longer message sat one line below where it belonged, on top of the text, with
an empty gap where it should have been.

Not the asynchronous emoji provider (the pictures are misplaced just as much once they are in
the cache) and not the emoji sequence (the table has the whole waving man, skin tone, gender
and all, so it becomes one `<img>`). It is Qt: with `Text.StyledText`, the pictures of
`<img>` tags are positioned from the plain layout while the text drawn is the **elided** one,
so everything after the first line is off by a line. The chat list is the only place that
elides text with pictures in it — the bubbles don't elide, quote previews are plain text.

`Text.RichText` was tried instead: the pictures land correctly, but `maximumLineCount` is
ignored, so a long preview runs over the rows below. Dropping `elide` for the picture case
fixes the position and keeps the two lines — at the price of the ellipsis.

So the preview is now cut to length *before* it is drawn (`Format.previewLine`): the message
becomes one flowing line (runs of whitespace and newlines become single spaces, as Jolla's
lists show them) and is cut at a word boundary with an ellipsis when it is longer than the two
lines hold. The length is measured in character widths, not in code units — an emoji is about
two characters wide however long its sequence is (the waving man is seven UTF-16 units) — and
the budget comes from the label's width and font size (measured: 534 px at font size 36 holds
about 33 characters). Qt's own eliding stays for previews without pictures.

The message's shape is in the demo data now (`core/src/demo.rs`): an emoji straight after the
text, another at the end of the next line, more lines than the preview shows.

## Renaming a linked device was refused (2026-10-30)

From the phone: Settings → Linked devices → rename another device answered "Something went
wrong: invalid request".

That message is the core's reply when it cannot even parse the request. The reason is the
envelope: the bridge puts the command's arguments and the request's own `id` and `cmd` into
**one** JSON object (`{"id": 7, "cmd": "setDeviceAlias", …args}`), and `setDeviceAlias` took
the device's hex id in a field called `id` — which the bridge then overwrote with the request
number. The core saw a command without its `device` field and refused the whole request.
Renaming *this* device (`setDeviceLabel`, no such field) always worked, which is why it looked
like a problem with other devices only.

Confirmed working on the phone (user, 2026-10-30), which is where the real device group is —
the emulator had none left to rename after the mishap below.

The field is called `device` now. The same trap had been met once before and only noted in
passing (`removeGifFavourite` takes `favourite`, "not `id`: the bridge's request id"), so this
time it is written down where it belongs — in `docs/ffi.md` next to the envelope — the bridge
warns when a command is called with an argument named `id` or `cmd`, and a test walks the
`Command` enum to make sure no field of either name comes back.

### A warning from that session

Driving the emulator, a pull-down on Settings → Linked devices was meant to refresh the list.
In Silica a pull-down *selects* the first pulley item, and that item is "Remove all linked
devices": the action ran, and the emulator — which carries the project's own test Threema ID —
left its device group. The list had shown only that one device, so nothing else was dropped, and no
data was lost; the app there now connects without multi-device.

For anything driven by script on that install: destructive pulley items are one gesture away,
so open a pulley with a screenshot before releasing it, or use taps only. The test ID is a
registered identity with a licence of its own — cheap to disturb, not free.

## Many favourite GIFs slowed everything down (2026-11-02)

Report: with many favourite GIFs the app slowed down badly. Each tile decrypted
its whole GIF on the GUI thread when it was created, then decoded every frame there at full size
and painted it through a framebuffer of its own. Tiles just outside the view had no reason to stop.

Now `bridge/gifdecoder.*` reads and decodes on one shared thread, sends frames scaled to the item,
and sends no further frame until the last was taken; `AnimatedImage` draws them as plain textures.
The GIF tab loads a tile only when it is asked to play (`onDemand`): on screen and the grid at
rest. So chats and the viewer gain too, and with auto-play off the tab decrypts no GIF at all. One
shared thread is deliberate: too many GIFs play slower rather than take more of the phone. Its cost
is that on a weak phone a full grid may visibly play below speed.

Measured in the emulator, 40 favourites, GIF tab open (`docs/emulator.md`; one run before, three after):

| | before | after |
|---|---|---|
| GUI thread, tab showing | 29 % of a core | 0 % |
| whole app, tab showing | 85 % | 32–38 % |
| GUI thread, 6 flicks | 523 ticks | 14–16 ticks |
| whole app, 6 flicks | 1451 ticks | 364–406 ticks |

The "interface stopped" lines did **not** improve there: 0–13 per 20 s before, 7–13 after, up to
965 ms. They are no measure in the emulator (they varied from 0 to 13 on the same code). While they
occurred, the GUI thread was mostly idle in `poll` and blocked on the render thread in about 6 % of
samples, and the compositor used about 3.5 times the app's CPU on two virtual CPUs. That points at
software rendering, but it is an inference: whether it feels right is for the phone
(`docs/voice-video-device-checklist.md`, Q).

`scripts/test-gifdecoder.sh` covers the decoder, in `scripts/check.sh --full`. The SDK target has no
GIF plugin, so the script installs it into its throwaway container, which takes network access.
The demo data now holds 40 favourite GIFs and a GIF message with a transparent part. `add` over
existing demo data fails with "internal" (cause not looked into, probably message ids): `remove` first.

## Videos were black; voice messages would not go away; the system's media controls (2026-11-02)

User report from the phone: videos stay black, a voice message is hard to get rid of once started
and resumes after re-opening the app; could they use the system's media controls?

**Black video.** `VideoOutput` finds its player through a property called `mediaObject` on its
source (or `videoSurface`). The source was a bare `QMediaPlayer`, which has neither, and then
VideoOutput shows nothing and says nothing while the sound and the clock run on. Now the source is
`VideoPlayer` itself, with a `mediaObject` property. Control run in the emulator: pointing it back
at the bare player gives exactly the report, black with the time running; with the fix the picture
shows. The emulator decodes in software through Qt's renderer. A phone goes through the Nemo
texture backend (`droideglsink`), which only the phone can confirm.

**Voice.** Nothing ever stopped a message: leaving the chat kept it loaded, closing the window kept
it playing (the process stays for the connection), and the only control was the bubble in that
chat. Now:

- Closing the window ends playback, voice and video (the window's close event). Not
  `visibleChanged`, which was the first attempt: the window goes invisible on minimizing as well,
  so it cut every message off when the app was put aside.
- Leaving a chat drops a paused message; one that is playing goes on and ends by itself. A private
  chat's stops with the page.
- A message left paused for 10 minutes is dropped (its decrypted audio is in memory until then).
- The app lock stops playback when it is on. `lockNow()` also runs on coming back after the grace
  time whenever a lock record exists, with the app lock off too (`appLock:false, graceSecs:30`),
  so the stop is inside the app-lock branch.
- Only the user starts sound. Qt's resource policy resumes a player by itself when it gets the audio
  back (`handleResourcesGranted`); `m_wantPlaying` turns that back into a pause. This is the likely
  source of the "resumes after re-opening", read from Qt's behaviour and not reproduced: the emulator
  has nothing that takes the audio away.
- The proximity sensor no longer restarts a message paused by hand, only one paused by taking the
  phone off the ear. Something already covering the sensor when it starts is not a gesture
  (1 s after the start). Stopping during the chime between two messages ends the queue.

**System controls.** `qml/components/MediaSession.qml` announces what is playing over MPRIS
(`Amber.Mpris`, Harbour-allowed, `amber-qml-plugin-mpris` required by the RPM), only while something
is loaded. Sailjail's `Audio` permission allows the name (`org.mpris.MediaPlayer2.*`), and our
`Microphone` includes `Audio`, so the `.desktop` needed no change. The service registers as
`org.mpris.MediaPlayer2.harbour-threesea.instance<n>`. The artist line is the chat's name, under the
same rule as the cover ("Hide content on the cover", the app lock; never for a private chat).

What Sailfish does with it: `lipstick-jolla-home-qt5/lockscreen/LockItem.qml` is the only user of
these controls, so they are on the **lock screen, not in the top menu**. They offer play/pause and
previous/next, **no position slider**. Seeking is implemented (`Seek`, `SetPosition`, the rate as the
playback speed) for whatever else speaks MPRIS. Next/previous are off: a message is not a playlist.
Amber's QML side is in milliseconds for position, duration and seek and converts for D-Bus; its
`qmltypes` don't say so, a first version multiplied by 1000 and the D-Bus test showed 714 s on a
4 s message.

Tested in the emulator (i486, media service installed for it, docs/emulator.md): video picture and
the control run above; over D-Bus Pause, Play, PlayPause, SetPosition, Seek, Stop, with the values
checked; the lock screen showing "Voice message", the sender and "Threesea", and its own button
playing and pausing; voice going on in the chat list and on the home screen; a paused message
surviving the minimize and dropped on going back; closing from the cover while playing stops it, the
service goes, the process stays, re-opening stays silent; Stop on a video closes its page; "Hide
content on the cover" taking the name off the metadata and the lock screen. The Lite RPM passes
Jolla's validator under the plain file name.

Seen once and not explained: on the first build, with the `visibleChanged` hook, the service stayed
up after a stop with nothing loaded (`trackid …/0`, `CanSeek` false). A `play()` that failed left
`active` stale and is fixed, but nothing shows that was this case, and six later runs did not
repeat it. Any stop clears it, closing the app included.

Not tested: all of it on the phone (`docs/voice-video-device-checklist.md`, section R), headset and
Bluetooth buttons, the proximity changes (no sensor in the emulator), the 10 minutes, the app lock's
stop, the automatic resume being held back.

Noticed on the way, not changed: Settings → Security says "App lock on" whenever a lock record
exists (`SettingsPage.qml`), also with `appLock:false`.

## Linking a number, and what a sync reports (2026-11-04)

A tester asked why no verification SMS arrived. Linking isn't needed to find friends: the address book
switch is enough, `match_address_book` never reads our own links (nor does Android's
`IdentityLookupClient`). Linking only lets others find you.

Done: the link dialog takes only numbers with a country code and shows the one the code will go to
(ADR 0016); `(0)` and Hungary's `06` in `normalize_phone`; a notice when Threema answers "already
linked", which sends no SMS or e-mail and used to show nothing, possibly what the tester hit (not
confirmed by them); a manual address book sync always reports its result.

Not fixed, waiting for a decision: the country for address book numbers without `+`, and the Italian
and US/Canadian rules (ADR 0016).

Tested: the core's tests; in the emulator, a national number (hint, can't accept), `+49 (0) 171 …`
("+49171…" shown, can accept; cancelled, nothing was sent), and the "none found" notice with an empty
address book. Long notices scroll, so their whole text can be read.

Not tested: an actual link, the "already linked" notices, the "entries checked" and failure notices,
and a real address book (`docs/voice-video-device-checklist.md`, section U).

Open, and it may matter more than the rest: the reader may not read anything. In both app runs tried in the
emulator the log says `Failed to attach key memory region for qtcontacts-sqlite-np`, `Failed to open
contacts transient store`, `Unable to open synchronous engine database connection`. A contact added by
hand to the emulator's database (no aggregate, so that alone may explain it) came back as "none found".
Whether this is the emulator, the sandbox or that entry isn't known; a phone with a real address book
decides it, and if it fails there too, the sync has never worked on a device. `AddressBookReader.qml` also
reads details right after the asynchronous `ensureComplete()`; the plugin's source suggests
`requiredProperty` already loads them, unconfirmed.
