# Protocol notes

Things learned while porting. Sources are upstream paths at the pinned tag.

## Who we say we are (client-info; ADR 0015)
- CSP and license check: `Threesea/<ver>;S;<lang>/<country>;<model>;<osver>`
  (`ClientInfo::Threesea` in our libthreema, values from `ClientVersion`). The shape is that of
  Threema's mobile clients. Threema suggested the string identify our project (mail, November 2026).
  **`S` is our own code**, not one Threema documents (`A`, `I`, `Q`, `W`, `B`, and libthreema's
  own `L`): we are none of those, and saying so is the point. The name in the version is there
  because a server-side reader has no way to know what `S` means.
- D2M device info (what the other devices show): platform `Unspecified` (the protobuf has no
  value for us), details `Threesea (<model>)`, the app version, the user's label.
- HTTPS: `Threesea/<ver> (Threema compatible; Sailfish OS)` (`core/src/https.rs`). libthreema's
  own user agent is dropped there. The app's own name comes first, followed by a compatibility
  token that the blob server expects (it answers 403 without "Threema" in the user agent).
  Threema is fine with this user agent and with our code `S` (asked, November 2026).
- Threema Web, the one place where we send another platform's value: `clientInfo.os` selects the
  browser's push-token parser, and Threema Web only parses the `threema-gateway` token for
  `android`, so we send that value as a protocol setting (`docs/research/threema-web.md`).
  `appVersion` names Threesea. The value goes only to the user's own browser, inside the end-to-end
  encrypted session.
- What Threema asks of third-party clients (mail, November 2026): no stability issues or high load
  on the servers, clearly marked as unofficial, no "Threema" branding without authorization, no
  circumventing the licensing.

## Calls: what ends a call
(`core/src/calls.rs`, `app_calls.rs`, `bridge/callmanager.cpp`, `core/src/call_video.rs`)
- An offer's `created_at` is the sender's clock, so offers are not aged: the server's short-lived
  queue and the 60 s ring time bound them, like Android (`VoipStateService`).
- A hangup that matches no running call is a missed call only if the chat's last 4 call lines
  don't name its call id (Android's `isMissedCall`; the id is the 5th field of `call|…`, so this
  holds after a restart). It never touches a call in progress. Offers whose call is over are ignored
  the same way (delivered again after a lost acknowledgement).
