# Research: the phone side of Threema Web

Status: research notes, 2026-10-26. Only public sources were read. Each claim gives its source.
Abbreviations used for sources:

- **[AND]** `third_party/threema-android/app/src/main/java/ch/threema/app/webclient/` (6.5.3-1216);
  **[AND-app]** `third_party/threema-android/app/src/main/java/ch/threema/` (outside the webclient package)
- **[GRADLE]** `third_party/threema-android/gradle/libs.versions.toml`, `app/build.gradle.kts`
- **[WEB]** threema-web, branch `master`, commit `f755cff` (2026-10-25), version 2.6.5:
  <https://github.com/threema-ch/threema-web> — mainly `src/services/webclient.ts`, `src/services/push.ts`,
  `src/userconfig.js`, `docs/qr_code.md`
- **[ARP]** Threema's App Remote Protocol spec (the messages between app and Threema Web), version 2.2.2:
  <https://github.com/threema-ch/app-remote-protocol> `schema/v2.yaml` (commit `335d8ce`, 2024-04-16).
  It is the authoritative message reference; this note only summarises it.
- **[SRTC]** <https://github.com/saltyrtc/saltyrtc-meta>: `Protocol.md`, `Task-RelayedData.md`,
  `Task-WebRTC.md`, `Chunking.md`
- **[CSP]** `third_party/libthreema/threema-protocols/src/csp.struct.yml` (vendored, matches 6.5.3)

---

## 0. Short answers

| Question | Answer |
|---|---|
| Does the phone need WebRTC? | **No, not if we choose the relayed-data task.** Threema Web always offers `v0.relayed-data.tasks.saltyrtc.org` besides the two WebRTC tasks ([WEB] `webclient.ts:509-529`), and it handles that task fully (it is what the iOS app uses). Android only offers WebRTC ([AND] `services/instance/state/SessionConnectionContext.java:110-119`). A phone that offers only relayed data gets relayed data. All traffic then goes over one WebSocket to `saltyrtc-XX.threema.ch`. |
| Who is initiator? | The browser is the initiator, the phone is the responder ([WEB] `webclient.ts:554` `asInitiator()`, [AND] `SessionConnectionContext.java:134` `asResponder()`). |
| How is the phone woken? | The browser calls `https://push-web.threema.ch/push`. For Threema Libre (no FCM) the push token is `threema-gateway;<ID>;<pubkey>`, so the push relay sends a `web-session-resume` (0xfe) CSP message from `*3MAPUSH` through the normal chat server. libthreema already decodes it, and our core has a hook for it. |
| Non-Threema endpoints? | None with relayed data. With WebRTC, the TURN servers are Threema's too. A **self-hosted** Threema Web puts any host into the QR code. We must refuse or ask about such hosts (§7 rule 6). |
| Effort | Android: 133 files, about 18,000 lines. Without UI and WebRTC, about 13,000 lines, most of them a thin mapping onto the app's services. See §7. |

---

## 1. Pairing and session set-up

### 1.1 QR code

The browser shows the code. The Android app scans it, Base64-decodes it and parses it ([AND] `activities/SessionsActivity.java:735-738`,
`services/WebSessionQRCodeParserImpl.java`; spec [WEB] `docs/qr_code.md`, builder [WEB] `src/services/qrcode.ts:39-76`).

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | Protocol version, u16 BE. It must equal `PROTOCOL_VERSION = 2` ([AND] `Protocol.java:9`, check in `SessionsActivity.java:761-787`) |
| 2 | 1 | Options: bit 0 = self-hosted (only changes error texts), bit 1 = permanent (the user set a password in the browser) |
| 3 | 32 | Initiator (browser) permanent public key |
| 35 | 32 | SaltyRTC auth token (NaCl secret key, valid for the one 'token' message) |
| 67 | 32 | SaltyRTC server permanent public key. All zero means none |
| 99 | 2 | SaltyRTC port, u16 BE |
| 101 | n | SaltyRTC host, UTF-8, no terminator |

The QR string is plain Base64. A QR that starts with the multi-device join prefix is rejected with a hint
([AND] `SessionsActivity.java:741-750`). The same payload can also arrive by intent (`processPayload`, `SessionsActivity.java:475-484`). Android asks the user before starting a session from an intent payload (`:465`).

