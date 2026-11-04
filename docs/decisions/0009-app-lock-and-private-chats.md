# 0009 — App lock and private chats

## Context
Android has three related features: an app lock (PIN, system screen lock or biometrics, with a
grace time), private chats (hidden from the chat list and search until unlocked, notifications
without content) and a master-key passphrase (the database key is encrypted with it and must be
entered after every start). The user decided (2026-10-22):

- Unlock with an own PIN/passphrase **or** the device lock code, the user's choice (the device
  code turned out not to be available to apps, see below).
- What the lock protects is a setting: a screen lock only (default), or additionally encrypting
  the database key with the passphrase.
- App lock and private chats both, like Android.
- Private chats are hidden like on Android; they are reached from the chat list's push-up menu
  (next to the archive), and opening that list already asks for the PIN.

## Decision
- **PIN/passphrase:** stored only as `scrypt(N=2^14, r=8, p=1)` with a random 16-byte salt in
  the SQLCipher store (setting `lock`); never in plain text, never logged. Checked in the core.
- **Screen lock (default):** the database key stays in memory, so the connection, delivery and
  notifications keep running. The UI shows the lock page on start and after the grace time.
  The core keeps an in-memory *unlocked* flag: while locked it leaves private chats out of every
  list (chats, archive, search, starred, media), refuses their messages, and redacts their live
  events (2026-10-23 security audit): `messageAdded` carries only conversation, direction, time
  and — for status lines, which are app-made codes — the body, so the "Private chat"
  notification works without content ever crossing to the UI layer; `messagesChanged` leaves
  their messages out, typing events are not forwarded. Enforcing this in the core keeps the
  QML dumb (§6) and makes a QML bug unable to leak a private chat.
- **PIN or passphrase:** digits on Silica's keypad, or (keyboard button top left, like the
  device lock) a passphrase; the screens follow the system's code entry (title in the upper part,
  dots right-aligned above the keypad with backspace beside them, cancel bottom left, accept
  bottom right once long enough).
- **Key encryption ("Encrypt stored data", needs ≥ 8 characters like Android's master key
  passphrase):** Secrets then holds `TSK1 || keyboard flag || salt || nonce ||
  XSalsa20-Poly1305(scrypt(N=2^16, r=8, p=1)(passphrase, salt), key)` instead of the key. The
  bridge asks for the passphrase before the core can start (state `locked`; nothing is received
  meanwhile, a notification says so if the app stays in the background) and keeps the key in
  memory only. Secrets cannot overwrite a secret, so replacing it goes through a staging copy
  (`databasekey-new`): at every step one valid copy exists, and the start finishes an interrupted
  replacement. Changing the passphrase re-encrypts; removing the lock stores the plain key again.
- **Device lock code: not possible.** Probed in the emulator on SFOS 5.1 with a device code set
  (2026-10-22): `org.nemomobile.devicelock`'s QML `Authenticator` reports no available methods
  to an ordinary app (its socket `/run/nemo-devicelock/socket` is `root:privileged`), and the
  D-Bus `RequestPermission` call returns without showing anything. Only privileged system apps
  (Settings) can ask for the device code. The module isn't Harbour-allowed either. Revisit if
  Jolla opens an API.
- **Private chats:** table `private_chats` (migration 19), a local mark like Android's
  `ConversationCategoryService`; marking needs a lock to be set up. Threema Safe carries
  Android's `private` flags for contacts, groups and lists.

## Consequences
- §7 gains: the lock PIN is stored only as a slow hash in the encrypted store; with key
  encryption on, the database key exists unencrypted only in memory after the user unlocked.
- A forgotten PIN in screen-lock mode can only be reset by removing the app's data from the
  device (restore from backup; see the update of 2026-11-04); in key-encryption mode the local data is lost without the passphrase,
  like Android.
- The screen lock does not protect against someone with root access to an unlocked phone; key
  encryption does, while the app is not running.

## Update 2026-10-23: separate passphrase for the stored data

