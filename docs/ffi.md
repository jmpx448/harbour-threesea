# Core ⇄ bridge protocol

The Rust core runs on its own thread (`core/src/runtime.rs`); the Qt bridge talks to it through
a small C API (`core/include/threesea_core.h`) and JSON messages. JSON keeps the boundary
small and debuggable; secrets inside messages are wiped on both sides after use.

## C API

| Function | Notes |
|---|---|
| `threesea_core_start(config_json, db_key, 32, callback, user_data)` | Starts the core thread. `db_key` comes from `SecretStore` (Sailfish Secrets) and is copied. |
| `threesea_core_send(core, request_json)` | Queues a request, never blocks. |
| `threesea_core_stop(core)` | Stops and joins the thread; no callbacks after it returns. |
| `threesea_core_version()` / `threesea_string_free()` | Version string. |
| `threesea_core_send_data(core, request_json, data, len, thumbnail, len)` | A request with binary attachments (`sendFile`, pictures); never blocks. |
| `threesea_media_read(core, message, thumbnail, &len)` / `threesea_picture_read(core, name, &len)` | Decrypted media for the UI's image providers (other threads). NULL while the message's private chat is hidden (ADR 0009). |
| `threesea_voice_decode(core, message, &len)` | Opus voice message → 16-bit PCM (no platform decoder); NULL as above. |
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

The arguments sit in the same object as `id` and `cmd`, so **no argument may be called `id` or
`cmd`**: the bridge would overwrite it with the request's own number and the core would refuse
the request as a whole ("invalid request"). Name it after what it is — `device`, `message`,
`favourite`. The bridge writes a warning when an argument of that name is passed.

Events (unsolicited): `{"event": "started", "version"}`, `{"event": "fatal", "kind", "message"}`,
`{"event": "safeChanged"}` (automatic Safe backup ran or its error state changed),
`{"event": "contactsChanged"}` (contacts imported in the background; the bridge reloads the model),
`{"event": "groupsChanged"}`, `{"event": "profileChanged"}` (the own profile/picture changed,
also when synced from another device),
`{"event": "connectionState", "state"}`, `{"event": "messageAdded", "message"}`,
`{"event": "messagesChanged", "messages"}` (acks, receipts), `{"event": "typing", "identity",
"typing"}`, `{"event": "serverMessage", "message", "canReconnect"}` (server alert / close). `{"event": "messageNotify", "message", "notify", "private"}`: a message another device received first (it leads) and that is still unread after a grace time, or at the end of the mediator's backlog — its `messageAdded` came with `notify: false`. `{"event": "privateChanged", "identity", "private"}`: a chat was marked private or unmarked, here, on a linked device or by a restore. `{"event": "deviceCookieChanged"}`: the chat server saw the ID log in with another device cookie (Android's rogue-device warning; the core has already cleared the server's flag).