- Offers during a call: rejected "busy" and recorded as missed (Android too).
- Rejected "unknown" means the other app could not take the call, not that anyone declined.
- Timeouts: media not up 60 s after setup (`INITIALIZATION_TIMEOUT_MS`), group calls 20 s
  (`TIMEOUT_CONNECTED_SIGNAL_MILLIS`). Running calls: libnice 0.1.21 has no consent freshness, so
  the bridge counts what enters `dtlssrtpdec` (ICE checks don't, media and SCTP heartbeats do): 1:1
  calls say "interrupted" after 10 s of nothing and fail after 30 s; group calls can be quiet with
  everyone muted and fail after 60 s. These limits are ours, not Android's.
- Remote VP8 pictures: at most 2560 on a side and 2560x1440 in area (`vp8_filter`, ahead of the
  decoder; the MAX profile is 1920x1080). A larger key frame stops that stream until a good one.

## Reporting spam
- From iOS 7.5.2 (`ServerAPIConnector.m`, `SpamDetector`, `GroupSpamDetector`), newer than Android's
  `identity/report_junk` (`APIConnector.reportJunk`), which has no group variant. Both are on the
  directory; the two-phase challenge is the usual one.
- `POST identity/report_identity`: `reporter_identity`, `peer_identity`, `peer_nickname` (explicit
  `null` when none). `POST identity/report_group`: `reporter_identity`, `group_creator_identity`,
  `group_creator_nickname`, `group_name` (always `null`, iOS doesn't send it either), `group_id`
  (decimal), `group_member_count`. The answer is `{"status": "success"}`; errors are
  `{"status": "error", "code", "message"}`, a 404 `ID_NOT_FOUND` for an unknown reporter or reportee.
  IDs must be alphanumeric, so a gateway's (`*…`) can't be reported (400 `INVALID_PARAMETER`), and
  reporter and reportee must differ. Only phase 1 was run against the server (2026-11-03): **the
  full report needs a real case.**
- Offered when: not dismissed or reported before; the contact (a group's creator) is unverified,
  unnamed, not blocked and not predefined, a 1:1 contact also not "group or deleted"; for a group
  also not left and not our own; no outgoing message yet; the first incoming one at most 7 days after
  the contact was created. Android's window is 1 day; we follow iOS's written steps.
- Blocking and deleting only after the report went through (Android; iOS doesn't wait).

## Libre / Threema Shop licensing
- Libre builds use `LicenseType.SERIAL` (`app/…/BuildFlavor.kt`). Key format `XXXXX-XXXXX`
  (A–Z, 0–9; `EnterSerialActivity.java`).
- Check: `POST {directory}check_license` with JSON `{licenseKey, deviceId, version, arch}`;
  response `{success, error | updateMessage, updateUrl}` (`APIConnector.java` ~L1095–1153).
  `version` = `AppVersion.getFullVersionString` (`<appVer>;<platform>;<lang>/<country>;<model>;<osver>`).
  We send the same string as in the handshake, see "Who we say we are" below. The server accepted
  such requests (2026-11-03: an invalid key got the normal "This license key is invalid or
  expired." answer).
- Rechecked: Android whenever its main screen opens (`CheckLicenseRoutine`), we at most daily from
  the hourly maintenance. Like Android only `success: false` with the directory's own words counts
  as a refusal; any other failure is ignored and retried. A 429 is not a refusal either
  (`DirectoryError::RateLimited`, kind `rateLimited`). Refused: the state is `needsLicense` again and stays so across restarts,
  nothing connects until a key is accepted (Android opens `EnterSerialActivity`).
- A 429 from the directory on any call starts a rest of an hour, kept across restarts
  (`Store::rest_directory_calls`): the license recheck, contact refresh, pending Safe import,
  e-mail link check, feature mask, group member lookups, the forward secrecy mask check and the
  hourly address book sync stay home until then. A rest under way keeps its end. What the user asks
  for still goes. Group members skipped or left without an answer stay in `MEMBER_LOOKUPS_OWED` and
  are looked up again on connecting and in maintenance. Android under the same 429 runs its worker
  about once a day; we have no scheduler to lean on.
- `deviceId`: random UUID persisted once (`DeviceIdProvider.kt`).
- Identity creation for Shop licenses also sends `licenseKey` + `deviceId`
  (`UserServiceImpl.createIdentityRequestDataJSON`); libthreema's `create_identity_request`
  doesn't for `Flavor::Consumer` → extend in `core`.
- Restoring an ID backup: the backup only holds ID + client key; the server group comes from
  `POST identity/fetch_priv` (two phases, directory challenge–response; `APIConnector
  .fetchIdentityPrivate`), which also returns linked e-mail/phone.
- Directory challenge–response (`tokenRespKeyPub`, `token` → `response`) is used by many
  endpoints; libthreema solves it (`ClientKey::solve_directory_challenge`, patch 0002).
- TLS pins are **leaf** SPKI pins: on 2026-10-21 `*.threema.ch` presented
  `vGQZ8hm2h+km+q7rnJ7kF9S17BwSY0rbhwjz6nIupf0=` (first pin in upstream's set). Fixture:
  `core/tests/fixtures/threema-ch-leaf.der`.
- Sockets to the chat server, the mediator, rendezvous relays and Threema Web's signalling server (`core/src/dial.rs`): every address on its own, IPv6 first, 3 s per IPv6 address and 15 s per IPv4 address, like Android's `CspSocket` (`CONNECT_TIMEOUT_IPV6`, `CONNECT_TIMEOUT`). A WebSocket's whole connect keeps a limit around that (the mediator's 25 s, login included).
- Directory base: Android uses `https://apip.threema.ch/`, libthreema `https://ds-apip.threema.ch/`. The `ds-`/`ds.` hosts are dual-stack (Android's "IPv6 for messages": `DIRECTORY_SERVER_IPV6_URL`, `CHAT_SERVER_IPV6_PREFIX` `ds.g-`, `BLOB_SERVER_IPV6_URL` in `app/build.gradle.kts`); we use them by default and drop the prefix for IPv4 only (Settings → Advanced, `https::server_url`).

## Servers and keys
- Production config: libthreema `common/config.rs` `Config::production()`.
- Chat server permanent public keys: `buildSrc/src/main/kotlin/config/PublicKeys.kt`
  (prod `450b9757…c909`, prodAlt `da7c7379…6f15`) — match libthreema.
- CSP is not TLS; server authentication is the protocol's key pinning.
- HTTPS pins (SHA-256 SPKI): `app/src/main/res/xml/network_security_config.xml`
  (threema.ch, threema.com, sfu.threema.ch, …). We must replicate these in our HTTPS client.

## Background on Libre
- Libre has no FCM: `forceThreemaPush = (this is Libre)`, i.e. a foreground service holding a
  persistent CSP connection (`ThreemaPushService.kt`), plus optional polling (`PollingHelper.java`).
  Our equivalent: keep the connection in-process (Lite) or in a systemd user service (Full).

## libthreema shape
- Sans-I/O, poll/response `TaskLoop`s; HTTPS requests are returned as data for the caller to run.
- Not `Send` (`Rc`/`RefCell`) → one core thread owns all protocol state.
- Examples worth reading: `lib/examples/bin/{csp_login,csp_e2e_receive,identity_create}.rs`.

## Chat server connection (CSP)
- `core/src/connection.rs` wraps libthreema's sans-I/O `CspProtocol` + `CspE2eProtocol` like its
  `csp_e2e_receive` example. Address from `Config.chat_server_address.addresses(server_group)`
  (`ds.g-<group>.0.threema.ch`, ports 5222/443). CSP is not TLS: libthreema pins the server key.
- Our storage sits behind libthreema's provider traits (`core/src/providers.rs`): contacts,
  conversations, nonces, settings. Without multi-device, the only instruction of the incoming
  task is `FetchSender` (directory lookup); contacts are written by libthreema directly.
- One device cookie per installation (random 16 bytes, stored). Echo request after 60 s of
  silence, reconnect if nothing comes back within another 60 s; backoff 1 s → 60 s. A rate limit on
  the incoming task's `FetchSender` ends the session too, and libthreema asks for at least 10 s:
  no attempt before that, wake-ups included, and the wait grows to 5 minutes while each login
  meets the limit again (the waiting message repeats the lookup every time).
- Outgoing (no libthreema task yet, LIB-51): patch 0003 encodes; we pick a fresh nonce, record it
  in the nonce table, send, and mark the message "sent" on the server's message-ack. Delivery and
  read receipts are delivery-receipt messages handled by libthreema and surfaced through
  `add_or_update_incoming_message`.
- Our nickname goes in the metadata of messages that distribute the profile. Three answers:
  a name; an empty string, which makes the receiver drop the name it knows (Android's
  `setNicknameFromRemote("")`); or nothing, which leaves it alone. An ID backup holds no nickname,
  so an ID restored from one, a new one, or one from a Threema Safe backup without a nickname says
  nothing until its user is asked once
  (`NICKNAME_UNANSWERED`, `NicknameDialog.qml`) and picks a name, or later takes one away. Android's
  wizard defaults the nickname to the ID, which also replaces the name friends knew.
- Receiving, in this order: everything a message says goes into the store in one step, the delivery
  receipt included (an outbox entry; sent after the next login if not now); the UI hears of it; then
  the acknowledgement; then what takes the network. So the connection can die at any point: what is
  stored is announced (`Session::drop`), and what the server sends again is known and only
  acknowledged. Forward secrecy answers and statuses the store holds nowhere else wait for the next
  connection (`FS_EVENTS_CARRIED`, saved when a connection ends; only IDs and Accept, Reject and
  Terminate envelopes, no message content). A store failure ends the connection without an acknowledgement, so the message
  comes again and holds up the queue behind it: the log gets SQLite's code, the user one notice per
  streak (`storageProblem`), since the server keeps messages only for a limited time.
- What fails the same way every time mustn't hold up everything behind it: after 3 tries in a row
  (`TRIES_BEFORE_DROPPING`) a message is acknowledged and dropped, with an error in the log, like
  Android, which acknowledges any failure but the network's at once. That goes for libthreema's own
  errors and SQLite's 0, ERROR, TOOBIG, CONSTRAINT and MISMATCH (`store::fails_every_time`), never
  for the state of the device: FULL, IOERR, READONLY, BUSY, LOCKED, NOMEM, CANTOPEN, CORRUPT, NOTADB
  wait as long as they last, and so do network and server errors.
- Reflected envelopes: applied and their nonce noted in one store step, then acknowledged; one the
  store failed on stays unacknowledged, so the mediator hands it out at the next login, by the same
  rule as above (without an end of that queue there is no promotion to leader either). Android
  acknowledges those too (with a TODO), but nothing else brings them to a follower.
- A leader stores a 1:1 message before reflecting it. Until the task is through it is in
  `awaiting_reflection`, which keeps `message_is_marked_used` from discarding the server's next copy
  as a reused id: that copy runs the reflection again, or the other devices would never get the
  message. A group message has no such check in its way.
- Forward secrecy: see below. Not yet: groups (LIB-53), typing-indicator sending, outgoing retries
  after server-side failures.
- Live-tested 2026-10-21 with the test ID: ECHOECHO echoes text; server ack → "sent",
  ECHOECHO also sends a "read" receipt.

## Threema Safe
Not in libthreema (only the URL template); implemented in `core/src/safe/` after upstream
`app/src/main/java/ch/threema/app/threemasafe/ThreemaSafeServiceImpl.java`. Format spec:
`domain/libthreema/threema-protocols/src/safe-backup.schema.yml` (version 1).
- Key: `scrypt(password, salt = Threema ID, N=2^16, r=8, p=1, 64 bytes)`; bytes 0–31 = backup
  ID, 32–63 = encryption key. Upstream test vector (`ThreemaSafeServiceTest.kt`) reproduced in
  our tests. We store the 64-byte master key (in SQLCipher), never the password — like Android.
- Blob: `nonce(24) || XSalsa20-Poly1305(gzip(JSON))`. Upstream gzips without compression to
  avoid compression oracles; we do the same (`flate2::Compression::none()`).
- Server: `https://safe-{first byte as 2 lowercase hex digits}.threema.ch/` (libthreema's unused
  helper doesn't pad with a zero). `GET config` → `{maxBackupBytes, retentionDays}` (live
  2026-10-21: 3 MiB, 180 days); `GET|PUT|DELETE backups/<backupId hex>`; 404 = no backup.
  Custom Safe servers are not supported (third-party endpoints, §7 rule 6).
- Upload policy (`upload_due`): unchanged JSON → refresh after half the retention; changed →
  at most every 23 h (a chat's new message counts as a change, as `lastUpdate` is part of the
  JSON); after an error or "Back up now" → immediately. Checked at core start and
  hourly while running. Change detection: SHA-256 of the JSON.
- `lastUpdate` (contacts, groups, lists; ms): the chat's newest message when writing, as Android
  lists a restored contact's chat only if it has one (`ConversationServiceImpl`). Reading: a chat
  with a `lastUpdate` is listed without messages from then on (`emptied_chats`, the stand-in of an
  emptied chat), so it survives our next upload; entries not imported yet keep the backup's value.
- Links: only confirmed ones go into a backup, or to a linked device (Android:
  `LinkingState_LINKED`), since a restore on Android links them again, with a mail or an SMS. An
  address waiting for its confirmation stays out, and starting a number's change drops the old
  number at once, as the server does.
- Restore: license first (Libre), derive, download, decrypt, parse (reject version > 1),
  `identity/fetch_priv` for the server group, keep nickname/links. Contacts, then groups and
  lists, wait in `safe_pending_import` until imported (directory lookups) **and are carried into
  new backups** meanwhile, so an automatic upload never drops them. What the restore keeps goes
  into the store in one transaction (the picture's files before it): killed half way, there is no
  ID yet and the restore can simply be repeated.
- Groups and lists (Android `getGroups`/`restoreGroups`, `…DistributionLists`): ids are
  lowercase hex of the **little-endian** 8 bytes (the wire order; list ids likewise from our
  `d:<u64>` key). Group `members` are all participants incl. the user while a member (we store
  everyone but the user). Restore: skip the legacy `deleted` flag and groups whose creator is
  unknown; look up unknown members once (group members as _group_ acquaintances, list members
  as direct contacts), drop those that don't exist; not a member → left; member but the creator
  isn't → left (Android: kicked). Existing groups/lists are kept. No picture (not in Safe). Until
  2026-10-22 our backups only carried groups/lists from an earlier restore, never our own.
- After a restore (Android's _Application Setup Steps_, `ApplicationSetupSteps.kt`), once the import
  is done and the connection up; the flag is stored with the restore, so a kill doesn't lose it:
  the feature mask goes out first, then for each group not left the state to all members if it is
  ours, else a `group-sync-request` to its creator (not one on the block list, ADR 0019). Then the
  _solicited contacts_ (people in those groups, and those with a chat; no revoked IDs, and no
  blocked ones) get an FS `Init` if they support FS (not with multi-device or FS off) and a request
  for their picture, 20 per round (connect, hourly). Deviations from Android: a creator's restore
  sends no `delete-profile-picture`, as Android does for a picture it merely lost; a contact we
  already have a session with gets no empty message, as that session is newer than the restore. A
  session's "established" line only appears in a chat with something in it: Android's doesn't list
  the chat, ours would.
- Passwords: 8–4096 chars; "weak" = single repeated char, ≤ 15 digits, or in upstream's
  `app/assets/passwords/bad_passwords.txt` (warning only, like Android).

## Forward secrecy (PFS)
Implemented in the vendored libthreema (`csp_e2e/fs/`, our commits before the public history),
byte-compatible with Android `DHSession`/`KDFRatchet`/`ForwardSecurityMessageProcessor`:
- Keys: 2DH `B2b(DH(I_CK,R_CK)||DH(I_FSSK,R_CK), salt "ke-2dh-"+I_ID)` (64-byte concat is the
  BLAKE2b *key*, as Android does, not input as the spec text suggests); 4DH via BLAKE2b-512 of the
  four DHs in initiator order, then `B2b(I, "ke-4dh-"+sender)`; ratchet `kdf-ck`, message key
  `kdf-aek`, personal `3ma-e2e`, counters start at 1, max skip 25000. Inner = `type||body`, no
  padding, XSalsa20Poly1305 with a zero nonce (so the turned ratchet is stored before sending).
- Versions 1.0–1.2 offered; Android's 2DH clamp to 1.0 mirrored. Types need 1.0 (text,
  location, file, polls, empty), 1.1 (receipts, typing, reactions, edits, deletes, profile
  pictures, calls) or 1.2 (groups).
- Incoming: libthreema's incoming task decapsulates (`CspE2eContextInit::fs_sessions`); the peer
  ratchet is turned in `acknowledge()` (`fs-commit-fn`). Control replies (Accept/Reject/Terminate)
  are queued through the store and sent by `core::connection`.
  An answer carries the key of whoever wrote (`enqueue_control`): a stranger's Init comes before
  the message that creates the contact, and a lost Accept costs a rejected first reply. Not with
  "block unknown senders" on (`providers::control_receiver`).
- Outgoing: only to contacts whose feature mask has `FORWARD_SECURITY_SUPPORT` (0x40), the mask
  alone decides, like Android; an existing session doesn't. We publish our mask
  (`identity/set_featuremask`, challenge–response) = 0x40. Encapsulated group messages name their
  group, as the spec requires (`group_identity`), and a Reject repeats it.
- The feature mask follows the peer: a Terminate (of a known session, or DISABLED_BY_LOCAL), a
  Reject, a message without forward secrecy, or an Init from someone whose stored mask says no looks
  up that one contact inside the connection (`fs_support_after`), and so does sending to someone with
  a session whose stored mask says no. At most once a minute per contact, across reconnects; an Init
  every 5 s, and it trusts only a lookup made for it, since the "no" from just before may be from a
  linking that was rolled back. What the lookup finds is stored on a fresh read of the contact. If the
  directory answers no FS, what's left of the sessions ends with a Terminate; without an answer they stay
  (DISABLED_BY_REMOTE) and the chat gets its line, as also when the daily refresh finds it
  (`fs_support_lost`, Android's `OnFSFeatureMaskDowngradedTask`).
- A peer Reject puts the message back into the queue at once, so that nothing can lose it, and it is
  re-sent after the mask lookup — Android asks the user first. With a group named: the Common Group Receive Steps, a message we
  don't have counts as a sync request if the group is ours, and only a message of ours goes out
  again, to the sender of the Reject. Without one (older clients, or our own messages from before
  we named it): by ID, and only for a member of that group.
- Forward secrecy off (multi-device, or Settings → Privacy): `fs_answering_only`. No sessions, but a
  message in an envelope gets a Reject with DISABLED_BY_LOCAL, as the spec says, so it is sent again
  without; other variants are discarded. Android answers with a Terminate, which loses the message.
- A message without an envelope on a session covering its type (`minimum_incoming_applied_version`,
  like Android's `warnIfMessageWithoutForwardSecurityReceived`) gives a status line if the contact's
  mask, just looked up, still says FS. Every such visible message gets its line, as on Android, right
  after it; a run of receipts or typing, which add no rows, gets one.
- Status lines (`fs_status_line`) only for sessions that existed, and only what we know: `fsReset`,
  `fsMessageRejected` (a message really goes out again), `fsUnavailable` (the mask says no),
  `fsMessageWithoutFs`, `fsEstablished`. `fsTerminated` only survives in old rows.
- A row that can't be a session counts as none and is removed; a session in an impossible state is
  ended and replaced when sending (`encapsulate_or_replace`). Should encapsulating still fail, the
  message counts as not sent. None of this ends the connection.
- Deleting a contact leaves its sessions alone, like Android (`MarkContactAsDeletedBackgroundTask`
  has no part in them; only `DeleteAllContactsBackgroundTask` deletes them, when the whole ID goes).
  The row stays (ADR 0022), so the person's next message comes in on its session.
- Simplifications vs Android (TODO): new sessions are stored right away (Android waits for the
  server ack of the Init); no 24 h keep-alive `empty` messages.
- ECHOECHO does not support FS (mask 0x9). Interop verified with the official Android app
  (2026-10-21). Android turns FS off (mask without 0x40) while multi-device is active.

## Reactions

- `Reaction` (0x82, protobuf `csp_e2e.Reaction`: `message_id` + `apply`/`withdraw` emoji bytes)
  needs `REACTION_SUPPORT` (0x400) on both sides; we announce it. Push flag, replay protected,
  no delivery receipts. Additive: several emoji per sender per message.
- Peers without it understand only the legacy delivery-receipt types 0x03 acknowledge (👍) and
  0x04 decline (👎), any skin tone mapped to the base emoji. A legacy reaction **replaces** all
  reactions of its sender on the message; there is no legacy withdraw, so the UI offers only
  👍/👎 for such contacts and doesn't withdraw.
- In a group, reactions work if any member supports them, and each member is chosen for when sending
  (`body_for_receiver`): without support a thumb goes as the legacy form; anything else goes as the
  `Reaction` itself, as the spec's footnote says (Android leaves them out).
- Receive: unknown referred messages and status lines (not reactable) are ignored; emoji > 64
  bytes, empty or non-UTF-8 are rejected by the decoder (libthreema).
- Outgoing reactions are not messages of their own; they wait in the `outbox` table until the
  server acknowledges them (reused for edits/deletions later).
- Qt 5.6 can't draw Twemoji's SVG-in-OpenType colour glyphs, so emoji render as the font's
  monochrome fallback — same as in Jolla's own apps.

## Edit and delete

- `EditMessage` (0x91: `message_id`, `text`) and `DeleteMessage` (0x92: `message_id`) need
  `EDIT_MESSAGE_SUPPORT` (0x100) / `DELETE_MESSAGE_SUPPORT` (0x200) on both sides; we announce
  both. Push, replay protected, no delivery receipts.
- Sender side only: own messages, at most 6 hours old. The receiver accepts any age but only
  from the original sender (we look up the message in the sender's direction only), and applies an
  edit to a text or a file's caption, as Android does. Editing our own captions isn't offered yet.
- A deletion applies to texts, locations and files only, as on Android; a poll stays what votes
  and its close refer to.
- A deleted message becomes a placeholder (kind `deleted`, content, reactions and poll entry removed,
  `edited_at` = deletion time). It has nothing left to send: a later Reject of it changes nothing.
  A message whose send hasn't begun, or that failed, is simply changed or removed without telling
  the peer. Pending but with its send begun (`written_out`, on record before the first byte goes out,
  as the ack may be lost or a group's members acknowledge one by one), someone may have it: it takes
  an edit or deletion like a sent one, and may need the feature for that, while a resend carries the
  new text to whoever doesn't have it yet (Android allows neither until the message is sent).
- "Delete" on its own removes a message from this device only (any message).
- Notes groups (a group of one, not left): every feature counts as supported, no age limit, and
  reactions, edits, deletions and votes still go through the outbox so that linked devices hear of
  them; the receiver list is just empty. "Delete for everyone" is offered only with linked devices.
- Editing must not trigger typing indicators.

## Blob server

- Download `GET https://ds-blobp-{first byte, 2 hex digits}.threema.ch/{blob id hex}`; only some
  prefixes exist in DNS (blob ids are server-generated, so real ids always resolve).
- The blob server answers **403 unless the user agent contains "Threema"**. We send `Threesea/<version> (Threema compatible; Sailfish OS)`.
- Upload `POST https://ds-blobp-upload.threema.ch/upload?persist=0`, multipart field `blob`
  (`blob.bin`, `application/octet-stream`), answer: blob id in hex. Done: `POST …/{id}/done`.
- A group's file goes up with `persist=1`, and without multi-device nobody sends `done` for it:
  the other members still need it. 1:1: `persist=0`, `done` by the recipient once fetched. Android's
  rules: `shouldPersistUploadForMessage` and `shouldMarkAsDone` in `MessageServiceImpl.java`. One
  deviation: without multi-device we send no `done` for our own 1:1 file fetched again after a
  clean-up (Android does), as its recipient may not have it yet.
- Time limits as on Android: 30 s to connect and 100 s without anything moving, no limit on the
  whole transfer (`BlobUploader.kt`, `BlobLoader.kt`: connect and read timeouts only). reqwest's
  `TCP_USER_TIMEOUT` of 30 s is raised to 120 s, or any blackout that long would end the transfer.
- An upload's progress is seen by the body being taken piece by piece; once all of it is taken,
  300 s for the answer. Taken means into the system's send buffer, not sent. **So uploads have a
  floor of about 75 to 85 kbit/s:** a probe on loopback saw 943 KB read between two takes (100 s:
  75 kbit/s) and 3.1 MB taken but unread when the last piece was (300 s: 83 kbit/s). Below that
  an upload can fail however steadily it moves; the real figures follow the phone's buffer sizes.
  reqwest offers no way to set `SO_SNDBUF`, so we leave it.
- A body sent in pieces can't be sent twice: reqwest hands a 307 or 308 back instead of following
  it, which then waits for a tap, and an HTTP/2 refused stream, which it would otherwise send again
  by itself, fails like a network error and costs a try and a pause.
- Libre needs no auth token (only OnPrem does).
- Sailjail: the pickers (`Sailfish.Pickers`) need `MediaIndexing` (Tracker) besides Pictures,
  Videos, Music, Documents, Downloads; without it the picker pages stay empty.
- Deprecated image, audio and video messages (0x02, 0x13, 0x14, group 0x43–0x45), which nobody may send
  any more: decoded into `FileMessage` as Android's `LegacyMessageTransformer` does, so they are
  ordinary file messages from there on. A 1:1 image has no file key: its blob is sealed with the
  contact's and our keys and a nonce from the message (`legacyNonce` in the body, kept from the UI). The
  rest use their key with nonces `00..01` and `00..02` (video thumbnail). The file JSON is read
  leniently, as Android's vendored `org.json` coerces: an optional field of another type counts as
  absent, a size may be a float or a numeric string, a media type that isn't a text means none; name
  and caption limits apply when sending only.

## Voice messages

- A voice message is a `file` message with an `audio/*` media type and rendering type media
  (Threema Android: `MimeUtil` → `VOICE_MESSAGE`); duration in seconds in metadata `d`, no
  thumbnail. Android records AAC in MP4, mono, 32 kbit/s, sends `audio/aac` named `*.aac`; iOS
  sends `audio/x-m4a`.
- Sailfish: AAC only exists through gst-droid (phone hardware codecs: `droidaenc`, `droidadec`).
  Jolla's ffmpeg has no AAC, and the SDK emulator (x86, no Android HAL) has neither AAC nor any
  Qt mediaservice.
- Policy (user, 2026-10-21): platform AAC first; automatic fallback to something without legal
  questions: **Opus in Ogg** (royalty-free), recorded as raw PCM (`QAudioInput`, 48 kHz mono) and
  encoded by the core with `opus-pure` (pure Rust, BSD; vendored with an i686 build fix), sent as
  `audio/ogg`, which Threema Android plays. Playback: the platform player (AAC and Opus); without
  one, Opus is decoded by the core and played through `QAudioOutput`. Received AAC without a
  platform player can't be played (we don't ship an AAC decoder).
- We record with `QAudioRecorder` (codec/container picked from what it offers: AAC in MP4; the
  lists are logged at start), play with `QMediaPlayer` from memory (`QBuffer`, decrypted audio
  never on disk). The recording sits in the app's private cache only until it is handed to the
  core. Without a recorder the microphone button is not offered.

## Groups

- Conversation key `g:<CREATOR>:<group id hex>`. Stored `members` = everyone but the user
  (creator included unless that's us) = the receivers of everything we send.
- Every group message is a separate 1:1 message per member with the same message id (FS per
  member, v1.2 encapsulates group types); `pending_receivers` tracks who still needs it; it is
  _sent_ once all were acknowledged. No delivery/read receipts in groups.
- Containers: creator messages (`group-setup`, `group-name`, `set/delete-profile-picture`) use
  `group-creator-container` (group id; the sender is the creator), everything else
  `group-member-container` (creator id + group id).
- Receive: the _Common Group Receive Steps_ (unknown group → `group-sync-request` to the creator,
  at most once an hour; left → empty setup (creator) or `group-leave` in reply; sender not a
  member → empty setup or sync request) and each control message's own steps, in
  `core/src/groups.rs` (unit-tested). Members we don't know are looked up and stored with
  acquaintance level _group_ (not shown as contacts): by the connection, right after the setup and
  before the next message, and for whatever that left open on reconnect and in maintenance
  (`Store::unresolved_members`: members of groups we are in, receivers of what is queued). The
  setup's nonce is used by then, so a failed lookup can't count on redelivery. Someone the directory
  doesn't know, or a contact that is revoked when the setup comes, is not kept as a member (Android
  neither) and nothing waits for them. A sync request's creator is looked up the same way.
- Anyone can name any ID in a setup or a group message, so what that makes us ask the directory is
  bounded: a setup listing over 256 (Android's `MAX_GROUP_SIZE`) is discarded, at most 512 IDs go to
  group lookups per hour (the rest wait for the next round), and an ID the directory didn't know is
  remembered for a day (in memory) and left out of setups and sync requests. An "added" line for
  someone with no contact waits for their lookup, so that no made-up ID gets one.
- A group-setup creates its sender as a _direct_ contact (csp.struct.yml, Android), a stranger's
  too, also under _block unknown_ (ADR 0019). Its dedicated blocking steps run in libthreema before
  anything is created: a creator on the block list, for a group the user isn't in, is discarded.
- A group message from someone with no contact row who is a member of a group we are in isn't
  discarded: libthreema just looked up their key to decrypt it, so it creates the contact at level
  _group_ (or keeps the message unacknowledged if that lookup failed). Anyone else is discarded.
- Queued while offline, a group's messages and control entries go out in the order the user made
  them (by `created_at`): the setup before the first text, a text before our leave.
- Deleting a group we left keeps the queued `group-leave` or empty setup until it is acknowledged:
  it is sent by the key alone, and otherwise the members keep writing and the creator's answer to
  the sync request brings the group back. All else queued for the group goes.
- Whoever left or was removed after something was queued for them doesn't get it
  (`providers::group_receivers`). Exceptions: a leave, an empty setup, and a sync request to the
  creator.
- Creator: create (setup + name + delete-picture to all), rename, change members (setup to those
  in the group, an empty setup to those removed, name/picture to added; the blob is persisted, so
  the same descriptor serves), dissolve (empty setup). A running call's start is repeated, with
  its `started_at` as `created-at`, to new members and to whoever gets the state on a sync request
  or a Reject, after a fresh peek rather than the spec's "most recent refresh" (no event for the
  UI: it can come while locked). Members: leave (`group-leave` to
  all). Answering sync requests: the state, or an empty setup for non-members.
- Threema Desktop refuses to link at all over a group naming someone it has no contact for
  (`dom/backend/join.ts`), and asserts the same on `GroupSync`. So no group data names a member
  without a contact (`join::group_sync_members`), a group whose creator has none stays out of it
  altogether (`join::group_syncable`), and a contact a group still names, a left group too, stays
  as a group acquaintance when deleted.
- Group pictures: JPEG, uploaded with `persist=1` (members fetch it without `done`), sent with
  sync answers and to new members.
- Group sync (`GroupSync`, multi-device): `member_identities` holds neither the creator nor the
  user, ours (above) has the creator. `groups::normalized_members` turns a synced list into ours
  and `join::group_sync_members` ours into a synced one, which also leaves out anyone without a
  contact, as the other devices wouldn't know them. An `Update` for a group we don't have is
  ignored, only `Create` makes one (like Android).
- `user_state`: `groups.kicked` tells of the groups we are out of (`left_group`) whether the creator
  removed us. Groups from before that column stay "left"; they can't be told apart.
- Group control reflected as an incoming message (another device leads) is discarded like on
  Android, and nothing goes out in reply: that device answered, and tells us by `GroupSync`. The
  status lines come from that sync: `member_state_changes` checked against the old and new
  lists (md-d2d.proto), and the name and `user_state` when they differ. Content is stored as it
  comes, whatever our lists say, as the leader ran the receive steps. Member changes made here
  fill `member_state_changes` for the others.
- While we lead, a group message that changes a group is reflected as `GroupSync`, since the
  reflected message itself tells the others nothing (`GroupOutcome.change`, csp.struct.yml's _MD_
  steps): a new group as `Create`, else only what changed. Unlike the spec it isn't reflected in a
  transaction before the change is stored, as the message handling can't wait for the mediator; it
  goes into `d2d_sync_queue` in the same store transaction as the change, so a crash keeps both or
  neither. The spec looks new members up first; here that takes the network, so the entry lists
  only people with a contact, and `group_member_lookups` notes the rest. Once looked up, one
  store transaction stores their contacts and queues `ContactSync.Create` and then a
  `GroupSync.Update` with them in the list (`ADDED` for those who came with a change, not for a new
  group's). No group sync names anyone before their contact has gone out: every contact a lookup stores goes
  out as `ContactSync.Create`, whatever the lookup was for, and a member found only by the retry is
  noted so that the list with them in it follows. A failed lookup is tried
  again on reconnect and hourly. Threema Desktop diffs the lists and uses only the map's `LEFT`
  (`model/group.ts`), so it shows such a late member of a new group as "added"; accepted, as
  the alternative is holding back the new group. `SyncQueued` makes the connection reflect the
  entry before it looks at the next message: Desktop drops a reflected message for a
  group it doesn't have. Migration 35 also marks a one-off: contacts looked up for groups
  before this reached no other device and Desktop drops a member list naming one, so the first
  start while in a device group queues every group-level contact as `ContactSync.Create` (Desktop
  discards those it has). A picture is reflected from the mirror once it is on this device;
  `group_picture_syncs` notes it until then, in the same step as the picture, and it is retried
  the same way; deleting the picture clears the note and the file, and a version 0 sends nothing.
- No periodic group sync: neither the spec (`csp-e2e.proto`) nor Android (6.5.3) has one; Android's
  `synchronizedAt` group field is unused ("might be used for periodic group sync", SE-146). We
  sync only on the spec's triggers (unknown group, sync requests) and once after a Safe restore,
  like Android.

## Polls

- `poll-setup` (0x15 / group 0x52): poll id (u64) + JSON (`d` question, `s` state, `a` multiple,
  `t` announce votes to everyone, `u` display mode, `c` choices `{i, n, o}`; when closing also
  `p` participants and per choice `r` votes per participant or `t` totals). Closing = the same
  poll id with `s: 1`; closed → open is ignored. A close changes the results only, by choice ID (the
  spec's "ignore any other fields"; Android also takes the names): choice texts stay what people
  voted for.
- `poll-vote` (0x16 / 0x53): creator id + poll id + JSON `[[choice id, 0|1], …]` with all
  choices. Sent to everyone if the poll announces votes, else only to its creator.
- Stored as a message of kind `poll` (its state is the body), `polls` table maps
  (conversation, creator, poll id) → message (migration 10). We create polls in list display
  mode. Polls can't be deleted for everyone (spec: user only).
- We now announce GROUP_SUPPORT (0x02) and POLL_SUPPORT (0x04) too.

## Quotes

- A quote is a text message starting with `> quote #<quoted message id, 16 lowercase hex>\n\n`
  (Android `QuoteUtil` "quote v2"); the quoted message is looked up only within the same
  conversation. The core strips the prefix (`text`) and reports `quoteId`; an edit goes out as the
  bare new text, as on Android (which would show a prefix in it as text), and the receiver keeps the
  original's quote. An incoming edit that does carry a prefix (older versions of this app) replaces it.

## Blocking and revoked IDs

- _Identity Blocked Steps_ (`providers::identity_blocked`, Android `IdentityBlockedSteps`):
  special contacts never; the block list; with "block unknown senders" on anyone who is neither
  a direct contact nor in a group we are still in, predefined contacts excepted.
- Receiving: libthreema discards what isn't exempt. group-setup and the requests we would answer
  with a group-sync-request have steps of their own (`groups::receive`, ADR 0019): someone on the
  block list can't bring us into a group we aren't in, and we don't ask them for one we don't
  have. A group we are in changes as usual, and "block unknown senders" plays no part here, as on
  Android. A blocked member's group poll is exempt so that polls stay in step when we send, but it
  is ignored here (Android's `processIncomingGroupMessage`).
- Forward secrecy: someone on the block list gets no session, no Accept and no Reject, their
  envelopes are acknowledged and discarded (`incoming.rs`, ours), unless we share an active group:
  their group control messages are exempt and may be encapsulated. Unknown senders with "block
  unknown senders" on keep a session for the same reason (their group-setup), and what isn't
  exempt is discarded after decrypting. Status lines only appear for contacts who aren't blocked
  either way (`providers::shows_fs_status`).
- Sending (`providers::may_receive`, in `Connection::encode` so nothing gets past it; Android's
  `OutgoingCspMessageHandle`): revoked IDs get nothing at all, blocked ones only what is exempt
  (libthreema's `is_exempt_from_blocking`: group-setup, name, leave, sync request, picture, group
  poll setup and vote, group call start). A group message queued before the block loses them as a
  receiver, a 1:1 message fails. Writing to someone, also through a list, makes them a direct
  contact and tells the other devices (Android's `setAcquaintanceLevelFromLocal`).
- Revoked IDs can't be written to (`contactRevoked`): `activity_state` Invalid, from the daily
  directory check. A list leaves revoked and blocked members out.
- Calls to someone the signalling wouldn't reach are refused before anything is sent
  (`contactRevoked`, `contactBlocked`, `contactUnknown`; Android's `VoipUtil` for blocked ones).

## Keys we trust

- Predefined contacts have the key from libthreema's `Config`; a directory or backup answer with
  another key is refused, the right one means fully verified with the known nickname
  (`contacts::pin_predefined`, in every directory lookup and after a Safe restore).
- Threema Safe: a fully verified contact keeps the backup's key and level whatever the directory
  says (`ThreemaSafeServiceImpl`). Everyone else restores unverified with the directory's key;
  address book matching raises them again.
- Address book match with another key than the stored one: the contact isn't changed except that
  "server verified" drops to unverified (Android `SynchronizeAndroidContactsUseCase`).

## Profile pictures

- 1:1 `set-profile-picture` (0x18: blob id, size, key; JPEG, nonce 00..01, blob persisted),
  `delete-profile-picture` (0x19), `contact-request-profile-picture` (0x1a).
- Distribution steps: before a message that distributes the user profile (text, location, file,
  poll) to a contact (not ECHOECHO / gateway IDs), send our picture once per upload, or a
  removal (at most weekly) if we have none / share with nobody. The upload is reused for 7
  days; changing the picture or the sharing setting, or a request from the contact, resets
  what each contact has.
- Received pictures: downloaded without `done`, stored encrypted (`contact:<ID>`), versioned in
  `contact_pictures` (migration 11). The first message of a contact without a picture triggers
  one `contact-request-profile-picture`, and so does opening the chat with them (the initial
  load, bounded by the request's backoff). Both are ours; Android asks in its application setup
  steps only. A request is a message like any other: it may start a forward secrecy session,
  and the contact may see that. Revoked and blocked IDs aren't asked.
- Who gets it (Android `PROFILEPIC_RELEASE_*`): everyone, nobody, or selected contacts (allow
  list, migration 18; contacts off the list get a removal like "nobody"). Changing the list
  resets only the changed contacts. Threema Safe carries it as `user.profilePicRelease`:
  `["*"]` everyone, `[null]` nobody, otherwise the IDs (Android `ThreemaSafeServiceImpl`); our
  backups now write it and restores apply it.
- Our picture in Threema Safe (`user.profilePic`, base64 JPEG, spec: q=60, ≤ 400×400, no
  metadata): the bridge makes that copy when the picture is set (media store `me-safe`, next to
  the 512 px `me`); pictures set before have no copy and go in as the 512 px JPEG. A restore
  stores it as both and keeps it only if it is a JPEG (Android drops undecodable ones).
- Not yet: user-defined pictures, gateway avatars.

## Keep messages and Clean up (not protocol)

Both go by when a message reached this device, as Android does: an incoming `created_at` is the
sender's clock. The phone's own clock remains a weak spot: set far ahead, it makes everything look
old and a maintenance run deletes too much. Android has the same limit and we leave it; a last-run
stamp written under such a clock doesn't stop later runs. Set far behind when a message came, its
arrival time would later look old: a `created_at` over a day after the arrival, and not in the
future, counts instead, which at worst keeps a message longer.

A pinned chat or one with a draft stays listed when its last message goes. Other 1:1 chats leave the
list; Android keeps every chat as an empty row, but a period chosen for privacy shouldn't leave a
list of who wrote.

Clean up takes private chats only while they are unlocked (it needs no PIN when the app lock is off).
A period already set for Keep messages applies to them whether hidden or not, also a shorter one set
while they are hidden.

## Share target (Sailfish OS, not Threema)

- Sailfish.Share "app share methods": the `.desktop` file lists `X-Share-Methods=threesea` with a
  `[X-Share Method threesea]` section (Description, Capabilities = MIME patterns,
  SupportsMultipleFiles); the share UI calls `org.sailfishos.share.share(a{sv})` on
  `<OrganizationName>.<ApplicationName>` (our D-Bus name) at `/share/threesea`, which a QML
  `ShareProvider { method: "threesea"; registerName: false }` implements. Resources are file
  paths, or `{name, data, type}` maps for text. Passes the Harbour validator.
- Test without Gallery: `gdbus call --session --dest org.threesea.harbour-threesea --object-path
  /share/threesea --method org.sailfishos.share.share "{'resources': <['/path']>}"`.

## Contact verification (QR)

- `3mid:<ID>,<public key hex>[,<expiry>]` (Android `ContactUrlUtil`, `ContactUrlResult`): the
  expiry is epoch seconds, a past one is refused (`qrExpired`), one that isn't a number means
  none. Scanning it: the key must equal the stored (or, for a new contact, the directory's) key
  → verification level _fully verified_; a new contact is added as direct contact.
- Generated with `qrcode`, decoded with `rqrr` (pure Rust); the camera viewfinder is grabbed
  (`VideoOutput.grabToImage`) every 600 ms and decoded as greyscale; a picture works too.
- Every scanned text goes through `qrKind` first (`qr_kind` in `core/src/app.rs`). The contacts
  page's scanner takes any kind, as Android's `AddContactActivity` does. Deliberately stricter
  here: a Threema Web code always asks what it grants before it pairs, and a link to an ID (no
  key: it adds, never verifies) asks before it adds. A refused or declined code stays quiet while
  the camera sees it. The question always says a browser can ask to be remembered: the code's
  permanent bit binds nothing, `keyPersisted` is honoured whatever it said, as on Android.


## Address book matching (opt-in)

Port of Android's `APIConnector.matchIdentities` / `SynchronizeContactsRoutine`:

- Hashes: HMAC-SHA256 with Android's fixed keys (`EMAIL_HMAC_KEY`, `MOBILENO_HMAC_KEY` in
  `APIConnector.java`). E-mail addresses are trimmed and lower-cased; for `gmail.com` /
  `googlemail.com` both domains are hashed. Phone numbers are normalised to E.164 without "+"
  (`core/src/address_book.rs`). Numbers without a country code take the country from Settings →
  Privacy; "Automatic" is the linked number's calling code, else the locale's (ADR 0016).
  Without any country they aren't hashed. Extensions and tones (`;12`, `ext. 12`) go, digits of other
  scripts count. Known gaps against libphonenumber: carrier codes in national or international
  prefixes (Brazil's `015 11 9…`, Korea's and Israel's carrier prefixes for calling abroad).
  Nothing unhashed leaves the device.
- Flow: `identity/match_token` (challenge response with the client key, token cached in the
  store and refreshed when the server rejects it), then `identity/match` with the hash lists.
- New people are looked up together (`identity/fetch_bulk`, a hundred per request). If that fails or
  the directory says too many requests, those found so far are kept and the command fails; the next
  sync gets the rest.
- Hits become contacts (verification level server-verified, acquired directly) named after the
  address book entry if they have no name yet. Such a name is marked `SyncState::Imported` and follows
  the entry at later syncs, as on Android; an entry without a name changes nothing. A name the user
  wrote or cleared is `Custom` and stays. Names from before this was kept (`Initial`) can't be told
  from the user's, so they stay too.
- Only one-to-one matches count, as on Android (`AndroidContactMatcher`): an ID claimed by different
  people's entries is left alone, and so is a person whose entries match several IDs (one of them may
  be a number shared with someone else). Entries with the same name count as one person; an entry
  without a name is its own and says nothing against a named one.
  Whoever the user deleted is on the exclusion list and isn't brought back; adding them by hand
  takes them off it.
- Sailfish side: `org.nemomobile.contacts` PeopleModel in QML (Harbour-allowed, Sailjail
  permission `Contacts`), only when the user enables it in Settings; then at most daily.

## Threema links

Port of Android's link handling (`AndroidManifest.xml` intent filters, `AppLinksActivity`,
`RecipientListBaseActivity`, `AddContactActivity`, `EnterSerialActivity`; hosts from
`app/build.gradle.kts`: `actionUrl` = `go.threema.ch`, `contactActionUrl` = `threema.id`):

| Link | Action |
|---|---|
| `https://threema.id/<ID>[?text=…]` | add contact; open the chat with the text, else the contact |
| `threema://compose?id=&text=`, `https://go.threema.ch/compose?…`, `https://threema.id/compose?…` | write (no `id`: choose the chat) |
| `threema://add?id=`, `https://go.threema.ch/add?id=` | add contact |
| `threema://license?key=`, `https://go.threema.ch/license?key=` | license key (setup only) |
| `threema://link_mobileno?code=` | the code of a pending phone link, digits only and at most 8 (`VerifyMobileNumberUseCase`); only from outside the app, never from a chat message |

Parsing is `core/src/links.rs`. Sailfish side: the `.desktop` declares
`x-scheme-handler/threema`, `x-url-handler/threema.id` and `x-url-handler/go.threema.ch`;
libcontentaction (`src/mime.cpp`: exact host, no `www.`) then calls our `X-Maemo-Method`
`activate` with the URL as `as`. Links tapped in messages go through the same parser; others
open with `Qt.openUrlExternally`.

## Mentions

Group texts and captions mention people as `@[<ID>]`, everyone as `@[@@@@@@@@]` (Android
`ConversationTextAnalyzer`, regex `@\[([0-9A-Z*@]{8})]`; `ContactService.ALL_USERS_PLACEHOLDER_ID`).
The text is stored and sent raw. The core adds the mentioned contacts' names to the message JSON
(`core/src/mentions.rs`); the UI shows "@Name", "@Me" and "@All" (Android's wording) in bubbles,
previews and notifications. Typing "@" in a group offers the members (and "All") above the input;
the input shows "@Name" and sending turns it back into `@[ID]`.

## Per-chat notifications

Like Android's `NotificationsActivity` (D2D sync `NotificationTriggerPolicyOverride`): a chat
is muted (`never`, optionally until a time; Android offers 1/2/4/8/24/144 h, we offer 1 h,
8 h, 1 day, 1 week and always) or, for groups, notifies only when we or everyone are mentioned
(`mentioned`). Local only (no multi-device yet): table `chat_notifications` (migration 12),
`core/src/chat_notifications.rs`; the core marks each `messageAdded` event with `notify`.

## Locations

`location` messages (libthreema `LocationMessage`: latitude, longitude, accuracy, name,
address) are received into the store as JSON and sent with `sendLocation`. On the wire there is
one line after the coordinates for a place given by a single text, and decoders read it as the
address, so a name alone goes out as the address (libthreema's encoder would drop it). The position comes
from QtPositioning (Sailjail permission `Location`, Harbour-allowed). No map preview and no
reverse geocoding: both need a tile or geocoding server outside Threema (§7.6). Received
locations open as `geo:` links in an installed maps app (e.g. Pure Maps); the bubble shows the
name, address and coordinates.

## Distribution lists

Like Android (`DistributionListMessageReceiver`): nothing in the protocol, only local. A list is
a chat of its own (key `d:<16 hex>`, table `distribution_lists`, migration 14); each message
sent there is recorded in the list's chat (as sent) and sent to every member that is a contact
and not blocked as a normal 1:1 message, with its own message id, which also shows in the
member's chat. Files are uploaded per member (forwarding path); quotes, reactions, edits,
deletions for everyone and polls are not offered, as on Android.

## ID revocation password
`identity/set_revocation_key` / `check_revocation_key` (Android `APIConnector`), both with the
directory's challenge-response. The key is base64 of the first 4 bytes of SHA-256 over the UTF-8
password; the password stays on the device. Revoking itself happens on myid.threema.ch/revoke,
not in the app (Android does the same). `lastChanged` is `yyyy-MM-ddTHH:mm:ss±HHMM`.

## Identity links (e-mail, phone number)
`identity/link_email` and `identity/link_mobileno` (Android `APIConnector.linkEmail` /
`linkMobileNo`): phase 1 sends `{identity, email | mobileNo, lang}` (phone also `httpsUrl: true`);
the answer has `linked: true` if it is already linked to this ID, otherwise the usual challenge
for phase 2. An empty address or number unlinks. E-mail is confirmed by a link in the mail;
whether that happened is phase 1 again without answering the challenge (`linked`). Phone: phase 2
returns `verificationId`; `{verificationId, code}` to `link_mobileno` confirms,
`link_mobileno_call` has the code read out by phone. Numbers are E.164 without `+`, and the user must
enter them with their country code; the dialog shows the result first (ADR 0016).
These are the only requests that send a plain address/number (CLAUDE.md §7.4 exception).

## Linking and removing devices: what counts as done (ADR 0017)

- Removing: inside a `DROP_DEVICE` transaction read the devices (`GetDevicesInfo`), drop only those
  listed, each confirmed by its `DropDeviceAck`. If no other device remains, drop this one last; the
  mediator may answer with close code 4113 instead of an ack, which counts as done. Nothing to
  commit then: the connection is gone anyway. Empty id list = Android's "deactivate if alone".
- Joining: `Registered` means the new device has all data and is registered on the mediator, so
  the list has it from then on. Limits: 90 s to begin the transaction, 5 minutes for the transfer
  (60 s per 1 MiB chunk), 3 minutes for `Registered`, 5 s to close the link, 30 s for the commit.
  A commit failing after `Registered` restarts the connection, which releases the lock.
