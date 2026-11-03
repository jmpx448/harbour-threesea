# Android → harbour-threesea mapping

Upstream paths are relative to `third_party/threema-android`. "lt" = provided by libthreema
(`domain/libthreema/lib/src`). Status: ✅ done · 🔨 in progress · ⏳ planned.

| Concern | Android (Kotlin/Java) | libthreema | Here | Status |
|---|---|---|---|---|
| License check (Libre/Shop serial) | `app/…/services/license/LicenseServiceSerial.java`, `APIConnector.java` (`check_license`) | — | `core::directory::check_license`, rechecked daily from `App::maintenance` (Android: `CheckLicenseRoutine`) | ✅ (live-tested with an invalid key) |
| Report spam (1:1, groups) | `ContactServiceImpl.reportSpam`, `ui/ReportSpamView.kt`; groups from iOS 7.5.2 (`SpamDetector`, `GroupSpamDetector`) | — | `core::app_spam`, `core::directory::report_identity`/`report_group`, `qml/pages/SpamReportDialog.qml` | 🔨 requests accepted by the server up to the challenge; the full report not yet run end to end |
| Identity creation (+ licenseKey, deviceId) | `UserServiceImpl.createIdentityRequestDataJSON` | lt `csp_e2e/identity/create.rs` (no license params for Consumer) | patch 0001 + `core::app::create_identity` | 🔨 (needs a real license to verify) |
| ID backup export/import | `domain/…/identitybackup/IdentityBackup.kt` | lt `id_backup/` ✅ | `core::app` (+ `directory::fetch_identity_private`) | 🔨 export ✅, restore needs license |
| Threema Safe | `ch.threema.app.threemasafe` | URL config only | `core::safe`, `core::app_safe` | ✅ backup + restore (contacts/groups imported once stored) |
| Directory lookup by ID (`identity/fetch_bulk`) | `APIConnector.fetchIdentities` | lt `csp_e2e/contacts/lookup.rs` (crate-private) | `core::contacts::lookup` | ✅ |
| Contacts (storage, names, verification levels) | `ch.threema.app.services.ContactService` | lt `model/contact.rs` | `core::contacts` (stores lt's `Contact` model), `core::app_contacts` | ✅ add/edit/delete, Safe import/export |
| Hashed phone/e-mail lookup, address-book sync | `ch.threema.app.services.SynchronizeContactsService` | — | `core::address_book`, `qml` `AddressBookReader` (opt-in) | ✅ |
| QR verification (level 2) | `ch.threema.app.qrscanner` | — | `rqrr` in core, `qml/pages/ScanQrPage.qml`, `MyQrCodePage.qml` | ✅ |
| CSP connection, reconnect, keepalive | `ch.threema.domain.protocol.connection` | lt `csp/` (sans-I/O) ✅ | `core::connection` (socket + loop) | ✅ connect, echo keepalive, backoff reconnect, offline queue |
| Incoming message processing | `ch.threema.app.processors` | lt `csp_e2e/` (text, location, receipts, legacy reactions) | lt `IncomingMessageTask` + `core::providers` (our store behind lt's provider traits) | ✅ text, location, receipts, typing |
| Outgoing tasks | `ch.threema.app.tasks` | helpers only (LIB-51) | patch 0003 encoder + `core::connection` queue | ✅ |
| Message types/coders | `ch.threema.domain.protocol.csp.messages`, `.coders` | lt `csp_e2e/message` (partial) | lt `csp_e2e/message` with our commits, `core::messages` | ✅ |
| Storage | `ch.threema.storage`, `ch.threema.data` | provider traits in lt `model/provider.rs` | `core::store` (SQLCipher): identity, settings, contacts, messages, nonces | ✅ |
| Forward secrecy (PFS) | `ch.threema.domain.fs`, `…csp.fs.ForwardSecurityMessageProcessor` | implemented by us in vendored lt `csp_e2e/fs/` (LIB-42) | lt `csp_e2e::fs` + `core::fs_store`, `core::connection` | ✅ 1:1 (versions 1.0–1.2); interop with Android verified |
| Groups | `ch.threema.app.groupflows`, `…tasks` | absent (LIB-53) | `core::groups`, `core::app_groups` | ✅ |
| Blob up/download | `ch.threema.domain.protocol.blob` | config + chunked crypto | `core::media`, `core::app_media` | ✅ |
| Background (Threema Push, persistent connection) | `ThreemaPushService.kt`, `PollingHelper.java` | — | `core::connection`, `BackgroundJob` in QML, systemd user unit (Full) | 🔨 battery use still to be measured on a phone |
| Message details (times, id, file type/size) | `ch.threema.app.messagedetails` | — | `core::messages` (migration 17), `qml/pages/MessageDetailsPage.qml` | ✅ |
| Voice player and its media session | `ch.threema.app.services.VoiceMessagePlayerService` (media3 `MediaSession`, audio focus) | — | `bridge/voiceplayer.cpp`, `qml/components/MediaSession.qml` (MPRIS instead of a media notification; closing the window stops it, unlike `onTaskRemoved`, by user report 2026-11-02) | ✅ |
| Media gallery (per chat) | `ch.threema.app.mediagallery` | — | `core::messages::conversation_media`, `qml/pages/MediaPage.qml` (all file messages, filtered by kind like Android's chips: photos, GIFs, videos, voice, audio, files) | ✅ |
| Animated images (GIFs) | `MessageServiceImpl` (`TYPE_IMAGE_ANIMATED`: unchanged, metadata `a`), `AnimatedImageDrawableMessagePlayer`, pref `gif_autoplay` | — | `bridge/filepreparer.cpp`, `bridge/animatedimage.cpp` (QMovie from memory), `core::app_media` (`animated`), favourites `core::gifs`, picked, added and removed in the emoji panel's GIF tab (`qml/components/EmojiPanel.qml`; ours, Android has none) | ✅ |
| Multi-device | `ch.threema.domain.protocol.multidevice`, `ch.threema.app.multidevice` | lt `d2m/`, `d2d_rendezvous/` | `core::transport`, `core::multidevice`, `core::reflected`, `core::sync`, `core::join`, `core::app_multidevice` (ADR 0011) | ✅ |
| 1:1 calls | `ch.threema.app.voip` (`VoipCallService`, `PeerConnectionClient`, `util/SdpPatcher`, `util/VoipVideoParams`, `signaling/CaptureState`) | lt call messages | `core::calls` (state), `core::sdp`, `core::call_video` (profiles, data channel), `core::app_calls`; media `bridge/callengine` (GStreamer webrtcbin), `bridge/callmanager`, `bridge/aec`; `qml/pages/CallPage.qml` (ADR 0012) | audio ✅, video ✅ |
| Android data backup (full ZIP) | `app/backuprestore/csv/{BackupService,RestoreService,Tags,RestoreSettings}`, `app/utils/{CSVReader,CSVWriter,CSVRow}`, `app/utils/FileHandlingZipOutputStream.kt` (Zip4j AES-256/AE, DEFLATE) | — | `core::android_backup` (container via the `zip` crate, own OpenCSV-exact CSV dialect, version gate 15–27, inventory); fixtures from pyzipper (`scripts/make-backup-fixtures.py`). Import into the running identity: next steps per `docs/android-backup-design.md` | 🔨 read/validate ✅, inspect/preview + merge ⏳ |