Call events (Full, ADR 0012): `callIncoming {identity, callId, sdp (patched), video}`,
`callRinging`, `callAnswer {sdp, video}` (`video`: both sides take video; else it is an audio
call now), `callCandidates {candidates}`, `callEnded {record: {peer, callId, outgoing, result,
duration, at}}` (`duration` counts from `callConnected`), `callChannelSend {data}` (base64, for the call's data channel),
`callRemoteCapture {device: camera|microphone, on}`, `callVideoParams {bitrateKbps, fps, width,
height}` (what our video sender may use: Android's quality profiles, the lower of both sides,
MAX only without a relay). Contact JSON has `canCall` and `canVideoCall`. The commands about a
running call name it with `callId`, and do nothing if another call is current by then: `callEnded`
of the earlier call can still be on its way. Before `callStart`'s reply the bridge has no id, so it
keeps its candidates and a hangup until then.

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
| `createIdentity` | – | `{identity}` (uses the stored license); error `identityExists` on a device with an ID |
| `restoreIdentityBackup` | `backup`, `password` | `{identity}` (needs a license first); error `identityExists` on a device with an ID |
| `exportIdentityBackup` | `password` (≥ 8 chars), `pin` | `{backup}` |
| `safeCheckPassword` | `password` | `{result: tooShort\|tooLong\|weak\|ok}` |
| `safeStatus` | – | `{enabled, server, lastBackup, lastError, size, maxBytes, retentionDays, restore}`; `restore` = `{pendingContacts, pendingGroups, pendingLists, importError, summary}` — what a restore still has to import (retried on reconnect, hourly and via `safeRetryImport`), why the last attempt failed, and the import's counts (`summary`: `{contacts, revokedContacts, groups, lists, skippedGroups: {deleted, unknownCreator, invalid}, at}`) |
| `safeEnable` | `password`, `pin` | status; derives the key, uploads a first backup |
| `safeBackupNow` | – | status |
| `safeDisable` | `pin` | status; forgets the key (server copy expires) |
| `safeRestore` | `identity`, `password` | `{identity, contacts, groups}` (needs a license first); error `identityExists` on a device with an ID |
| `safeRetryImport` | – | status; tries the rest of a restored backup's import now |
| `androidBackupInspect` | `path`, `password` | `{version, identity (match\|otherIdentity\|keyMismatch\|unreadable\|missing), inventory: {contactChats, groupChats, listChats, mediaEntries, mediaBytes (declared, an estimate), thumbnails, contactAvatars, contactProfilePictures, groupAvatars, ownAvatar, has…, unknownEntries}}` — looks into an Android data backup ZIP, entirely locally; errors `backupDecrypt` (password), `backupVersion`, `backupInvalid` |
| `androidBackupImport` | `path`, `password`, `scope` (`metadata`\|`history`) | `metadata`: `{contacts: {added, existing, updated, keyConflicts}, groups: {added, existing, skippedDeleted, skippedUnknownCreator}, lists: {added, existing}, pictures: {own, contacts, groups, kept, localAvatarsNotImported, skipped}, nonces: {csp, d2d, existing, skipped, missing}, invalidRows}` — merges the archive's contacts, groups, lists and pictures into the running identity, offline, without sending anything; existing local data wins; idempotent. Only a backup of the same ID and key (`backupIdentity` otherwise). `history`: starts the background job (messages, media, reactions; one at a time) and replies `{started}`; progress and outcome come as events: `{"event": "androidBackupProgress", step (messages\|media), done, total}` and `{"event": "androidBackupDone", ok, summary \| kind}` with `summary` = `{chats, skippedChats, imported, existing, collisions, unsent, invalidRows, unsupported: {category: n}, compat, mediaFiles, thumbnails, mediaMissing, mediaFailed, reactions, reactionsUnmatched, polls, uidConflicts, foreignGroupChats, cancelled}`; it ends with `chatsChanged`/`contactsChanged` so the lists reload. Old messages are placed into history by time (sort_key), never resent; outgoing rows that never left Android stay failed; existing rows (live edits, tombstones) always win; repeated runs resume/add nothing. |
| `androidBackupCancel` | – | `{}`; stops a running history import between batches (done work stays; running the import again continues) |
| `androidBackupExport` | `directory?`, `password?` (omitted: the stored schedule settings), `pin` | `{started}`; writes an Android-format backup (format 27: settings, identity, contacts/groups/lists, messages, local media+thumbnails, pictures, reaction files; polls/status lines are counted as skipped, nonces are not written) as a background job; `{"event": "androidExportProgress", done, total}` and `{"event": "androidExportDone", ok, path, summary \| kind}`. The file is `threema-backup-<date>-<time>.zip`, written as `.part` first; afterwards the rotation keeps the newest `keep` of our files in the folder |
| `androidBackupSchedule` | `directory?`, `password?` (min 8 chars; kept in the encrypted store for unattended runs), `interval?` (`off`\|`daily`\|`weekly`), `keep?`, `pin` | status; maintenance starts a backup when one is due |
| `androidBackupExportStatus` | – | `{directory, interval, keep, lastBackup, lastError, passwordSet, running}` |
| `contacts` | – | `{contacts: [contact…]}` (direct contacts, sorted by name); also fills `backend.contacts` |
| `contact` | `identity` | contact |
| `addContact` | `identity` | contact (directory lookup, unverified) |
| `updateContact` | `identity`, `firstName`, `lastName` | contact |
| `deleteContact` | `identity` | `{}` |
| `profile` | – | `{identity, nickname, publicKey, linkedEmail, linkedMobile, pictureVersion, pictureShare (everyone\|nobody\|selected), pictureRecipients: [ID]}` |
| `linkEmail` | `email` ("" unlinks) | profile (`linkedEmail`, `linkedEmailPending` until the link in the e-mail is opened; checked again on `profile` and in maintenance) |
| `linkMobile` | `number` (with its country code, `+` or `00`; "" unlinks) | profile (`linkedMobilePending`: E.164 without `+` waiting for the SMS code); errors `phoneInvalid` |
| `normalizeMobile` | `number` | `{number}`: what `linkMobile` would send, E.164 without `+`, or `null` if it can't be linked. No network |
| `verifyMobile` | `code` | profile (`linkedMobile`); errors `noVerification`, server message for a wrong code |
| `callMobile` | – | profile; Threema calls and reads the code |
| `revocationStatus` | optional `cached` (no network: last known state) | `{set, setAt (ms or null), checked}` (`identity/check_revocation_key`) |
| `setRevocationPassword` | `password` (8–256 chars), `pin` | revocation status; only base64(SHA-256(password)[0..4]) goes to the directory |
| `setProfilePictureShare` | `share` (everyone\|nobody\|selected), optional `recipients` (the new allow list) | profile |
| `setProfilePictureRecipient` | `identity`, `share` (bool) | the contact JSON (`pictureRecipient`: bool while sharing with selected contacts, else null) |
| `setNickname` | `nickname` (≤ 32 bytes) | profile |

| `settings` | – | Safe `settings` object (`readReceipts`, `sendTyping`, `blockUnknown`, …) |
| `setSetting` | `name` (`readReceipts`\|`sendTyping`\|`blockUnknown`\|`videoCalls`\|`groupCalls`\|`receiveProfilePictures`\|…), `value` | settings |
| `deleteAllData` | `pin` | `{}`; wipes identity, contacts, settings (Safe copy on the server stays) |

| `conversations` | optional `archived` (the archive instead), `private` (the private chats; error `locked` unless unlocked) | `{conversations: [{identity, displayName, verification, unread, lastMessage}]}` newest first |
| `messages` | `identity`, optional `limit` (200), `since` (local id: load back to this message), `before` / `after` (local id: only the page before or after it in history order, the chat's window; default 100) | `{identity, messages: [message…], private, firstUnread}` oldest first (`firstUnread`: local id of the oldest unread incoming message, null when none; the initial page is widened to reach it, bounded at 1000); error `locked` for a private chat while they are hidden |
| `sendText` | `identity`, `text` (≤ 6000 bytes), `quote` (message id hex, optional) | message (state pending; sent when connected) |
| `quotedMessage` | `identity`, `messageId` (hex) | message JSON or null (quoted messages not in the loaded list) |
| `markRead` | `identity` | `{read}`; sends read receipts if enabled (global setting or contact override); also clears a hand-set unread mark |
| `setConversationUnread` | `identity`, `unread` | `{}`; Android's MARKED_AS_UNREAD: local only (not reflected), gone as soon as the chat is read or a message arrives; `conversations` carries `markedUnread` |
| `setTyping` | `identity`, `typing` (bool) | `{sent}`; typing indicator, only if enabled (global `sendTyping` or contact override) and still fresh (≤ 5 s) when the connection sends it |
| `react` | `identity`, `messageId` (hex), `emoji`, `apply` (bool) | changed message JSON; `Reaction` (0x82) if the contact has `REACTION_SUPPORT`, else legacy 👍/👎 receipt, else error `reactionNotSupported`; sent via the outbox |
| `editMessage` | `identity`, `messageId` (hex), `text` | changed message JSON; our text messages: unsent ones are just changed, sent ones need the contact's `EDIT_MESSAGE_SUPPORT` and < 6 h (`notSupportedByContact`, `messageTooOld`); `EditMessage` via the outbox |
| `deleteMessage` | `identity`, `messageId` (hex), `outgoing`, `everyone` | `{removed: true}` (this device, or never sent) or the placeholder message JSON (kind `deleted`); for everyone: ours, `DELETE_MESSAGE_SUPPORT`, < 6 h |
| `sendFile` | `identity`, `rendering` (`media`/`file`), `mediaType`, `name`, `caption`, `width`, `height`, `path` (if no attachment) + attachments (file, JPEG thumbnail) via `threesea_core_send_data` | the outgoing message JSON (kind `file`, `uploadState` 1); stored encrypted, uploaded, then sent. The bridge's `backend.sendFile(identity, path, asPhoto, caption, cb)` prepares photos (≤ 2048 px JPEG, EXIF dropped, 512 px thumbnail) |
| (voice) | `backend.sendVoice(identity, path, durationMs, cb)` → `sendFile` with `audio/aac`, rendering `media`, `duration` (s) | QML context objects `voiceRecorder` (`available`, `recording`, `duration`, `start/stop/cancel`, `finished(path, duration)`) and `voicePlayer` (`messageId`, `playing`, `position`, `duration`, `play(id)/pause/stop/seek`) |
| `retryUpload` | `message` (local id) | `{}` |
| `downloadFile` | `message` (local id) | `{}`; fetch the file (or retry the thumbnail) |
| `saveFile` | `message` (local id), `directory` (the UI passes `StandardPaths.download`) | `{path}`; decrypted copy outside the encrypted store, sanitised name, " (n)" if taken, an identical existing file is reused |
| `deleteConversation` | `identity` | `{}`; also `{"event": "conversationRead", "identity"}`, so the UI takes the notification away |
| `connectionState` | – | `{state: disconnected\|connecting\|connected}` |
| `reconnect` | – | `{}`; skip the backoff wait |
| `searchMessages` | `query` (≥ 2 characters), optional `identity` (one chat) | `{results: [message JSON + snippet (the matching raw text/caption/name), displayName (chat), group, senderName]}`, newest first, at most 100; mentions match by name |
| `createDistributionList` | `name` (may be empty), `members` | the list JSON `{key, name, displayName, members: [{identity, displayName}], pinned}` |
| `distributionList` | `key` | the list JSON |
| `updateDistributionList` | `key`, optional `name`, `members` | the list JSON |
| `setDraft` | `identity`, `text` (empty: none) | `{}`; `conversations` entries carry `draft` |
| `draft` | `identity` | `{text}` |
| `lockStatus` | – | `{lock: {keyboard, appLock, graceSecs} \| null, unlocked, graceTimes}` (ADR 0009) |
| `setLock` | `enabled`; a new `pin` (≥ 4 chars) with `keyboard`, or none to keep it; `appLock`, `graceSecs`; if a lock exists `currentPin` | lock status; errors `wrongPin`, `pinTooShort`, `privateChatsExist` (removing) |
| `unlock` | `pin`; `privateChats: true` makes private chats readable (until `lock`), otherwise it only checks (app lock) | lock status; errors `lockNeeded` (no lock stored: no PIN opens anything, marks from a restore or a linked device wait for one), `wrongPin`, `pinLockout` (message: seconds until a PIN is checked again; after more than 3 wrong PINs in a row, 30 s up to an hour, like Android; also for `setLock`'s `currentPin`) |
| `lock` | – | lock status (private chats hidden again) |
| (PIN-guarded) | `pin` on `exportIdentityBackup`, `safeEnable`, `safeDisable`, `androidBackupExport`, `androidBackupSchedule`, `setRevocationPassword`, `startLinking`, `deleteAllData` | if a lock exists they need its PIN, whether or not private chats are open: errors `wrongPin`, `pinLockout`. The UI asks through `unlock` without `privateChats`, changing nothing |
| `setPrivate` | `identity`, `private` | `{identity, private}`; errors `lockNeeded`, `locked` (unmarking while locked). Private chats leave the chat list, archive, search, starred and media while locked; reading them returns `locked`; `messageAdded` carries `private` |
| `emptyConversation` | `identity` | `{}`; deletes the messages, the chat stays in `conversations` with a `lastMessage` of kind `empty` (Android's "Empty chat"; migration 20) |
| `setSyncExcluded` / `syncExcluded` | `identity`, `excluded` / – | `{excluded: [{identity, displayName}]}`; address book matching skips these IDs (Safe `syncExcludedIds`) |
| `refreshContacts` | – | `{changed}`; contact states, keys and feature masks from the directory now |
| `setContactPolicies` | `identity`, optional `readReceipts`, `typing` (0 like the app setting, 1 always, 2 never) | the contact JSON (`readReceipts`, `typing`) |
| `clearContactPolicies` | – | `{cleared}`; every contact back to the app settings (Android's "Clear individual settings") |
| `setNetwork` | `wifi` (bridge: Wi-Fi/Ethernet vs. mobile data) | `{}`; starts downloads the new network allows |
| `mediaSettings` / `setMediaSettings` | – / optional `autoDownloadWifi`, `autoDownloadMobile` (lists of `image`, `audio`, `video`, `file`), `autoSave`, `autoDeleteDays` | `{autoDownloadWifi, autoDownloadMobile, autoSave, autoDeleteDays, picturesAvailable}` (defaults like Android: all on Wi-Fi, photos on mobile data). `autoDeleteDays` (0: off) is Android's auto-delete: maintenance runs `cleanUp`'s rules twice a day |
| `storageInfo` | – | `{mediaBytes, databaseBytes}` |
| `cleanUp` | `days` (0: all), `messages` | `{removed}`; media files only (thumbnails stay, re-downloadable) or whole messages; starred messages, open polls and messages still on their way are kept |
| `displaySettings` / `setDisplaySettings` | – / optional `lastNameFirst`, `sortByLastName`, `showInactive` | `{lastNameFirst, sortByLastName, showInactive}`; `contacts` then sorts by `sortKey` (also the section letter) and leaves inactive IDs out if hidden |
| `multiDevice` | – | `{active}` (multi-device: connected through the mediator, ADR 0011) |
| `linkedDevices` | – | `{devices: [{id (hex), current, label, alias, platform, platformDetails, appVersion, connectedSince, lastDisconnectAt}], label}`, this device first; `[]` without multi-device. `label` is this device's own name, `alias` a name the user gave another device here |
| `setDeviceLabel` | `label` (≤ 64 bytes, empty: the default "Sailfish OS") | `{label}`; what the other devices show for this one. It travels in the D2M client-hello, so the connection starts again |
| `setDeviceAlias` | `device` (hex), `alias` (empty: its own label again) | `{}`; a name for our own lists — a device's label is encrypted with the device group key, so no device can rename another one |
| `dropDevices` | `ids` (hex) | the remaining devices; own id: multi-device ends, back to the chat server |
| `deactivateMultiDevice` | – | `{active: false}`: all other devices dropped, then this one; feature mask with forward security again |
| `gifFavourites` | – | `{favourites: [{id, width, height}]}`, newest first |
| `addGifFavourite` | `message` (a GIF message), or attached GIF + thumbnail / `path` with `width`, `height` (bridge `addGifFavourite(path)`) | `{id, favourites}`; the same GIF (SHA-256) is kept once and moves to the front; error `notGif` |
| `removeGifFavourite` / `sendGifFavourite` | `favourite` / `identity`, `favourite` | the list / the sent message like `sendFile` (field `favourite`, not `id`: the bridge's request id) |
| `updateSettings` / `setUpdateCheck` / `checkForUpdate` | – / `enabled` / – | `{available (this build has the check), enabled (missing: not asked yet), checkedAt, current, latest: {version, url} (only if newer)}`; `setUpdateCheck` errors in a store build. Checks run in the background (`checking: true` meanwhile; event `updateChecked` when done, `updateAvailable {version, url}` once per new version); automatic ones at most one attempt a day (ADR 0014) |
| `setLogLevel` | `level`: `warn` (default), `info`, `debug` | `{}`; applies at once (also start config `logLevel`; `THREESEA_LOG`, an env filter, overrides both for development) |
| `logText` | — | `{text}`: the in-app log (what also goes to the journal; fixed texts, kinds, codes and counts only), bounded tail of the rotated files |
| `logPackage` | `directory` | `{path}`: writes `threesea-log-<stamp>.txt` there — device facts (versions, model, flags) and the log; sharing it stays the user's step |
| `logClear` | — | `{}`; removes the in-app log files |
| `forwardSecurity` | — | `{on, multiDevice}`; `on` is false while multi-device is active (SE-199) or the user switched it off |
| `setForwardSecurity` | `on` | `{on}`; off terminates every session (DisabledByLocal) and the feature mask drops the FS bit, like multi-device activation; messages stay end-to-end encrypted either way (user decision 2026-10-28) |
| `networkSettings` / `setNetworkSettings` | – / `ipv6` | `{ipv6}`; off: Threema's IPv4-only hosts (`apip`, `blobp-…`, `g-…` instead of the dual-stack `ds-…`); the chat server connection starts again |
| `setArchived` | `identity`, `archived` | `{}`; archiving unpins; an incoming message unarchives; `conversations` has `archivedCount` |
| `chatMedia` | `identity`, optional `kinds` (default: the saved filter) | `{messages (each with mediaKind: image\|gif\|video\|voice\|audio\|file), kinds, counts: {kind: n}}`, newest first (media gallery) |
| `setMediaKinds` | `kinds` | `{kinds}`; the gallery filter, saved for all chats (absent: all, like Android) |
| `message` | `message` (local id) | the message JSON or null (message details) |
| `setVoiceListened` | `message` (local id) | changed message JSON or null; sets `body.listened` on a voice (file) message, so the chat knows which voice messages are still new and where ear playback starts |
| `setStarred` | `message` (local id), `starred` | the message JSON (field `starred`) |
| `starredMessages` | optional `identity` | `{results: [message + displayName, group, senderName, snippet]}`, newest first |
| `setPinned` | `identity`, `pinned` | `{}`; pinned chats come first in `conversations` (field `pinned`: their place, else null) |
| `movePinned` | `identity`, `offset` (−1 up, 1 down) | `{}` |
| `sendLocation` | `identity`, `latitude`, `longitude`, optional `accuracy` (m), `name`, `address` | the outgoing message JSON (kind `location`, body `{latitude, longitude, accuracy, name, address}`) |
| `setBlocked` | `identity`, `blocked` | contact JSON (or `{}` for a non-contact); the block list is `blockedContacts` in the app settings (in Threema Safe too); libthreema discards their messages, sending to them fails with `contactBlocked`. Contact JSON has `blocked` |
| `blockedContacts` | – | `{blocked: [{identity, displayName}]}` |
| `forwardMessage` | `message` (local id), `identity` (target chat) | the new message JSON; texts without their quote, files re-uploaded from the encrypted store with a new key (errors `fileNotDownloaded`, `notForwardable`) |
| `forwardMessages` | `messages` (local ids, in order), `identity` | `{forwarded, failed}` |
| `setChatNotifications` | `identity` (ID or group key), `policy` (`never`\|`mentioned`\|null), `until` (ms or null) | `{policy, until}`; also in contact/group JSON as `notifications`, conversations carry `muted`, `messageAdded` events `notify` (the setting allows a notification) |
| `chatTones` | – | `{tones: {key: path}}`: the chats with a tone of their own, without hidden private chats (the bridge caches these for `alert`) |
| `setChatTone` | `identity` (ID or group key), `tone` (path, empty or null: the tone of all chats) | `{}`; `locked` for a hidden private chat |
| `importChatTones` | `tones` `{key: path}` | `{}`; the tones the app kept in its settings file until 0.2.0, also those of hidden private chats |
| `exportChat` | `identity`, `media`, `directory`, `password?`, `me?`, `offsetMinutes?` | `{path, name, messages, files}`; a non-empty `password` under 8 characters: `passwordTooShort` |
| `parseLink` | `url` | `{action: contact\|compose\|add\|license, identity, text, key}` or error `linkInvalid` (protocol-notes "Threema links") |
| `myQrCode` | – | `{text, size, modules}`: `3mid:<ID>,<public key hex>` and its QR modules (`size`² chars `0`/`1`) |
| `verifyQrCode` | `text` | contact JSON; adds the contact if new, fully verified if the key matches (errors `qrInvalid`, `qrMismatch`). Camera frames / pictures are decoded first with the C function `threesea_qr_decode(grey, width, height)` |
| `callTurnServers` | `identity` (who the call is with) | `{urls, username, password, relayOnly}` for the media engine (`identity/turn_cred`, cached until expiry; `relayOnly`: the peer is unverified or not a contact, or the setting `relayThreemaCalls` is on; then the dual-stack servers, like Android) |
| `callStart` | `identity`, `sdp` (engine's offer), `video` | `{callId}`; the SDP is patched (`sdp.rs`) and sent as call-offer. Only with `calls: true` in the start config (Chum, ADR 0012) |
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
| `matchAddressBook` | `entries`: `[{firstName, lastName, emails, phones}]` | `{matched, added}`; hashed lookup (protocol-notes "Address book matching") |

File message `body`: `{rendering, mediaType, name, size, caption, width, height, duration, animated, thumbnail, fileState, thumbnailState, uploadState}`, states 0 remote, 1 loading, 2 on the device, 3 failed. Media: `image://threesea/<id>/thumb|file` (bridge), `threesea_media_read` (FFI).

Message JSON: `{id (local), conversation, messageId (hex), outgoing, kind (text\|location), text,
body, createdAt (ms), updatedAt, state, quoteId (hex or null), editedAt (ms or null; deletion time for kind `deleted`), sentAt, deliveredAt, readAt (outgoing; readAt also when we read an incoming one), receivedAt (incoming) — ms or null when unknown (older messages, skipped states), reactions: [{sender ("" = us), emoji}], mentions + mentionsMe (only if the text/caption has `@[…]`: `{ID: name}`, "" = us; see protocol-notes "Mentions")}`; outgoing state 0 pending, 1 sent, 2 delivered, 3 read,
4 failed; incoming 0 unread, 1 read.

Contact JSON: `{identity, displayName, firstName, lastName, nickname, verification (0–2), work,
workVerified, inactive, invalid, hidden, canEdit, canDelete (contact supports edits / deletions for everyone), reactions (contact has REACTION_SUPPORT: any emoji;
else only 👍/👎, which can't be withdrawn), canCall (O2O_AUDIO_CALL_SUPPORT), publicKey (hex)}`.

Error kinds (mapped to translated text in `qml/components/Errors.js`): `noLicense`,
`noIdentity`, `licenseFormat`, `server` (message from Threema, shown as-is), `network`,
`backupDecrypt`, `passwordTooShort`, `passwordTooLong`, `safeNotFound`, `safeTooLarge`,
`identityFormat`, `contactInvalid`, `contactIsSelf`, `contactNotFound`, `messageTooLong`,
`internal`.

QML usage: `backend.call("checkLicense", {licenseKey: k}, function(ok, result, kind, message) {…})`.
`backend.state` / `backend.identity` follow `status` replies and drive the root page.

### Groups, polls, pictures, linking, group calls, Threema Web, emoji

| Command | Arguments | Result / notes |
|---|---|---|
| `groups` / `group` | – / `key` (group conversation key `g:<creator>:<hex>`) | group JSON list / one group (title, members with names and `pictureVersion`, creator, left; the single group also `private`) |
| `createGroup` | `name`, `members` (IDs) | the new group; we are its creator |
| `renameGroup` / `setGroupMembers` / `leaveGroup` | `key` and `name` / `members` / – | the changed group; only the creator renames or changes members; the creator leaving disbands it |
| `setGroupPicture` / `removeGroupPicture` | `key`; attachment 0: JPEG | the changed group (creator only) |
| `setProfilePicture` / `removeProfilePicture` | attachment 0: JPEG / – | the profile |
| `createPoll` | `identity`, `description`, `choices`, `multiple`, `announce` | the poll message |
| `vote` / `closePoll` | `message` (local id) and `choices` (choice ids) / `message` | the changed poll message; closing also adds the `pollClosed` message. Error `locked` in a hidden private chat |
| `qrKind` | `text` | `{kind: "contact" \| "web" \| "device" \| "unknown"}` |
| `startLinking` | `code` (Threema for Desktop's QR code), `pin` | `{choices: [[emoji ×3] ×3], correct}` for the emoji check (ADR 0011) |
| `confirmLinking` / `cancelLinking` | – | the linked devices once the new device registered / `{}` |
| `groupCallRefresh` | `conversation` | `{}`; the state comes as the `groupCall` event (ADR 0012) |
| `groupCallJoin` | `conversation`, `fingerprint` (hex SHA-256 of our DTLS certificate), `create` | `{offer, candidates, participantId}` for the media engine |
| `groupCallChannel` | `data` (base64, from the SFU's data channel) | `{}`; answers go out as `groupCallSend` events |
| `groupCallMute` / `groupCallCamera` | `muted` / `on` | `{}` (tells the others; the engine switches the device) |
| `groupCallTick` / `groupCallLeave` | – | every second while in a call (rekeying, state) / leave |
| `webSessions` | – | `{sessions: [{id, host, selfHosted, persistent, client, createdAt, lastConnection, state}]}` (ADR 0013) |
| `webPair` | `qr`, `allowHost` | the new session; a host outside threema.ch first answers `{confirmHost}` and needs `allowHost` |
| `webStart` / `webStop` / `webDelete` | `session` | `{}`: connect a stored session / disconnect / forget it (also ends its task's authority) |
| `webAcceptCertificate` | `session` | `{}`: pin a self-hosted server's new certificate and connect again |
| `recentEmoji` / `useEmoji` | – / `emoji` | `{emoji: [text]}` newest first / record one as used |

## D-Bus (notification actions)

Service `org.threesea.harbour-threesea`, object `/`, interface `org.threesea.harbourthreesea`:
`openChat(identity)` (tap), `reply(identity, token, text)` (input action "Reply": lipstick appends
the typed text), `markRead(identity, token)`, `activate()`, `activate(urls)`, `showUpdate()`,
`background()`. `identity` is a Threema ID or a group key. `token`: a random value per chat, put
into the notification's actions and kept in the app's private settings; calls without the
current token are ignored, since any process on the session bus could call the service and
lipstick calls through a helper (`mliteremoteaction`) the sandbox can't identify (audits S07/D04).
Lipstick raises the app window for non-input actions (Mark as read); quick reply stays in Events.

## Database key encryption (C functions, ADR 0009)

`threesea_key_wrap(key, 32, passphrase, keyboard, &len)` → blob, `threesea_key_unwrap(blob, len,
passphrase, &len)` → key or NULL, `threesea_key_wrapped(blob, len)` → -1 / 0 (digits) / 1
(keyboard). scrypt: call off the GUI thread. The bridge keeps the blob in Sailfish Secrets
instead of the key; `backend.state` is `locked` until `backend.unlockDatabase(passphrase, cb)`.
`backend.dataEncrypted`, `backend.setKeyEncryption(on, passphrase, keyboard, cb)` and
`backend.checkPassphrase(passphrase, cb)` drive Settings → Security; independent of the app lock.