### 1.2 Signalling server

- Host: `saltyrtc-{prefix}.threema.ch`, where `{prefix}` is the first byte of the **initiator's** public key in lowercase hex. Port 443 ([WEB] `userconfig.js:13`, substitution `webclient.ts:536-541`). The phone does not build this name. It uses host and port from the QR code ([AND] `SessionConnectionContext.java:122`). An OPPF can override them ([AND] `SessionsActivity.java:870-881`, OnPrem only).
- Server key: `b1337fc8402f7db8ea639e05ed05d65463e24809792f91eca29e88101b4a2171` ([WEB] `userconfig.js:19`). It is in the QR code, and the phone passes it to `withServerKey` ([AND] `SessionConnectionContext.java:125-127`). With it, SaltyRTC checks the server's signed 'server-auth' ([SRTC] Protocol.md "Server's Permanent Key").
- URL: `wss://<host>:<port>/<initiator public key, 64 lowercase hex chars>`, WebSocket subprotocol `v1.saltyrtc.org` ([SRTC] Protocol.md "Subprotocol", "Connecting To a Signalling Server").
- Pings: Android uses a 60 s WebSocket ping, the browser 30 s. Connect timeout 5 s ([AND] `SessionConnectionContext.java:73-77,132-133`; [WEB] `webclient.ts:550`).
- IPv4/IPv6: dual stack, or IPv4 only if the user disabled WebRTC IPv6 ([AND] `services/instance/SessionInstanceServiceImpl.java:662-673`).
- TLS: `SSLUtil.getSSLSocketFactory(host)` is the platform default factory, with explicit pinning only for OnPrem ([AND-app] `app/utils/SSLUtil.kt`). The platform trust manager still applies `network_security_config.xml`, which pins `threema.ch` with `includeSubdomains="true"` (`app/src/main/res/xml/network_security_config.xml:8-22`). So `saltyrtc-*.threema.ch` is in effect pinned to the `threema.ch` pin set. Our `core/src/https.rs` already matches subdomains of `threema.ch` against that set, so reusing `https::tls_config()` gives the same behaviour.

### 1.3 SaltyRTC roles and keys

[SRTC] Protocol.md sections "Client Handshake", "'token' Message", "'key' Message" and "'auth' Message" describe the handshake:

1. Client↔server: `server-hello` → `client-hello`/`client-auth` → `server-auth`. The responder sends `client-hello` with its permanent public key, and the server assigns it an address 0x02..0xff.
2. **New pairing.** The responder sends a `token` message, encrypted with the QR auth token (secret box). It carries the responder's **permanent** public key. Then both sides send `key` messages with fresh **session** public keys, encrypted with the permanent keys. Then they send `auth` messages, encrypted with the session keys. The responder's `auth` lists its tasks, the initiator's `auth` names the chosen task.
3. **Trusted reconnect.** If both sides already trust each other's permanent keys, there is no auth token and no `token` message. Each side checks the other's permanent key directly. If only one side trusts the other, the initiator answers with `drop-responder` 3005 ([SRTC] "Client Handshake").

Keys on the phone ([AND] `SessionInstanceServiceImpl.java:603-656`, model `storage/models/WebClientSessionModel.java`):

| Key | Where | Lifetime |
|---|---|---|
| Responder (phone) permanent key pair | `new KeyStore(cryptoProvider)` on `start`. The private key is saved in `WebClientSessionModel.privateKey` (column `private_key`) | per session, persistent |
| Initiator (browser) permanent public key | `model.key` (column `key`), plus `key256` = hex SHA-256 of it. Pushes use `key256` to find the session ([AND] `services/SessionServiceImpl.java:251-256`) | per session |
| Auth token | Only used in `initiatorInfo(permanentKey, authToken)` on first start (`SessionInstanceServiceImpl.java:623-625`). Not stored | one handshake |
| Session keys | Generated by the SaltyRTC library for each connection | one connection |

Resuming a saved session builds a new SaltyRTC client with `withTrustedPeerKey(model.key)` and `new KeyStore(cryptoProvider, model.privateKey)` (`SessionInstanceServiceImpl.java:651-653`). No new QR scan is needed.