The user chose Android's model: encrypting the stored data has its **own passphrase** (Settings →
Security → Encryption of stored data), independent of the app lock. Either can be on without the
other. The lock no longer records anything about the key (`keyEncrypted` removed); whether the
key is encrypted is what Secrets holds (the bridge keeps the encrypted key to check the passphrase
before changing or removing it). The keypad/keyboard kind of the passphrase is read from the
stored blob. Minimum length stays 8 (Android's master key passphrase).

## Update 2026-11-04: the lock is a state, the page follows it

The lock used to be just its page: any pop or replace could remove it, and whether it was due was
only decided when the app came back. Now `window.locked` stands from the moment the app locks
until the PIN was accepted, and `settleStack()` (`qml/harbour-threesea.qml`) keeps the page on top,
also after a transition or when anything else took it away. The license page goes under it, since
it shows the stored key. Links, notifications and shares wait until after the PIN.

The grace time runs on a timer while away, so the cover, the media controls and the private chats
close when it ends; coming back and each background wake-up also compare the clock, because timers
sleep with the device. A voice message playing at that moment plays to its end without the name
and nothing follows it (with the phone in a pocket it would otherwise end after the grace time);
coming back past the grace time stops it, and "Immediately" stops it on leaving. A running call's
page is the one page allowed above the lock: it shows only who calls.

## Update 2026-11-04: one gate in the core

Private marks can arrive without a lock (Threema Safe restore, a linked device). Those chats stay
hidden, and `unlock` answers `lockNeeded` until a PIN is set, so no PIN opens them; the new PIN
then does, and the marks can be taken off. Removing the lock or the ID ends what was unlocked.

Whatever takes the ID or the chats off the device or gives someone else a hold on the ID (ID
backup, data backup and its schedule, turning Threema Safe on or off, the revocation password,
linking a device, removing the ID) needs the PIN in the core whenever a lock exists, also within
the grace time, like Android. The UI asks before opening those pages (`window.pushWithPin`);
linking asks on `LinkDevicePage`, where every route ends. The app locking closes the page
holding a PIN, the lock settings included. Scheduled backups run without it. So
a forgotten PIN can't be reset from inside the app any more: that takes deleting the app's data
folder, then restarting the phone, then restoring from a backup; Settings → Security → App lock
says so with the folder's path, and the README, which a locked-out user can still read. The restart
is a step of its own because with "Keep running in the background" on (the default) closing the
window leaves the process running, with the deleted database still open and the same lock up, and
the setting is behind the lock. The Full flavour's login service starting the app again is fine:
the folder is gone by then. Doing it in the other order isn't: the service would have the database
open again before the delete. All this is by reading the code, still to be confirmed on a phone
(`docs/voice-video-device-checklist.md`).

It covers the app lock only. The wrapped key of the stored data's passphrase lives in Sailfish
Secrets, not in the folder: after the delete and the restart the passphrase page comes back, and
the right passphrase then starts an empty app. A forgotten passphrase has no way back in; nothing
in the app removes that secret. The lock page offers no removal: nothing there tells the owner
from anyone else holding the phone. Creating or restoring an ID starts only on a
device without one (`identityExists`), so those commands can't replace an ID around the PIN.

A mark from anywhere (the UI, a linked device, a restore) goes out as `CoreEvent::PrivateChanged`:
the media readers follow at once, the chat's notification closes, a connected browser loses the
group or list.

Leaving a private chat hides the private chats at once, and that holds with files still being
prepared, which reach the core only after the lock. The lock names them (`pendingFiles`) and they
alone may still be stored and sent, for 60 s, counted per chat: a write-only pass, which opens
nothing else, answers `{}` and is cleared by an unlock or deleting the data. What comes after it is
refused as locked, and the UI says so. A private distribution list gets no pass, as its members'
copies are made by reading its own back.

## Update 2026-11-04: notifications respect the lock

While the app is locked (`window.lockActive()`, which also compares the clock, since timers sleep
with the device) one notification stands for everything, like on Android: neither who nor what, no
Reply or Mark as read, and tapping it only opens the app. It keeps the chosen sound and vibration.
When the lock comes on, what is on the screen about a chat closes, and with the app ending under a
lock as well. Its token only starts its grace, as a tap may still be on its way. Messages from before the first look at the lock wait for its
answer. A Reply or Mark as read tapped earlier and still on its way waits for the PIN in memory, as
it does for the passphrase, and the notification says so. This is a matter for the window, not the
core: the core has no app lock state of its own, its `unlocked` flag is the private chats'.
