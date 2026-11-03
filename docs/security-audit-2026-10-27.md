# Security audit — 27 October 2026

> **Status (updated November 2026):** the findings below describe the code as it was audited; the
> commits fixing them are from before the public history. Fixed: V01 (no media or channel data
> before the certificate check, group calls check the SFU's certificate), S01–S04 (except the FFI
> media readers, which have no database on their threads and only an in-process caller), S05, S06,
> S08; hardening: PIN attempt pauses, hosts checked before connecting, Safe decompression bounded;
> CI and fuzzing exist (`.github/workflows/`, `core/fuzz/`). Open: the device check of notification
> Reply (S07; since D04 the actions carry a per-chat token), and an inventory of native libraries
> on a device (the SDK's is in `deep-audit-2026-10-27/sdk-inventory.txt`). The probes in
> `security-audit-2026-10-27/` check for the vulnerable behaviour and so now fail for the fixed
> findings; the regression tests that replace them are in `core/src/app_web_security_tests.rs`,
> `core/src/group_call_frames.rs`, `core/src/web/chunks.rs` and `tests/callengine`. Details:
> `docs/STATUS.md`, Phase 5. The re-audit of the same day
> ([deep-audit-2026-10-27.md](deep-audit-2026-10-27.md)) found S01, S04 and S06 incomplete
> (D01–D03); those follow-ups are fixed too.

This review found two high-severity authorization/lifecycle defects, five medium-severity findings, and one low-severity storage finding. A separate potentially high-severity call-authentication issue requires device validation. The highest priorities are revoking Web sessions when deleting an identity and preventing Web clients from modifying private groups.

Application source was not changed. The accompanying offline probes reproduce seven vulnerable behaviors; their passing results mean the defects are present. An eighth probe exercises malformed parsers without a panic. No messages were sent to real users, identities deleted, or production services attacked during testing.

## Scope and evidence

- Baseline: a commit before the public history. During review, the code advanced through launcher-artwork changes. Concurrent performance edits appeared in `qml/pages/ChatPage.qml` and `qml/components/MessageDelegate.qml`. These changes were preserved and are outside the baseline review; the reported defects concern unchanged code paths.
- Reviewed the Rust application core, storage and key handling, custom and vendored protocol code, Web integration, call cryptography, Qt/C++ bridge, QML entry points, media handling, build configuration, and dependency declarations. Vendored cryptographic/protocol code received targeted review and its library tests, not a formal cryptographic audit.
- Threat actors considered: malicious chat participants; compromised paired browsers; malicious media intermediaries/SFUs; local processes permitted to access the session bus; and an attacker obtaining local stored files. A paired browser is trusted for ordinary chat access, but must remain excluded from private chats and lose authority when revoked.
- **Reproduced** means a bounded offline test reached the affected application logic. Browser probes model an already authenticated session and call the same core handlers as the session task; they do not establish a real browser/WebRTC connection.
- **Source-supported** means the defective behavior is visible in the code, but its full device-level exploit was not exercised. Severity is qualitative and includes the stated prerequisites; no CVSS scores are asserted.

| ID | Severity | Finding | Evidence |
|---|---|---|---|
| S01 | High | Web authority survives identity deletion; queued commands survive session revocation | Reproduced |
| S02 | High | Paired browsers can modify private groups | Reproduced |
| S03 | Medium | Existing Web subscriptions and deferred downloads disclose content after a chat becomes private | Reproduced |
| S04 | Medium | Private-chat authorization is missing from several core/FFI operations | Reproduced for core commands; FFI source-supported |
| S05 | Medium | Unauthenticated group-call frames mutate key state; authenticated frames can be replayed | Reproduced |
| S06 | Medium | Media and Web reassembly lack effective aggregate resource limits | Source-supported |
| S07 | Medium | Session-bus reply/read actions lack caller and lock authorization | Source-supported; device permission dependent |
| S08 | Low | A crash can leave a plaintext voice recording in cache | Source-supported |
| V01 | Potentially high | Call media may flow before DTLS peer identity verification | Static finding; runtime exposure unverified |

## Findings

### S01 — Web sessions retain authority across identity deletion and revocation

**Locations:** [app_contacts.rs](../core/src/app_contacts.rs), `delete_all_data` at line 155; [app_web.rs](../core/src/app_web.rs), `web_delete` at 200, `on_web_event` at 276, `send_web` at 335. The deletion UI calls the core and refreshes it in place: [ProfileSettingsPage.qml](../qml/pages/settings/ProfileSettingsPage.qml), line 77.

Deleting the identity stops the main connection, wipes the store, and deletes stored media. It does not stop or clear the running Web sessions. Those sessions retain their authenticated channels and process requests against whichever identity and database contents the same core subsequently holds. The persistent session records disappear during the wipe, so the surviving session can also disappear from the stored-session list.

The probe `audit_browser_survives_identity_deletion` creates an authenticated-session stand-in, deletes all data, and verifies that no `Stop` was sent. After simulating creation/restoration of a different identity in that core, the old session successfully requests the new identity's conversation contents. No re-pairing is required by the handler.

A related revocation race exists even for explicit `webDelete`: it removes the session, but `FromWeb::Message` dispatch does not check whether its sender still has authority. `audit_queued_browser_command_runs_after_session_deletion` confirms that a command delivered after revocation still inserts an outgoing message. The test demonstrates command execution and queuing, not actual network transmission.

**Impact and prerequisites:** an already paired browser that stays connected can cross an identity boundary. A revoked browser can also have already queued commands executed. Restarting the process removes the live-session condition; it does not make in-process deletion safe.

**Fix:** revoke a per-session/per-identity generation before changing identity state; check it on every incoming event and outgoing completion. Cancel and join session tasks, clear subscriptions and deferred requests, and reset other identity-bound call/linking/background work before wiping or installing an identity. Use generations rather than only numeric session IDs, which can be reused.

**Regression checks:** old sessions cannot read or mutate after delete/recreate; queued commands and late task completions are rejected after revocation; late events from an old session cannot affect a new session with the same numeric ID.

### S02 — Web management bypasses the private-group boundary

**Locations:** [app_web_manage.rs](../core/src/app_web_manage.rs), `update/group` at line 215 and `delete/group` at 268; [app_groups.rs](../core/src/app_groups.rs), `own_group` at 97; compare [app_web_data.rs](../core/src/app_web_data.rs), `web_receiver_key` at 535.

Ordinary browser conversation access rejects private chats through `web_receiver_key`. Group-management requests instead accept the raw group ID and invoke `setGroupMembers`, `renameGroup`, picture changes, or leave/delete operations. The downstream ownership check verifies group ownership/left status, but does not enforce the browser's private-chat restriction. Locked receiver metadata exposes the group ID, so discovery need not depend on guessing it.

`audit_browser_can_add_member_to_private_group` confirms that the normal browser access check rejects a private group while an `update/group` request successfully adds a member to it. The group is owned by the local identity. The test changes the local membership/outbox state; it does not deliver a live group update.

**Impact and prerequisites:** a compromised paired browser can change membership of a private group owned by the user, potentially exposing subsequent group messages to an added contact. It can also disrupt protected groups through management actions. This is not an unpaired-browser or arbitrary remote-sender attack.

**Fix:** enforce the browser's private-target restriction before every group and distribution-list management action, independent of whether the local UI is unlocked. Centralize this authorization so direct and indirect commands cannot bypass it.

**Regression checks:** deny member, name, picture, leave, delete, and list-management operations on private targets, both while the UI is locked and unlocked; keep authorized ordinary-group operations working.

### S03 — A privacy change does not revoke existing browser data flows

**Locations:** [app_web_data.rs](../core/src/app_web_data.rs), `web_on_core_event` at line 596 and `web_refresh` at 672; [app_web_media.rs](../core/src/app_web_media.rs), `web_blobs_changed` at 189.

Marking a conversation private prevents new ordinary browser reads, but leaves its existing message subscription active. `MessagesChanged` forwards subscribed messages without rechecking privacy. `web_refresh` skips new messages for private chats without removing those subscriptions. Deferred blob completion similarly returns downloaded bytes without rechecking whether the conversation has become private.

Two probes confirm the effects: after subscribing and marking a chat private, a later edit delivers `NEW PRIVATE CONTENT`; an authorized pending blob request also delivers `NEW PRIVATE FILE` after the privacy transition. The latter probe supplies the pending-request state and completes its local download without contacting a blob service.

**Impact and prerequisites:** a previously authorized browser receives newly changed content or newly completed media after access should have ended. Content the browser already received cannot be recalled; this finding concerns later disclosures.

**Fix:** recheck authorization immediately before every browser emission and asynchronous completion. On privacy changes, remove affected subscriptions and pending downloads/replies; treat store errors as denied access.

**Regression checks:** subscribe/download, mark private, then edit/complete; verify that no sensitive payload reaches any browser session. Cover every event type, not just new-message refreshes.

### S04 — Private-chat protection is inconsistent at the core boundary

**Locations:** [app.rs](../core/src/app.rs), `Draft` at line 1265 and `SetStarred` at 1286; [app_media.rs](../core/src/app_media.rs), `forward_message` at 806 and `save_file` at 861; [ffi.rs](../core/src/ffi.rs), `threesea_media_read` at 128 and `threesea_voice_decode` at 196. The intended check is [app_messages.rs](../core/src/app_messages.rs), `check_readable` at 126.

The ordinary `message` operation checks private-chat access, but `setStarred` returns the complete message without that check, `draft` returns private draft text, and forwarding reads and queues private content without checking access to the source conversation. Saving media and the direct media/voice FFI readers likewise omit private-chat authorization.

`audit_private_core_commands_bypass_read_gate` verifies that the same locked private message is denied by `message`, returned by `setStarred`, and forwarded to another contact. It also retrieves a private draft. FFI and save-to-file omissions were established by source inspection.

There is a related event-redaction gap: [providers.rs](../core/src/providers.rs), line 106, preserves the bodies of private `status` messages on the assumption they contain only application codes. [groups.rs](../core/src/groups.rs), line 557, constructs `groupRenamed|<name>`, which contains user-controlled group text. This subcase was not separately exercised.

**Impact and prerequisites:** code able to invoke the UI/core API can bypass the intended core-enforced private-chat restriction. This does not establish that an arbitrary remote chat sender can invoke the whole JSON/FFI API, nor that all these commands are exposed to a browser. Severity reflects the documented defense boundary and the risk of stale or overlooked UI paths.

**Fix:** authorize source conversations for every read/export/forward operation and every mutation that returns content. Give media readers a revocable authorization check rather than direct unconditional store access. Redact user-controlled status arguments and clear relevant UI caches when access ends.

**Regression checks:** exercise the complete command/FFI matrix with known message IDs while locked; include drafts, stars, forwards, saved files, thumbnails, voice decoding, and status events.

### S05 — Invalid group-call frames can poison ratchet state; replay is accepted

**Location:** [group_call_frames.rs](../core/src/group_call_frames.rs), `decrypt` at line 183, key removal at 200, AEAD authentication at 213.

The decoder selects the frame's claimed key epoch, discards older keys, and advances the current ratchet according to the unauthenticated footer before verifying the AEAD tag. A rejected frame therefore changes persistent receive state. There is also no receive window rejecting an already accepted media-frame sequence number.

`audit_invalid_frame_poisoning_and_replay` decrypts a valid frame, successfully decrypts its identical replay, then changes the ratchet byte to 255 without producing a valid tag. The forged frame is rejected, but the original valid frame can no longer be decrypted because the receive ratchet has already advanced.

**Impact and prerequisites:** an actor able to inject or modify the end-to-end encrypted frames delivered through the media transport, such as a malicious SFU, can disrupt a participant's media until suitable new key state arrives and can replay authenticated audio/video frames. Outer transport protection limits who can reach this decoder; this is not arbitrary unauthenticated Internet packet injection. No key recovery or plaintext forgery was demonstrated. An SFU can already drop traffic, which limits the incremental availability impact.

**Fix:** derive candidate state without mutating the committed ratchet, authenticate the frame, then commit changes. Maintain a bounded replay window per participant/key, accommodating legitimate packet reordering and resetting it safely on key transitions.

**Regression checks:** every invalid-tag/epoch/ratchet input leaves state unchanged; identical frames are rejected after the first acceptance; legitimate out-of-order traffic remains usable.

### S06 — Resource budgets do not bound inbound media and Web reassembly

**Locations:** [https.rs](../core/src/https.rs), line 350; [media.rs](../core/src/media.rs), line 375; [app_media.rs](../core/src/app_media.rs), line 610; [web/chunks.rs](../core/src/web/chunks.rs), lines 14–16 and 61–99; [mediaimageprovider.cpp](../bridge/mediaimageprovider.cpp), line 65; [animatedimage.cpp](../bridge/animatedimage.cpp), line 71; [voice.rs](../core/src/voice.rs), line 64.

There are two independently reachable resource problems:

1. The HTTP client buffers the complete response before the blob layer checks its 100 MiB limit. Pending automatic downloads are spawned without a shared concurrency/byte budget. Native image/animation readers lack an application-level decoded-size budget, and voice decoding accumulates decoded PCM. Even individually permitted compressed media can expand or multiply into memory pressure. An oversized HTTP response specifically requires the endpoint to serve it; the audit did not establish that a sender can bypass the real blob service's limits. Multiple permitted downloads and high-expansion media are separate concerns.
2. Web reassembly permits 64 pending messages of up to 128 MiB payload each, an 8 GiB aggregate payload allowance before copies and metadata. The size check counts only payload bytes. Header-only chunks with distinct serial numbers can grow the per-message map without consuming that byte allowance. There is no suitable chunk-count/global-byte budget or expiration for this case.

**Impact and prerequisites:** a sender whose media is downloaded or opened can cause memory/CPU pressure; an already paired malicious browser can exhaust reassembly resources. No intentional OOM or native decoder exploitation was attempted. These are availability findings, not demonstrated memory-corruption vulnerabilities.

**Fix:** stream HTTP bodies through actual byte limits; cap simultaneous and aggregate downloads; reject excessive decoded dimensions/frame counts/durations before large allocations; enforce global Web reassembly bytes, chunk counts, serial bounds, and timeouts. Apply budgets before allocation. A `Content-Length` check alone is insufficient.

**Regression checks:** use small configurable test budgets to reject oversized/chunked responses, high-expansion media, many concurrent downloads, and large numbers of zero-payload chunks without risking the test host. The separate Rust Web image decoder already checks dimensions; carry equivalent protection into native rendering paths.

### S07 — D-Bus notification actions bypass caller and lock checks

**Locations:** [dbusinterface.cpp](../bridge/dbusinterface.cpp), exported slots at line 29, `reply` at 61 and `markRead` at 67; [harbour-threesea.qml](../qml/harbour-threesea.qml), handlers at lines 558 and 562.

The session-bus object exposes notification reply and mark-read operations without verifying the caller. Their QML handlers invoke `sendText` and `markRead` without consulting the application/private-chat lock. The open-chat path has a visibility gate; it does not protect these separate action paths.

**Impact and prerequisites:** a local process allowed to reach this session-bus name can submit messages as the loaded identity and change read state, including while the UI is locked. Sailjail policy can prevent other sandboxed applications from reaching the name, so actual reachability depends on device permissions. This is not a network-facing service. Same-user execution is a meaningful prerequisite; it does not by itself grant the application's Secrets-protected identity to the caller.

The earlier [22 October audit](security-audit.md) already records this unresolved issue and an attempted caller check that was reverted. This audit rates it medium because the operation impersonates the user; that rating remains conditional on bus reachability.

**Fix:** authenticate the supported notification-service caller using a tested device-side mechanism, or require an authorized UI interaction for reply. Enforce a deliberate lock policy in the core as well. Do not rely solely on hiding the notification action or hard-code an unverified process identity.

**Regression checks:** on a device, legitimate notification actions work; a second process with bus access cannot send or mark read; locked/private cases follow the documented policy. No real D-Bus reply was sent during this audit.

### S08 — Crashes leave plaintext voice recordings outside encrypted storage

**Location:** [voicerecorder.cpp](../bridge/voicerecorder.cpp), constructor at line 47, cleanup at 142, `outputPath` at 159.

The platform AAC recorder writes `voice-recording.m4a` in the application cache. Send/discard/normal destruction remove it, but a crash bypasses that cleanup. The constructor does not sweep a leftover recording, so it can remain until a later operation deletes it. The in-memory Opus fallback avoids this particular file.

**Impact and prerequisites:** someone with access to the cache or a filesystem copy can recover an unsent recording despite encryption of the message store. Filesystem permissions and device encryption reduce exposure. No evidence was found of a remote reader for this cache.

**Fix:** sweep stale voice recordings at startup, use explicitly restrictive permissions, and prefer memory or encrypted staging where the platform supports it. Crash cleanup reduces persistence but does not encrypt a recording while capture is active.

Camera captures also use plaintext staging. Unlike the voice file, their dedicated directory is already removed at core startup in [threesebackend.cpp](../bridge/threesebackend.cpp), line 297. That mitigation should be retained. User-requested Downloads/Gallery exports are deliberate plaintext exports, not counted here as vulnerabilities.

## Call-authentication validation required

### V01 — The 1:1 media pipeline starts before peer certificate acceptance

**Locations:** [callengine.cpp](../bridge/callengine.cpp), microphone pipeline around line 105, `PLAYING` at 194, queued connection-state callback at 420, and `remoteCertificateMatches` at 791. Related group path: [groupcallengine.cpp](../bridge/groupcallengine.cpp), `onConnectionState` at 1187.

The microphone pipeline is linked to `webrtcbin` and placed in `PLAYING` before the application verifies the remote DTLS certificate against the end-to-end authenticated SDP fingerprint. Verification occurs only after the transport reports `CONNECTED`, through a queued Qt callback. The code explicitly documents that its `webrtcbin` path does not perform this fingerprint verification itself. The application does not install a media-blocking gate that waits for the result; `m_connected` is an application status flag.

**Potential impact:** if the shipped GStreamer stack releases media when DTLS connects, an active media-path attacker able to complete the transport/ICE exchange with a wrong certificate could receive transient microphone/video data before the queued callback rejects the call. This is not evidence of a persistent undetected takeover. Exact ordering, amount of media exposure, and the relevant shipped GStreamer behavior were not measured; treat the confidentiality impact as a high-priority validation item rather than a reproduced exploit.

The group-call connection handler also has no explicit remote-certificate comparison. Investigate authentication of the SFU transport on the shipped stack. Group media has an additional end-to-end encryption layer, so this observation does not establish disclosure of group audio/video to the SFU or an intermediary.

**Fix/validation:** authenticate the DTLS peer before releasing any media or data in either direction, ideally in the transport's verification mechanism. If application gates are necessary, ensure they operate before any media can flow and cannot deadlock the handshake. On a device, connect a controlled peer with a deliberately wrong certificate, delay the Qt event loop, and capture transport traffic. Assert zero outgoing intelligible media and zero delivered incoming data before acceptance; check normal handshake, reconnect, and failure paths.

## Additional hardening and dependency observations

- The core PIN check has no attempt counter/backoff in [app_messages.rs](../core/src/app_messages.rs), line 185. Salted scrypt is useful, but short PINs still need online throttling and a documented offline-strength model. This was not assigned a separate exploit severity.
- HTTPS host rejection occurs inside certificate verification in [https.rs](../core/src/https.rs), line 95. This prevents an authenticated HTTP exchange with an unapproved host, but DNS/TCP/TLS setup can already occur. Validate destination policy before initiating a connection, particularly for user-supplied rendezvous URLs. The older audit's claim that unapproved hosts are rejected before *any connection* is stronger than this implementation supports. No arbitrary HTTP response/content exfiltration was demonstrated.
- Safe restore decompresses authenticated input with `read_to_end` in [safe/mod.rs](../core/src/safe/mod.rs), line 114. Add a decompressed-size limit as defense in depth; this is a different trust condition from unauthenticated remote input.
- The refreshed RustSec scan found **zero advisories and zero warnings across 286 lockfile packages**. Database commit: `3279c99911354c400e1d41e47c7d60e39258b147`, updated `2026-10-26T22:32:25+01:00`; tool: cargo-audit 0.22.2. Raw result: [dependencies.json](security-audit-2026-10-27/dependencies.json). This is a known-advisory check, not proof of dependency safety.
- Cargo does not fully inventory the platform's Qt, GStreamer, codec, or other native libraries. The installed `libsqlite3-sys 0.35.0` vendored source identifies SQLCipher **4.6.1** and SQLite **3.46.1**. Review an upgrade and produce a native-library inventory for the actual RPM/device. SQLite's [security notes](https://www.sqlite.org/cves.html) distinguish vulnerabilities requiring attacker-controlled SQL or database files; no such exploitable prerequisite was established here, so an old version alone is not reported as a confirmed application vulnerability.
- Vendored libthreema has substantial project-specific changes, including PFS work, and opus-pure has local modifications. Passing upstream tests and a clean package advisory scan do not assess all local changes. Maintain a reviewed patch inventory and add protocol interoperability/fuzz testing.
- Release Rust builds use `--locked`, and the toolchain is pinned. The SDK container uses a tag rather than an immutable base digest and downloads `rustup-init` without a separately pinned checksum in [scripts/sdk/Dockerfile](../scripts/sdk/Dockerfile). Pin build inputs and retain provenance/SBOMs. No repository CI was present to enforce advisory and security-regression checks.

## Controls observed working in the reviewed paths

- SQLCipher protects the database; media uses authenticated encryption and a separate derived key. Tests cover encrypted storage and wrong-key handling. Sailfish Secrets integration uses an owner-only collection; the app-lock representation uses salted scrypt.
- Reviewed SQL value paths use bound parameters. No externally controlled table/column interpolation was identified.
- HTTPS combines certificate validation with SPKI restrictions for normal service traffic. Custom Web servers use an explicit user-authorized trust/pinning path. No blanket accept-any-certificate bypass was found in these paths.
- Message edits/deletes check sender identity. Group creator identity is derived from authenticated message context in the codec; a group payload alone cannot assert arbitrary creator authority. The browser bugs above should not be mistaken for a demonstrated protocol-level group takeover by any sender.
- QML content display generally uses plain text, with escaping in reviewed rich-text paths; the QML network layer is restricted. No automatic remote-image tracking or message-text script execution was identified in those paths.
- The Rust core denies unsafe code except in the explicit FFI boundary. Buffer-release code wipes decrypted media buffers. These controls do not remove native decoder or caller-contract risks.
- Targeted secret/logging checks found no obvious production credentials in the inspected application files or routine plaintext-message/key logging. This was not a complete Git-history secret scan or a proof that every runtime error is free of sensitive context.

## Validation and limits

| Check | Result |
|---|---|
| `cargo test --workspace --locked --offline` | 142 passed, 7 ignored; no failures |
| `cargo clippy --workspace --all-targets --locked --offline` | Passed without warnings |
| Vendored `libthreema` library tests | 3,680 passed; no failures |
| Offline audit probes | 7 vulnerability reproductions + 1 parser smoke test passed |
| Refreshed RustSec scan | 286 packages; 0 vulnerabilities, 0 warnings |

Commands and environment are recorded in [validation.txt](security-audit-2026-10-27/validation.txt). The probe source and runner are [probes.rs](security-audit-2026-10-27/probes.rs) and [run-probes.sh](security-audit-2026-10-27/run-probes.sh). Run from the repository with:

```sh
sh docs/security-audit-2026-10-27/run-probes.sh
```

The runner creates a disposable copy of the core under `/tmp`, uses cached dependencies, and reuses the repository's `target` directory. It does not patch the production source tree. Its assertions describe the vulnerable baseline; after fixes, replace them with assertions that unauthorized operations are rejected.

The parser smoke test covers 104 deterministic malformed byte strings across 21 CSP message types (2,184 decode attempts), plus voice, link, rendezvous, and Web QR parsers. This is a small crash smoke test, not coverage-guided fuzzing or a memory-safety assessment.

The seven ignored integration tests were not run because they require live-service behavior. There was no Sailfish device/Qt SDK execution: the host lacked the required Qt build environment and Docker socket access was unavailable. C++ bridge behavior, D-Bus policy, DTLS ordering, RPM hardening, native-library versions on the device, sanitizers, and on-device exploitability remain unverified. No production service penetration testing, destructive resource testing, exhaustive repository-history scan, formal cryptographic proof, or independent protocol certification was performed.

## Remediation order

1. Fix S01 and S02 before trusting Web pairing across private-chat or identity boundaries. Implement session generations and centralized browser authorization together; then test S03's asynchronous paths.
2. Close S04's core/FFI authorization gaps and S05's authentication-before-state-change/replay issues. Convert the supplied reproductions into rejecting regressions.
3. Validate V01 on the actual shipped media stack before treating calls as fully authenticated from their first media packet.
4. Add S06 resource budgets, validate S07 against the device notification service, and remove S08 crash leftovers. Establish native dependency inventory and automated security checks.
