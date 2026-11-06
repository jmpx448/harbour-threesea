# Android → harbour-threesea mapping

Upstream paths are relative to `third_party/threema-android`. "lt" = provided by libthreema
(`domain/libthreema/lib/src`). Status: ✅ done · 🔨 in progress · ⏳ planned.

| Concern | Android (Kotlin/Java) | libthreema | Here | Status |
|---|---|---|---|---|
| License check (Libre/Shop serial) | `app/…/services/license/LicenseServiceSerial.java`, `APIConnector.java` (`check_license`) | — | `core::directory::check_license`, rechecked daily from `App::maintenance` (Android: `CheckLicenseRoutine`) | ✅ (live-tested with an invalid key) |
| Report spam (1:1, groups) | `ContactServiceImpl.reportSpam`, `ui/ReportSpamView.kt`; groups from iOS 7.5.2 (`SpamDetector`, `GroupSpamDetector`) | — | `core::app_spam`, `core::directory::report_identity`/`report_group`, `qml/pages/SpamReportDialog.qml` | 🔨 requests accepted by the server up to the challenge; the full report not yet run end to end |
| Identity creation (+ licenseKey, deviceId) | `UserServiceImpl.createIdentityRequestDataJSON` | lt `csp_e2e/identity/create.rs` (no license params for Consumer) | patch 0001 + `core::app::create_identity` | ✅ (verified with a Threema Shop license, 2026-10-21) |
| ID backup export/import | `domain/…/identitybackup/IdentityBackup.kt` | lt `id_backup/` ✅ | `core::app` (+ `directory::fetch_identity_private`); as text, QR code and share like `ExportIDResultActivity`, scanned on restore like `WizardIDRestoreActivity` (printing left out: no print service on the device) | ✅ (export and restore verified with a Threema Shop license) |
| Threema Safe | `ch.threema.app.threemasafe` | URL config only | `core::safe`, `core::app_safe` | ✅ backup + restore (contacts/groups imported once stored) |
| Setup steps after a restore | `app/protocolsteps/ApplicationSetupSteps.kt` | – | `core::app_safe` (`restore_setup_step`), `groups::resync_restored`, `fs_store::start_session` | ✅ |
| Directory lookup by ID (`identity/fetch_bulk`) | `APIConnector.fetchIdentities` | lt `csp_e2e/contacts/lookup.rs` (crate-private) | `core::contacts::lookup` | ✅ |
| Contact refresh (`identity/check`, `checkInterval`) | `APIConnector.checkIdentityStates`, `ContactUpdateWorker` | – | `core::directory::check_identities`, `App::refresh_contacts` | ✅ |
| Contacts (storage, names, verification levels) | `ch.threema.app.services.ContactService` | lt `model/contact.rs` | `core::contacts` (stores lt's `Contact` model), `core::app_contacts` | ✅ add/edit/delete, Safe import/export |
| Deleting a contact (row stays, chat goes) | `MarkContactAsDeletedBackgroundTask` | — | `App::delete_contact`, ADR 0022 | ✅ |
| Hashed phone/e-mail lookup, address-book sync | `ch.threema.app.services.SynchronizeContactsService` | — | `core::address_book`, `qml` `AddressBookReader` (opt-in); one-to-one matches only, numbers without a country code by a setting | 🔨 built and tested on the host; untested on a phone with a real address book (checklist U) |
| QR verification (level 2) | `ch.threema.app.qrscanner` | — | `rqrr` in core, `qml/pages/ScanQrPage.qml`, `MyQrCodePage.qml` | ✅ |
| CSP connection, reconnect, keepalive | `ch.threema.domain.protocol.connection` | lt `csp/` (sans-I/O) ✅ | `core::connection` (socket + loop) | ✅ connect, echo keepalive, backoff reconnect, offline queue |
| Incoming message processing | `ch.threema.app.processors` | lt `csp_e2e/` (text, location, receipts, legacy reactions) | lt `IncomingMessageTask` + `core::providers` (our store behind lt's provider traits) | ✅ text, location, receipts, typing |
| Outgoing tasks | `ch.threema.app.tasks` | helpers only (LIB-51) | patch 0003 encoder + `core::connection` queue | ✅ |
| Message types/coders | `ch.threema.domain.protocol.csp.messages`, `.coders` | lt `csp_e2e/message` (partial) | lt `csp_e2e/message` with our commits, `core::messages` | ✅ |
| Storage | `ch.threema.storage`, `ch.threema.data` | provider traits in lt `model/provider.rs` | `core::store` (SQLCipher): identity, settings, contacts, messages, nonces | ✅ |
| Choosing a place to send | `ch.threema.app.location` (`PoiRepository`, `LocationPickerActivity`, `LocationAutocompleteActivity`) | — | `core::places` (`findPlaces`), `qml/pages/PlacePickerPage.qml`; the map: `qml/pages/MapPickerDialog.qml`, received ones `LocationPage.qml`, `components/ThreemaMap.qml` (MapLibre, ADR 0032) | ✅ nearby, by name, on the map |
| Forward secrecy (PFS) | `ch.threema.domain.fs`, `…csp.fs.ForwardSecurityMessageProcessor` | implemented by us in vendored lt `csp_e2e/fs/` (LIB-42) | lt `csp_e2e::fs` + `core::fs_store`, `core::connection` | ✅ 1:1 (versions 1.0–1.2); interop with Android verified |
| Groups | `ch.threema.app.groupflows`, `…tasks` | absent (LIB-53) | `core::groups`, `core::app_groups` | ✅ |
| Blob up/download | `ch.threema.domain.protocol.blob` | config + chunked crypto | `core::media`, `core::app_media` | ✅ |
| A chat's polls and the reminder of open ones | `ComposeMessageFragment.java` (`menu_poll_show_all`), `OpenPollNoticeView` | — | `core::app_polls` (`polls`), `qml/pages/PollsPage.qml`, banner in `ChatPage.qml` | ✅ |
| Background (Threema Push, persistent connection) | `ThreemaPushService.kt`, `PollingHelper.java` | — | `core::connection`, `BackgroundJob` in QML, systemd user unit (Full) | 🔨 battery use still to be measured on a phone |
| Message details (times, id, file type/size) | `ch.threema.app.messagedetails` | — | `core::messages` (migration 17), `qml/pages/MessageDetailsPage.qml` | ✅ |
| Voice player and its media session | `ch.threema.app.services.VoiceMessagePlayerService` (media3 `MediaSession`, audio focus) | — | `bridge/voiceplayer.cpp`, `qml/components/MediaSession.qml` (MPRIS instead of a media notification; closing the window stops it, unlike `onTaskRemoved`, by user report 2026-11-02) | ✅ |
| Deprecated media messages | `ch.threema.domain.protocol.csp.coders.LegacyMessageTransformer` | — | libthreema `csp_e2e/message/codec.rs` (`DeprecatedMedia`), `core::app_media::LegacySeal` for a 1:1 image | ✅ |
| Metadata and generic names of media sent (`VideoTranscoder`, `MessageServiceImpl`) | `app/…/video/transcoder/VideoTranscoder.java`, `services/MessageServiceImpl.java` | — | `core::mp4` (in place, no re-encoding needed; ADR 0025), `app_media::generic_media_name`, sizes in `bridge/filepreparer.cpp` | ✅ |
| Photo editor, "Draw reply" | `ImagePaintActivity`, `CropImageActivity`, `ComposeMessageFragment.sendImageReply` | — | `qml/pages/EditPhotoPage.qml` (cut, turn, pen, text), `bridge/photoedit.cpp` (burnt in at full resolution); stickers and face blur not ported; a reply goes out from memory (ADR 0029) | ✅ |
| Choosing a profile or group picture | `ui/AvatarEditView.java` (gallery, camera, crop) | — | `qml/components/PictureChooser.qml`, square mode of `EditPhotoPage.qml` | ✅ |
| Media gallery (per chat) | `ch.threema.app.mediagallery` | — | `core::messages::conversation_media`, `qml/pages/MediaPage.qml` (all file messages, filtered by kind like Android's chips: photos, GIFs, videos, voice, audio, files) | ✅ |
| Animated images (GIFs) | `MessageServiceImpl` (`TYPE_IMAGE_ANIMATED`: unchanged, metadata `a`), `AnimatedImageDrawableMessagePlayer`, pref `gif_autoplay` | — | `bridge/filepreparer.cpp`, `bridge/animatedimage.cpp` over `bridge/gifdecoder.cpp` (decoded off the GUI thread, from memory), `core::gif` (sent without comments and application data), `core::app_media` (`animated`), favourites `core::gifs`, picked, added and removed in the emoji panel's GIF tab (`qml/components/EmojiPanel.qml`; ours, Android has none) | ✅ |
| Multi-device | `ch.threema.domain.protocol.multidevice`, `ch.threema.app.multidevice` | lt `d2m/`, `d2d_rendezvous/` | `core::transport`, `core::multidevice`, `core::reflected`, `core::sync`, `core::join`, `core::app_multidevice` (ADR 0011) | ✅ |
| 1:1 calls | `ch.threema.app.voip` (`VoipCallService`, `PeerConnectionClient`, `util/SdpPatcher`, `util/VoipVideoParams`, `signaling/CaptureState`) | lt call messages | `core::calls` (state; `VoipStateService`), `core::sdp`, `core::call_video` (profiles, data channel, VP8 size limit), `core::app_calls`; media `bridge/callengine` (GStreamer webrtcbin), `bridge/callmanager`, `bridge/aec`; `qml/pages/CallPage.qml` (ADR 0012) | audio ✅, video ✅ |
| Android data backup (full ZIP) | `app/backuprestore/csv/{BackupService,RestoreService,Tags,RestoreSettings}`, `app/utils/{CSVReader,CSVWriter,CSVRow}`, `app/utils/FileHandlingZipOutputStream.kt` (Zip4j AES-256/AE, DEFLATE) | — | `core::android_backup` (container via the `zip` crate, own OpenCSV-exact CSV dialect, version gate 15–27; `import.rs`, `export.rs`), `core::app_android_backup`; fixtures from pyzipper (`scripts/make-backup-fixtures.py`). Import into the running identity, on a device with no ID first its ID (`androidBackupRestore`), and export by hand or on a schedule: `docs/android-backup-design.md` | ✅ import, restore at first start and export; not yet restored on a real Android device (`docs/android-backup-device-checklist.md`) |
| Distribution lists | `services/DistributionListServiceImpl.java` | — | `core::distribution_lists`, `qml/pages/DistributionListPage.qml` | ✅ |
| Polls | `services/poll/PollServiceImpl.java`, `poll/` | lt messages | `core::polls`, `qml/pages/PollPage.qml`, `PollResultsPage.qml`, `CreatePollDialog.qml` | ✅ |
| Reactions | `emojireactions/`, `MessageServiceImpl` (legacy thumbs for old clients) | lt messages | `core::reactions`, chips and "Reactions" in `MessageDetailsPage.qml` | ✅ |
| Profile pictures | `services/AvatarService.java`, `ProfilePictureRecipientsServiceImpl.java`, `profilepicture/` | — | `core::profile_pictures`, `qml/components/PictureChooser.qml`; one gate for showing them (ADR 0023), asked for after a restore (ADR 0026) | ✅ |
| App lock and private chats | `applock/`, `services/LockAppServiceImpl.kt`, `pinlock/` | — | `core::applock`, `qml/pages/AppLockPage.qml`, `UnlockPage.qml`, `PinSetupPage.qml` (ADR 0009) | ✅ |
| Notifications | `services/notification/` (`NotificationServiceImpl.kt`, `NotificationActionService.kt`) | — | `core::chat_notifications`, `qml/harbour-threesea.qml` and `qml/components/NotificationActions.js` (Nemo.Notifications) with actions over D-Bus (`bridge/dbusinterface.cpp`, per-chat tokens) | ✅ |
| Chat export | `backuprestore/ExportConversationServiceImpl.java` | — | `core::app_chat_export`, `qml/pages/ExportChatDialog.qml` | ✅ |
| Group calls | `voip/groupcall/` | lt call messages | `core::group_calls`, `core::app_group_calls`, `core::group_call_frames`, `core::group_call_sdp`; media `bridge/groupcallengine`, `bridge/groupcallmanager`; `qml/pages/GroupCallPage.qml` (ADR 0012, `docs/research/group-calls.md`) | ✅ (Full; phone tests pending) |
| Threema Web | `webclient/` | — | `core::web` (SaltyRTC, chunks, images), `core::app_web*`, `qml/pages/settings/WebSessionsPage.qml` (ADR 0013, `docs/research/threema-web.md`) | ✅ |
| Searching lists and the chats by name | `ConversationsFragment.kt` (`filter`), `ContactsSectionFragment.java` (search in the list), `GlobalSearchActivity` | — | `core` `find_chats_json` for the search page (hidden private chats stay out), `bridge/listfilter` for the contact, share and member lists | ✅ |

## Message actions

| Android | Here |
|---|---|
| `MessageServiceImpl.resendMessage` (same ID, polls a new one), tap on a failed bubble (`ComposeMessageFragment`) | `resendMessage` (`app_messages.rs`), tap or "Send again" in `MessageDelegate.qml` |
| `FileChatAdapterDecorator`, `MessageServiceImpl.cancelMessageUpload` and downloads | `cancelTransfer` (`app_media.rs`), "Stop sending" and "Stop download" in the bubble's menu |
| `copySelectedMessagesToClipboard`, `menu_share` (text), gallery's select all | `selectionActions` (`Format.js`), `MessageOut.qml`, the panels in `ChatPage.qml` and `MediaPage.qml` |
| `EmojiReactionsOverviewActivity` | "Reactions" on `MessageDetailsPage.qml`, opened by holding a chip |
