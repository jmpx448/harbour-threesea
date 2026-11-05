# Core ⇄ bridge protocol

The Rust core runs on its own thread (`core/src/runtime.rs`); the Qt bridge talks to it through
a small C API (`core/include/threesea_core.h`) and JSON messages. JSON keeps the boundary
small and debuggable; secrets inside messages are wiped on both sides after use.

## C API

| Function | Notes |
|---|---|
| `threesea_core_start(config_json, db_key, 32, callback, user_data)` | Starts the core thread. `db_key` comes from `SecretStore` (Sailfish Secrets) and is copied. |
| `threesea_core_send(core, request_json)` | Queues a request, never blocks. |
| `threesea_core_stop(core)` | Stops and joins the thread, however long what it is doing takes; no callbacks after it returns. |
| `threesea_core_quit(core)` | For the end of the process: requests in the queue and the running one get 3 s, then a wait that goes on is dropped and what was queued behind it and file work on its way get 1 s, and a file that doesn't make it gets no answer; 5.5 s at most. True: as `threesea_core_stop`. False: the thread still runs and can still call back, so the caller keeps `user_data` and leaves with `_exit`. |
| `threesea_data_removal_mark(data_dir)` | The core's mark of a removal of the ID that never finished (the app was killed in it): -1 none, else whether linked devices remain (0 or 1). The bridge then finishes it. |
| `threesea_data_remove(data_dir)` | With the core stopped: removes everything in the data directory (database, media, logs), the last step of removing the ID (ADR 0018). False if something stayed. |
| `threesea_core_version()` / `threesea_string_free()` | Version string. |
| `threesea_core_send_data(core, request_json, data, len, thumbnail, len)` | A request with binary attachments (`sendFile`, pictures); never blocks. |
| `threesea_media_read(core, message, thumbnail, &len)` / `threesea_picture_read(core, name, &len)` | Decrypted media for the UI's image providers (other threads). NULL while the message's private chat is hidden (ADR 0009); of a contact's picture also while "Show profile pictures" is off (their `pictureVersion` is 0 then, too). |
| `threesea_voice_decode(core, message, &len)` | Opus voice message → 16-bit PCM (no platform decoder); NULL as above. |
| `threesea_video_size(path, &width, &height)` | A video's picture size as shown, from its index (MP4, QuickTime); false if it can't be told. For deciding whether to scale it (Full). |
| `threesea_buffer_free(buffer, len)` | Wipes and frees a buffer from the readers above or the key functions. |
| `threesea_qr_decode(grey, width, height)` | First QR code in a grey image (camera scanning); no core needed. |
| `threesea_group_call_encrypt` / `_decrypt` / `_vp8_packets` | Group call frame encryption on GStreamer's threads (ADR 0012). |
| `threesea_call_vp8_filter` | Whether a received VP8 frame may reach the decoder (its size), on GStreamer's threads. |
| `threesea_key_wrap` / `_unwrap` / `_wrapped` | The database key wrapped with a passphrase (scrypt; "Encrypt stored data", ADR 0009). |

`callback(user_data, json)` runs **on the core thread**; the bridge copies the string and posts
it to the GUI thread (`ThreeseBackend::handleMessage`).

Start config: `{"dataDir", "appVersion", "language", "country", "model", "osVersion",
"picturesDir", "calls" (Full build), "logLevel", "updateCheck" (builds for GitHub)}`.

## Messages

Request: `{"id": 7, "cmd": "<name>", ...args}` →
reply `{"reply": 7, "ok": true, "result": {...}}` or
`{"reply": 7, "ok": false, "error": {"kind": "<kind>", "message": "..."}}`.
Replies carry their request's id and need not come in order: a command that waits for the network
or a slow file runs beside the core's loop and replies when done, and the commands behind it are
answered meanwhile (ADR 0028). A request that depends on an earlier one waits for its reply.

The arguments sit in the same object as `id` and `cmd`, so **no argument may be called `id` or
`cmd`**: the bridge would overwrite it with the request's own number and the core would refuse
the request as a whole ("invalid request"). Name it after what it is — `device`, `message`,
`favourite`. The bridge writes a warning when an argument of that name is passed.