On the browser side, `request/keyPersisted` tells the phone that the browser has stored the trusted key, encrypted with the user's password in `localStorage` ([WEB] `src/services/keystore.ts` `TrustedKeyStoreService`). Android then sets `model.isPersistent = true` ([AND] `services/instance/message/receiver/KeyPersistedRequestHandler.java`, `SessionServiceImpl.java:263-272`). Non-persistent sessions are deleted when they end ([AND] `SessionServiceImpl.java:321-330`).

Other fields Android stores per session: `created`, `last_connection`, `client` (description from the browser's user agent, [AND] `usecases/DetectBrowserUseCase.kt`), `label`, `self_hosted`, `protocol_version`, `salty_host`, `salty_port`, `server_key`, `push_token`, `state`.

---

## 2. Tasks: WebRTC or relayed data

### 2.1 What each side offers

| Side | Tasks offered, in order | Source |
|---|---|---|
| Browser (initiator) | `v1.webrtc.tasks.saltyrtc.org`, `v0.webrtc.tasks.saltyrtc.org` (only if `supportsWebrtcTask()`), then **always** `v0.relayed-data.tasks.saltyrtc.org` | [WEB] `webclient.ts:509-529`. `browser_info.ts:81-91` returns false only for mobile Safari and Firefox/Chrome on iOS |
| Android (responder) | `v1.webrtc…`, `v0.webrtc…` with handover | [AND] `SessionConnectionContext.java:110-119` |
| iOS | `v0.relayed-data.tasks.saltyrtc.org` | [ARP] `concepts.procedures`: "Android: v0.webrtc…; iOS: v0.relayed-data…" |

The initiator "MUST choose the first task in its own list of supported tasks that is also contained in the list of supported tasks provided by the responder" ([SRTC] Protocol.md:1175-1181). So:

- If the phone offers only relayed data, the browser picks relayed data. Nothing in [WEB] ties relayed data to iOS. `onTaskEstablished` decides only by the task name (`webclient.ts:889-896`). The relayed-data path is fully implemented: data handler `:1136-1144`, sending with chunk cache `:4019-4056`, `sendChunk` `:4060-4111`, resumption `:724-790`.
- If we offered WebRTC as well, every desktop browser would pick WebRTC. So **offer relayed data only**.

Behaviour that depends on the **task**, not on the OS. It is the "iOS-like" protocol variant, and we must implement it:

- Full `id`/`ack` wire-message acknowledgement. `requiresTemporaryIdBackwardsCompatibility` is false for relayed data (`webclient.ts:433-435`), so the initial requests are sent with an `id` and wait for an `ack` (§4.3).
- Chunk cache, `connectionAck` and session resumption (§5).
- The unread count survives disconnects (`webclient.ts:1358-1361`), and a peer disconnect only shows "reconnecting" (`:1152-1170`).

Behaviour that depends on **`clientInfo.os`**: push token parsing. Only for `os = "android"` does the browser parse the `threema-gateway;` prefix. `"ios"` always means APNs (`webclient.ts:3355-3378`). So we send `os: "android"` as a protocol setting (`appVersion` names Threesea; the value goes only to the user's own browser, inside the encrypted session) (see §3). Other OS branches are cosmetic: audio MIME check `:1833` and message-id logging `:4348`.

**Uncertain:** relayed data with `os: "android"` is a combination official Android apps do not use, so it had no test coverage. The code reading above finds nothing that breaks it. Test it early (step 1 in §7).

### 2.2 Relayed-data transport

- The task has no task data in `auth` (`Nil`). After the handshake, each payload is a SaltyRTC `{"type":"data","p":<payload>}` message, encrypted with the session keys (NaCl box) and relayed by the server ([SRTC] Task-RelayedData.md). `application` messages are forbidden in Threema's use ([ARP] `concepts.procedures` "Maintaining Chunk Order"). A `close` ends the session.
- `p` is a **binary chunk** ([WEB] `webclient.ts:4103` `relayedDataTask.sendMessage(chunk.buffer)`, `:1138` receive).
- Chunking: [SRTC] Chunking.md **unreliable/unordered** mode. Each chunk has a 9-byte header: options byte (`0x00`, or `0x01` on the last chunk), u32 BE message id, u32 BE serial number. Chunk size 64 KiB ([WEB] `webclient.ts:100`, `:4031-4038`). The browser uses the outgoing *message sequence number* as the chunk message id. Chunks hold **plain msgpack**. There is no second encryption layer here, because SaltyRTC already encrypts each `data` message.

### 2.3 For comparison: the WebRTC task that Android uses

- After the task is chosen, Android creates a `PeerConnection` ([AND] `webrtc/PeerConnectionWrapper.java`). The browser sends the offer and the phone answers. ICE candidates go through the SaltyRTC task. Then the signalling moves to a negotiated data channel ("handover", `SessionConnectionContext.java:333-515`).
- The browser opens the data channel `THREEMA` ([WEB] `webclient.ts:176,1081`). Messages are **encrypted then chunked**: `task.createCryptoContext(dc.id)`, then the unreliable/unordered chunker, max 256 KiB ([AND] `webrtc/DataChannelContext.java:43,92-107,136-162`; [WEB] `webclient.ts:1086-1122`).
- ICE: Android takes TURN credentials from the directory, `POST identity/turn_cred` with type `"web"`, cached via `TurnServerCache("web", 6 h)` ([AND] `Config.java`, `PeerConnectionWrapper.java:98-116`; `domain/.../APIConnector.java:997-1002`). The browser uses `turn(s):ds-turn-{prefix}.threema.ch:443` with static credentials ([WEB] `userconfig.js:33-41`). All hosts are Threema's.
- For us, this would mean a second webrtcbin data-channel stack (Chum only), DTLS/SCTP, and the handover logic. Relayed data avoids all of it and also works in the Harbour flavour.

Cost of relayed data: everything, including blobs, goes through Threema's relay instead of peer-to-peer. That is the same as on iOS. The SaltyRTC end-to-end encryption still applies.

---

## 3. Wake-up and push

1. On every `response/clientInfo` the phone sends a `pushToken` ([AND] `converter/ClientInfo.java:101-111`):
   - FCM token as is, or `hms;<token>`, or
   - **with no push token (Libre): `threema-gateway;<own Threema ID>;<own public key hex>`**.
2. The browser removes the prefix and sets the type to `threema-gateway` ([WEB] `webclient.ts:3357-3372`). It stores the token with the trusted key ([WEB] `keystore.ts:103-141`).
3. To wake the phone, the browser POSTs form data to `https://push-web.threema.ch/push` ([WEB] `userconfig.js:48`, `push.ts:153-220`): `type=threema-gateway`, `session=<hex SHA-256 of the initiator public key>`, `version=2`, `affiliation`, `identity`, `public_key`, `ttl`, `collapse_key`.
4. The push relay (Threema's server, not public) sends a CSP message of type `0xfe web-session-resume` from `*3MAPUSH`. Its payload is JSON `{"wcs": <session hash>, "wca": <affiliation>, "wct": <unix s>, "wcv": <protocol version>}`. The message is exempt from blocking, has short-lived server queuing, and is not reflected ([CSP] `csp.struct.yml:5058-5103`; [AND-app] `processors/IncomingMessageTask.kt:188-201` → `processors/push/IncomingWebSessionResumeMessageTask.kt` → `utils/PushUtil.java:63-66,136-143,251-280`).
5. `SessionWakeUpServiceImpl.resume(sha256, version, affiliation)` finds the session by `key256` and calls `resume()`. If that cannot happen now (no network, master key locked), the wakeup is queued for 60 s, or 20 s after a disconnect ([AND] `services/SessionWakeUpServiceImpl.java:51-52,98-170,274-360`).

Status in our tree: libthreema decodes this message (`third_party/libthreema/lib/src/model/message.rs:348` `WebSessionResumeMessage { push_payload }`). Our core sees it in `core/src/providers.rs:196-205` (`NoShortcuts::handle_web_session_resume`, which ignores it today). Since our chat-server connection stays up in the background (Chum), the wakeup needs **no** FCM. It is the same path Android Libre uses.

Without push: SaltyRTC needs both peers on the server at the same time. The browser waits as initiator on its path, so the phone can connect as responder whenever it likes, for example when the user opens the session or when the app starts with a saved session. [ARP] `concepts.sessionResumption` also lists "Triggered by App": a session is active but disconnected, and an outgoing message is queued. I did not trace whether Android implements this trigger. A session with push disabled still works while both sides stay connected. The browser also re-sends pushes while it is waiting for acks or after 3 s without incoming data ([WEB] `webclient.ts:833-875`, `:4066-4108`).

---

## 4. Application protocol (ARP)

### 4.1 Envelope

Each message is one msgpack **map** ([AND] `Protocol.java`; [WEB] `threema.WireMessage`; [ARP]):

| Field | Type | Meaning |
|---|---|---|
| `type` | str | `request`, `response`, `update`, `create`, `delete` |
| `subType` | str | see §4.2 |
| `args` | map, optional | small parameters (receiver `type`/`id`, `messageId`, `mode`, `refMsgId`, …) |
| `data` | any, optional | payload (lists of conversations, message bodies, blobs as `bin`) |
| `id` | str, optional | the sender requests an acknowledgement ([ARP] `concepts.ack`) |
| `ack` | map, optional | `{id, success, error?}` confirming an earlier `id` |

Legacy: when a browser on the WebRTC task sends `args.temporaryId`, Android echoes it in `args.temporaryId`/`success`/`error` ([AND] `services/instance/MessageHandler.java:46-80`). The browser translates that into `ack` ([WEB] `webclient.ts:3793-3812`). We should send the real `ack` field. Echoing `temporaryId` as well costs nothing.

Error codes: [AND] `Protocol.java` `ERROR_*` (`internalError`, `badRequest`, `invalidIdentity`, `invalidContact`, `invalidGroup`, `invalidConversation`, `valueTooLong`, `blobDownloadFailed`, `blobDecryptFailed`, …). Image limits: thumbnails JPEG ≤ 350 px, high-res avatars PNG ≤ 512 px, low-res avatars ≤ 48 px (`Protocol.java:98-114`).

### 4.2 Message types

Direction is "→app" (browser to phone), "app→" or both. **M** = needed for a minimal useful session (list chats, read messages, send text, receive updates). **L** = later. All entries are from [ARP] `messages`. The Android handler is under [AND] `services/instance/message/{receiver,updater}/`.

**Connection and session**

| Message | Dir | Meaning | |
|---|---|---|---|
| `update/connectionInfo` | both | first message of every connection: connection ID and optional `resume {id, sequenceNumber}` | M |
| `request/connectionAck` | both | "send me your incoming chunk counter now" | M |
| `update/connectionAck` | both | `{sequenceNumber}` = number of chunks received. The peer prunes its chunk cache | M |
| `update/connectionDisconnect` | both | the sender is about to close: `{reason: stop/delete/disable/replace/…}` ([AND] `converter/ConnectionDisconnect.java`) | M |
| `request/clientInfo` → `response/clientInfo` | →app / app→ | browser UA → device, `os`, `osVersion`, `appVersion`, `isWork`, `pushToken`, `configuration`, `capabilities` ([AND] `converter/ClientInfo.java`). A critical request: the session fails without it | M |
| `request/keyPersisted` | →app | the browser stored the trusted key, so the session becomes persistent. No reply | M |
| `update/confirm` | app→ | error confirmation of an `id` (or success where no other reply is defined) | M |
| `update/alert` | app→ | show an error or warning in the browser | L |
| `request/batteryStatus` (deprecated), `update/batteryStatus` | →app / app→ | battery percent and charging. The app subscribes automatically after the handshake | L |
| `update/voipStatus` | app→ | call ringing/started/finished… (auto-subscribed) | L |

**Profile**

| Message | Dir | Meaning | |
|---|---|---|---|
| `request/profile` → `response/profile` | | own ID, public key, nickname, avatar. Subscribes to updates | M |
| `update/profile` | both | change nickname/avatar (→app); profile changed (app→) | L |

**Receivers (contacts, groups, distribution lists)**

| Message | Dir | Meaning | |
|---|---|---|---|
| `request/receivers` → `response/receivers` | | all contacts/groups/lists (`converter/Receiver.java`, `Contact.java`, `Group.java`, `DistributionList.java`). Subscribes | M |
| `update/receiver` | app→ | one receiver new, modified or removed (`mode`) | M |
| `update/receivers` | app→ | whole receiver list of one type replaced | M |
| `request/contactDetail` → `response/contactDetail` | | system-contact details (e-mails, phones) | L |
| `create/contact` | both | add a contact by ID / confirmation | L |
| `update/contact` | both | edit first/last name, avatar | L |
| `create/group`, `update/group`, `delete/group` (`deleteType` leave/delete) | both | group management | L |
| `request/groupSync` | →app | force a group sync | L |
| `create/`, `update/`, `delete/distributionList` | both | distribution lists | L |

**Conversations and messages**

| Message | Dir | Meaning | |
|---|---|---|---|
| `request/conversations` → `response/conversations` | | conversation list (`converter/Conversation.java`: type, id, position, messageCount, unreadCount, latestMessage, isStarred, notifications…), optional `maxSize` | M |
| `update/conversation` | both | conversation changed (app→); star or archive (→app) | M (app→), L (→app) |
| `request/messages` → `response/messages` | | page of messages for a receiver. `refMsgId` pages backwards, `more` says if older ones exist. Subscribes the conversation | M |
| `update/messages` | app→ | messages new, modified or removed (`mode`) in a subscribed conversation | M |
| `update/activeConversation` | →app | which chat is open in the browser (Android uses it for notifications) | L |
| `request/read` | →app | mark messages up to `messageId` as read (send read receipts) | M |
| `update/typing` | both | typing indicator (contact→browser; user typing→contact) | L (cheap) |
| `create/textMessage` | both | send text (+ optional quote). The reply carries the new `messageId` | M |
| `create/fileMessage` | both | send a file/image/voice (blob bytes inside `data`) | L |
| `delete/message` | →app | delete a message locally | L |
| `delete/cleanReceiverConversation` | →app | empty a chat | L |
| `request/ack` | →app | agree/decline a message. Android maps this to 👍/👎 emoji reactions ([AND] `AcknowledgeRequestHandler.java`) | L |
| `request/thumbnail` → `response/thumbnail` | | JPEG thumbnail of a media message | L |
| `request/blob` → `response/blob` | | full file of a message (the phone downloads and decrypts it from the blob server) | L |
| `request/avatar` → `response/avatar`, `update/avatar` | | avatar of a receiver, low- or high-res | L (first media step) |

Android dispatcher wiring: [AND] `SessionInstanceServiceImpl.java` constructor (about lines 160-560). It ignores `request/batteryStatus` and `update/connectionInfo` (`IgnoreRequestHandler`, `ConnectionInfoUpdateHandler`).

### 4.3 Acknowledgement and subscription rules

- If a message has an `id`, the peer must confirm it after processing, with `ack: {id, success, error?}`. On success the confirmation is part of the defined reply (for example `response/conversations`). On error it is `update/confirm` ([ARP] `concepts.ack`). If the phone was disconnected while an `id` was still pending, the browser keeps waking it until the `ack` arrives.
- `request/receivers|conversations|messages|profile` subscribe to the matching `update/*` *after* the reply is sent ([ARP] `concepts.subscription`). The browser auto-subscribes to `update/typing`. The app auto-subscribes the browser to `update/batteryStatus` and `update/voipStatus` ([ARP] `concepts.procedures`).

---

## 5. Minimal handshake once the channel is up

Sequence for relayed data, from [WEB] `onHandover` (`webclient.ts:1065-1145`), `onConnectionEstablished` (`:935-1050`) and [ARP] `concepts.connectionId/sequenceNumber/chunkCache`:

1. Both sides derive the **connection ID**: NaCl box of an empty plaintext with the SaltyRTC **session** keys (own session secret key, peer's session public key) and the 24-byte nonce `"connectionidconnectionid"`. The result is the 16-byte MAC ([WEB] `webclient.ts:106,1071-1073`; [ARP] `concepts.connectionId`). Both sides compute the same value.
2. Both sides send `update/connectionInfo {id, resume?: {id: previousId, sequenceNumber: incomingChunksOfPreviousConnection}}` **before anything else**. The browser fails the session if the first message it receives is anything else (`webclient.ts:4188-4202`).
   - Android sends a fixed fake ID `01..08` ×4 and no `resume` ([AND] `converter/ConnectionInfo.java`). The browser detects that and never resumes, which means a "full reconnect" every time (`webclient.ts:724-731`). That is a valid minimal implementation. If our real ID did not match the browser's, the browser would throw and fail the session (`:732-736`). So either derive the ID correctly or send the fake one.
3. If both sides request `resume` with matching previous IDs, each side prunes its chunk cache up to the peer's `sequenceNumber`, remaps the remaining chunks onto the new connection and re-sends them. Otherwise both discard their caches ([ARP] `concepts.sequenceNumber`).
4. Without resumption, the browser sends `request/clientInfo`, `request/profile`, `request/receivers`, `request/conversations` and `request/batteryStatus` (`webclient.ts:2335-2346`). The UI starts when the first four have been answered (`:996-1026`).
5. Steady state:
   - Each side counts every chunk it sends and receives (u32, starting at 0). It caches all outgoing chunks. Chunks of the blacklisted types (`connectionAck`, `connectionInfo`, `connectionDisconnect`, `batteryStatus`, `typing`) are cached only as placeholders.
   - Each side sends `update/connectionAck` 10 s after receiving a chunk, and at once when it gets `request/connectionAck`. The browser asks once its cache passes 2 MiB (`webclient.ts:102,4039-4050`).
   - Message IDs inside the chunker are the per-session outgoing **message** sequence number. It is not reset on a resumed connection (`:798-806`).

Protocol version: QR `version` and `wcv` in the push must be 2 ([AND] `Protocol.java:9`). The browser checks app versions only for display.

---

## 6. Security and our rules (CLAUDE.md §7)

| Topic | Finding | Consequence for us |
|---|---|---|
| Key material stored | Phone permanent private key and browser public key per session ([AND] `WebClientSessionModel`) | store them in SQLCipher, zeroize after use, never log them (§7 rules 2–3) |
| Session authorisation | The user scans the QR code, which proves possession of the auth token. Later wakeups resume **without asking**, as long as Threema Web is enabled and the session exists. The browser needs the user's password to unlock its trusted key. Android shows a persistent notification while a session runs ([AND] `services/SessionAndroidService.java`) | same model. Sailfish: settings page with sessions list, enable switch and a notification while active |
| Who sees what | Signalling server: initiator path (browser public key), IPs, sizes and timing. Content is NaCl-boxed end-to-end with session keys | with relayed data all content passes the relay, still end-to-end encrypted |
| Servers contacted by the phone | `saltyrtc-XX.threema.ch:443` (WSS). Through the existing CSP connection: the `*3MAPUSH` messages. With WebRTC only: `ds-turn-*.threema.ch` / the TURN servers from `identity/turn_cred` | Threema only. Keep `https::tls_config()` pinning on the WSS. The SaltyRTC server key from the QR gives a second check |
| Self-hosted Threema Web | The QR may carry **any host** (`self-hosted` flag, `SALTYRTC_QR_HOST`, [WEB] `userconfig.js:20-25`). Android only restricts this by MDM (`isWebHostAllowed`) | **ask the user / decide** (§10 "ask, don't guess" on non-Threema servers): accept only `*.threema.ch` with the known server key, or warn before connecting elsewhere |
| Pushes | The browser sends our ID and public key (not secret) to `push-web.threema.ch`. Threema already knows them | none |
| Data exposure | The browser gets every chat, contact and file the phone has, including thumbnails and blobs on request | hide private chats (ADR 0009) from Threema Web unless the app is unlocked. Android: open question, I did not check how it filters hidden chats |
| Plaintext on the phone | Message bodies go msgpack → SaltyRTC box in memory only | handled in `core` (§7 rule 1). Nothing persisted except the session row |

---

## 7. Size and suggested steps

### 7.1 Android code size

`find … -type f | xargs wc -l` over [AND]: **133 files, 18,069 lines** (Java/Kotlin, including licence headers).

| Part | Files | Lines | Port? |
|---|---|---|---|
| `activities/`, `adapters/` (session list, diagnostics UI) | 6 | 2,421 | QML rewrite (small) |
| `webrtc/` + `app/webrtc/` flow control | 4 + 3 | 999 + 337 | **not needed** with relayed data |
| `services/instance/state/` (connection state machine, SaltyRTC set-up) | 9 | 1,513 | rewrite (relayed data, simpler) |
| `services/` top (session service, wakeups, QR parser, Android services) | 14 | 2,040 | partly |
| `services/instance/` top (dispatcher, handler base, `SessionInstanceServiceImpl`) | 5 | 1,509 | yes |
| `services/instance/message/receiver/` (→app handlers) | 32 | 4,337 | yes, in steps |
| `services/instance/message/updater/` (app→ updates) | 10 | 1,335 | yes |
| `converter/` (model → msgpack) | 26 | 2,934 | yes |

Not in Android's tree: the SaltyRTC client itself (`org.saltyrtc:saltyrtc-client` 0.14.2, `saltyrtc-task-webrtc` 0.18.1, `chunked-dc` 1.0.1) and `msgpack-core` 0.9.12 ([GRADLE] `libs.versions.toml:37,66,73-74`). We need our own:

- **SaltyRTC responder + relayed-data task + chunker in Rust (`core`).** References: `saltyrtc-client-rs` 0.9.0 (MIT/Apache, about 7,200 lines, protocol state machine in `src/protocol/`, about 4,400 lines) and `saltyrtc-task-relayed-data-rs` 0.2.0 (has a C FFI, probably what iOS uses). Both are in maintenance mode and built on tokio-core 0.1 / futures 0.1, so they are references and not dependencies. A sans-I/O responder-only implementation should be about 1,000–1,500 lines. It needs NaCl box/secretbox (libthreema's `crypto_box`/`crypto_secretbox`, already in the tree), msgpack, and `tokio-tungstenite` (already in the tree).
- msgpack: a new dependency (e.g. `rmpv`/`rmp-serde`). Justify it in the commit (§7 rule 7).

Estimate: core about 4,000–6,000 lines of Rust (transport about 1,500, ARP and converters about 2,500, handlers mapping onto existing `app_*` functions), bridge/QML about 600 lines.

### 7.2 Steps

1. **Pairing + SaltyRTC + relayed data + hello.** QR parse (camera scan exists: `rqrr`), WSS to the QR host with pinning, SaltyRTC responder (server handshake, token/key/auth, offer only `v0.relayed-data`), chunker/unchunker, `update/connectionInfo` (fake ID first), `request/clientInfo` with `os:"android"` and the `threema-gateway` token, `ack`/`update/confirm`, `connectionAck`, `connectionDisconnect`. Store the session (keys in SQLCipher). Settings → "Threema Web" page with scan, list and remove. **Goal: the browser passes "connecting" against web.threema.ch.** This also tests the relayed-data + `android` combination (§2.1).
2. **Read-only.** `profile`, `receivers`, `conversations`, `messages` (paging), subscriptions and `update/receiver(s)`, `update/conversation`, `update/messages` driven by the core's existing events. `request/read`. `keyPersisted` + trusted-key resume. Handle `web-session-resume` from `*3MAPUSH` in `providers.rs`.
3. **Sending.** `create/textMessage` (with quote), `update/typing` both ways, `update/activeConversation`, `delete/message`, `delete/cleanReceiverConversation`, `update/conversation` (star/archive), `request/ack` → reactions.
4. **Media and avatars.** `request/avatar` / `update/avatar`, `request/thumbnail`, `request/blob`, `create/fileMessage` (64 KiB chunks over the relay, so watch memory and the 2 MiB ack trigger).
5. **Everything else.** Contacts/groups/distribution lists create/update/delete, `groupSync`, `contactDetail`, `profile` edits, `batteryStatus`, `voipStatus`, `alert`, real connection-ID derivation and chunk-cache **resumption** (wakeup without a full reload), private-chat filtering, diagnostics.

Open points to decide or test:

- Flavours: relayed data needs no WebRTC, so it could go into Harbour as well as Chum. Wakeups need the background CSP connection (Chum). In Harbour, Web works only while the app runs. Ask the user (§10).
- Policy for self-hosted hosts (§6).