Events (unsolicited): `{"event": "started", "version"}`, `{"event": "upgrading"}` (before `started`: an existing store's schema is being brought up to date, which can take a while),
`{"event": "fatal", "kind", "message"}` (the core didn't start. `kind`: `storeKey` the key doesn't fit or the first page is damaged, which nobody can tell apart; `storeDamaged`, `storeTooNew` the data comes from a newer version, `diskFull`, `upgradeNoRoom` the upgrade ran out of room and was rolled back, `storeUnreadable` I/O, permissions, a file that won't open, `store` anything else about the store, `internal`. `message`: a code for support, empty for `internal`),
`{"event": "safeChanged"}` (Safe went on or off, a backup ran, its error state changed, or the warning in `safeStatus` stands: hourly then),
`{"event": "backupReminder"}` (a new ID has no backup of any kind and hasn't declined one: after a month, then every three months),
`{"event": "safeServerCopyLeft"}` (`safeDisable` couldn't delete the server copy; the window tells the user, as the page may be gone),
`{"event": "contactsChanged"}` (contacts imported in the background; the bridge reloads the model),
`{"event": "groupsChanged"}`, `{"event": "profileChanged"}` (the own profile/picture changed,
also when synced from another device), `{"event": "settingsChanged"}` (changed on another device;
what shows the settings loads them again),
`{"event": "connectionState", "state"}`, `{"event": "messageAdded", "message"}`,
`{"event": "messagesChanged", "messages"}` (acks, receipts), `{"event": "typing", "identity",
"typing"}`, `{"event": "offlineReason", "reason"}`: what Threema's servers said, or why they don't let us in, for the UI to show until the state is `connected` again: `{"kind": "server", "text"}` (an alert or close error in the chat server's words, not "Another connection"), `{"kind": "mediator", "code"}` (a 41xx close, which the UI words) or `{"kind": "loginRefused"}` (the server has ended every login for at least five minutes, without a word and with nothing else going wrong in between). The server's words come without markup. `{"event": "messageNotify", "message", "notify", "private"}`: a message another device received first (it leads) and that is still unread after a grace time, or at the end of the mediator's backlog — its `messageAdded` came with `notify: false`. `{"event": "privateChanged", "identity", "private"}`: a chat was marked private or unmarked, here, on a linked device or by a restore. `{"event": "deviceCookieChanged"}`: the chat server saw the ID log in with another device cookie (Android's rogue-device warning; the core has already cleared the server's flag). `{"event": "storageProblem", "active"}`: the store fails while receiving, so what the server sends stays unacknowledged and comes again (`active: true`, once per streak; the code is in the log), or works again (`false`). `{"event": "backlog", "active"}`: the servers have begun handing out what they kept (`true`, after a login or on becoming the leader), or are through with it or the connection has ended (`false`). Everything received in between is one piece of news to the UI, which alerts once at the end. `{"event": "wakeSettled"}`: what a `wake` was for has happened (something was received and has stopped coming; after a new login, the servers' queues are through; transfers running have ended) or won't soon (the attempt failed, the next is a while off): the device may sleep again. `{"event": "deviceRemovalFailed"}`: after the error reply of `dropDevices` or `deactivateMultiDevice`, so the UI can tell the user when the page that asked is gone.

Call events (Full, ADR 0012): `callIncoming {identity, callId, sdp (patched), video, private}`,
`callRinging`, `callAnswer {sdp, video}` (`video`: both sides take video; else it is an audio
call now), `callCandidates {candidates}`, `callEnded {record: {peer, callId, outgoing, result,
duration, at}}` (`duration` counts from `callConnected`), `callChannelSend {data}` (base64, for the call's data channel),
`callRemoteCapture {device: camera|microphone, on}`, `callVideoParams {bitrateKbps, fps, width,
height}` (what our video sender may use: Android's quality profiles, the lower of both sides,
MAX only without a relay). Contact JSON has `canCall` and `canVideoCall`. The commands about a
running call name it with `callId`, and do nothing if another call is current by then: `callEnded`
of the earlier call can still be on its way. Before `callStart`'s reply the bridge has no id, so it
keeps its candidates and a hangup until then.

A group call also shows in its chat (Full, ADR 0012) as status lines: `groupCallStarted|<name>`,
`groupCallYouStarted`, `groupCallEnded`. The first comes without `notify`; a `messageNotify` follows
once a peek found the call running. In a private chat while locked the line has no name, no
`groupCall` event comes for that group, and `groupCallParticipants {conversation, participants}`
comes with empty names. On locking, `groupCallsLocked {keep}` names the groups whose running calls
the UI may keep; the private ones come back when their chat refreshes.

Group call events (Full, ADR 0012): `groupCall {conversation, call}`, `groupCallOffer {sdp}`,
`groupCallParticipants {participants}`, `groupCallChannelSend {data}`, `groupCallAlone`, and
`groupCallMicrophoneOff`: the call we joined has 4 or more others, so we start muted like Android
(the core already counts the microphone as off; the bridge mutes the engine).

| `cmd` | Args | Result |
|---|---|---|
| `status` | – | `{state: needsLicense\|needsIdentity\|ready, identity}` |
| `checkLicense` | `licenseKey` | `{updateMessage}`; stores the key on success |
| `spamPotential` | `identity` | `{potential}`: this chat (contact or group) offers a spam report |
| `dismissSpam` | `identity` | `{}`; this chat offers no report any more |
| `reportSpam` | `identity`, `block` | `{}`; reports to Threema's directory, then with `block` blocks the sender or group creator, leaves the group and deletes the chat. Nothing else happens if the report fails |
| `licenseStatus` | – | `{key, refused}`: the stored key, and the directory's message if it refused it |
| `forgetLicense` | – | status; removes the stored key to enter another; error `identityExists` on a device with an ID |
| `createIdentity` | – | `{identity}` (uses the stored license); error `identityExists` on a device with an ID |
| `restoreIdentityBackup` | `backup`, `password` | `{identity}` (needs a license first); error `identityExists` on a device with an ID |
| `exportIdentityBackup` | `password` (≥ 8 chars), `pin` | `{backup, qr: {size, bits}}` (`qr`: the modules of a QR code holding the backup text, like `myQrCode`'s) |
| `safeCheckPassword` | `password` | `{result: tooShort\|tooLong\|weak\|ok}` |
| `safeStatus` | – | `{enabled, askSetup (a new ID's question whether to turn it on, until answered), server, lastBackup, lastError, warningDays, size, maxBytes, retentionDays, restore}`; `backupReminder`: a new ID's reminder is still running; `warningDays`: backups have failed for at least a week, the days since the last that worked (else since the first failure), otherwise null; `restore` = `{pendingContacts, pendingGroups, pendingLists, importError, summary}` — what a restore still has to import (retried on reconnect, hourly and via `safeRetryImport`), why the last attempt failed, and the import's counts (`summary`: `{contacts, revokedContacts, groups, lists, skippedGroups: {deleted, unknownCreator, invalid}, at}`) |
| `safeEnable` | `password`, `pin` | status; derives the key, uploads a first backup |
| `safeBackupNow` | – | status |
| `safeDisable` | `pin` | status with `serverCopyDeleted`; deletes the backup from the server, then forgets the key. Off even if the deletion fails (`serverCopyDeleted: false` and the event `safeServerCopyLeft`, as the page may be gone by then; that copy expires with the server's retention) |
| `safeRestore` | `identity`, `password` | `{identity, contacts, groups}` (needs a license first); error `identityExists` on a device with an ID |
| `safeSearchId` | `phone`, `email` (either may be empty) | `{identities}`, the IDs linked to them, for a device with no ID yet (needs a license; hashed in the core, posted without a match token, inactive IDs included); errors `identityExists`, `idSearchInput` (nothing to look for) |
| `safeDeclineSetup` | – | status; the answer to `askSetup`: go on without a backup, which ends the reminder too |
| `safeRetryImport` | – | status; tries the rest of a restored backup's import now |
| `androidBackupRestore` | `path`, `password` | `{identity, imported}` for a device with no ID yet (needs a license): the ID from a Data backup, and its contacts, groups, lists and pictures (`imported`, shaped like `androidBackupImport`'s reply with scope `metadata`; null if that part failed after the ID stood, for the import to try again). The history follows with `androidBackupImport`. Errors `identityExists`, `backupDecrypt`, `backupVersion`, `backupInvalid`, `backupNoIdentity` |
| `dismissDataImportHint` | – | profile; ends `dataImportUnfinished` (in `profile`: a restore from a Data backup brought the ID and no history import has finished since, for the chat list's hint) |
| `androidBackupInspect` | `path`, `password` | `{version, identity (match\|otherIdentity\|keyMismatch\|unreadable\|missing), inventory: {contactChats, groupChats, listChats, mediaEntries, mediaBytes (declared, an estimate), thumbnails, contactAvatars, contactProfilePictures, groupAvatars, ownAvatar, has…, unknownEntries}}` — looks into an Android data backup ZIP, entirely locally; errors `backupDecrypt` (password), `backupVersion`, `backupInvalid` |
| `androidBackupImport` | `path`, `password`, `scope` (`metadata`\|`history`) | `metadata`: `{contacts: {added, existing, updated, keyConflicts}, groups: {added, existing, skippedDeleted, skippedUnknownCreator}, lists: {added, existing}, pictures: {own, contacts, groups, kept, localAvatarsNotImported, skipped}, nonces: {csp, d2d, existing, skipped, missing}, invalidRows}` — merges the archive's contacts, groups, lists and pictures into the running identity, sending nothing to anyone but the user's other linked devices, which get the contacts and groups it added and the own and group pictures it filled in (not lists: no device sync has them, nor messages); an import that fails leaves no row behind; existing local data wins; idempotent. If it added contacts, the regular contact refresh runs right after the reply, for what they support. Only a backup of the same ID and key (`backupIdentity` otherwise). `history`: starts the background job (messages, media, reactions; one at a time) and replies `{started}`; progress and outcome come as events: `{"event": "androidBackupProgress", step (messages\|media), done, total}` and `{"event": "androidBackupDone", ok, summary \| kind}` with `summary` = `{chats, skippedChats, tooLargeChats, damagedChats, leftOutChats: [{conversation, name, reason (notHere\|tooLarge\|damaged\|otherKey)}], keyConflictChats, keyConflictRows, imported, existing, collisions, unsent, invalidRows, unsupported: {category: n}, compat, mediaFiles, thumbnails, mediaMissing, mediaFailed, reactions, reactionsUnmatched, reactionFilesLeftOut, polls, uidConflicts, foreignGroupChats, cancelled}`; it ends with `chatsChanged`/`contactsChanged` so the lists reload. Old messages are placed into history by time (sort_key), never resent; outgoing rows that never left Android stay failed; existing rows (live edits, tombstones) always win, and so do existing reactions: one there is never changed, and an archive's reactions come once, so importing it again doesn't bring back one removed here; repeated runs resume/add nothing. A reaction file over the memory bound or damaged is left out and counted (`reactionFilesLeftOut`). A list of used nonces over the memory bound goes unread and reports `nonces.missing`. A chat file over the memory bound (`tooLargeChats`) or damaged (`damagedChats`) is left out and counted, the rest goes on. `leftOutChats` names at most 100 of the chats left out, never a private one and none without a name; the counts hold them all. Nothing the archive holds in the name of a contact whose stored key differs goes in: their chat (`keyConflictChats`), group messages and reactions (`keyConflictRows`), picture (`pictures.skipped`). Except what names them without speaking for them: a group the import creates keeps them as creator or member, with the group's name and picture from the archive, and their votes in a poll of someone else stay in that poll. A store error ends the job with `ok: false`; importing again continues. |
| `androidBackupCancel` | – | `{}`; stops a running history import between batches (done work stays; running the import again continues) |
| `androidBackupExport` | `directory?`, `password?` (omitted: the stored schedule settings), `pin` | `{started}`; writes an Android-format backup (format 27: settings, identity, contacts/groups/lists, messages, local media+thumbnails, pictures, reaction files; status lines other than 1:1 calls and polls in lists are counted as skipped, nonces go out hashed; chats without a contact row try the directory first, else count in `summary.skipped.chatsWithoutContact`; media that can't be read count in `summary.skipped.filesUnreadable`, the backup goes on) as a background job; `{"event": "androidExportProgress", done, total}` and `{"event": "androidExportDone", ok, path, summary \| kind}`; errors and `kind`: `backupRunning`, `backupNoFolder`, `backupNoPassword`, `passwordTooShort`, `backupWrite` (the folder), `backupNoRoom` (its storage). The file is `threema-backup-<date>-<time>.zip`, written as `.part` first; afterwards the rotation keeps the newest `keep` of our files in the folder |
| `androidBackupSchedule` | `directory?`, `password?` (min 8 chars; kept in the encrypted store for unattended runs), `interval?` (`off`\|`daily`\|`weekly`), `keep?`, `pin` | status; maintenance starts a backup when one is due, at most every 6 hours after one that didn't succeed. A backup stops with `backupNoRoom` rather than leave less than 100 MiB free, or before it starts if the room is less than the last backup took. `daily`/`weekly` without a folder or password given or stored: `backupNoFolder`, `backupNoPassword`, and nothing is stored |
| `androidBackupExportStatus` | – | `{directory, interval, keep, lastBackup, lastError, chatsLeftOut, filesLeftOut, passwordSet, running}`; `chatsLeftOut`: chats the last backup could not hold because no contact key could be fetched for them , `filesLeftOut`: media files it could not read (each: private ones count only while the private chats are open) |
| `contacts` | – | `{contacts: [contact…]}` (direct contacts, sorted by name with accents under their letter, each with its `section`: the letter, or `#` for all names that start otherwise, at the end); also fills `backend.contacts` |
| `contact` | `identity` | contact |
| `addContact` | `identity` | contact (directory lookup, unverified) |
| `updateContact` | `identity`, `firstName`, `lastName` | contact |
| `deleteContact` | `identity` | `{}`; out of the contact list, its chat, what was chosen for it and what was queued for it gone, and on the exclusion list of the address book sync (ADR 0022); a hidden private chat stays as it is. Also `contactsChanged`, and the events of `deleteConversation` |
| `declineNickname` | – | profile; the question for a nickname isn't asked again, messages keep leaving contacts' stored name alone |
| `profile` | – | `{identity, nickname, askNickname (a new or ID-backup-restored ID: ask once), publicKey, linkedEmail, linkedMobile, pictureVersion, pictureShare (everyone\|nobody\|selected), pictureRecipients: [ID]}` |
| `linkEmail` | `email` ("" unlinks) | profile (`linkedEmail`, `linkedEmailPending` until the link in the e-mail is opened; checked again on `profile` and in maintenance) |
| `linkMobile` | `number` (with its country code, `+` or `00`; "" unlinks) | profile (`linkedMobilePending`: E.164 without `+` waiting for the SMS code; `linkedMobile` is empty meanwhile, as the server has dropped an old number); errors `phoneInvalid` |
| `normalizeMobile` | `number` | `{number}`: what `linkMobile` would send, E.164 without `+`, or `null` if it can't be linked. No network |
| `verifyMobile` | `code`, `fromLink?` (the SMS's link: an already linked number with nothing pending counts as done) | profile (`linkedMobile`); errors `noVerification`, server message for a wrong code |
| `callMobile` | – | profile; Threema calls and reads the code |
| `revocationStatus` | optional `cached` (no network: last known state) | `{set, setAt (ms or null), checked}` (`identity/check_revocation_key`) |
| `setRevocationPassword` | `password` (8–256 chars), `pin` | revocation status; only base64(SHA-256(password)[0..4]) goes to the directory |
| `setProfilePictureShare` | `share` (everyone\|nobody\|selected), optional `recipients` (the new allow list) | profile |
| `setProfilePictureRecipient` | `identity`, `share` (bool) | the contact JSON (`pictureRecipient`: bool while sharing with selected contacts, else null) |
| `setNickname` | `nickname` (≤ 32 bytes) | profile |

| `settings` | – | Safe `settings` object (`readReceipts`, `sendTyping`, `blockUnknown`, …) |
| `setSetting` | `name` (`readReceipts`\|`sendTyping`\|`blockUnknown`\|`videoCalls`\|`groupCalls`\|`showProfilePictures`\|…), `value` | settings |
| `deleteAllData` | `pin` | `{linkedDevicesRemain}` (true: this phone could not get through to the server in time, linked devices keep the ID); stops backups, calls, browsers and the connection, leaves the device group, marks the removal, then wipes the store in one transaction, media and logs (ADR 0018). A refused PIN is the only error, and nothing has happened then; a store that refuses to empty doesn't stop the removal. The UI goes through `backend.removeAllData(pin)`, which checks the PIN with `unlock` first, ends calls and players, and afterwards replaces the database and its key (Safe copy on the server stays) |

| `conversations` | optional `archived` (the archive instead), `private` (the private chats; error `locked` unless unlocked) | `{conversations: [{identity, displayName, verification, unread, lastMessage}]}` newest first |
| `unreadSummary` | | `{unreadTotal, unreadFrom, unreadChat}` as in `conversations` (the chat list's unread count, whom the cover names and that chat's key; archive and private chats don't count), without building a list; asked for at each event that can change them |
| `messages` | `identity`, optional `limit` (200), `around` (local id: a window of `limit` messages with this one in the middle; a message that is gone gives the newest ones), `before` / `after` (local id: only the page before or after it in history order, the chat's window; default 100) | `{identity, messages: [message…], private, firstUnread, reachesNewest}` oldest first (`firstUnread`: local id of the oldest unread incoming message, null when none; the initial page is widened to reach it, bounded at 1000; `reachesNewest`: false when newer messages lie beyond an `around` window; `before` / `after` answer without these two); error `locked` for a private chat while they are hidden |
| `sendText` | `identity`, `text` (≤ 6000 bytes), `quote` (message id hex, optional) | message (state pending; sent when connected) |
| `quotedMessage` | `identity`, `messageId` (hex) | message JSON or null (quoted messages not in the loaded list) |
| `markRead` | `identity` | `{read}`; queues read receipts if enabled (global setting or contact override) and, with linked devices, the read state for them, in the store until sent; also clears a hand-set unread mark |
| `setConversationUnread` | `identity`, `unread` | `{}`; Android's MARKED_AS_UNREAD: local only (not reflected), gone as soon as the chat is read or a message arrives; `conversations` carries `markedUnread` |
| `setTyping` | `identity`, `typing` (bool) | `{sent}`; typing indicator, only if enabled (global `sendTyping` or contact override) and still fresh (≤ 5 s) when the connection sends it |
| `react` | `identity`, `messageId` (hex), `emoji` (one emoji; anything else is `invalid emoji`), `apply` (bool) | changed message JSON; `Reaction` (0x82) if the contact has `REACTION_SUPPORT`, else legacy 👍/👎 receipt, else error `reactionNotSupported`; sent via the outbox |
| `editMessage` | `identity`, `messageId` (hex), `text` | changed message JSON; our text messages: unsent ones are just changed, sent ones need the contact's `EDIT_MESSAGE_SUPPORT` and < 6 h (`notSupportedByContact`, `messageTooOld`); `EditMessage` via the outbox |
| `deleteMessage` | `identity`, `messageId` (hex), `outgoing`, `everyone` | `{removed: true}` (this device, or never sent) or the placeholder message JSON (kind `deleted`); for everyone: ours, `DELETE_MESSAGE_SUPPORT`, < 6 h |
| `sendFile` | `identity`, `rendering` (`media`/`file`), `mediaType`, `name`, `caption`, `width`, `height`, `path` (if no attachment) + attachments (file, JPEG thumbnail) via `threesea_core_send_data` | the outgoing message JSON (kind `file`, `uploadState` 1); stored encrypted, uploaded, then sent. The reply comes when the file is stored: reading, encoding and encrypting it run on a blocking thread, one file at a time and in the order asked, and requests behind it are answered meanwhile, so this reply (and `saveFile`'s) can come after theirs. The bridge's `backend.sendFile(identity, path, asPhoto, caption, cb)` prepares photos (≤ 2048 px JPEG, EXIF and text dropped, 512 px thumbnail); `backend.holdPicture(message, crop, rotation, marks)` then `sendHeldPicture(held, identity, cb)` does the same from the picture of a message, edited in memory (ADR 0029). An in-app capture is removed once answered, unless `cb` returns its path on a refusal: then it stays for another try. A video sent as media loses its metadata here in the core (`core/src/mp4.rs`, protocol-notes.md), its `width` and `height` come from the file, and pictures and videos sent as media get a generic `name`. A GIF sent as a picture keeps only its frames, timing and loop count (`core/src/gif.rs`), and one that doesn't add up is refused with kind `pictureKeepsMetadata`. A video it can't clean is refused with kind `videoKeepsMetadata`, nothing stored, unless the request has `keepMetadata: true` (the user's answer, ADR 0025): then rendering `file`, unchanged. `backend.sendVideo(identity, path, frame, seconds, caption, cb, keepMetadata)`; without a frame, no thumbnail. |
| (voice) | `backend.sendVoice(identity, path, durationMs, cb)` → `sendFile` with `audio/aac`, rendering `media`, `duration` (s) | QML context objects `voiceRecorder` (`available`, `recording`, `duration`, `start/stop/cancel`, `finished(path, duration)`) and `voicePlayer` (`messageId`, `playing`, `position`, `duration`, `play(id)/pause/stop/seek`; `play` decrypts on a worker, then `messageId` changes, or `failed` says it can't be played; `videoPlayer` likewise, with `loading`) |
| `retryUpload` | `message` (local id) | `{}`; an upload that failed tries again, with its automatic tries starting over. By itself it tries again after growing pauses (1 min to 8 h, five times) while connected, unless the server refused it or the file can't be read. An upload cut short by the end of the process counts as such a try and shows as failed until its pause ends. |
| `downloadFile` | `message` (local id) | `{}`; fetch the file (or retry the thumbnail) |
| `saveFile` | `message` (local id), `directory` (the UI passes `StandardPaths.download`) | `{path, openable}`; decrypted copy outside the encrypted store, sanitised name, " (n)" if taken, an identical existing file is reused. `openable`: the saved name is a document, media type or contact card that may go to the system's app; anything else stays saved only |
| `deleteConversation` | `identity` | `{}`; also `{"event": "conversationRead", "identity"}`, so the UI takes the notification away, and `{"event": "chatDeleted", "identity"}`, so that a page open on the chat keeps no draft |
| `connectionState` | – | `{state: disconnected\|connecting\|connected}` |
| `reconnect` | – | `{}`; "Connect now": skip the backoff wait, also the one after a server said not to come back by ourselves; if connected, verify with an echo |
| `wake` | – | `{waiting}`; the same without the user asking (timer, network change, back in the app): leaves a wait a server asked for until that time has passed by the clock. `waiting`: `wakeSettled` follows |
| `findChats` | `query` (≥ 2 characters; words in any order, any case, accents or none) | `{chats: [{identity, displayName, group?, distributionList?, pictureVersion}], contacts: [{identity, displayName, pictureVersion}]}`, at most 30 each; chats newest first by name, a 1:1 chat or contact also by first and last name, nickname and ID. A contact whose chat is among `chats` isn't in `contacts`. A private chat is left out unless unlocked; its contact stays as in `contacts` |
| `searchMessages` | `query` (≥ 2 characters), optional `identity` (one chat) | `{results: [message JSON + snippet (the matching raw text/caption/name), displayName (chat), group, senderName]}`, newest first, at most 100; mentions match by name |
| `createDistributionList` | `name` (may be empty), `members` | the list JSON `{key, name, displayName, members: [{identity, displayName}], pinned}` |
| `distributionList` | `key` | the list JSON |
| `updateDistributionList` | `key`, optional `name`, `members` | the list JSON |
| `setDraft` | `identity`, `text` (empty: none; mentions as `@[ID]`), optional `quote` (message id hex) | `{}`; `conversations` entries carry `draft` and, with mentions, `draftMentions` (`{ID: name}`). `sendText` and a `sendFile`'s caption take a draft holding the same text away once the core has accepted them; the UI writes the draft first, so a refused send keeps it |
| `draft` | `identity` | `{text, mentions, quote}` (the message JSON, or null if it is gone); `{text: null}` when there is none |
| `lockStatus` | – | `{lock: {keyboard, appLock, graceSecs} \| null, unlocked, graceTimes}` (ADR 0009) |
| `setLock` | `enabled`; a new `pin` (≥ 4 chars) with `keyboard`, or none to keep it; `appLock`, `graceSecs`; if a lock exists `currentPin` | lock status; errors `wrongPin`, `pinTooShort`, `privateChatsExist` (removing) |
| `unlock` | `pin`; `privateChats: true` makes private chats readable (until `lock`), otherwise it only checks (app lock) | lock status; errors `lockNeeded` (no lock stored: no PIN opens anything, marks from a restore or a linked device wait for one), `wrongPin`, `pinLockout` (message: seconds until a PIN is checked again; after more than 3 wrong PINs in a row, 30 s up to an hour, like Android; also for `setLock`'s `currentPin`) |
| `lock` | optional `pendingFiles` (`{chat: count}`, sent by `backend.lockWithFilesOnTheirWay()`: the files of a chat being left that are still being prepared) | lock status (private chats hidden again, at once). `pendingFiles` gives a write-only pass for hidden private chats other than lists: `sendFile` takes exactly that many for each, answering `{}`; it expires after 60 s and goes with any successful `unlock` and `deleteAllData`; a `lock` without it leaves a pass alone. Everything else about those chats still answers `locked` (ADR 0009) |
| (PIN-guarded) | `pin` on `exportIdentityBackup`, `safeEnable`, `safeDisable`, `androidBackupExport`, `androidBackupSchedule`, `setRevocationPassword`, `startLinking`, `deleteAllData` | if a lock exists they need its PIN, whether or not private chats are open: errors `wrongPin`, `pinLockout`. The UI asks through `unlock` without `privateChats`, changing nothing |
| `setPrivate` | `identity`, `private` | `{identity, private}`; errors `lockNeeded`, `locked` (unmarking while locked). Private chats leave the chat list, archive, search, starred and media while locked; reading them returns `locked`; `messageAdded` carries `private` |
| `emptyConversation` | `identity` | `{}`; deletes the messages, the chat stays in `conversations` with a `lastMessage` of kind `empty` (Android's "Empty chat"; migration 20) |
| `setSyncExcluded` / `syncExcluded` | `identity`, `excluded` / – | `{excluded: [{identity, displayName}]}`; address book matching skips these IDs (Safe `syncExcludedIds`) |
| `refreshContacts` | – | `{changed}`; contact states and feature masks from the directory now (`identity/check`), revoked IDs included |
| `setContactPolicies` | `identity`, optional `readReceipts`, `typing` (0 like the app setting, 1 always, 2 never) | the contact JSON (`readReceipts`, `typing`) |
| `clearContactPolicies` | – | `{cleared}`; every contact back to the app settings (Android's "Clear individual settings") |
| `setNetwork` | `wifi` (bridge: Wi-Fi/Ethernet vs. mobile data) | `{}`; starts downloads the new network allows |
| `mediaSettings` / `setMediaSettings` | – / optional `autoDownloadWifi`, `autoDownloadMobile` (lists of `image`, `audio`, `video`, `file`), `autoSave`, `autoDeleteDays` | `{autoDownloadWifi, autoDownloadMobile, autoSave, autoDeleteDays, picturesAvailable}` (defaults like Android: all on Wi-Fi, photos on mobile data). `autoDeleteDays` (0: off) is Android's auto-delete: maintenance runs `cleanUp`'s rules for whole messages twice a day, leaving notes groups alone |
| `storageInfo` | – | `{mediaBytes, databaseBytes}` |
| `cleanUp` | `days` (0: all), `messages` | `{removed}`; media files only (thumbnails stay; `downloadFile` can fetch the file while the server still has it, which for a received 1:1 file is usually over; a file with no blob on the server stays) or whole messages with their status lines; starred messages, open polls and messages still on their way are kept; private chats only while unlocked; age is by `arrivedAt`. After whole messages went and a quarter or more of the database is free, it is rewritten to give the space back (the core waits for it; not when more than 128 MiB stay, since the copy lies in memory) |
| `displaySettings` / `setDisplaySettings` | – / optional `lastNameFirst`, `sortByLastName`, `showInactive` | `{lastNameFirst, sortByLastName, showInactive}`; `contacts` then sorts by name, or last name first, and leaves inactive and revoked IDs out if hidden |
| `multiDevice` | – | `{active, reflectedLeftOut}` (`active`: multi-device, connected through the mediator, ADR 0011; `reflectedLeftOut`: how many reflected messages from someone with no contact here found no room to wait for one, and weren't kept here) |
| `clearReflectedLeftOut` | – | as `multiDevice`, the count at 0 |
| `linkedDevices` | – | `{devices: [{id (hex), current, label, alias, platform, platformDetails, appVersion, connectedSince, lastDisconnectAt}], label, active}`, this device first; `[]` without multi-device. `label` is this device's own name, `alias` a name the user gave another device here. Error `network` without a connection. Only this device on the list: the connection checks in a transaction whether it is alone and ends multi-device if so (event `multiDeviceChanged`); the same runs about hourly |
| `setDeviceLabel` | `label` (≤ 64 bytes, empty: the default "Sailfish OS") | `{label}`; what the other devices show for this one. It travels in the D2M client-hello, so the connection starts again |
| `setDeviceAlias` | `device` (hex), `alias` (empty: its own label again) | `{}`; a name for our own lists — a device's label is encrypted with the device group key, so no device can rename another one |
| `dropDevices` | `ids` (hex) | `{devices, label, active}`: the remaining devices as the mediator confirmed them. When no other device remains (or with the own id) this one leaves too: `{devices: [], active: false}`, multi-device ends, back to the chat server, feature mask with forward security again. Error `network` without a connection |
| `deactivateMultiDevice` | `leaveLocally` (optional) | `{active: false}`: all other devices dropped, then this one; feature mask with forward security again. An error changes nothing: the devices are still linked, and multi-device stays on to try again. With `leaveLocally` this device only forgets the group, without the mediator: the others stay linked and it can no longer remove them |
| `gifFavourites` | – | `{favourites: [{id, width, height}]}`, newest first |
| `addGifFavourite` | `message` (a GIF message), or attached GIF + thumbnail / `path` with `width`, `height` (bridge `addGifFavourite(path)`) | `{id, favourites}`; the same GIF (SHA-256) is kept once and moves to the front; error `notGif` |
| `removeGifFavourite` / `sendGifFavourite` | `favourite` / `identity`, `favourite` | the list / the sent message like `sendFile` (field `favourite`, not `id`: the bridge's request id) |
| `updateSettings` / `setUpdateCheck` / `checkForUpdate` | – / `enabled` / – | `{available (this build has the check), enabled (missing: not asked yet), checkedAt, current, latest: {version, url} (only if newer)}`; `setUpdateCheck` errors in a store build. Checks run in the background (`checking: true` meanwhile; event `updateChecked {ok, manual}` when done: `manual` if the user asked for it, by `checkForUpdate` or turning the check on, also while the daily one was running, `updateAvailable {version, url}` once per new version); automatic ones at most one attempt a day (ADR 0014) |
| `setImageSize` | `side`: longest side in pixels, 0 for as large as they are | `{}`; Settings → Image size for pictures Threema Web sends (the bridge's FilePreparer reads the setting itself for the chat's); sent at start and on change, 1600 until then |
| `setLogLevel` | `level`: `warn` (default), `info`, `debug` | `{}`; applies at once (also start config `logLevel`; `THREESEA_LOG`, an env filter, overrides both for development) |
| `logText` | — | `{text}`: the in-app log (what also goes to the journal; fixed texts, kinds, codes and counts only), bounded tail of the rotated files |
| `logPackage` | `directory` | `{path}`: writes `threesea-log-<stamp>.txt` there — device facts (versions, model, flags) and the log; sharing it stays the user's step |
| `logClear` | — | `{}`; removes the in-app log files |
| `forwardSecurity` | — | `{on, multiDevice}`; `on` is false while multi-device is active (SE-199) or the user switched it off |
| `setForwardSecurity` | `on` | `{on}`; off terminates every session (DisabledByLocal) and the feature mask drops the FS bit, like multi-device activation; messages stay end-to-end encrypted either way (user decision 2026-10-28) |
| `networkSettings` / `setNetworkSettings` | – / `ipv6` | `{ipv6}`; off: Threema's IPv4-only hosts (`apip`, `blobp-…`, `g-…` instead of the dual-stack `ds-…`); the chat server connection starts again |
| `setArchived` | `identity`, `archived` | `{}`; archiving unpins; an incoming message or one we write unarchives; `conversations` has `archivedCount` |
| `chatMedia` | `identity`, optional `kinds` (default: the saved filter), `limit` (default 60; 0: only the counts), `before` (the `next` of the page before) | `{messages (each with mediaKind: image\|gif\|video\|voice\|audio\|file), kinds, next (where the next page starts, null at the end), counts: {kind: n} (without `before`)}`, newest first (media gallery) |
| `setMediaKinds` | `kinds` | `{kinds}`; the gallery filter, saved for all chats (absent: all, like Android) |
| `message` | `message` (local id) | the message JSON or null (message details) |
| `setVoiceListened` | `message` (local id) | changed message JSON or null, also as a `messagesChanged` event for the other pages; sets `body.listened` on a voice (file) message, so the chat knows which voice messages are still new and where ear playback starts |
| `setStarred` | `message` (local id), `starred` | the message JSON (field `starred`) |
| `starredMessages` | optional `identity` | `{results: [message + displayName, group, senderName, snippet]}`, newest first |
| `setPinned` | `identity`, `pinned` | `{}`; pinned chats come first in `conversations` (field `pinned`: their place, else null) |
| `movePinned` | `identity`, `offset` (−1 up, 1 down) | `{}` |
| `sendLocation` | `identity`, `latitude`, `longitude`, optional `accuracy` (m), `name`, `address` | the outgoing message JSON (kind `location`, body `{latitude, longitude, accuracy, name, address}`) |
| `findPlaces` | `latitude`, `longitude` (where to search around), optional `query` (by name, 3 characters at least; else nearby) | `{places: [{name, latitude, longitude, distance (m)}]}`, at most 30; asks `poi.threema.ch`, which gets the position and the query; `places` is empty for a shorter query or a position of 0/0, with nothing sent |
| `setBlocked` | `identity`, `blocked` | contact JSON (or `{}` for a non-contact); the block list is `blockedContacts` in the app settings (in Threema Safe too); libthreema discards their messages, sending to them fails with `contactBlocked` (to a revoked ID: `contactRevoked`). Contact JSON has `blocked` and `invalid` (revoked) |
| `blockedContacts` | – | `{blocked: [{identity, displayName}]}` |
| `forwardMessage` | `message` (local id), `identity` (target chat) | the new message JSON; texts without their quote, files re-uploaded from the encrypted store with a new key; a picture sent as media is cleaned first like one from Threema Web, at its own size (ADR 0025), and refused if it can't be; a thumbnail is made anew or left out (errors `fileNotDownloaded`, `notForwardable`, `pictureKeepsMetadata`, `videoKeepsMetadata`) |
| `forwardMessages` | `messages` (local ids, in order), `identity` | `{forwarded, failed, errors}`, `errors` the kinds of error that came up, each once; the UI sends one id at a time, so that other commands get their turn between them |
| `setChatNotifications` | `identity` (ID or group key), `policy` (`never`\|`mentioned`\|null), `until` (ms or null) | `{policy, until}`; also in contact/group JSON as `notifications`, conversations carry `muted`, `messageAdded` events `notify` (the setting allows a notification) |
| `chatTones` | – | `{tones: {key: path}}`: the chats with a tone of their own, without hidden private chats (the bridge caches these for `alert`) |
| `setChatTone` | `identity` (ID or group key), `tone` (path, empty or null: the tone of all chats) | `{}`; `locked` for a hidden private chat |
| `importChatTones` | `tones` `{key: path}` | `{}`; the tones the app kept in its settings file until 0.2.0, also those of hidden private chats |
| `exportChat` | `identity`, `media`, `directory`, `password?`, `me?`, `offsetMinutes?` | `{path, name, messages, files}`; a non-empty `password` under 8 characters: `passwordTooShort` |
| `parseLink` | `url` | `{action: contact\|compose\|add\|license\|verifyMobile, identity, text, key, code}` or error `linkInvalid` (protocol-notes "Threema links") |
| `myQrCode` | – | `{text, size, modules}`: `3mid:<ID>,<public key hex>` and its QR modules (`size`² chars `0`/`1`) |
| `verifyQrCode` | `text` | contact JSON; fully verified if the key matches, a new contact added only then (errors `qrInvalid`, `qrExpired`, `qrMismatch`). Camera frames / pictures are decoded first with the C function `threesea_qr_decode(grey, width, height)` |
| `callTurnServers` | `identity` (who the call is with) | `{urls, username, password, relayOnly}` for the media engine (`identity/turn_cred`, cached until expiry; `relayOnly`: the peer is unverified or not a contact, or the setting `relayThreemaCalls` is on; then the dual-stack servers, like Android) |
| `callStart` | `identity`, `sdp` (engine's offer), `video` | `{callId}`; the SDP is patched (`sdp.rs`) and sent as call-offer. Only with `calls: true` in the start config (Chum, ADR 0012). Refused with `contactRevoked`, `contactBlocked` or `contactUnknown` (block unknown senders) for someone the signalling wouldn't reach |
| `callAccept` | `callId`, `sdp` (engine's answer), `video` | `{}` |
| `callReject` / `callHangUp` | `callId` | `{}`; the call ends with a status message `call\|result\|outgoing (0/1)\|seconds\|callId` (imported lines have no call id) |
| `callFailed` | `callId`, `reason` (`certificate`, `connection`, `other`) | `{}`; the media side gave up: the peer is told (hangup, or reject "unknown" before the answer went out), the status message says `failed` |
| `callTimeout` | `callId` | `{}`; nobody answered within the ring time (60 s): incoming → rejected "timeout" (missed), outgoing → hung up (unanswered) |
| `callConnected` | `relayed` | `{}`; media is up: our video quality profile goes to the peer (`callChannelSend`), `callVideoParams` follows |
| `callChannel` | `data` (base64, from the call's data channel) | `{}`; events `callRemoteCapture` or `callVideoParams` |
| `callCapture` | `device` (`camera`\|`microphone`), `on` | `{}`; tells the peer (`callChannelSend`) |
| `setVideoQuality` | `quality` (`auto`\|`low`\|`max`) | the settings JSON, which also has `videoCalls` (switched with `setSetting`; synced like Android's video policy, not in Threema Safe) and `videoQuality` |
| `callCandidates` | `callId`, `candidates`: `[{candidate, sdpMid, sdpMLineIndex, ufrag}]` | `{}` |
| `callState` | – | `{call: {identity, callId, phase (calling\|ringing\|incoming\|active), outgoing, video, startedAt, acceptedAt} \| null}` |
| `matchAddressBook` | `entries`: `[{firstName, lastName, emails, phones}]`, `automatic?` (the hourly sync, which rests with the directory's rate limit: error `rateLimited`) | `{matched, added}`; hashed lookup (protocol-notes "Address book matching"). A directory error while looking up new people comes as the command's error after the others were taken |
| `addressBookCountry` | – | `{chosen (a region, "" = automatic), automaticCode (the calling code automatic stands for: the linked number's, else the locale's; "" if neither is listed), countries: [{region, code}]}` |
| `setAddressBookCountry` | `country` (a region of the list, "" = automatic) | as `addressBookCountry`; local to this device |

File message `body`: `{rendering, mediaType, name, fileType, size, caption, width, height, duration, animated, thumbnail, fileState, thumbnailState, uploadState, fileOnDemand}`, states 0 remote, 1 loading, 2 on the device, 3 failed; `fileOnDemand`: cleaned up by the user, the file downloads only through `downloadFile`. Part downloads that the network broke off try again by themselves as uploads do (`retryUpload`), a file only as far as it would come by itself on this network. The core's own notes, for no one else to read: `uploadRetry`, `fileRetry`, `thumbnailRetry` (`{tries, at}`), `uploadedFile` (`{at, mirror, scope, persist}`). `name` comes without characters that reorder text; `fileType` (image\|audio\|video\|pdf\|document\|contact\|archive\|other) follows from the name it would be saved under, not from `mediaType`, and is what the icon shows. Media: `image://threesea/<id>/thumb|file` (bridge), `threesea_media_read` (FFI).

Message JSON: `{id (local), conversation, messageId (hex), outgoing, kind (text\|location), text,
body, createdAt (ms; of an incoming message the sender's clock), arrivedAt (ms; when it reached this device, what the chat list and retention go by; createdAt where that isn't known), updatedAt, state, quoteId (hex or null), editedAt (ms or null; deletion time for kind `deleted`), sentAt, deliveredAt, readAt (outgoing; readAt also when we read an incoming one), receivedAt (incoming) — ms or null when unknown (older messages, skipped states), sortKey (the place in the chat's history, with `id` as tie break: higher is newer; a row's id alone doesn't say, imported rows are placed between), writtenOut (outgoing: its send has begun, so someone may have it even while it is pending; changing it takes an edit message), reactions: [{sender ("" = us), emoji}], mentions + mentionsMe (only if the text/caption has `@[…]`: `{ID: name}`, "" = us; see protocol-notes "Mentions")}`; outgoing state 0 pending, 1 sent, 2 delivered, 3 read,
4 failed; incoming 0 unread, 1 read.

Contact JSON: `{identity, displayName, firstName, lastName, nickname, verification (0–2), work,
workVerified, inactive, invalid, hidden (out of the contact list: known from a group, or deleted), neededByGroup (hidden, and a member of a group we are in: nothing to delete), canEdit, canDelete (contact supports edits / deletions for everyone), reactions (contact has REACTION_SUPPORT: any emoji;
else only 👍/👎, which can't be withdrawn), canCall (O2O_AUDIO_CALL_SUPPORT), publicKey (hex)}`.

Error kinds (mapped to translated text in `qml/components/Errors.js`): `noLicense`,
`noIdentity`, `licenseFormat`, `server` (message from Threema, shown as-is), `rateLimited` (the directory's 429),
`serverUnexplained` (it said no without a reason), `reportRefused`, `identityExists`, `serverTrouble` (a
Threema server's unexpected status or unreadable answer; its text stays out of the UI), `network`,
`backupDecrypt`, `backupVersion`, `backupNoIdentity`, `passwordTooShort`, `passwordTooLong`, `safeNotFound`,
`safeTooLarge`, `safeDamaged`,
`identityFormat`, `idSearchInput`, `contactInvalid`, `contactIsSelf`, `contactNotFound`, `messageTooLong`,
`storeKey`, `storeDamaged`, `storeTooNew`, `diskFull`, `upgradeNoRoom`, `storeUnreadable` (the store failed, see
`fatal`; the message is only a code for support), `internal`.

QML usage: `backend.call("checkLicense", {licenseKey: k}, function(ok, result, kind, message) {…})`. A `null` result
stays `null` in the callback; a reply without one (an error) gives an empty object.
`backend.state` / `backend.identity` follow `status` replies and drive the root page.

### Groups, polls, pictures, linking, group calls, Threema Web, emoji

| Command | Arguments | Result / notes |
|---|---|---|
| `groups` / `group` | – / `key` (group conversation key `g:<creator>:<hex>`) | group JSON list / one group (title, members with names and `pictureVersion`, creator, left, isNotes; the single group also `private`, and `canDelete`: deleting for everyone means something, which in a notes group takes linked devices) |
| `createGroup` | `name`, `members` (IDs) | the new group; we are its creator |
| `renameGroup` / `setGroupMembers` / `leaveGroup` | `key` and `name` / `members` / – | the changed group; only the creator renames or changes members; the creator leaving disbands it |
| `setGroupPicture` / `removeGroupPicture` | `key`; attachment 0: JPEG | the changed group (creator only) |
| `setProfilePicture` / `removeProfilePicture` | attachment 0: JPEG / – | the profile |
| `createPoll` | `identity`, `description`, `choices`, `multiple`, `announce` | the poll message |
| `vote` / `closePoll` | `message` (local id) and `choices` (choice ids) / `message` | the changed poll message; closing also adds the `pollClosed` message. Error `locked` in a hidden private chat; `pollResultsTooLong` when the results don't fit a message (the poll stays open) |
| `polls` | `identity` | `{polls: [message], unanswered, question}`: the chat's polls, the open ones first, then the closed, each newest first; how many open ones have no vote of ours and the question of the newest of those (`null` if none). Error `locked` in a hidden private chat |
| `qrKind` | `text` | `{kind: "contact" \| "web" \| "device" \| "link" \| "idBackup" \| "unknown"}` for the scanner, no network; `error`: the error kind to show instead when the code can't be used (`qrInvalid`, `qrExpired`, `webVersion`, `webDisabled`, `webAlreadyPaired`, `linkVersion`); `identity` for contact codes and links; for `web` `host` and `confirmHost` (the server must be allowed: `webPair` with `allowHost`) |
| `startLinking` | `code` (Threema for Desktop's QR code), `pin` | `{choices: [[emoji ×3] ×3], correct}` for the emoji check (ADR 0011) |
| `confirmLinking` / `cancelLinking` | – | the linked devices once the new device registered (`devices` may be empty if the connection dropped right after) / `{}`. Errors of `confirmLinking`: `linkFailed` before the ID left this device (a device group made just for this is left again); `linkRemoved` after it, with the group removed again through the mediator; `linkUncertain` after it, when the new device may be linked: multi-device stays on and Linked devices shows the truth. Can take up to about 11 minutes in the worst case; the core answers nothing else meanwhile |
| `groupCallRefresh` | `conversation` | `{}`; the state comes as the `groupCall` event (ADR 0012) |
| `groupCallJoin` | `conversation`, `fingerprint` (hex SHA-256 of our DTLS certificate), `create` | `{offer, candidates, participantId}` for the media engine |
| `groupCallChannel` | `data` (base64, from the SFU's data channel) | `{}`; answers go out as `groupCallSend` events |
| `groupCallMute` / `groupCallCamera` | `muted` / `on` | `{}` (tells the others; the engine switches the device) |
| `groupCallTick` / `groupCallLeave` | – | every second while in a call (rekeying, state) / leave |
| `webSetEnabled` | `enabled` | like `webSessions`: the Threema Web switch; off refuses `webPair`, `webStart` and wake-ups (`webDisabled`) and disconnects running sessions (telling the browser `disable`), keeping the pairings the browser stored |
| `webSessions` | – | `{enabled, sessions: [{id, host, selfHosted, persistent, client, label, createdAt, lastConnection, state}], servers: [host], certificates: [{id, host}]}` (ADR 0013); `selfHosted` goes by the host; `servers`: the self-hosted servers the user allowed; `certificates`: their new certificates waiting for the user's word |
| `webPair` | `qr`, `allowHost` | the new session; a host outside threema.ch first answers `{confirmHost}` and needs `allowHost`; `webDisabled` while switched off, `webAlreadyPaired` while this browser's session runs or is stored |
| `webStart` / `webStop` / `webDelete` | `session` | `{}`: connect a stored session / disconnect / forget it (also ends its task's authority) |
| `webAcceptCertificate` | `session` | `{}`: pin a self-hosted server's new certificate and connect again; `{scanAgain: true}`: pinned, but the session was never stored by the browser, so its code must be scanned again |
| `webDeclineCertificate` | `session` | `{}`: not this certificate; the pin stays, and the session asks again when it next connects |
| `webDeleteAll` | – | like `webSessions`: stop and forget every session, telling running browsers `delete`; the allowed servers stay |
| `webRename` | `session`, `label` (empty: none) | `{}`: a name for this phone's list; `webSessionNotFound` if the session is gone |
| `webRemoveServer` | `host` | like `webSessions`: take back an allowed server's permission and pin, and remove its sessions |
| `recentEmoji` / `useEmoji` | – / `emoji` | `{emoji: [text]}` newest first / record one as used |

## D-Bus (notification actions)

Service `org.threesea.harbour-threesea`, object `/`, interface `org.threesea.harbourthreesea`:
`openChat(identity)` (tap), `reply(identity, token, text)` (input action "Reply": lipstick appends
the typed text), `markRead(identity, token)`, `activate()`, `activate(urls)`, `showUpdate()`,
`background()`. `identity` is a Threema ID or a group key. `token`: a random value per chat, put
into the notification's actions and kept in the app's private settings; calls without the
current token are ignored, since any process on the session bus could call the service and
lipstick calls through a helper (`mliteremoteaction`) the sandbox can't identify (audits S07/D04).
For a non-blank reply the window then says that it wasn't sent, without its chat or text.
Lipstick raises the app window for non-input actions (Mark as read); quick reply stays in Events.
The window runs Reply and Mark as read only once it knows the app lock is open: otherwise they wait
in memory for the PIN or passphrase (ADR 0009).

## Database key encryption (C functions, ADR 0009)

`threesea_key_wrap(key, 32, passphrase, keyboard, &len)` → blob, `threesea_key_unwrap(blob, len,
passphrase, &len)` → key or NULL, `threesea_key_wrapped(blob, len)` → -1 / 0 (digits) / 1
(keyboard). scrypt: call off the GUI thread. The bridge keeps the blob in Sailfish Secrets
instead of the key; `backend.state` is `locked` until `backend.unlockDatabase(passphrase, cb)`.
`backend.dataEncrypted`, `backend.setKeyEncryption(on, passphrase, keyboard, cb)` and
`backend.checkPassphrase(passphrase, cb)` drive Settings → Security; independent of the app lock.
