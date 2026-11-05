// SPDX-License-Identifier: AGPL-3.0-only
//! The application core: owns the store and network clients and executes commands from the UI.
//!
//! Protocol with the bridge (see docs/ffi.md): the UI sends JSON requests
//! `{"id": <n>, "cmd": "<name>", ...args}`; the core answers each with
//! `{"reply": <n>, "ok": true, "result": {...}}` or
//! `{"reply": <n>, "ok": false, "error": {"kind": "<kind>", "message": "..."}}`,
//! and may emit unsolicited `{"event": "<name>", ...}` messages.

use std::{path::PathBuf, sync::Arc};

use libthreema::{
    common::{
        config::{Config, ConfigEnvironment, Flavor},
        keys::ClientKey,
    },
    csp_e2e::{
        CspE2eProtocolError,
        identity::create::{
            CreateIdentityContext, CreateIdentityLoop, CreateIdentityResponse,
            CreateIdentityResult, CreateIdentityTask, SerialLicense,
        },
    },
    id_backup::{IdentityBackupData, decrypt_identity_backup, encrypt_identity_backup},
};
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{
    detached::Lane,
    directory::{self, ClientVersion, DirectoryError},
    https::HttpsClient,
    store::{Store, StoreError, StoredIdentity, keys},
};

/// Minimum password length for ID backups (same as Android).
pub const MIN_BACKUP_PASSWORD_LENGTH: usize = 8;

/// How often a running app asks the directory about its license.
const LICENSE_CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

/// How often maintenance runs while the app does.
pub(crate) const MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// A check is due without an earlier one, after the interval, and when the clock went backwards.
fn license_check_due(checked_at: Option<i64>, now: i64) -> bool {
    checked_at.is_none_or(|at| now < at || now - at >= LICENSE_CHECK_INTERVAL_MS)
}

/// Static configuration passed by the bridge at start.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartConfig {
    /// Directory for the database and files.
    pub data_dir: PathBuf,
    /// App version.
    pub app_version: String,
    /// UI language, e.g. `de`.
    pub language: String,
    /// Country, e.g. `DE`.
    pub country: String,
    /// Device model.
    pub model: String,
    /// OS version.
    pub os_version: String,
    /// The user's Pictures folder (auto-saving received media), if known.
    #[serde(default)]
    pub pictures_dir: Option<PathBuf>,
    /// This build can make and take calls (Full flavour, ADR 0012).
    #[serde(default)]
    pub calls: bool,
    /// Log level (Settings → Advanced): `warn` (default), `info`, `debug`.
    #[serde(default)]
    pub log_level: Option<String>,
    /// This build may check GitHub for a newer release (a build for GitHub, not a store; ADR
    /// 0014). Whether it does is the user's choice.
    #[serde(default)]
    pub update_check: bool,
}

/// A command from the UI.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Command {
    /// Where the user is in onboarding.
    Status,
    /// Check a Threema Shop license key and store it on success.
    CheckLicense {
        license_key: Zeroizing<String>,
    },
    /// The stored license key and why Threema's directory refused it, if it did.
    LicenseStatus,
    /// Forget the stored license key, to enter another. Only on a device without an ID.
    ForgetLicense,
    /// Whether this chat offers a spam report.
    SpamPotential {
        identity: String,
    },
    /// Not spam: this chat offers no report any more.
    DismissSpam {
        identity: String,
    },
    /// Report this chat as spam, and with `block` block the sender (or group creator) and delete it.
    ReportSpam {
        identity: String,
        #[serde(default)]
        block: bool,
    },
    /// Create a new Threema ID using the stored license.
    CreateIdentity,
    /// Restore an ID from an ID backup string.
    RestoreIdentityBackup {
        backup: Zeroizing<String>,
        password: Zeroizing<String>,
    },
    /// Export the own ID as an ID backup string.
    ExportIdentityBackup {
        password: Zeroizing<String>,
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// Rate a prospective Threema Safe password (`tooShort|tooLong|weak|ok`).
    SafeCheckPassword {
        password: Zeroizing<String>,
    },
    /// Threema Safe state for the settings page.
    SafeStatus,
    /// Enable Threema Safe with a password and upload a first backup.
    SafeEnable {
        password: Zeroizing<String>,
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// Upload a backup now.
    SafeBackupNow,
    /// Disable Threema Safe: delete the backup from the server, then forget the key. Disabled even
    /// if the deletion fails; the reply's `serverCopyDeleted` says how it went.
    SafeDisable {
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// Restore an ID (and profile, contacts, groups, settings) from Threema Safe.
    SafeRestore {
        identity: String,
        password: Zeroizing<String>,
    },
    /// Try the rest of a restored backup's import (contacts, groups, lists) now.
    SafeRetryImport,
    /// Look into an Android data backup (ZIP) on the device: format version, whether it
    /// belongs to the running identity, and what it holds. Local only; nothing is imported.
    AndroidBackupInspect {
        path: String,
        password: Zeroizing<String>,
    },
    /// Merge an Android data backup into the running identity. `scope` picks what:
    /// `metadata` = contacts, groups, distribution lists and pictures (offline, no sending
    /// side effects, synchronous); `history` = messages, media and reactions as a background
    /// job reporting `androidBackupProgress`/`androidBackupDone` events.
    AndroidBackupImport {
        path: String,
        password: Zeroizing<String>,
        scope: String,
    },
    /// Stop a running Android history import between batches (done work stays; the import
    /// resumes idempotently when run again).
    AndroidBackupCancel,
    /// Write an Android-format data backup (ZIP) of this device's data into a folder, as a
    /// background job (`androidExportProgress`/`androidExportDone` events). Omitted fields
    /// fall back to the stored schedule settings.
    AndroidBackupExport {
        directory: Option<String>,
        password: Option<Zeroizing<String>>,
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// Configure automatic Android-format backups: folder, password (kept in the encrypted
    /// store so unattended runs can use it), interval (`off`/`daily`/`weekly`) and how many
    /// files the rotation keeps.
    AndroidBackupSchedule {
        directory: Option<String>,
        password: Option<Zeroizing<String>>,
        interval: Option<String>,
        keep: Option<u32>,
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// The automatic-backup settings and the last run.
    AndroidBackupExportStatus,
    /// All contacts.
    Contacts,
    /// One contact.
    Contact {
        identity: String,
    },
    /// Add a contact by Threema ID (looked up at the directory; unverified).
    AddContact {
        identity: String,
    },
    /// Change a contact's first/last name.
    UpdateContact {
        identity: String,
        first_name: Option<String>,
        last_name: Option<String>,
    },
    /// A contact's own read receipt / typing choice (0 like the app setting, 1 send, 2 don't);
    /// fields left out stay.
    SetContactPolicies {
        identity: String,
        #[serde(default)]
        read_receipts: Option<u8>,
        #[serde(default)]
        typing: Option<u8>,
    },
    /// Android's "Clear individual settings": every contact back to the app settings.
    ClearContactPolicies,
    /// Delete a contact.
    DeleteContact {
        identity: String,
    },
    /// Own profile (ID, nickname, public key).
    Profile,
    /// Change the own nickname.
    /// The user turned down the question for a nickname.
    DeclineNickname,
    SetNickname {
        nickname: String,
    },
    /// App settings (Safe `settings` object).
    Settings,
    /// Change one boolean setting (`readReceipts`, `sendTyping`, `blockUnknown`).
    SetSetting {
        name: String,
        value: bool,
    },
    /// Link an e-mail address to the ID (empty: unlink). Sends the address to Threema, which
    /// mails a confirmation link; the profile then has `linkedEmailPending` (CLAUDE.md §7.4).
    LinkEmail {
        email: String,
    },
    /// Link a phone number (with its country code; empty: unlink). Threema sends an SMS with a
    /// code: `linkedMobilePending` until `verifyMobile`.
    LinkMobile {
        number: String,
    },
    /// What `LinkMobile` would make of a number (`null`: it can't be linked), for showing it
    /// first. No network.
    NormalizeMobile {
        number: String,
    },
    /// The SMS code of a pending phone link.
    VerifyMobile {
        code: String,
        /// From the link in Threema's SMS.
        #[serde(default)]
        from_link: bool,
    },
    /// Have Threema call and read the code instead.
    CallMobile,
    /// Whether a revocation password is set: `{set, setAt (ms or null), checked}`; asks the
    /// directory unless `cached` (then the last known state, `checked: false` if never asked).
    RevocationStatus {
        #[serde(default)]
        cached: bool,
    },
    /// Set the revocation password (8–256 characters; only a 4-byte hash goes to Threema).
    SetRevocationPassword {
        password: String,
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// Delete the ID and all local data.
    DeleteAllData {
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// The chat list; `archived`: the archived chats instead; `private`: the private chats
    /// (only while unlocked). The result also has `archivedCount` and `privateCount`.
    Conversations {
        #[serde(default)]
        archived: bool,
        #[serde(default)]
        private: bool,
    },
    /// What the cover shows, without the list: `{unreadTotal, unreadFrom}` as in `Conversations`.
    UnreadSummary,
    /// `{lock: {keyboard, appLock, graceSecs} | null, unlocked, graceTimes}`
    /// (ADR 0009).
    LockStatus,
    /// Set up or change the lock (`enabled` with a new `pin`, or keeping the current one), or
    /// remove it. If a lock exists, `currentPin` must open it. (Encrypting the stored data has
    /// its own passphrase, kept by the bridge: ADR 0009.)
    SetLock {
        enabled: bool,
        #[serde(default)]
        pin: Option<String>,
        #[serde(default)]
        keyboard: bool,
        #[serde(default)]
        app_lock: bool,
        #[serde(default)]
        grace_secs: u32,
        #[serde(default)]
        current_pin: Option<String>,
    },
    /// Check the PIN. With `privateChats` the private chats become readable (until `lock`);
    /// otherwise it only answers whether the PIN is right (the app lock).
    Unlock {
        pin: String,
        #[serde(default)]
        private_chats: bool,
    },
    /// Lock again (grace time over, private chats left).
    /// Hide the private chats. `pendingFiles` (`{chat: count}`): files already on their way to
    /// this core from a chat being left, which `sendFile` still takes ([`crate::app_media::WritePass`]).
    Lock {
        #[serde(default)]
        pending_files: Option<std::collections::HashMap<String, u32>>,
    },
    /// Mark a chat private or not (needs a lock; unmarking needs the app unlocked).
    SetPrivate {
        identity: String,
        private: bool,
    },
    /// Move a chat to the archive, or back.
    SetArchived {
        identity: String,
        archived: bool,
    },
    /// Mark a chat unread by hand, or take the mark away (Android's MARKED_AS_UNREAD, local
    /// only). Reading the chat or a new message in it clears it too.
    SetConversationUnread {
        identity: String,
        unread: bool,
    },
    /// Multi-device: what the other devices show for this one (empty: the default). Reconnects,
    /// since the label travels in the D2M client-hello.
    SetDeviceLabel {
        label: String,
    },
    /// Multi-device: the name this device shows for another one of the group (empty: its own
    /// label again). Local, like the chat list's own names. The device's hex id is `device`,
    /// not `id`: the request's own `id` sits in the same object (`Request`, flattened), so a
    /// command field of that name would be overwritten by it.
    SetDeviceAlias {
        device: String,
        alias: String,
    },
    /// Messages of a chat, oldest first.
    Messages {
        identity: String,
        limit: Option<u32>,
        /// A window of `limit` messages around this one (local id), e.g. a search hit.
        #[serde(default)]
        around: Option<i64>,
        /// Only the page before this message (local id) in history: scrolling back.
        #[serde(default)]
        before: Option<i64>,
        /// Only the page after this message (local id) in history: scrolling forward again
        /// after the chat let its newest messages go (the page keeps a bounded window).
        #[serde(default)]
        after: Option<i64>,
    },
    /// Search texts, captions, file and place names (all chats, or `identity`'s):
    /// `{results: [message + displayName (chat name)]}`, newest first.
    SearchMessages {
        query: String,
        #[serde(default)]
        identity: Option<String>,
    },
    /// Send a text message.
    /// `quote`: message id (hex) of a message in the same chat to quote.
    SendText {
        identity: String,
        text: String,
        #[serde(default)]
        quote: Option<String>,
    },
    /// A message of a chat by its message id (hex), e.g. a quoted one.
    QuotedMessage {
        identity: String,
        message_id: String,
    },
    /// Mark a chat as read (sends read receipts if enabled).
    MarkRead {
        identity: String,
    },
    /// The user started (`typing: true`) or stopped typing in a chat. Sent only if enabled
    /// (globally or per contact) and while connected; the UI repeats `true` every 10 s.
    SetTyping {
        identity: String,
        typing: bool,
    },
    /// Send a file. The file is attachment 0 (or read from `path` if that is empty), an
    /// optional JPEG thumbnail attachment 1. `rendering`: `file`, `media` or `sticker`.
    SendFile {
        identity: String,
        #[serde(default)]
        path: Option<String>,
        name: Option<String>,
        media_type: String,
        #[serde(default)]
        caption: Option<String>,
        rendering: String,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        /// Seconds (audio: a voice message if `rendering` is `media`).
        #[serde(default)]
        duration: Option<f64>,
        /// `opus`: attachment 0 is 16-bit mono PCM at 48 kHz, to be sent as Ogg Opus.
        #[serde(default)]
        encode: Option<String>,
        /// The user said so: a video to send as media whose metadata can't be taken out goes as
        /// a plain file, as it is. Without this it is refused.
        #[serde(default)]
        keep_metadata: bool,
    },
    /// Save a file message's file (decrypted) into `directory`; returns `{path}`.
    SaveFile {
        message: i64,
        directory: String,
    },
    /// Retry a failed upload.
    RetryUpload {
        message: i64,
    },
    /// All groups.
    Groups,
    /// One group (`key`: its conversation key).
    Group {
        key: String,
    },
    /// Create a group with us as creator.
    CreateGroup {
        name: String,
        members: Vec<String>,
    },
    /// Rename our group.
    RenameGroup {
        key: String,
        name: String,
    },
    /// Set the members of our group.
    SetGroupMembers {
        key: String,
        members: Vec<String>,
    },
    /// Leave a group (the creator disbands it).
    LeaveGroup {
        key: String,
    },
    /// Address book entries to look up on Threema (hashed).
    MatchAddressBook {
        entries: Vec<crate::address_book::Entry>,
        /// The hourly sync, not the user's request.
        #[serde(default)]
        automatic: bool,
    },
    /// Send a location: `latitude`, `longitude` (degrees), optional `accuracy` (m), `name`,
    /// `address`. Returns the outgoing message.
    SendLocation {
        identity: String,
        latitude: f64,
        longitude: f64,
        #[serde(default)]
        accuracy: Option<f64>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        address: Option<String>,
    },
    /// Block (or unblock) a Threema ID: its messages are discarded and we cannot write to it.
    /// Returns the contact JSON if it is a contact, else `{}`.
    SetBlocked {
        identity: String,
        blocked: bool,
    },
    /// The blocked IDs with names: `{blocked: [{identity, displayName}]}`.
    BlockedContacts,
    /// Leave a Threema ID out of address book matching or not (Android's exclusion list).
    SetSyncExcluded {
        identity: String,
        excluded: bool,
    },
    /// The exclusion list: `{excluded: [{identity, displayName}]}`.
    SyncExcluded,
    /// Look the contacts up again now (states, keys, feature masks), not only once a day.
    RefreshContacts,
    /// A new distribution list (members: Threema IDs of contacts).
    CreateDistributionList {
        name: String,
        members: Vec<String>,
    },
    DistributionList {
        key: String,
    },
    /// Rename it and/or change its members.
    UpdateDistributionList {
        key: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        members: Option<Vec<String>>,
    },
    /// Pin a chat to the top of the chat list (at the end of the pinned ones), or unpin it.
    SetPinned {
        identity: String,
        pinned: bool,
    },
    /// Move a pinned chat up (negative `offset`) or down.
    MovePinned {
        identity: String,
        offset: i64,
    },
    /// Send a message (text or file) again to another chat; returns the new message.
    ForwardMessage {
        message: i64,
        identity: String,
    },
    /// The unsent text of a chat's input (empty: none), with its mentions as `@[ID]` and the
    /// message it quotes (message id, hex), kept like Android's drafts; returned as `draft` and
    /// `draftMentions` in `conversations` and by `draft`. A send of the same text takes it away.
    SetDraft {
        identity: String,
        text: String,
        #[serde(default)]
        quote: Option<String>,
    },
    Draft {
        identity: String,
    },
    /// A page of a chat's media gallery, newest first: `{messages (with mediaKind), kinds, counts,
    /// next}`. `kinds` filters (default: the saved filter, see `setMediaKinds`); `before` is the
    /// `next` of the page before.
    ChatMedia {
        identity: String,
        #[serde(default)]
        kinds: Option<Vec<String>>,
        #[serde(default)]
        limit: Option<u32>,
        #[serde(default)]
        before: Option<String>,
    },
    /// The media gallery's filter (kinds of `MEDIA_KINDS`), saved for all chats like Android.
    SetMediaKinds {
        kinds: Vec<String>,
    },
    /// One message by local id (message details), or null.
    Message {
        message: i64,
    },
    /// Star a message (or not): local, like Android's starred messages.
    SetStarred {
        message: i64,
        starred: bool,
    },
    /// A voice message was played: remember that it is not new any more (local).
    SetVoiceListened {
        message: i64,
    },
    /// Starred messages, newest first (all chats, or `identity`'s), like search results.
    StarredMessages {
        #[serde(default)]
        identity: Option<String>,
    },
    /// Several, in the given order: `{forwarded, failed, errors}`, `errors` being the kinds of
    /// error that came up, each once.
    ForwardMessages {
        messages: Vec<i64>,
        identity: String,
    },
    /// A chat's notifications: `policy` `never` (muted; until `until` ms if set) or
    /// `mentioned` (groups), none for on. Returns `{policy, until}`.
    SetChatNotifications {
        identity: String,
        policy: Option<String>,
        until: Option<i64>,
    },
    /// The chats with a tone of their own, `{tones: {key: path}}`; hidden private chats not.
    ChatTones,
    /// A chat's own tone; `tone` empty or none: the tone of all chats.
    SetChatTone {
        identity: String,
        tone: Option<String>,
    },
    /// Tones the app kept in its settings file before: taken over whatever the lock's state.
    ImportChatTones {
        tones: std::collections::HashMap<String, String>,
    },
    /// A Threema link (`threema://…`, `threema.id/…`): what it asks for (`links::Link`).
    ParseLink {
        url: String,
    },
    /// Our contact QR code.
    MyQrCode,
    /// A scanned contact QR code: verify (and add) the contact.
    VerifyQrCode {
        text: String,
    },
    /// What a scanned QR code is and whether it can be used (docs/ffi.md).
    QrKind {
        text: String,
    },
    /// Our profile picture (attachment 0: JPEG).
    SetProfilePicture,
    RemoveProfilePicture,
    /// Who gets our profile picture: `everyone`, `nobody` or `selected` (with `recipients`, the
    /// new allow list, if given).
    SetProfilePictureShare {
        share: String,
        #[serde(default)]
        recipients: Option<Vec<String>>,
    },
    /// Put a contact on the profile picture allow list (`share`) or take it off.
    SetProfilePictureRecipient {
        identity: String,
        share: bool,
    },
    /// Create a poll in a conversation.
    CreatePoll {
        identity: String,
        description: String,
        choices: Vec<String>,
        multiple: bool,
        announce: bool,
    },
    /// Vote on poll message `message` (local id) with these choice ids.
    Vote {
        message: i64,
        choices: Vec<i64>,
    },
    /// Close our poll message `message`.
    ClosePoll {
        message: i64,
    },
    /// Set our group's picture (attachment 0: JPEG).
    SetGroupPicture {
        key: String,
    },
    /// Remove our group's picture.
    RemoveGroupPicture {
        key: String,
    },
    /// Download a file message's file (or retry its thumbnail).
    /// (`message` is the local message id: `id` is the request's.)
    DownloadFile {
        message: i64,
    },
    /// Change the text of one of our text messages.
    EditMessage {
        identity: String,
        message_id: String,
        text: String,
    },
    /// Delete a message for everyone (ours only) or just on this device.
    DeleteMessage {
        identity: String,
        message_id: String,
        outgoing: bool,
        everyone: bool,
    },
    /// Apply (`apply: true`) or withdraw an emoji reaction to message `messageId` (hex).
    React {
        identity: String,
        message_id: String,
        emoji: String,
        apply: bool,
    },
    /// Delete a chat's messages.
    DeleteConversation {
        identity: String,
    },
    /// The network changed (bridge): `wifi` for Wi-Fi or Ethernet. Starts downloads that the
    /// new network allows.
    SetNetwork {
        wifi: bool,
    },
    /// Settings → Appearance: `{lastNameFirst, sortByLastName, showInactive}`.
    DisplaySettings,
    SetDisplaySettings {
        #[serde(default)]
        last_name_first: Option<bool>,
        #[serde(default)]
        sort_by_last_name: Option<bool>,
        #[serde(default)]
        show_inactive: Option<bool>,
    },
    /// Multi-device (Threema Desktop): `{active}`.
    MultiDevice,
    /// The device group's devices: `{devices: [{id, current, label, platform, ...}]}`.
    LinkedDevices,
    /// Remove devices (hex ids) from the device group; returns the remaining devices.
    DropDevices {
        ids: Vec<String>,
    },
    /// Remove all devices including this one: multi-device off. With `leaveLocally` this device
    /// only leaves on its own: the others stay linked.
    DeactivateMultiDevice {
        #[serde(default)]
        leave_locally: bool,
    },
    /// Linking Threema Desktop: the scanned code; connects to it and returns
    /// `{choices: [[emoji ×3] ×3], correct}` for the emoji check.
    StartLinking {
        code: String,
        /// The app lock's PIN (`App::check_pin`).
        pin: Option<Zeroizing<String>>,
    },
    /// The user picked the right emojis: multi-device on (if it isn't), send the data. Returns
    /// the linked devices once the new device has registered.
    ConfirmLinking,
    CancelLinking,
    /// Calls (ADR 0012, see app_calls.rs and docs/ffi.md).
    CallTurnServers {
        identity: String,
    },
    CallStart {
        identity: String,
        sdp: String,
        #[serde(default)]
        video: bool,
    },
    CallAccept {
        call_id: u32,
        sdp: String,
        #[serde(default)]
        video: bool,
    },
    CallReject {
        call_id: u32,
    },
    CallHangUp {
        call_id: u32,
    },
    CallFailed {
        call_id: u32,
        reason: String,
    },
    CallTimeout {
        call_id: u32,
    },
    CallCandidates {
        call_id: u32,
        candidates: Vec<crate::calls::Candidate>,
    },
    CallState,
    CallConnected {
        #[serde(default)]
        relayed: bool,
    },
    CallChannel {
        data: String,
    },
    CallCapture {
        device: String,
        on: bool,
    },
    SetVideoQuality {
        quality: String,
    },
    /// The country choice of address book numbers without a country code.
    AddressBookCountry,
    /// `country`: a region of the list, or empty for automatic.
    SetAddressBookCountry {
        country: String,
    },
    GroupCallRefresh {
        conversation: String,
    },
    GroupCallJoin {
        conversation: String,
        /// Hex SHA-256 of our DTLS certificate.
        fingerprint: String,
        /// Start a call if none runs.
        #[serde(default)]
        create: bool,
    },
    GroupCallChannel {
        data: String,
    },
    GroupCallLeave,
    GroupCallMute {
        muted: bool,
    },
    GroupCallCamera {
        on: bool,
    },
    /// Every second while in a group call: rekeys, call state updates, the announcement.
    GroupCallTick,
    /// Threema Web (ADR 0013), see app_web.rs (`session`: `id` is the request's own).
    WebSessions,
    WebSetEnabled {
        enabled: bool,
    },
    WebPair {
        qr: String,
        #[serde(default)]
        allow_host: bool,
    },
    WebStart {
        session: i64,
    },
    WebStop {
        session: i64,
    },
    WebDelete {
        session: i64,
    },
    WebAcceptCertificate {
        session: i64,
    },
    WebDeclineCertificate {
        session: i64,
    },
    WebRemoveServer {
        host: String,
    },
    /// Favourite GIFs: `{favourites: [{id, width, height}]}`, newest first.
    GifFavourites,
    /// The emoji picker's recently used emoji (`{emoji: [text]}`, newest first) / use one.
    RecentEmoji,
    UseEmoji {
        emoji: String,
    },
    /// Keep a GIF: from a message, or from the device (attached GIF and thumbnail, or `path`).
    /// Returns `{id, favourites}`.
    AddGifFavourite {
        #[serde(default)]
        message: Option<i64>,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
    },
    /// (Not `id`: that is the request's.)
    RemoveGifFavourite {
        favourite: i64,
    },
    /// Send a favourite to a chat (like `sendFile`).
    SendGifFavourite {
        identity: String,
        favourite: i64,
    },
    /// Settings → Advanced, and once after setup (ADR 0014): `{available (this build can),
    /// enabled (null: not asked yet), checkedAt, current, latest: {version, url} | null}`;
    /// `latest` only if newer than this version.
    UpdateSettings,
    /// Check for updates or not; on: checks at once. Returns the update settings.
    SetUpdateCheck {
        enabled: bool,
    },
    /// "Check now": starts a check (event `updateChecked` when done); returns the update
    /// settings, with `checking`.
    CheckForUpdate,
    /// Settings → Advanced: how much is logged (`warn`, `info`, `debug`; device-local, kept by
    /// the bridge and passed in the start configuration too).
    /// Settings → Image size, for pictures the browser sends: the longest side in pixels, 0 for as
    /// large as they are. The UI has the setting and tells us when we start and when it changes;
    /// until then the default holds.
    SetImageSize {
        side: u32,
    },
    SetLogLevel {
        level: String,
    },
    /// Settings → Advanced: `{on, multiDevice}` — forward security (forced off while
    /// multi-device is active, SE-199).
    ForwardSecurity,
    /// Switch forward security (user decision 2026-10-28: an escape when session churn
    /// floods contacts with security notices). Off terminates every session
    /// (DisabledByLocal) and the feature mask drops the FS bit; messages stay end-to-end
    /// encrypted either way.
    SetForwardSecurity {
        on: bool,
    },
    /// The in-app log as text (Settings → Advanced → Log; bounded tail).
    LogText,
    /// Write a support package (device facts and the log) into `directory`; `{path}`.
    LogPackage {
        directory: String,
    },
    /// Remove the in-app log files.
    LogClear,
    /// Settings → Advanced: `{ipv6}` (Android's "IPv6 for messages", on by default here).
    NetworkSettings,
    /// Change it; the chat server connection starts again with the other hosts.
    SetNetworkSettings {
        ipv6: bool,
    },
    /// Settings → Media & storage: `{autoDownloadWifi, autoDownloadMobile, autoSave}`.
    MediaSettings,
    /// Change any of them (lists of `image`, `audio`, `video`, `file`).
    SetMediaSettings {
        #[serde(default)]
        auto_download_wifi: Option<Vec<String>>,
        #[serde(default)]
        auto_download_mobile: Option<Vec<String>>,
        /// Largest file that downloads by itself, in MiB (0: any size).
        #[serde(default)]
        auto_download_limit: Option<u32>,
        #[serde(default)]
        auto_save: Option<bool>,
        #[serde(default)]
        auto_delete_days: Option<u32>,
    },
    /// `{mediaBytes, databaseBytes}`.
    /// Write one chat as a ZIP into `directory` (Android's "Export chat"): `messages.txt` and,
    /// with `media`, the pictures and files that are on the device. `me` is the UI's word for
    /// our own messages, `offsetMinutes` the device's distance from UTC.
    #[serde(rename_all = "camelCase")]
    ExportChat {
        identity: String,
        #[serde(default)]
        media: bool,
        directory: String,
        #[serde(default)]
        password: Option<Zeroizing<String>>,
        #[serde(default)]
        me: Option<String>,
        #[serde(default)]
        offset_minutes: Option<i64>,
    },
    StorageInfo,
    /// Emulator build only (feature `demo`): fill the store with made-up chats, or remove them
    /// again. Nothing is sent and nothing leaves the device.
    #[cfg(feature = "demo")]
    #[serde(rename_all = "camelCase")]
    DemoData {
        #[serde(default)]
        remove: bool,
        /// How long the demo chat with a long history is (default 400).
        #[serde(default)]
        messages: Option<u32>,
    },
    /// Delete media (`messages: false`) or whole messages older than `days` (0: all), in every
    /// chat; starred messages are kept. `{removed}`.
    CleanUp {
        days: u32,
        #[serde(default)]
        messages: bool,
    },
    /// Delete a chat's messages but keep it in the list (Android's "Empty chat").
    EmptyConversation {
        identity: String,
    },
    /// Current connection state.
    ConnectionState,
    /// "Connect now": reconnect now if disconnected, also while a server asked us to stay away.
    Reconnect,
    /// The same without the user asking: timer, network change, back in the app.
    Wake,
}

#[derive(Deserialize)]
struct Request {
    /// The UI's request number; the reply carries it back. The command's own fields sit in the
    /// same object, so none of them may be called `id` or `cmd` (the bridge warns if one is).
    id: u64,
    #[serde(flatten)]
    command: Command,
}

/// Errors shown to the user. `kind` is stable and mapped to translated text in QML.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// No license stored yet.
    #[error("no license")]
    NoLicense,
    /// No identity yet.
    #[error("no identity")]
    NoIdentity,
    /// License key has the wrong format.
    #[error("license key format")]
    LicenseFormat,
    /// Server rejected the request with a (possibly localized) message.
    #[error("{0}")]
    Server(String),
    /// Network/TLS problem.
    #[error("{0}")]
    Network(String),
    /// A Threema server answered with an unexpected status or something unreadable. The text is
    /// for the log's kind of reader; the UI has its own.
    #[error("{0}")]
    ServerTrouble(String),
    /// Wrong backup password or damaged backup.
    #[error("backup could not be decrypted")]
    BackupDecrypt,
    /// An Android data backup in a format this importer does not know (the version).
    #[error("backup format version {0}")]
    BackupVersion(i64),
    /// An Android data backup of another identity (or the same ID with another key): never
    /// merged into this one.
    #[error("backup belongs to another identity")]
    BackupOtherIdentity,
    /// An Android data backup that cannot be read (damaged, truncated, not a backup).
    #[error("{0}")]
    BackupInvalid(String),
    /// A data backup that could not be written into its folder.
    #[error("{0}")]
    BackupWrite(String),
    /// The folder's storage has no room for a data backup.
    #[error("no room for the backup")]
    BackupNoRoom,
    /// A data backup is being written already (one at a time).
    #[error("a backup is already running")]
    BackupRunning,
    /// A data backup needs a folder, and none was given or stored.
    #[error("no backup folder chosen")]
    BackupNoFolder,
    /// A data backup needs a password, and none was given or stored.
    #[error("no backup password")]
    BackupNoPassword,
    /// Password too short.
    #[error("password too short")]
    PasswordTooShort,
    /// Password too long.
    #[error("password too long")]
    PasswordTooLong,
    /// No Threema Safe backup for this ID/password.
    #[error("no Threema Safe backup found")]
    SafeNotFound,
    /// Threema Safe backup larger than the server allows.
    #[error("{0}")]
    SafeTooLarge(String),
    /// A Threema Safe backup that decrypts but can't be read.
    #[error("{0}")]
    SafeDamaged(String),
    /// Creating or restoring an ID while this device has one (removing it needs the PIN).
    #[error("this device already has an ID")]
    IdentityExists,
    /// Threema ID has the wrong format.
    #[error("invalid Threema ID")]
    IdentityFormat,
    /// The ID does not exist or was revoked.
    #[error("this Threema ID does not exist or was revoked")]
    ContactInvalid,
    /// The ID is the user's own.
    #[error("this is your own Threema ID")]
    ContactIsSelf,
    /// No such contact.
    #[error("no such contact")]
    ContactNotFound,
    /// Message text too long.
    #[error("message too long")]
    MessageTooLong,
    /// Closing this poll would not fit a message, with everyone's votes in it.
    #[error("poll results too long")]
    PollResultsTooLong,
    /// Not a Threema contact QR code.
    #[error("not a Threema contact code")]
    QrInvalid,
    #[error("not a Threema link")]
    LinkInvalid,
    #[error("this contact is blocked")]
    ContactBlocked,
    #[error("this contact's ID was revoked")]
    ContactRevoked,
    /// "Block unknown senders" is on, and this is neither a direct contact nor in a group with us.
    #[error("this person is blocked as an unknown sender")]
    ContactUnknown,
    #[error("the file has not been downloaded")]
    FileNotDownloaded,
    #[error("this message cannot be forwarded")]
    NotForwardable,
    /// The code's key isn't the contact's: not the right person or device.
    #[error("the code does not match this contact")]
    QrMismatch,
    /// The contact code carries an expiry that has passed.
    #[error("the code has expired")]
    QrExpired,
    /// No such group.
    #[error("no such group")]
    GroupNotFound,
    /// The group was left (or we were removed).
    #[error("not a member of this group any more")]
    GroupLeft,
    /// Only the group's creator can do this.
    #[error("only the group creator can do this")]
    NotGroupCreator,
    /// File larger than Threema's blob limit (100 MB).
    #[error("file too large")]
    FileTooLarge,
    /// Linking: not a Threema Desktop code / a Threema Web code / an app version we can't link.
    #[error("not a device linking code")]
    LinkUnknownCode,
    #[error("Threema Web code")]
    LinkWebCode,
    #[error("unsupported version")]
    LinkVersion,
    /// Linking failed on the way (network, the other device).
    #[error("linking failed: {0}")]
    LinkFailed(String),
    /// Linking failed with the ID already handed over; the device group was removed again.
    #[error("linking failed after the handover; the device group was removed")]
    LinkRemoved,
    /// Linking failed with the ID already handed over, and the new device may be in the group.
    #[error("linking failed after the handover; the new device may be linked")]
    LinkUncertain,
    /// A video to send as media whose metadata can't be taken out (docs/decisions/0025): only with
    /// `keepMetadata`, as a plain file.
    #[error("the video keeps its metadata")]
    VideoKeepsMetadata,
    /// A picture someone else sent, to forward as media, whose metadata can't be taken out.
    #[error("the picture keeps its metadata")]
    PictureKeepsMetadata,
    /// Only GIFs can be favourite GIFs.
    #[error("not a GIF")]
    NotGif,
    /// Group calls: none running to join / the call is full / its protocol version is too new.
    #[error("no group call running")]
    GroupCallNotRunning,
    #[error("group call full")]
    GroupCallFull,
    #[error("group call version not supported")]
    GroupCallVersion,
    /// Threema Web: not its code / another protocol version / no such session / this browser is
    /// already in the list / switched off.
    #[error("not a Threema Web code")]
    WebCodeInvalid,
    #[error("Threema Web version not supported")]
    WebVersion,
    #[error("no such Threema Web session")]
    WebSessionNotFound,
    #[error("this browser already has a Threema Web session")]
    WebAlreadyPaired,
    #[error("Threema Web is off")]
    WebDisabled,
    /// The contact's app can't do this (edit or delete for everyone).
    #[error("not supported by the contact")]
    NotSupportedByContact,
    /// Too late to edit or delete for everyone (6 hours).
    #[error("message too old")]
    MessageTooOld,
    /// The contact's app only understands thumbs up/down reactions (and none can be withdrawn).
    #[error("reaction not supported by the contact")]
    ReactionNotSupported,
    /// Private chats can't be read while the app is locked.
    #[error("locked")]
    Locked,
    /// Not a phone number we can link.
    #[error("invalid phone number")]
    PhoneInvalid,
    /// Not an e-mail address.
    #[error("invalid e-mail address")]
    EmailInvalid,
    /// No phone number is waiting for its code.
    #[error("no phone number to verify")]
    NoVerification,
    /// Wrong PIN or passphrase.
    #[error("wrong PIN")]
    WrongPin,
    /// Too many wrong PINs: no PIN is checked for this many seconds.
    #[error("{0}")]
    PinLockout(i64),
    /// Too short a PIN or passphrase.
    #[error("PIN too short")]
    PinTooShort,
    /// Private chats need a lock to be set up first.
    #[error("no lock set up")]
    LockNeeded,
    /// The lock can't be removed while there are private chats.
    #[error("there are private chats")]
    PrivateChatsExist,
    /// The directory said too many requests.
    #[error("rate limited")]
    RateLimited,
    /// The directory said no and gave no reason.
    #[error("the server gave no reason")]
    ServerUnexplained,
    /// The directory didn't take a spam report.
    #[error("report not accepted")]
    ReportRefused,
    /// The encrypted store failed in a way the UI tells apart (`StoreError::kind`). The message
    /// is only a code for support.
    #[error("{code}")]
    StoreFailed { kind: &'static str, code: String },
    /// Bad request from the UI or internal failure.
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::NoLicense => "noLicense",
            Self::NoIdentity => "noIdentity",
            Self::LicenseFormat => "licenseFormat",
            Self::Server(_) => "server",
            Self::Network(_) => "network",
            Self::ServerTrouble(_) => "serverTrouble",
            Self::BackupDecrypt => "backupDecrypt",
            Self::BackupVersion(_) => "backupVersion",
            Self::BackupOtherIdentity => "backupIdentity",
            Self::BackupInvalid(_) => "backupInvalid",
            Self::BackupWrite(_) => "backupWrite",
            Self::BackupNoRoom => "backupNoRoom",
            Self::BackupRunning => "backupRunning",
            Self::BackupNoFolder => "backupNoFolder",
            Self::BackupNoPassword => "backupNoPassword",
            Self::PasswordTooShort => "passwordTooShort",
            Self::PasswordTooLong => "passwordTooLong",
            Self::SafeNotFound => "safeNotFound",
            Self::SafeTooLarge(_) => "safeTooLarge",
            Self::SafeDamaged(_) => "safeDamaged",
            Self::IdentityFormat => "identityFormat",
            Self::ContactInvalid => "contactInvalid",
            Self::ContactIsSelf => "contactIsSelf",
            Self::ContactNotFound => "contactNotFound",
            Self::MessageTooLong => "messageTooLong",
            Self::PollResultsTooLong => "pollResultsTooLong",
            Self::FileTooLarge => "fileTooLarge",
            Self::VideoKeepsMetadata => "videoKeepsMetadata",
            Self::PictureKeepsMetadata => "pictureKeepsMetadata",
            Self::NotGif => "notGif",
            Self::GroupCallNotRunning => "groupCallNotRunning",
            Self::GroupCallFull => "groupCallFull",
            Self::GroupCallVersion => "groupCallVersion",
            Self::WebCodeInvalid => "webCodeInvalid",
            Self::WebVersion => "webVersion",
            Self::WebSessionNotFound => "webSessionNotFound",
            Self::WebAlreadyPaired => "webAlreadyPaired",
            Self::WebDisabled => "webDisabled",
            Self::LinkUnknownCode => "linkUnknownCode",
            Self::LinkWebCode => "linkWebCode",
            Self::LinkVersion => "linkVersion",
            Self::LinkFailed(_) => "linkFailed",
            Self::LinkRemoved => "linkRemoved",
            Self::LinkUncertain => "linkUncertain",
            Self::GroupNotFound => "groupNotFound",
            Self::QrInvalid => "qrInvalid",
            Self::LinkInvalid => "linkInvalid",
            Self::FileNotDownloaded => "fileNotDownloaded",
            Self::ContactBlocked => "contactBlocked",
            Self::ContactRevoked => "contactRevoked",
            Self::ContactUnknown => "contactUnknown",
            Self::NotForwardable => "notForwardable",
            Self::QrMismatch => "qrMismatch",
            Self::QrExpired => "qrExpired",
            Self::GroupLeft => "groupLeft",
            Self::NotGroupCreator => "notGroupCreator",
            Self::ReactionNotSupported => "reactionNotSupported",
            Self::NotSupportedByContact => "notSupportedByContact",
            Self::MessageTooOld => "messageTooOld",
            Self::Locked => "locked",
            Self::WrongPin => "wrongPin",
            Self::PinLockout(_) => "pinLockout",
            Self::PhoneInvalid => "phoneInvalid",
            Self::EmailInvalid => "emailInvalid",
            Self::NoVerification => "noVerification",
            Self::PinTooShort => "pinTooShort",
            Self::LockNeeded => "lockNeeded",
            Self::IdentityExists => "identityExists",
            Self::PrivateChatsExist => "privateChatsExist",
            Self::RateLimited => "rateLimited",
            Self::ServerUnexplained => "serverUnexplained",
            Self::ReportRefused => "reportRefused",
            Self::StoreFailed { kind, .. } => kind,
            Self::Internal(_) => "internal",
        }
    }

    /// Opening the store: whatever the reason, it is why the app doesn't start.
    fn store_failed(error: &StoreError) -> Self {
        Self::StoreFailed {
            kind: error.kind(),
            code: error.code(),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        if error.kind() == "store" {
            Self::Internal(error.to_string())
        } else {
            Self::store_failed(&error)
        }
    }
}

impl From<DirectoryError> for ApiError {
    fn from(error: DirectoryError) -> Self {
        match error {
            DirectoryError::Network(e) => Self::Network(e.to_string()),
            DirectoryError::Server(message) => Self::Server(message),
            DirectoryError::RateLimited => Self::RateLimited,
            DirectoryError::Unexplained => Self::ServerUnexplained,
            DirectoryError::ReportRefused => Self::ReportRefused,
            other @ (DirectoryError::Status(_) | DirectoryError::InvalidResponse(_)) => {
                Self::ServerTrouble(other.to_string())
            }
        }
    }
}

/// `XXXXX-XXXXX`, A–Z and 0–9 (`EnterSerialActivity.java`). Input is normalized to upper case.
pub fn normalize_license_key(input: &str) -> Option<String> {
    let key = input.trim().to_ascii_uppercase();
    let bytes = key.as_bytes();
    let valid = bytes.len() == 11
        && bytes[5] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 5 || b.is_ascii_uppercase() || b.is_ascii_digit());
    valid.then_some(key)
}

/// Up to 0.2.0 a busy directory, or one that gave no reason, counted as refusing the license, and
/// that stuck until the user checked the key. Those were our words, never the directory's.
fn forget_false_refusal(store: &Store) -> Result<(), StoreError> {
    const NOT_REFUSALS: [&str; 2] = [
        "Too many requests, please try again later.",
        "Unknown server error",
    ];
    if store
        .setting(keys::LICENSE_REFUSED)?
        .is_some_and(|refused| NOT_REFUSALS.contains(&refused.as_str()))
    {
        store.delete_setting(keys::LICENSE_REFUSED)?;
    }
    Ok(())
}

/// What creating an ID says to the user. libthreema flattens the directory's own words into
/// `ServerError` along with every other server problem, so its prefix is what tells them apart
/// (a test keeps an eye on it). Only those words are for the user to read.
fn create_identity_error(error: CspE2eProtocolError) -> ApiError {
    const DIRECTORYS_WORDS: &str = "Custom (possibly localized) error from server: ";
    match error {
        CspE2eProtocolError::NetworkError(message) => ApiError::Network(message),
        CspE2eProtocolError::RateLimitExceeded => ApiError::RateLimited,
        CspE2eProtocolError::ServerError(message) => match message.strip_prefix(DIRECTORYS_WORDS) {
            Some(words) => ApiError::Server(words.to_owned()),
            None => ApiError::Internal(message),
        },
        other => ApiError::Internal(other.to_string()),
    }
}

fn random_device_id() -> String {
    // UUID v4 formatting, like Android's `UUID.randomUUID()`.
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = data_encoding::HEXLOWER.encode(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The part of the core that a command or a round of maintenance keeps using while it waits for
/// the network, beside the core loop: tasks hold on to this, and whatever else they want done
/// comes back to the loop (`crate::detached`). Everything in it is good for use at any `.await`,
/// as no borrow of its cells is held across one (clippy's `await_holding_refcell_ref`).
pub struct Shared {
    pub(crate) store: std::rc::Rc<Store>,
    pub(crate) https: HttpsClient,
    pub(crate) version: ClientVersion,
    pub(crate) config: Arc<Config>,
    /// Channel to the running chat server connection, if any.
    pub(crate) connection: std::cell::RefCell<
        Option<tokio::sync::mpsc::UnboundedSender<crate::connection::ToConnection>>,
    >,
    /// Last known connection state, for `connectionState`.
    pub connection_state: std::cell::Cell<&'static str>,
    /// Encrypted files of messages (shared with the bridge's image provider).
    pub(crate) media: Arc<crate::media::MediaStore>,
    /// Media downloads report their progress here (into the core loop).
    pub(crate) media_events: tokio::sync::mpsc::UnboundedSender<crate::providers::CoreEvent>,
    /// Contacts the request being handled changed beyond what its command says of itself, such as
    /// making them direct, for the other devices.
    pub(crate) made_direct: std::cell::RefCell<Vec<String>>,
    /// Contacts this command made that the other devices don't have yet: they go as a `Create`,
    /// as those devices ignore an update of someone unknown (`ReflectedContactSyncTask`).
    pub(crate) new_contacts: std::cell::RefCell<Vec<String>>,
    /// Chats a message brought back from the archive during the current command.
    pub(crate) unarchived: std::cell::RefCell<Vec<String>>,
    /// This build has a media engine (ADR 0012).
    pub(crate) calls_supported: bool,
    /// Group calls: the SFU credentials while they last.
    pub(crate) sfu: std::rc::Rc<std::cell::RefCell<crate::app_group_calls::SfuCache>>,
    pub(crate) group_calls_watched:
        std::rc::Rc<std::cell::RefCell<std::collections::HashSet<String>>>,
    /// Cached TURN servers, with their expiry (ms).
    pub(crate) turn_servers: std::cell::RefCell<Option<(crate::directory::TurnServers, i64)>>,
    /// Device linking between the scan and the emoji confirmation.
    pub(crate) pending_link: std::cell::RefCell<Option<crate::rendezvous::RendezvousLink>>,
    /// Events for the UI caused by handling a command (e.g. a Safe import the command ran);
    /// the core loop emits and clears them after every command.
    pub(crate) pending_ui_events: std::cell::RefCell<Vec<&'static str>>,
    /// When the pending Safe import was last tried automatically (reconnect rate limit).
    pub(crate) last_import_attempt: std::cell::Cell<Option<tokio::time::Instant>>,
    /// When this device last looked whether it is alone in its device group.
    pub(crate) last_group_check: std::cell::Cell<Option<tokio::time::Instant>>,
    /// Private chats readable (ADR 0009); memory only, starts locked.
    pub(crate) unlocked: std::cell::Cell<bool>,
    /// The app's data directory (store, media, the in-app log).
    pub(crate) data_dir: PathBuf,
    /// A data backup added contacts: what they support is unknown until the next refresh.
    pub(crate) imported_contacts_unchecked: std::cell::Cell<bool>,
    /// In the core loop, from the UI: what waits or takes long goes beside the loop. Others
    /// asking the same (Threema Web) need the answer in place.
    pub(crate) in_background: std::cell::Cell<bool>,
}

impl core::ops::Deref for App {
    type Target = Shared;

    fn deref(&self) -> &Shared {
        &self.shared
    }
}

/// The core. Lives on the core thread only (libthreema state is not `Send`).
pub struct App {
    /// What work beside the core loop (a task with a network wait) needs as well.
    pub(crate) shared: std::rc::Rc<Shared>,
    /// Its task, to stop it wherever it is.
    pub(crate) connection_task: Option<tokio::task::JoinHandle<()>>,
    /// When maintenance last ran, by the clock.
    pub(crate) maintenance_ran_at: std::time::SystemTime,
    /// A wake-up found it overdue.
    pub(crate) maintenance_overdue: bool,
    /// Uploads and downloads running.
    pub(crate) transfers: crate::app_media::RunningTransfers,
    /// A wake-up is over but for those: `wakeSettled` follows when they have ended.
    pub(crate) wake_settled_held: bool,
    media_events_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::providers::CoreEvent>>,
    /// Files of hidden private chats still on their way to the core.
    pub(crate) write_pass: Option<crate::app_media::WritePass>,
    /// Binary attachments of the request being handled (e.g. `sendFile`: file, thumbnail).
    pub(crate) attachments: Vec<zeroize::Zeroizing<Vec<u8>>>,
    /// The request being handled.
    pub(crate) current_request: u64,
    /// What it changes for the other devices.
    pub(crate) current_change: Option<crate::app_multidevice::SyncChange>,
    /// Commands and maintenance waiting for the network beside the core loop.
    pub(crate) detached: crate::detached::DetachedWork,
    /// How many times we left a group call: a join that was under way is no use by now.
    pub(crate) group_call_leaves: u64,
    /// The groups whose call we are joining, with the SFU still to answer.
    pub(crate) group_calls_joining: Vec<String>,
    /// Settings → Image size (`setImageSize`).
    pub(crate) image_size: u32,
    /// The command handled went to a blocking thread: its reply comes later.
    pub(crate) reply_later: bool,
    /// How many of those are still to be finished.
    pub(crate) file_work_running: usize,
    /// One at a time.
    pub(crate) file_work_turn: std::rc::Rc<tokio::sync::Semaphore>,
    pub(crate) file_work_done:
        tokio::sync::mpsc::UnboundedSender<crate::app_media::FinishedFileWork>,
    file_work_done_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::app_media::FinishedFileWork>>,
    /// The build may check for updates (ADR 0014); a check is under way.
    pub(crate) update_check_built: bool,
    pub(crate) update_check_running: std::rc::Rc<std::cell::Cell<bool>>,
    /// The user asked for the running check, by starting it or while it ran.
    pub(crate) update_check_manual: std::rc::Rc<std::cell::Cell<bool>>,
    pub(crate) calls: crate::calls::Calls,
    pub(crate) call_patcher: Option<crate::sdp::SdpPatcher>,
    pub(crate) call_media: crate::app_calls::CallMedia,
    pub(crate) group_call: Option<crate::app_group_calls::ActiveGroupCall>,
    /// Threema Web sessions running (ADR 0013).
    pub(crate) web: crate::app_web::WebSessions,
    /// A running Android history import's cancel flag (set true when it ends).
    pub(crate) android_import: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    /// A running Android-format export's done flag.
    pub(crate) android_export: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    /// Tells the running export to give up (the ID is being removed).
    pub(crate) android_export_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// When the core loop next looks for failed transfers to try again.
    pub(crate) transfer_retry_at: Option<tokio::time::Instant>,
    /// On an unmetered network (Wi-Fi/Ethernet, from the bridge): auto-download policy.
    pub(crate) on_wifi: bool,
    pub(crate) pictures_dir: Option<PathBuf>,
    /// `deleteAllData` went through: this core only waits for the bridge to stop it.
    pub(crate) identity_removed: bool,
}

impl App {
    /// Open the store and set up clients.
    ///
    /// # Errors
    /// Store or TLS setup failures.
    pub fn new(start: &StartConfig, db_key: &[u8; 32]) -> Result<Self, ApiError> {
        std::fs::create_dir_all(&start.data_dir).map_err(|e| ApiError::Internal(e.to_string()))?;
        let media = crate::media::MediaStore::open(&start.data_dir, db_key)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Self::with_media(start, db_key, Arc::new(media), &|| {})
    }

    /// Like [`Self::new`] with a media store the caller shares (with the bridge). `upgrading` is
    /// called before the store's schema is brought up to date, which can take a while.
    ///
    /// # Errors
    /// Store or TLS setup failures.
    pub fn with_media(
        start: &StartConfig,
        db_key: &[u8; 32],
        media: Arc<crate::media::MediaStore>,
        upgrading: &dyn Fn(),
    ) -> Result<Self, ApiError> {
        std::fs::create_dir_all(&start.data_dir).map_err(|e| ApiError::Internal(e.to_string()))?;
        let (media_events, media_events_rx) = tokio::sync::mpsc::unbounded_channel();
        let (file_work_done, file_work_done_rx) = tokio::sync::mpsc::unbounded_channel();
        let store = Store::open_with(&start.data_dir.join("threesea.db"), db_key, upgrading)
            .map(std::rc::Rc::new)
            .map_err(|error| ApiError::store_failed(&error))?;
        forget_false_refusal(&store).map_err(|error| ApiError::store_failed(&error))?;
        let shared = Shared {
            store,
            https: HttpsClient::new().map_err(|e| ApiError::Internal(e.to_string()))?,
            version: ClientVersion {
                app_version: start.app_version.clone(),
                language: start.language.clone(),
                country: start.country.clone(),
                model: start.model.clone(),
                os_version: start.os_version.clone(),
                arch: std::env::consts::ARCH.to_owned(),
            },
            config: Arc::new(Config::from(ConfigEnvironment::Production)),
            connection: std::cell::RefCell::default(),
            connection_state: std::cell::Cell::new("disconnected"),
            media,
            media_events,
            made_direct: std::cell::RefCell::default(),
            new_contacts: std::cell::RefCell::default(),
            unarchived: std::cell::RefCell::default(),
            calls_supported: start.calls,
            sfu: std::rc::Rc::default(),
            group_calls_watched: std::rc::Rc::default(),
            turn_servers: std::cell::RefCell::default(),
            pending_link: std::cell::RefCell::default(),
            pending_ui_events: std::cell::RefCell::default(),
            last_import_attempt: std::cell::Cell::default(),
            last_group_check: std::cell::Cell::default(),
            unlocked: std::cell::Cell::default(),
            data_dir: start.data_dir.clone(),
            imported_contacts_unchecked: std::cell::Cell::new(false),
            in_background: std::cell::Cell::new(false),
        };
        let app = Self {
            shared: std::rc::Rc::new(shared),
            connection_task: None,
            maintenance_ran_at: std::time::SystemTime::now(),
            maintenance_overdue: false,
            transfers: crate::app_media::RunningTransfers::default(),
            wake_settled_held: false,
            media_events_rx: Some(media_events_rx),
            write_pass: None,
            attachments: Vec::new(),
            current_request: 0,
            current_change: None,
            detached: crate::detached::DetachedWork::default(),
            group_call_leaves: 0,
            group_calls_joining: Vec::new(),
            image_size: crate::web::images::SENT_MAX,
            reply_later: false,
            file_work_running: 0,
            file_work_turn: std::rc::Rc::new(tokio::sync::Semaphore::new(1)),
            file_work_done,
            file_work_done_rx: Some(file_work_done_rx),
            update_check_built: start.update_check,
            update_check_running: std::rc::Rc::default(),
            update_check_manual: std::rc::Rc::default(),
            calls: crate::calls::Calls::default(),
            call_patcher: None,
            call_media: crate::app_calls::CallMedia::default(),
            group_call: None,
            web: crate::app_web::WebSessions::default(),
            android_import: None,
            android_export: None,
            android_export_cancel: Arc::default(),
            transfer_retry_at: None,
            on_wifi: false,
            pictures_dir: start.pictures_dir.clone(),
            identity_removed: false,
        };
        app.apply_display_settings()?;
        app.apply_picture_gate();
        app.apply_network_settings()?;
        app.apply_multi_device();
        Ok(app)
    }
}

impl Shared {
    /// Turn forward security off or on again, like [`Self::activate_multi_device`] does for
    /// a device group: off announces it (feature mask without FS), terminates every session
    /// with DisabledByLocal once connected, and reconnects so the running session follows.
    async fn set_forward_security(&self, on: bool) -> Result<Value, ApiError> {
        if on {
            self.store.delete_setting(keys::FORWARD_SECURITY)?;
            // Off and on again before the next login: nothing to end (unless multi-device
            // still needs it).
            if self.store.multi_device()?.is_none() {
                self.store
                    .delete_setting(crate::multidevice::SETTING_FS_TERMINATE)?;
            }
        } else {
            self.store.set_setting(keys::FORWARD_SECURITY, "0")?;
            self.store
                .set_setting(crate::multidevice::SETTING_FS_TERMINATE, "1")?;
        }
        tracing::info!(on, "forward security switched");
        if let Err(error) = self.publish_feature_mask().await {
            // Published again at the next start; the choice holds regardless.
            tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
        }
        self.tell_connection(crate::connection::ToConnection::Reconnect);
        Ok(json!({ "on": on }))
    }
}

impl App {
    /// Settings → Advanced → Log → "Save support package": the facts a report needs, then
    /// the log, as one text file in `directory` (sharing it stays the user's step). Nothing
    /// in it is personal: versions, model, flags, and the log's fixed texts, kinds, codes
    /// and counts (CLAUDE.md §7.3).
    fn write_log_package(&self, directory: &str) -> Result<Value, ApiError> {
        let now = crate::messages::now_ms();
        let header = format!(
            "Threesea support package ({} UTC)\n\
             App {} (core {}), calls {}\n\
             Sailfish OS {}, {} ({})\n\
             Locale {}-{}\n\
             Multi-device {}, connection {}\n\
             ----------------------------------------\n",
            crate::android_backup::export::time_stamp(now),
            self.version.app_version,
            crate::VERSION,
            if self.calls_supported {
                "built in"
            } else {
                "not built in"
            },
            self.version.os_version,
            self.version.model,
            self.version.arch,
            self.version.language,
            self.version.country,
            if matches!(self.store.multi_device(), Ok(Some(_))) {
                "on"
            } else {
                "off"
            },
            self.connection_state.get(),
        );
        let name = format!(
            "threesea-log-{}.txt",
            crate::android_backup::export::time_stamp(now)
        );
        let path = std::path::Path::new(directory).join(name);
        std::fs::write(
            &path,
            header + &crate::runtime::log_text(&self.data_dir, usize::MAX),
        )
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(json!({ "path": path.to_string_lossy() }))
    }

    fn network_settings_json(&self) -> Value {
        json!({ "ipv6": !crate::https::IPV4_ONLY.load(std::sync::atomic::Ordering::Relaxed) })
    }

    /// Load the IP version into [`crate::https::IPV4_ONLY`] (start, change).
    fn apply_network_settings(&self) -> Result<(), ApiError> {
        let on = self.store.setting(keys::IPV4_ONLY)?.as_deref() == Some("1");
        crate::https::IPV4_ONLY.store(on, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// The receiver of media progress events (taken once by the core loop).
    pub fn take_media_events(
        &mut self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<crate::providers::CoreEvent>> {
        self.media_events_rx.take()
    }

    pub(crate) fn take_file_work(
        &mut self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<crate::app_media::FinishedFileWork>> {
        self.file_work_done_rx.take()
    }
}

impl Shared {
    /// Remember events the current command caused, for [`Self::take_ui_events`].
    pub(crate) fn note_ui_events(&self, events: &[&'static str]) {
        let mut pending = self.pending_ui_events.borrow_mut();
        for event in events {
            if !pending.contains(event) {
                pending.push(event);
            }
        }
    }
}

impl App {
    /// The events the last command caused (emitted by the core loop after the reply).
    pub fn take_ui_events(&mut self) -> Vec<&'static str> {
        self.pending_ui_events.take()
    }

    /// [`Self::handle`] with binary attachments for the command.
    pub async fn handle_with(
        &mut self,
        request_json: &str,
        attachments: Vec<zeroize::Zeroizing<Vec<u8>>>,
    ) -> String {
        self.attachments = attachments;
        let reply = self.handle(request_json).await;
        self.attachments.clear();
        reply
    }

    /// [`Self::handle_with`] for the UI's requests in the core loop. `None`: the command's long
    /// part went elsewhere, and the core loop replies when it is done
    /// ([`Self::finish_file_work`], [`Self::finish_detached`]).
    pub(crate) async fn handle_from_ui(
        &mut self,
        request_json: &str,
        attachments: Vec<zeroize::Zeroizing<Vec<u8>>>,
    ) -> Option<String> {
        self.in_background.set(true);
        let reply = self.handle_with(request_json, attachments).await;
        self.in_background.set(false);
        (!std::mem::take(&mut self.reply_later)).then_some(reply)
    }

    /// Parse and execute one request; returns the reply JSON.
    pub async fn handle(&mut self, request_json: &str) -> String {
        let request: Request = match serde_json::from_str(request_json) {
            Ok(request) => request,
            Err(error) => {
                // Do not echo the request: it may contain secrets.
                tracing::warn!(%error, "invalid request from UI");
                let id = serde_json::from_str::<Value>(request_json)
                    .ok()
                    .and_then(|v| v.get("id").and_then(Value::as_u64));
                return json!({"reply": id, "ok": false,
                    "error": {"kind": "internal", "message": "invalid request"}})
                .to_string();
            }
        };
        self.current_change = sync_change(&self.store, &request.command);
        self.current_request = request.id;
        let outcome = self.execute(request.command).await;
        let change = self.current_change.take();
        if self.reply_later {
            return String::new();
        }
        self.reply(request.id, change, outcome).await
    }

    /// What follows a command, whatever became of it, and its reply JSON.
    pub(crate) async fn reply(
        &mut self,
        request: u64,
        change: Option<crate::app_multidevice::SyncChange>,
        outcome: Result<Value, ApiError>,
    ) -> String {
        let new_contacts = self.new_contacts.take();
        if !new_contacts.is_empty() {
            let change = crate::app_multidevice::SyncChange::NewContacts(new_contacts);
            self.reflect_change(change, &json!({})).await;
        }
        let made_direct = self.made_direct.take();
        if !made_direct.is_empty() {
            let change = crate::app_multidevice::SyncChange::Contacts(made_direct);
            self.reflect_change(change, &json!({})).await;
        }
        for conversation in self.unarchived.take() {
            let change = crate::app_multidevice::SyncChange::Conversation(conversation);
            self.reflect_change(change, &json!({})).await;
        }
        match outcome {
            Ok(result) => {
                if let Some(change) = change {
                    self.reflect_change(change, &result).await;
                }
                json!({"reply": request, "ok": true, "result": result})
            }
            Err(error) => {
                if matches!(error, ApiError::RateLimited) {
                    self.store.rest_directory_calls();
                }
                tracing::info!(kind = error.kind(), "command failed");
                json!({"reply": request, "ok": false,
                    "error": {"kind": error.kind(), "message": error.to_string()}})
            }
        }
        .to_string()
    }

    async fn execute(&mut self, command: Command) -> Result<Value, ApiError> {
        match command {
            Command::Status => self.status(),
            Command::CheckLicense { license_key } => {
                self.detached(Lane::InTurn, |app| async move {
                    app.check_license(&license_key).await
                })
                .await
            }
            Command::LicenseStatus => self.license_status(),
            Command::ForgetLicense => self.forget_license(),
            Command::SpamPotential { identity } => self.spam_potential(&identity),
            Command::DismissSpam { identity } => self.dismiss_spam(&identity),
            Command::ReportSpam { identity, block } => self.report_spam(&identity, block).await,
            Command::CreateIdentity => {
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.create_identity().await },
                )
                .await
            }
            Command::RestoreIdentityBackup { backup, password } => {
                self.detached(Lane::InTurn, |app| async move {
                    app.restore_identity_backup(backup, password).await
                })
                .await
            }
            Command::ExportIdentityBackup { password, pin } => {
                self.check_pin(pin)?;
                self.detached(Lane::Free, |app| async move {
                    app.export_identity_backup(password).await
                })
                .await
            }
            Command::SafeCheckPassword { password } => Ok(Self::safe_check_password(&password)),
            Command::SafeStatus => self.safe_status(),
            Command::SafeEnable { password, pin } => {
                self.check_pin(pin)?;
                self.detached(Lane::InTurn, |app| async move {
                    app.safe_enable(password).await
                })
                .await
            }
            Command::SafeBackupNow => {
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.safe_backup_now().await },
                )
                .await
            }
            Command::SafeDisable { pin } => {
                self.check_pin(pin)?;
                self.detached(Lane::InTurn, |app| async move { app.safe_disable().await })
                    .await
            }
            Command::SafeRestore { identity, password } => {
                self.detached(Lane::InTurn, |app| async move {
                    app.safe_restore(&identity, password).await
                })
                .await
            }
            Command::SafeRetryImport => {
                self.detached(Lane::InTurn, |app| async move {
                    let events = app.try_pending_import().await;
                    app.note_ui_events(&events);
                    app.safe_status()
                })
                .await
            }
            Command::AndroidBackupInspect { path, password } => {
                self.detached(Lane::Free, |app| async move {
                    app.android_backup_inspect(path, password).await
                })
                .await
            }
            Command::AndroidBackupImport {
                path,
                password,
                scope,
            } => {
                if scope == "history" {
                    self.android_backup_import_history(path, password)
                } else {
                    self.detached(Lane::InTurn, |app| async move {
                        app.android_backup_import(path, password, &scope).await
                    })
                    .await
                }
            }
            Command::AndroidBackupCancel => {
                if let Some(cancel) = &self.android_import {
                    cancel.set(true);
                }
                Ok(json!({}))
            }
            Command::AndroidBackupExport {
                directory,
                password,
                pin,
            } => {
                self.check_pin(pin)?;
                self.android_backup_export(directory, password)
            }
            Command::AndroidBackupSchedule {
                directory,
                password,
                interval,
                keep,
                pin,
            } => {
                self.check_pin(pin)?;
                self.android_backup_schedule(directory, password, interval, keep)
            }
            Command::AndroidBackupExportStatus => self.android_export_status(),
            Command::Contacts => self.contacts_json(),
            Command::Contact { identity } => self.contact_json(&identity),
            Command::AddContact { identity } => {
                self.detached(Lane::Free, |app| async move {
                    let result = app.add_contact(&identity).await?;
                    let added = result["identity"].as_str().unwrap_or_default().to_owned();
                    if app.sync_included_again(&added)? {
                        let change = crate::app_multidevice::SyncChange::Settings;
                        app.reflect_change(change, &json!({})).await;
                    }
                    Ok(result)
                })
                .await
            }
            Command::UpdateContact {
                identity,
                first_name,
                last_name,
            } => self.update_contact(&identity, first_name, last_name),
            Command::DeleteContact { identity } => {
                let result = self.delete_contact(&identity)?;
                // It is on the address book's exclusion list now, which they keep too.
                let change = crate::app_multidevice::SyncChange::Settings;
                self.reflect_change(change, &json!({})).await;
                Ok(result)
            }
            Command::SetContactPolicies {
                identity,
                read_receipts,
                typing,
            } => self.set_contact_policies(&identity, read_receipts, typing),
            Command::ClearContactPolicies => self.clear_contact_policies(),
            Command::Profile => {
                // A pending e-mail link may have been confirmed meanwhile.
                if self.store.setting(keys::LINKED_EMAIL_PENDING)?.is_some() {
                    self.detached(Lane::Free, |app| async move {
                        let _ = app.check_email_link().await;
                        app.profile_json()
                    })
                    .await
                } else {
                    self.profile_json()
                }
            }
            Command::RevocationStatus { cached } => {
                self.detached(Lane::Free, move |app| async move {
                    app.revocation_status(cached).await
                })
                .await
            }
            Command::LinkEmail { email } => {
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.link_email(&email).await },
                )
                .await
            }
            Command::LinkMobile { number } => {
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.link_mobile(&number).await },
                )
                .await
            }
            Command::NormalizeMobile { number } => Ok(json!({
                "number": crate::address_book::international_phone(&number),
            })),
            Command::VerifyMobile { code, from_link } => {
                self.detached(Lane::InTurn, move |app| async move {
                    app.verify_mobile(&code, from_link).await
                })
                .await
            }
            Command::CallMobile => {
                self.detached(Lane::InTurn, |app| async move { app.call_mobile().await })
                    .await
            }
            Command::SetRevocationPassword { password, pin } => {
                self.check_pin(pin)?;
                self.detached(Lane::InTurn, |app| async move {
                    app.set_revocation_password(&password).await
                })
                .await
            }
            Command::SetNickname { nickname } => self.set_nickname(&nickname),
            Command::DeclineNickname => self.decline_nickname(),
            Command::Settings => self.settings_json(),
            Command::SetSetting { name, value } => self.set_app_setting(&name, value),
            Command::DeleteAllData { pin } => {
                self.check_pin(pin)?;
                self.delete_all_data().await
            }
            Command::Conversations { archived, private } => {
                self.conversations_json(archived, private)
            }
            Command::UnreadSummary => self.unread_summary_json(),
            Command::LockStatus => self.lock_status(),
            Command::SetLock {
                enabled,
                pin,
                keyboard,
                app_lock,
                grace_secs,
                current_pin,
            } => {
                self.check_lock(current_pin.as_deref())?;
                let lock = enabled.then_some(crate::applock::LockOptions {
                    pin: pin.as_deref().or(current_pin.as_deref()),
                    new_pin: pin.is_some(),
                    keyboard,
                    app_lock,
                    grace_secs,
                });
                self.set_lock(lock)
            }
            Command::Unlock { pin, private_chats } => {
                // Nothing to check a PIN against: no PIN opens anything. Private marks can exist
                // without a lock (Safe restore, a linked device); a PIN set now opens them.
                if self.store.lock()?.is_none() {
                    return Err(ApiError::LockNeeded);
                }
                self.check_lock(Some(&pin))?;
                self.write_pass = None;
                if private_chats {
                    self.unlocked.set(true);
                    self.update_media_gate();
                    // Their names again, in a private group's call that went on.
                    self.emit_group_call_participants();
                }
                self.lock_status()
            }
            Command::Lock { pending_files } => {
                self.unlocked.set(false);
                self.update_media_gate();
                self.group_calls_locked();
                if let Some(files) = pending_files {
                    self.grant_write_pass(files);
                }
                self.lock_status()
            }
            Command::SetPrivate { identity, private } => self.set_private(&identity, private),
            Command::SetArchived { identity, archived } => {
                let key = self.conversation(&identity)?.0;
                self.store.set_archived(&key, archived)?;
                // Chats were once seen landing in the archive unasked (an unexplained oddity).
                tracing::info!(archived, "chat archive state set from the UI");
                Ok(json!({}))
            }
            Command::SetDeviceLabel { label } => self.set_device_label(&label),
            Command::SetDeviceAlias { device, alias } => self.set_device_alias(&device, &alias),
            Command::SetConversationUnread { identity, unread } => {
                let key = self.conversation(&identity)?.0;
                self.store.set_marked_unread(&key, unread)?;
                Ok(json!({}))
            }
            Command::Messages {
                identity,
                limit,
                around,
                before,
                after,
            } => self.messages_json(&identity, limit, around, before, after),
            Command::SearchMessages { query, identity } => {
                self.search_messages(&query, identity.as_deref())
            }
            Command::SendText {
                identity,
                text,
                quote,
            } => self.send_text(&identity, &text, quote.as_deref()),
            Command::QuotedMessage {
                identity,
                message_id,
            } => self.quoted_message(&identity, &message_id),
            Command::MarkRead { identity } => self.mark_read(&identity),
            Command::SetTyping { identity, typing } => self.set_typing(&identity, typing),
            Command::DownloadFile { message } => self.download_file(message),
            Command::Groups => self.groups_json(),
            Command::Group { key } => self.group_info(&key),
            Command::CreateGroup { name, members } => self.create_group(&name, &members),
            Command::RenameGroup { key, name } => self.rename_group(&key, &name),
            Command::SetGroupMembers { key, members } => self.set_group_members(&key, &members),
            Command::LeaveGroup { key } => self.leave_group(&key),
            Command::SetGroupPicture { key } => self.set_group_picture(&key).await,
            Command::SendLocation {
                identity,
                latitude,
                longitude,
                accuracy,
                name,
                address,
            } => self.send_location(
                &identity,
                &json!({
                    "latitude": latitude,
                    "longitude": longitude,
                    "accuracy": accuracy,
                    "name": name,
                    "address": address,
                }),
            ),
            Command::CreateDistributionList { name, members } => {
                self.create_distribution_list(&name, &members)
            }
            Command::DistributionList { key } => self.distribution_list_json(&key),
            Command::UpdateDistributionList { key, name, members } => {
                self.update_distribution_list(&key, name.as_deref(), members.as_deref())
            }
            Command::SetPinned { identity, pinned } => {
                let key = self.conversation(&identity)?.0;
                self.store.set_pinned(&key, pinned)?;
                Ok(json!({}))
            }
            Command::MovePinned { identity, offset } => {
                let key = self.conversation(&identity)?.0;
                self.store.move_pinned(&key, offset)?;
                Ok(json!({}))
            }
            Command::SetBlocked { identity, blocked } => self.set_blocked(&identity, blocked),
            Command::BlockedContacts => self.blocked_contacts_json(),
            Command::SetSyncExcluded { identity, excluded } => {
                self.set_sync_excluded(&identity, excluded)
            }
            Command::SyncExcluded => self.sync_excluded_json(),
            Command::RefreshContacts => {
                self.detached(Lane::InTurn, |app| async move {
                    let changed = app.refresh_contacts(true).await?;
                    Ok(json!({ "changed": changed }))
                })
                .await
            }
            Command::ForwardMessage { message, identity } => {
                // Boxed: all the commands' futures together must fit the stack.
                Box::pin(self.forward_message(message, &identity)).await
            }
            Command::SetDraft {
                identity,
                text,
                quote,
            } => self.set_draft(&identity, &text, quote.as_deref()),
            Command::Draft { identity } => self.draft(&identity),
            Command::ChatMedia {
                identity,
                kinds,
                limit,
                before,
            } => self.chat_media(&identity, kinds, limit, before.as_deref()),
            Command::SetMediaKinds { kinds } => {
                let kinds: Vec<&str> = crate::messages::MEDIA_KINDS
                    .into_iter()
                    .filter(|k| kinds.iter().any(|c| c == k))
                    .collect();
                self.store
                    .set_setting(keys::MEDIA_KINDS, &kinds.join(","))?;
                Ok(json!({ "kinds": kinds }))
            }
            Command::Message { message } => match self.store.message(message)? {
                Some(m) => {
                    self.check_readable(&m.conversation)?;
                    Ok(self.store.message_json(&m))
                }
                None => Ok(Value::Null),
            },
            Command::SetVoiceListened { message } => {
                // It changes and returns the message: not one of a hidden private chat.
                if let Some(m) = self.store.message(message)? {
                    self.check_readable(&m.conversation)?;
                }
                let changed = self.store.set_listened(message)?;
                // It may have been played elsewhere than in its chat (the media page), which has
                // to know as well.
                if let Some(changed) = &changed {
                    let _ = self
                        .media_events
                        .send(crate::providers::CoreEvent::MessagesChanged(vec![
                            changed.clone(),
                        ]));
                }
                Ok(changed.map_or(Value::Null, |m| self.store.message_json(&m)))
            }
            Command::SetStarred { message, starred } => {
                // It returns the message: not one of a hidden private chat.
                if let Some(m) = self.store.message(message)? {
                    self.check_readable(&m.conversation)?;
                }
                Ok(self
                    .store
                    .set_starred(message, starred)?
                    .map_or(Value::Null, |m| self.store.message_json(&m)))
            }
            Command::StarredMessages { identity } => {
                self.starred_messages_json(identity.as_deref())
            }
            Command::ForwardMessages { messages, identity } => {
                let (mut forwarded, mut failed) = (0, 0);
                let mut errors: Vec<&str> = Vec::new();
                for message in messages {
                    match Box::pin(self.forward_message(message, &identity)).await {
                        Ok(_) => forwarded += 1,
                        Err(error) => {
                            tracing::warn!(%error, "forwarding failed");
                            failed += 1;
                            if !errors.contains(&error.kind()) {
                                errors.push(error.kind());
                            }
                        }
                    }
                }
                Ok(json!({ "forwarded": forwarded, "failed": failed, "errors": errors }))
            }
            Command::SetChatNotifications {
                identity,
                policy,
                until,
            } => self.set_chat_notifications(&identity, policy.as_deref(), until),
            Command::ChatTones => self.chat_tones(),
            Command::SetChatTone { identity, tone } => {
                self.set_chat_tone(&identity, tone.as_deref())
            }
            Command::ImportChatTones { tones } => {
                for (conversation, tone) in &tones {
                    self.store.set_chat_tone(conversation, Some(tone))?;
                }
                Ok(json!({}))
            }
            Command::ParseLink { url } => crate::links::parse(&url)
                .map(|link| link.to_json())
                .ok_or(ApiError::LinkInvalid),
            Command::MyQrCode => self.my_qr_code(),
            Command::MatchAddressBook { entries, automatic } => {
                if automatic && self.store.directory_resting() {
                    return Err(ApiError::RateLimited);
                }
                self.detached(Lane::InTurn, |app| async move {
                    app.match_address_book(entries).await
                })
                .await
            }
            Command::VerifyQrCode { text } => {
                self.detached(
                    Lane::Free,
                    |app| async move { app.verify_qr_code(&text).await },
                )
                .await
            }
            Command::QrKind { text } => self.classify_qr(&text),
            Command::SetProfilePicture => self.set_profile_picture(),
            Command::RemoveProfilePicture => self.remove_profile_picture(),
            Command::SetProfilePictureShare { share, recipients } => {
                self.set_profile_picture_share(&share, recipients)
            }
            Command::SetProfilePictureRecipient { identity, share } => {
                self.set_profile_picture_recipient(&identity, share)
            }
            Command::CreatePoll {
                identity,
                description,
                choices,
                multiple,
                announce,
            } => self.create_poll(&identity, &description, &choices, multiple, announce),
            Command::Vote { message, choices } => self.vote(message, &choices),
            Command::ClosePoll { message } => self.close_poll(message),
            Command::RemoveGroupPicture { key } => {
                // In turn: behind a new picture still on its way up, or that one would win.
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.remove_group_picture(&key) },
                )
                .await
            }
            Command::SendFile {
                identity,
                path,
                name,
                media_type,
                caption,
                rendering,
                width,
                height,
                duration,
                encode,
                keep_metadata,
            } => {
                let file = crate::app_media::OutgoingFile {
                    identity,
                    path,
                    name,
                    media_type,
                    caption,
                    rendering,
                    width,
                    height,
                    duration,
                    encode,
                    keep_metadata,
                };
                // Only this command: nothing else that ends in sending a file uses a pass.
                let under_pass = self.take_write_pass(&file.identity);
                if self.in_background.get() {
                    self.send_file_in_background(file, under_pass)
                } else if under_pass {
                    Box::pin(self.send_file_under_pass(file)).await
                } else {
                    Box::pin(self.send_file(file)).await
                }
            }
            Command::RetryUpload { message } => self.retry_upload(message),
            Command::SaveFile { message, directory } => {
                if self.in_background.get() {
                    self.save_file_in_background(message, directory)
                } else {
                    Box::pin(self.save_file(message, directory)).await
                }
            }
            Command::EditMessage {
                identity,
                message_id,
                text,
            } => self.edit_message(&identity, &message_id, &text),
            Command::DeleteMessage {
                identity,
                message_id,
                outgoing,
                everyone,
            } => self.delete_message(&identity, &message_id, outgoing, everyone),
            Command::React {
                identity,
                message_id,
                emoji,
                apply,
            } => self.react(&identity, &message_id, &emoji, apply),
            Command::DeleteConversation { identity } => self.delete_conversation(&identity),
            Command::SetNetwork { wifi } => {
                let changed = self.on_wifi != wifi;
                self.on_wifi = wifi;
                if changed {
                    self.download_pending_media();
                }
                Ok(json!({}))
            }
            Command::MultiDevice => self.multi_device_json(),
            Command::LinkedDevices => {
                self.detached(Lane::Free, |app| async move {
                    app.linked_devices_checked().await
                })
                .await
            }
            Command::DropDevices { ids } => {
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.drop_devices(&ids).await },
                )
                .await
            }
            Command::DeactivateMultiDevice { leave_locally } => {
                self.detached(Lane::InTurn, move |app| async move {
                    app.deactivate_multi_device(leave_locally).await
                })
                .await
            }
            Command::StartLinking { code, pin } => {
                self.check_pin(pin)?;
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.start_linking(&code).await },
                )
                .await
            }
            Command::ConfirmLinking => {
                self.detached(
                    Lane::InTurn,
                    |app| async move { app.confirm_linking().await },
                )
                .await
            }
            Command::CancelLinking => {
                // In turn: a page closing while the connection comes up asks this before there is
                // anything to cancel.
                self.detached(Lane::InTurn, |app| async move {
                    let link = app.pending_link.take();
                    if let Some(link) = link {
                        link.close().await;
                    }
                    Ok(json!({}))
                })
                .await
            }
            Command::CallTurnServers { identity } => {
                self.detached(Lane::Free, |app| async move {
                    app.call_turn_servers(&identity).await
                })
                .await
            }
            Command::CallStart {
                identity,
                sdp,
                video,
            } => self.call_start(&identity, &sdp, video),
            Command::CallAccept {
                call_id,
                sdp,
                video,
            } => self.call_accept(call_id, &sdp, video),
            Command::CallReject { call_id } => Ok(self.call_reject(call_id)),
            Command::CallHangUp { call_id } => Ok(self.call_hang_up(call_id)),
            Command::CallFailed { call_id, reason } => Ok(self.call_failed(call_id, &reason)),
            Command::CallTimeout { call_id } => Ok(self.call_timeout(call_id)),
            Command::CallCandidates {
                call_id,
                candidates,
            } => Ok(self.call_candidates(call_id, candidates)),
            Command::CallState => Ok(self.call_state()),
            Command::CallConnected { relayed } => Ok(self.call_connected(relayed)),
            Command::CallChannel { data } => self.call_channel(&data),
            Command::CallCapture { device, on } => self.call_capture(&device, on),
            Command::SetVideoQuality { quality } => self.set_video_quality(&quality),
            Command::AddressBookCountry => self.address_book_country_json(),
            Command::SetAddressBookCountry { country } => self.set_address_book_country(&country),
            Command::GroupCallRefresh { conversation } => {
                Ok(self.group_call_refresh(&conversation))
            }
            Command::GroupCallJoin {
                conversation,
                fingerprint,
                create,
            } => Box::pin(self.group_call_join(conversation, fingerprint, create)).await,
            Command::GroupCallChannel { data } => self.group_call_channel(&data),
            Command::GroupCallLeave => Ok(self.group_call_leave()),
            Command::GroupCallMute { muted } => Ok(self.group_call_mute(muted)),
            Command::GroupCallCamera { on } => Ok(self.group_call_camera(on)),
            Command::GroupCallTick => self.group_call_tick(),
            Command::WebSessions => self.web_sessions_json(),
            Command::WebSetEnabled { enabled } => self.web_set_enabled(enabled),
            Command::WebPair { qr, allow_host } => self.web_pair(&qr, allow_host),
            Command::WebStart { session } => self.web_start(session),
            Command::WebStop { session } => Ok(self.web_stop(session)),
            Command::WebDelete { session } => self.web_delete(session),
            Command::WebAcceptCertificate { session } => self.web_accept_certificate(session),
            Command::WebDeclineCertificate { session } => Ok(self.web_decline_certificate(session)),
            Command::WebRemoveServer { host } => self.web_remove_server(&host),
            Command::GifFavourites => self.gif_favourites_json(),
            Command::RecentEmoji => self.recent_emoji_json(),
            Command::UseEmoji { emoji } => self.use_emoji(&emoji),
            Command::AddGifFavourite {
                message,
                path,
                width,
                height,
            } => self.add_gif_favourite(message, path.as_deref(), width, height),
            Command::RemoveGifFavourite { favourite } => self.remove_gif_favourite(favourite),
            Command::SendGifFavourite {
                identity,
                favourite,
            } => Box::pin(self.send_gif_favourite(identity, favourite)).await,
            Command::UpdateSettings => self.update_settings_json(),
            Command::SetUpdateCheck { enabled } => {
                if !self.update_check_built {
                    return Err(ApiError::Internal("no update check in this build".into()));
                }
                self.store
                    .set_setting(keys::UPDATE_CHECK, if enabled { "1" } else { "0" })?;
                if enabled {
                    self.start_update_check(true);
                }
                self.update_settings_json()
            }
            Command::CheckForUpdate => {
                self.start_update_check(true);
                self.update_settings_json()
            }
            Command::SetImageSize { side } => {
                self.image_size = side;
                Ok(json!({}))
            }
            Command::SetLogLevel { level } => {
                crate::runtime::set_log_level(&level);
                Ok(json!({}))
            }
            Command::ForwardSecurity => Ok(json!({
                "on": !self.store.forward_security_disabled()?
                    && self.store.multi_device()?.is_none(),
                "multiDevice": self.store.multi_device()?.is_some(),
            })),
            Command::SetForwardSecurity { on } => {
                self.detached(Lane::InTurn, move |app| async move {
                    app.set_forward_security(on).await
                })
                .await
            }
            Command::LogText => Ok(json!({
                // A small tail: the page lays the whole text out at once.
                "text": crate::runtime::log_text(&self.data_dir, 64 * 1024),
            })),
            Command::LogPackage { directory } => self.write_log_package(&directory),
            Command::LogClear => {
                crate::runtime::log_clear(&self.data_dir);
                Ok(json!({}))
            }
            Command::NetworkSettings => Ok(self.network_settings_json()),
            Command::SetNetworkSettings { ipv6 } => {
                self.store
                    .set_setting(keys::IPV4_ONLY, if ipv6 { "0" } else { "1" })?;
                self.apply_network_settings()?;
                self.tell_connection(crate::connection::ToConnection::Reconnect);
                Ok(self.network_settings_json())
            }
            Command::MediaSettings => self.media_settings_json(),
            Command::DisplaySettings => self.display_settings_json(),
            Command::SetDisplaySettings {
                last_name_first,
                sort_by_last_name,
                show_inactive,
            } => self.set_display_settings(last_name_first, sort_by_last_name, show_inactive),
            Command::SetMediaSettings {
                auto_download_wifi,
                auto_download_mobile,
                auto_download_limit,
                auto_save,
                auto_delete_days,
            } => {
                for (key, list) in [
                    (keys::AUTO_DOWNLOAD_WIFI, auto_download_wifi),
                    (keys::AUTO_DOWNLOAD_MOBILE, auto_download_mobile),
                ] {
                    if let Some(list) = list {
                        let kinds: Vec<&str> = crate::app_media::AUTO_DOWNLOAD_WIFI
                            .into_iter()
                            .filter(|k| list.iter().any(|l| l == k))
                            .collect();
                        self.store.set_setting(key, &kinds.join(","))?;
                    }
                }
                if let Some(limit) = auto_download_limit {
                    self.store
                        .set_setting(keys::AUTO_DOWNLOAD_LIMIT, &limit.to_string())?;
                }
                if let Some(on) = auto_save {
                    self.store
                        .set_setting(keys::AUTO_SAVE_MEDIA, if on { "1" } else { "0" })?;
                }
                if let Some(days) = auto_delete_days {
                    self.store
                        .set_setting(keys::AUTO_DELETE_DAYS, &days.to_string())?;
                    // Start over, so turning it on clears the old messages at once.
                    self.store.delete_setting(keys::AUTO_DELETE_LAST)?;
                }
                self.download_pending_media();
                self.media_settings_json()
            }
            #[cfg(feature = "demo")]
            Command::DemoData { remove, messages } => self.demo_data(remove, messages),
            Command::ExportChat {
                identity,
                media,
                directory,
                password,
                me,
                offset_minutes,
            } => {
                self.detached(Lane::InTurn, move |app| async move {
                    app.export_chat(&identity, media, &directory, password, me, offset_minutes)
                        .await
                })
                .await
            }
            Command::StorageInfo => Ok(json!({
                "mediaBytes": self.media.size(),
                "databaseBytes": self.store.size()?,
            })),
            Command::CleanUp { days, messages } => self.clean_up(days, messages),
            Command::EmptyConversation { identity } => {
                let key = self.readable_chat(&identity)?.0;
                self.store.empty_conversation(&key)?;
                let store = std::rc::Rc::clone(&self.store);
                self.media
                    .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
                Ok(json!({}))
            }
            Command::ConnectionState => Ok(self.connection_json()),
            Command::Reconnect => Ok(self.connect_now()),
            Command::Wake => Ok(self.wake_connection()),
        }
    }

    fn status(&self) -> Result<Value, ApiError> {
        let identity = self.store.identity()?;
        let state = match (&identity, self.license_valid()?) {
            (_, false) => "needsLicense",
            (None, true) => "needsIdentity",
            (Some(_), true) => "ready",
        };
        Ok(json!({
            "state": state,
            "identity": identity.map(|i| i.threema_id),
        }))
    }
}

impl Shared {
    fn device_id(&self) -> Result<String, ApiError> {
        if let Some(id) = self.store.setting(keys::DEVICE_ID)? {
            return Ok(id);
        }
        let id = random_device_id();
        self.store.set_setting(keys::DEVICE_ID, &id)?;
        Ok(id)
    }

    async fn check_license(&self, input: &str) -> Result<Value, ApiError> {
        let license_key =
            Zeroizing::new(normalize_license_key(input).ok_or(ApiError::LicenseFormat)?);
        let device_id = self.device_id()?;
        let ok =
            directory::check_license(&self.https, &license_key, &device_id, &self.version).await?;
        self.store.set_setting(keys::LICENSE_KEY, &license_key)?;
        self.store.delete_setting(keys::LICENSE_REFUSED)?;
        self.store.set_setting(
            keys::LICENSE_CHECKED_AT,
            &crate::messages::now_ms().to_string(),
        )?;
        Ok(json!({ "updateMessage": ok.update_message }))
    }

    /// There is a license key and Threema's directory has not refused it.
    pub(crate) fn license_valid(&self) -> Result<bool, ApiError> {
        Ok(self.store.setting(keys::LICENSE_KEY)?.is_some()
            && self.store.setting(keys::LICENSE_REFUSED)?.is_none())
    }
}

impl App {
    fn license_status(&self) -> Result<Value, ApiError> {
        Ok(json!({
            "key": self.store.setting(keys::LICENSE_KEY)?,
            "refused": self.store.setting(keys::LICENSE_REFUSED)?,
        }))
    }

    /// The way back from the ID step when the directory turns the key down there: until an ID
    /// exists, nothing else depends on it.
    fn forget_license(&mut self) -> Result<Value, ApiError> {
        self.check_no_identity()?;
        self.store.delete_setting(keys::LICENSE_KEY)?;
        self.store.delete_setting(keys::LICENSE_REFUSED)?;
        self.store.delete_setting(keys::LICENSE_CHECKED_AT)?;
        self.status()
    }

    /// Ask the directory about the stored license again, at most daily (Android: whenever its
    /// main screen opens; we run for weeks, and many starts shouldn't mean many requests). Returns
    /// whether the license was refused just now.
    #[cfg(test)]
    pub(crate) async fn recheck_license(&mut self) -> Result<bool, ApiError> {
        let refused = self.shared.ask_about_license().await?;
        if refused {
            self.license_refused();
        }
        Ok(refused)
    }

    /// See [`Shared::note_license_answer`].
    #[cfg(test)]
    fn apply_license_answer(
        &mut self,
        answer: Result<directory::LicenseOk, DirectoryError>,
    ) -> Result<bool, ApiError> {
        let refused = self.shared.note_license_answer(answer)?;
        if refused {
            self.license_refused();
        }
        Ok(refused)
    }

    /// Nothing goes to Threema's servers from here on.
    pub(crate) fn license_refused(&mut self) {
        self.web_stop_all(crate::app_web::StopReason::Stop);
        self.stop_connection();
    }
}

impl Shared {
    /// [`App::recheck_license`]'s work; what follows a refusal is up to the caller.
    pub(crate) async fn ask_about_license(&self) -> Result<bool, ApiError> {
        let Some(license_key) = self.store.setting(keys::LICENSE_KEY)?.map(Zeroizing::new) else {
            return Ok(false);
        };
        let checked_at = self
            .store
            .setting(keys::LICENSE_CHECKED_AT)?
            .and_then(|at| at.parse().ok());
        if !license_check_due(checked_at, crate::messages::now_ms())
            || self.store.directory_resting()
        {
            return Ok(false);
        }
        let device_id = self.device_id()?;
        let answer =
            directory::check_license(&self.https, &license_key, &device_id, &self.version).await;
        self.note_license_answer(answer)
    }

    /// Only the directory's own "no" counts, like Android: being offline or a server having a bad
    /// day must not cost anyone their messages. Those try again at the next maintenance. Whether
    /// that "no" came.
    fn note_license_answer(
        &self,
        answer: Result<directory::LicenseOk, DirectoryError>,
    ) -> Result<bool, ApiError> {
        match answer {
            Ok(_) => {
                self.store.set_setting(
                    keys::LICENSE_CHECKED_AT,
                    &crate::messages::now_ms().to_string(),
                )?;
                Ok(false)
            }
            Err(DirectoryError::Server(message)) => {
                tracing::warn!("the directory refused the license");
                self.store.set_setting(keys::LICENSE_REFUSED, &message)?;
                Ok(true)
            }
            Err(DirectoryError::RateLimited) => {
                self.store.rest_directory_calls();
                Ok(false)
            }
            // A server without words for it, or no answer at all: that's not about the key.
            Err(error) => {
                tracing::info!(%error, "license check not answered");
                Ok(false)
            }
        }
    }

    /// The error for the UI from a directory call that runs by itself: a 429 also starts the rest.
    pub(crate) fn directory_error(&self, error: DirectoryError) -> ApiError {
        if matches!(error, DirectoryError::RateLimited) {
            self.store.rest_directory_calls();
        }
        error.into()
    }

    pub(crate) fn license(&self) -> Result<SerialLicense, ApiError> {
        Ok(SerialLicense {
            license_key: self
                .store
                .setting(keys::LICENSE_KEY)?
                .ok_or(ApiError::NoLicense)?,
            device_id: self.device_id()?,
        })
    }

    /// Creating or restoring replaces the stored ID, so it starts only on a device without one:
    /// taking an ID off the device goes through the PIN (ADR 0009), not around it.
    pub(crate) fn check_no_identity(&self) -> Result<(), ApiError> {
        if self.store.identity()?.is_some() {
            return Err(ApiError::IdentityExists);
        }
        Ok(())
    }

    /// The one way a created or restored ID gets stored. Asked again here because the network
    /// waits since the first check: the old ID's key would be gone from the device, and a later
    /// automatic Safe backup would put the new one over its copy.
    fn store_new_identity(&self, identity: &StoredIdentity) -> Result<(), ApiError> {
        self.check_no_identity()?;
        self.store.set_identity(identity)?;
        Ok(())
    }

    async fn create_identity(&self) -> Result<Value, ApiError> {
        self.check_no_identity()?;
        let mut task = CreateIdentityTask::new(CreateIdentityContext {
            client_info: self.version.client_info(),
            config: Arc::clone(&self.config),
            flavor: Flavor::Consumer,
            serial_license: Some(self.license()?),
        });
        let CreateIdentityResult {
            identity,
            client_key,
            server_group,
        } = loop {
            match task.poll().map_err(create_identity_error)? {
                CreateIdentityLoop::Instruction(instruction) => {
                    let result = self.https.send(instruction.request).await;
                    task.response(CreateIdentityResponse { result })
                        .map_err(create_identity_error)?;
                }
                CreateIdentityLoop::Done(result) => break result,
            }
        };
        self.store_new_identity(&StoredIdentity {
            threema_id: identity.to_string(),
            client_key: Zeroizing::new(*client_key.as_bytes()),
            server_group: server_group.to_string(),
        })?;
        self.store.set_setting(keys::NICKNAME_UNANSWERED, "ask")?;
        tracing::info!("identity created");
        Ok(json!({ "identity": identity.to_string() }))
    }

    async fn restore_identity_backup(
        &self,
        backup: Zeroizing<String>,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
        self.check_no_identity()?;
        // Libre requires a valid license before an ID can be used, also when restoring.
        let _ = self.license()?;
        // Key derivation is deliberately slow: keep it off the core loop.
        let data = tokio::task::spawn_blocking(move || {
            decrypt_identity_backup(&password, backup.trim()).map(|(_, data)| data)
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|_| ApiError::BackupDecrypt)?;
        let threema_id = data.threema_id.to_string();
        self.adopt_identity(&threema_id, &data.client_key).await?;
        // An ID backup holds no nickname. Threema Safe has its own, if the backup had one.
        self.store.set_setting(keys::NICKNAME_UNANSWERED, "ask")?;
        tracing::info!("identity restored");
        Ok(json!({ "identity": threema_id }))
    }

    /// Take over an existing ID (from an ID backup or Threema Safe): learn its server group and
    /// linked e-mail/phone via `identity/fetch_priv` (as `UserServiceImpl.restoreIdentity`), then
    /// store it.
    pub(crate) async fn adopt_identity(
        &self,
        threema_id: &str,
        client_key: &ClientKey,
    ) -> Result<(), ApiError> {
        self.check_no_identity()?;
        let private =
            directory::fetch_identity_private(&self.https, threema_id, client_key).await?;
        self.store_adopted_identity(threema_id, client_key, private)
    }

    /// The local half of [`Self::adopt_identity`].
    pub(crate) fn store_adopted_identity(
        &self,
        threema_id: &str,
        client_key: &ClientKey,
        private: directory::PrivateIdentityData,
    ) -> Result<(), ApiError> {
        self.store_new_identity(&StoredIdentity {
            threema_id: threema_id.to_owned(),
            client_key: Zeroizing::new(*client_key.as_bytes()),
            server_group: private.server_group,
        })?;
        for (key, value) in [
            (keys::LINKED_EMAIL, private.email),
            (keys::LINKED_MOBILE, private.mobile_no),
        ] {
            match value {
                Some(value) => self.store.set_setting(key, &value)?,
                None => self.store.delete_setting(key)?,
            }
        }
        Ok(())
    }

    async fn export_identity_backup(&self, password: Zeroizing<String>) -> Result<Value, ApiError> {
        if password.chars().count() < MIN_BACKUP_PASSWORD_LENGTH {
            return Err(ApiError::PasswordTooShort);
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let threema_id = identity
            .threema_id
            .parse()
            .map_err(|_| ApiError::Internal("stored identity invalid".into()))?;
        let data = IdentityBackupData {
            threema_id,
            client_key: ClientKey::from(*identity.client_key),
        };
        let backup = tokio::task::spawn_blocking(move || encrypt_identity_backup(&password, &data))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(json!({ "backup": backup }))
    }
}

/// What a command changes that other devices of the group keep too (multi-device).
pub(crate) fn sync_change(
    store: &crate::store::Store,
    command: &Command,
) -> Option<crate::app_multidevice::SyncChange> {
    use crate::app_multidevice::SyncChange;
    Some(match command {
        // The command takes it as it comes (spaces, lower case), and so must this.
        Command::DeleteContact { identity }
        | Command::UpdateContact { identity, .. }
        | Command::SetContactPolicies { identity, .. } => {
            SyncChange::Contacts(vec![crate::contacts::parse_identity(identity)?.to_string()])
        }
        Command::SetPrivate { identity, .. }
        | Command::SetArchived { identity, .. }
        | Command::SetPinned { identity, .. }
        | Command::SetChatNotifications { identity, .. } => {
            SyncChange::Conversation(identity.clone())
        }
        // Its pin or archive mark goes with it. (Of a left group the other devices hear nothing
        // from this, as the row is gone by then.)
        Command::DeleteConversation { identity } if chat_is_marked(store, identity) => {
            SyncChange::Conversation(identity.clone())
        }
        Command::CreateGroup { .. } => SyncChange::Group {
            key: None,
            created: true,
            previous_members: None,
        },
        Command::RenameGroup { key, .. } | Command::LeaveGroup { key } => SyncChange::Group {
            key: Some(key.clone()),
            created: false,
            previous_members: None,
        },
        // The list as it is before the command runs: the difference goes along.
        Command::SetGroupMembers { key, .. } => SyncChange::Group {
            key: Some(key.clone()),
            created: false,
            previous_members: store.group(key).ok().flatten().map(|group| group.members),
        },
        Command::SetSetting { .. }
        | Command::SetBlocked { .. }
        | Command::SetSyncExcluded { .. } => SyncChange::Settings,
        Command::SetNickname { .. }
        | Command::SetProfilePictureShare { .. }
        | Command::SetProfilePictureRecipient { .. }
        | Command::LinkEmail { .. }
        | Command::LinkMobile { .. }
        | Command::VerifyMobile { .. } => SyncChange::Profile { picture: false },
        Command::SetProfilePicture | Command::RemoveProfilePicture => {
            SyncChange::Profile { picture: true }
        }
        Command::SetGroupPicture { key } | Command::RemoveGroupPicture { key } => {
            SyncChange::GroupPicture(key.clone())
        }
        _ => return None,
    })
}

fn chat_is_marked(store: &crate::store::Store, conversation: &str) -> bool {
    let among = |chats: Result<Vec<String>, _>| {
        chats.is_ok_and(|chats| chats.iter().any(|chat| chat == conversation))
    };
    among(store.pinned_chats()) || among(store.archived_chats())
}

/// What a scanned QR code is, for the one place that decides what to do with it, however it
/// reached the scanner: `kind` (`contact`, `web`, `device`, `link`, `unknown`), and `error`, the
/// error kind to show instead when the code can't be used. No network, nothing stored.
pub(crate) fn qr_kind(text: &str, now_seconds: i64) -> Value {
    use crate::{links::Link, rendezvous::LinkError, web::qr::QrError};

    let text = text.trim();
    if text.starts_with("3mid:") {
        return match crate::qr::parse(text) {
            None => json!({ "kind": "contact", "error": ApiError::QrInvalid.kind() }),
            Some(code) if code.is_expired(now_seconds) => {
                json!({ "kind": "contact", "error": ApiError::QrExpired.kind(), "identity": code.identity })
            }
            Some(code) => json!({ "kind": "contact", "identity": code.identity }),
        };
    }
    match crate::web::qr::parse(text) {
        Ok(qr) => {
            return json!({ "kind": "web", "host": qr.host });
        }
        // Any long enough Base64 has "a version": only a small number looks like Threema Web's.
        Err(QrError::Version(version)) if (1..=0xff).contains(&version) => {
            return json!({ "kind": "web", "error": ApiError::WebVersion.kind() });
        }
        Err(_) => {}
    }
    match crate::rendezvous::parse_join_code(text) {
        Ok(_) => return json!({ "kind": "device" }),
        // The join prefix leaves no doubt what it is.
        Err(LinkError::Version) => {
            return json!({ "kind": "device", "error": ApiError::LinkVersion.kind() });
        }
        Err(_) => {}
    }
    match crate::links::parse(text) {
        Some(
            Link::Contact { identity, .. }
            | Link::Add { identity }
            | Link::Compose {
                identity: Some(identity),
                ..
            },
        ) => json!({ "kind": "link", "identity": identity }),
        _ => json!({ "kind": "unknown" }),
    }
}

#[cfg(test)]
mod tests {
    fn web_code(version: u16, options: u8, host: &str) -> String {
        let mut bytes = version.to_be_bytes().to_vec();
        bytes.push(options);
        bytes.extend([1; 32]);
        bytes.extend([2; 32]);
        bytes.extend([0; 32]);
        bytes.extend(443_u16.to_be_bytes());
        bytes.extend(host.as_bytes());
        data_encoding::BASE64.encode(&bytes)
    }

    const NOW: i64 = 1_800_000_000;

    fn kind(text: &str) -> serde_json::Value {
        super::qr_kind(text, NOW)
    }

    #[test]
    fn qr_kinds() {
        let key = "ab".repeat(32);
        let contact = kind(&format!("3mid:abcd1234,{key}"));
        assert_eq!(contact["kind"], "contact");
        assert_eq!(contact["identity"], "ABCD1234");
        assert!(contact.get("error").is_none());
        assert_eq!(kind("https://example.org")["kind"], "unknown");
        assert_eq!(kind("")["kind"], "unknown");
    }

    #[test]
    fn a_damaged_or_expired_contact_code_says_so() {
        let key = "ab".repeat(32);
        assert_eq!(kind("3mid:ABCD1234,4a6a1b34")["error"], "qrInvalid");
        let past = kind(&format!("3mid:ABCD1234,{key},{}", NOW - 1));
        assert_eq!(past["kind"], "contact");
        assert_eq!(past["error"], "qrExpired");
        let future = kind(&format!("3mid:ABCD1234,{key},{}", NOW + 1));
        assert!(future.get("error").is_none());
        let unreadable = kind(&format!("3mid:ABCD1234,{key},soon"));
        assert!(unreadable.get("error").is_none());
    }

    #[test]
    fn a_web_code_says_where_it_goes() {
        let code = kind(&web_code(2, 0x02, "saltyrtc-01.threema.ch"));
        assert_eq!(code["kind"], "web");
        assert_eq!(code["host"], "saltyrtc-01.threema.ch");
    }

    #[test]
    fn only_a_small_version_makes_a_web_code_of_another_version() {
        let old = kind(&web_code(1, 0, "x.threema.ch"));
        assert_eq!(old["kind"], "web");
        assert_eq!(old["error"], "webVersion");
        // Long enough Base64 starting with anything else is not Threema Web's.
        assert_eq!(
            kind(&web_code(0x4142, 0, "x.threema.ch"))["kind"],
            "unknown"
        );
        assert_eq!(kind(&web_code(0, 0, "x.threema.ch"))["kind"], "unknown");
    }

    #[test]
    fn a_join_code_of_another_version_is_still_a_device_code() {
        let good = kind(&crate::rendezvous::test_join_code(2));
        assert_eq!(good["kind"], "device");
        assert!(good.get("error").is_none());
        let old = kind(&crate::rendezvous::test_join_code(1));
        assert_eq!(old["kind"], "device");
        assert_eq!(old["error"], "linkVersion");
    }

    #[test]
    fn links_with_an_id_are_links_and_others_unknown() {
        for text in [
            "https://threema.id/ABCD1234",
            "https://threema.id/ABCD1234?text=hello",
            "threema://add?id=ABCD1234",
            "https://go.threema.ch/add?id=ABCD1234",
            "threema://compose?id=ABCD1234",
        ] {
            let code = kind(text);
            assert_eq!(code["kind"], "link", "{text}");
            assert_eq!(code["identity"], "ABCD1234", "{text}");
        }
        for text in [
            "threema://compose?text=hello",
            "threema://license?key=ABCDE-12345",
            "https://threema.id/short",
        ] {
            assert_eq!(kind(text)["kind"], "unknown", "{text}");
        }
    }

    use super::*;

    #[test]
    fn license_key_format() {
        assert_eq!(
            normalize_license_key(" abcde-12345 ").as_deref(),
            Some("ABCDE-12345")
        );
        assert!(normalize_license_key("ABCDE12345").is_none());
        assert!(normalize_license_key("ABCDE-1234").is_none());
        assert!(normalize_license_key("ABCDÉ-12345").is_none());
        assert!(normalize_license_key("ABC-E-12345").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_session_termination_left_once_forward_security_is_back() {
        let fs_terminate = crate::multidevice::SETTING_FS_TERMINATE;
        let (_dir, app) = app();
        // A linking that activated multi-device and was rolled back.
        app.activate_multi_device().await.unwrap();
        assert!(app.store.setting(fs_terminate).unwrap().is_some());
        app.leave_device_group_locally().await;
        assert!(app.store.multi_device().unwrap().is_none());
        assert!(
            app.store.setting(fs_terminate).unwrap().is_none(),
            "the rollback must not end every session at the next login"
        );
        // The switch off and on again before any login.
        app.set_forward_security(false).await.unwrap();
        assert!(app.store.setting(fs_terminate).unwrap().is_some());
        app.set_forward_security(true).await.unwrap();
        assert!(app.store.setting(fs_terminate).unwrap().is_none());
        // Off stays off through a rollback: its termination still has to happen.
        app.set_forward_security(false).await.unwrap();
        app.activate_multi_device().await.unwrap();
        app.leave_device_group_locally().await;
        assert!(app.store.setting(fs_terminate).unwrap().is_some());
    }

    #[test]
    fn new_device_cookie_skips_its_own_change_indication() {
        let (_dir, app) = app();
        let first = app.device_cookie().unwrap();
        assert!(
            app.store
                .setting(keys::DEVICE_COOKIE_FRESH)
                .unwrap()
                .is_some(),
            "a new cookie's first change indication is ours"
        );
        app.store.delete_setting(keys::DEVICE_COOKIE_FRESH).unwrap();
        assert_eq!(app.device_cookie().unwrap().0, first.0, "stable");
        assert!(
            app.store
                .setting(keys::DEVICE_COOKIE_FRESH)
                .unwrap()
                .is_none(),
            "an existing cookie is not new"
        );
    }

    #[test]
    fn device_id_is_uuid_v4() {
        let id = random_device_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }

    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "US".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        (dir, app)
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    #[test]
    fn license_is_checked_daily() {
        const DAY: i64 = LICENSE_CHECK_INTERVAL_MS;
        assert!(license_check_due(None, 1000), "never checked");
        assert!(!license_check_due(Some(1000), 1000 + DAY - 1));
        assert!(license_check_due(Some(1000), 1000 + DAY));
        assert!(license_check_due(Some(1000), 999), "the clock went back");
    }

    /// The chat's window: a jump gets a bounded page around its target and says what lies
    /// beyond it, also with a lot unread; the ordinary opening still reaches the first unread.
    #[test]
    fn messages_around_a_message_stay_a_window() {
        let (_dir, app) = app();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        let ids: Vec<i64> = (0..400_u64)
            .map(|n| {
                app.store
                    .insert_incoming("ABCD1234", n + 1, "text", "x", 10)
                    .unwrap()
                    .unwrap()
                    .id
            })
            .collect();
        let rows = |value: &Value| -> Vec<i64> {
            let rows = value["messages"].as_array().unwrap();
            rows.iter().map(|m| m["id"].as_i64().unwrap()).collect()
        };

        let jump = app
            .messages_json("ABCD1234", Some(60), Some(ids[100]), None, None)
            .unwrap();
        assert_eq!(rows(&jump), ids[70..=130]);
        assert_eq!(jump["reachesNewest"], false);
        assert_eq!(jump["firstUnread"], ids[0]);

        let near_end = app
            .messages_json("ABCD1234", Some(60), Some(ids[390]), None, None)
            .unwrap();
        assert_eq!(rows(&near_end), ids[360..]);
        assert_eq!(near_end["reachesNewest"], true);

        let opening = app
            .messages_json("ABCD1234", Some(60), None, None, None)
            .unwrap();
        assert_eq!(rows(&opening), ids, "widened to reach the first unread");
        assert_eq!(opening["reachesNewest"], true);
    }

    /// With an ID and a key, in the state of a running app.
    fn licensed_app() -> (tempfile::TempDir, App) {
        let (dir, app) = app();
        app.store
            .set_setting(keys::LICENSE_KEY, "ABCDE-12345")
            .unwrap();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "30".into(),
            })
            .unwrap();
        (dir, app)
    }

    fn state(app: &mut App) -> String {
        app.status().unwrap()["state"].as_str().unwrap().to_owned()
    }

    #[test]
    fn only_the_directorys_no_stops_the_app() {
        let (_dir, mut app) = licensed_app();
        assert_eq!(state(&mut app), "ready");

        // No answer: being offline or a broken server costs nobody their messages.
        for answer in [
            DirectoryError::Status(500),
            DirectoryError::InvalidResponse("garbled".into()),
            DirectoryError::RateLimited,
            DirectoryError::Unexplained,
        ] {
            assert!(!app.apply_license_answer(Err(answer)).unwrap());
            assert_eq!(state(&mut app), "ready");
        }
        assert_eq!(app.store.setting(keys::LICENSE_CHECKED_AT).unwrap(), None);

        // An answer that says yes is remembered: no second request until the next day.
        let yes = directory::LicenseOk {
            update_message: None,
        };
        assert!(!app.apply_license_answer(Ok(yes)).unwrap());
        assert_eq!(state(&mut app), "ready");
        assert!(
            app.store
                .setting(keys::LICENSE_CHECKED_AT)
                .unwrap()
                .is_some()
        );

        // The directory's no: back to the license page, and it stays that way after a restart.
        let no = DirectoryError::Server("This license key is invalid or expired.".into());
        assert!(app.apply_license_answer(Err(no)).unwrap());
        assert_eq!(state(&mut app), "needsLicense");
        let status = app.license_status().unwrap();
        assert_eq!(status["key"], "ABCDE-12345", "for the page to show");
        assert_eq!(status["refused"], "This license key is invalid or expired.");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn too_many_requests_leaves_the_connection_running() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = licensed_app();
                assert!(app.ensure_connection().is_some(), "licensed: connects");
                assert!(
                    !app.apply_license_answer(Err(DirectoryError::RateLimited))
                        .unwrap()
                );
                assert!(app.connection.borrow().is_some(), "still running");
                assert_eq!(app.store.setting(keys::LICENSE_REFUSED).unwrap(), None);
                assert_eq!(app.store.setting(keys::LICENSE_CHECKED_AT).unwrap(), None);
                assert_eq!(state(&mut app), "ready");
            })
            .await;
    }

    /// Asking during a rest doesn't lengthen it: the hourly address book sync would otherwise keep
    /// it going for as long as the phone stays awake, license recheck included.
    #[tokio::test(flavor = "current_thread")]
    async fn a_directory_rest_ends_when_it_said_it_would() {
        let (_dir, mut app) = licensed_app();
        let automatic = r#"{"id":1,"cmd":"matchAddressBook","entries":[],"automatic":true}"#;
        let until = (crate::messages::now_ms() + 600_000).to_string();
        app.store
            .set_setting(keys::DIRECTORY_RETRY_AFTER, &until)
            .unwrap();
        let reply: Value = serde_json::from_str(&app.handle(automatic).await).unwrap();
        assert_eq!(reply["error"]["kind"], "rateLimited");
        app.store.rest_directory_calls();
        assert_eq!(
            app.store.setting(keys::DIRECTORY_RETRY_AFTER).unwrap(),
            Some(until),
            "neither the sync's own refusal nor another 429 moves the end"
        );

        // Over: the sync goes again, and a new 429 starts a new rest.
        let over = (crate::messages::now_ms() - 1).to_string();
        app.store
            .set_setting(keys::DIRECTORY_RETRY_AFTER, &over)
            .unwrap();
        let reply: Value = serde_json::from_str(&app.handle(automatic).await).unwrap();
        assert_eq!(reply["ok"], true);
        app.store.rest_directory_calls();
        assert!(app.store.directory_resting());
    }

    /// Without another device, too, a group's unknown member isn't lost to a rest or a failed
    /// request: messages to them wait for the contact.
    #[test]
    fn group_members_looked_up_during_a_rest_wait_for_its_end() {
        let (_dir, app) = licensed_app();
        assert!(app.store.multi_device().unwrap().is_none());
        app.store.rest_directory_calls();
        app.fetch_members(vec!["EFGH5678".to_owned(), "IJKL9012".to_owned()]);
        app.fetch_members(vec!["EFGH5678".to_owned()]);
        assert_eq!(app.member_lookups_due(), ["EFGH5678", "IJKL9012"]);

        // Any answer ends the wait: a contact, or that the directory doesn't know the ID.
        let found = crate::contacts::test_contact("EFGH5678");
        assert!(
            !crate::sync::members_looked_up(
                &app.store,
                vec![
                    (
                        "EFGH5678".to_owned(),
                        crate::contacts::Lookup::Found(Box::new(found))
                    ),
                    ("IJKL9012".to_owned(), crate::contacts::Lookup::Invalid),
                ],
            )
            .unwrap()
            .queued
        );
        assert!(app.store.contact("EFGH5678").unwrap().is_some());
        assert!(app.member_lookups_due().is_empty());
        assert_eq!(app.store.setting(keys::MEMBER_LOOKUPS_OWED).unwrap(), None);
    }

    /// Nothing automatic asks the directory during a rest. Without the gates these would either
    /// reach the server or, offline, leave an error behind.
    #[tokio::test(flavor = "current_thread")]
    async fn automatic_directory_calls_rest_after_too_many_requests() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = licensed_app();
                assert!(!app.store.directory_resting());
                assert!(
                    !app.apply_license_answer(Err(DirectoryError::RateLimited))
                        .unwrap()
                );
                assert!(app.store.directory_resting(), "the 429 starts the rest");

                app.store
                    .set_setting(keys::LINKED_EMAIL_PENDING, "1")
                    .unwrap();
                app.store
                    .set_setting(keys::LINKED_EMAIL, "someone@example.org")
                    .unwrap();
                app.store
                    .set_setting(
                        keys::SAFE_PENDING_IMPORT,
                        r#"{"contacts":[{"identity":"EFGH5678"}]}"#,
                    )
                    .unwrap();

                assert!(!app.recheck_license().await.unwrap());
                assert!(!app.check_email_link().await.unwrap());
                app.publish_feature_mask().await.unwrap();
                assert_eq!(
                    app.store.setting(keys::PUBLISHED_FEATURE_MASK).unwrap(),
                    None,
                    "publishes after the rest"
                );

                app.maintenance().await;
                assert_eq!(app.store.setting(keys::SAFE_IMPORT_ERROR).unwrap(), None);
                assert!(app.on_connected().await.is_empty());
                assert_eq!(app.store.setting(keys::SAFE_IMPORT_ERROR).unwrap(), None);
                assert_eq!(app.store.setting(keys::LICENSE_CHECKED_AT).unwrap(), None);

                // The hourly address book sync stays home; the user's own request goes on.
                let reply: Value = serde_json::from_str(
                    &app.handle(
                        r#"{"id":1,"cmd":"matchAddressBook","entries":[],"automatic":true}"#,
                    )
                    .await,
                )
                .unwrap();
                assert_eq!(reply["error"]["kind"], "rateLimited");
                let reply: Value = serde_json::from_str(
                    &app.handle(r#"{"id":2,"cmd":"matchAddressBook","entries":[]}"#)
                        .await,
                )
                .unwrap();
                assert_eq!(reply["ok"], true);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_second_id_takes_the_place_of_the_first() {
        let (_dir, app) = licensed_app();
        let other = StoredIdentity {
            threema_id: "EFGH5678".into(),
            client_key: Zeroizing::new([8; 32]),
            server_group: "31".into(),
        };
        // Refused before any request: nothing reaches the directory.
        assert!(matches!(
            app.adopt_identity("EFGH5678", &ClientKey::from([8; 32]))
                .await,
            Err(ApiError::IdentityExists)
        ));
        assert!(matches!(
            app.store_new_identity(&other),
            Err(ApiError::IdentityExists)
        ));
        assert_eq!(
            app.store.identity().unwrap().unwrap().threema_id,
            "ABCD1234"
        );
    }

    #[test]
    fn the_license_key_can_be_changed_until_there_is_an_id() {
        let (_dir, mut app) = app();
        app.store
            .set_setting(keys::LICENSE_KEY, "ABCDE-12345")
            .unwrap();
        assert_eq!(state(&mut app), "needsIdentity");
        assert_eq!(app.forget_license().unwrap()["state"], "needsLicense");
        assert_eq!(app.store.setting(keys::LICENSE_KEY).unwrap(), None);

        let (_dir, mut app) = licensed_app();
        assert!(matches!(
            app.forget_license(),
            Err(ApiError::IdentityExists)
        ));
        assert_eq!(state(&mut app), "ready");
    }

    #[test]
    fn a_new_or_restored_id_is_asked_for_a_nickname_once() {
        fn ask(app: &App) -> bool {
            app.profile_json().unwrap()["askNickname"] == true
        }
        let unanswered = |app: &App| app.store.setting(keys::NICKNAME_UNANSWERED).unwrap();

        // No question for an ID from before there was one.
        let (_dir, mut app) = licensed_app();
        assert!(!ask(&app));

        // Leaving the profile page's empty field is no answer.
        app.store
            .set_setting(keys::NICKNAME_UNANSWERED, "ask")
            .unwrap();
        assert!(ask(&app));
        app.set_nickname("  ").unwrap();
        assert_eq!(unanswered(&app).as_deref(), Some("ask"));

        // Not now: not asked again, and still nothing is removed at contacts.
        app.decline_nickname().unwrap();
        assert!(!ask(&app));
        assert_eq!(unanswered(&app).as_deref(), Some("asked"));

        // A name answers it, and taking it away later is meant.
        app.set_nickname("Tester").unwrap();
        assert_eq!(unanswered(&app), None);
        app.set_nickname("").unwrap();
        assert_eq!(app.store.setting(keys::NICKNAME).unwrap(), None);
        assert_eq!(unanswered(&app), None);
        app.decline_nickname().unwrap();
        assert_eq!(unanswered(&app), None, "nothing to decline any more");
    }

    /// Runs libthreema's real task against these answers: what comes out of it is what the user
    /// would read.
    fn create_identity_answered(answers: Vec<libthreema::https::HttpsResult>) -> ApiError {
        let (_dir, app) = app();
        app.store
            .set_setting(keys::LICENSE_KEY, "ABCDE-12345")
            .unwrap();
        let mut task = CreateIdentityTask::new(CreateIdentityContext {
            client_info: app.version.client_info(),
            config: Arc::clone(&app.config),
            flavor: Flavor::Consumer,
            serial_license: Some(app.license().unwrap()),
        });
        let mut answers = answers.into_iter();
        loop {
            match task.poll().map_err(create_identity_error) {
                Ok(CreateIdentityLoop::Instruction(_)) => {
                    let result = answers.next().expect("asked once too often");
                    if let Err(error) = task
                        .response(CreateIdentityResponse { result })
                        .map_err(create_identity_error)
                    {
                        return error;
                    }
                }
                Ok(CreateIdentityLoop::Done(_)) => panic!("an ID was created"),
                Err(error) => return error,
            }
        }
    }

    fn status(status: u16, body: &str) -> libthreema::https::HttpsResult {
        Ok(libthreema::https::HttpsResponse {
            status,
            body: body.as_bytes().to_vec(),
        })
    }

    #[test]
    fn creating_an_id_says_what_went_wrong_in_our_words() {
        assert!(matches!(
            create_identity_answered(vec![Err(libthreema::https::HttpsError::Unreachable(
                "details".into()
            ))]),
            ApiError::Network(_)
        ));
        assert!(matches!(
            create_identity_answered(vec![status(429, "")]),
            ApiError::RateLimited
        ));
        assert!(matches!(
            create_identity_answered(vec![status(500, "")]),
            ApiError::Internal(_)
        ));

        // The directory's own words come through without libthreema's decoration.
        let challenge = json!({
            "token": data_encoding::BASE64.encode(b"challenge"),
            "tokenRespKeyPub": data_encoding::BASE64.encode(&[2; 32]),
        })
        .to_string();
        let refused = create_identity_answered(vec![
            status(200, &challenge),
            status(200, r#"{"success":false,"error":"This key is used up."}"#),
        ]);
        assert!(
            matches!(&refused, ApiError::Server(words) if words == "This key is used up."),
            "{refused:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_sms_link_says_confirmed_when_the_number_already_is() {
        let (_dir, mut app) = licensed_app();
        let link = r#"{"id":1,"cmd":"verifyMobile","code":"1234","fromLink":true}"#;
        let typed = r#"{"id":2,"cmd":"verifyMobile","code":"1234"}"#;

        // Nothing linked: there is nothing to confirm, whichever way the code came.
        let reply: Value = serde_json::from_str(&app.handle(link).await).unwrap();
        assert_eq!(reply["error"]["kind"], "noVerification");

        // Tapped twice, or after typing the code.
        app.store
            .set_setting(keys::LINKED_MOBILE, "41790000000")
            .unwrap();
        let reply: Value = serde_json::from_str(&app.handle(link).await).unwrap();
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result"]["linkedMobile"], "41790000000");

        // The dialog is only there with something pending, and keeps its error.
        let reply: Value = serde_json::from_str(&app.handle(typed).await).unwrap();
        assert_eq!(reply["error"]["kind"], "noVerification");
    }

    #[test]
    fn a_refusal_that_was_never_the_directorys_is_forgotten() {
        let (_dir, mut app) = licensed_app();
        for text in [
            "Too many requests, please try again later.",
            "Unknown server error",
        ] {
            app.store.set_setting(keys::LICENSE_REFUSED, text).unwrap();
            forget_false_refusal(&app.store).unwrap();
            assert_eq!(state(&mut app), "ready", "{text}");
        }
        let real = "This license key is invalid or expired.";
        app.store.set_setting(keys::LICENSE_REFUSED, real).unwrap();
        forget_false_refusal(&app.store).unwrap();
        assert_eq!(state(&mut app), "needsLicense");
    }

    #[test]
    fn rate_limits_and_silence_have_their_own_kinds() {
        for (error, kind) in [
            (DirectoryError::RateLimited, "rateLimited"),
            (DirectoryError::Unexplained, "serverUnexplained"),
            (DirectoryError::ReportRefused, "reportRefused"),
            (DirectoryError::Server("Its words".into()), "server"),
        ] {
            assert_eq!(ApiError::from(error).kind(), kind);
        }
    }

    /// What goes on after a refusal: a login in progress would go through with the old key, and
    /// send what is queued. So it is stopped where it stands, not only told at its next read.
    #[tokio::test(flavor = "current_thread")]
    async fn a_login_in_progress_ends_with_the_stop() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = licensed_app();
                let (login_answer, login) = tokio::sync::oneshot::channel::<()>();
                app.connection_task = Some(tokio::task::spawn_local(async move {
                    let _ = login.await;
                    panic!("the login went on");
                }));
                tokio::task::yield_now().await;
                app.stop_connection();
                app.connection_gone().await;
                assert!(login_answer.send(()).is_err(), "nobody waits for it now");
            })
            .await;
    }

    /// The wipe can't begin inside what the task had open, and comes after all it writes on its
    /// way out.
    #[tokio::test(flavor = "current_thread")]
    async fn the_wipe_waits_until_the_connection_is_gone() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = licensed_app();
                let store = std::rc::Rc::clone(&app.store);
                app.connection_task = Some(tokio::task::spawn_local(async move {
                    let _step = store.step().unwrap();
                    std::future::pending::<()>().await;
                }));
                tokio::task::yield_now().await;
                app.delete_all_data().await.unwrap();
                assert!(app.store.identity().unwrap().is_none());
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_connection_after_the_directorys_no() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = licensed_app();
                assert!(app.ensure_connection().is_some(), "licensed: connects");
                let no = DirectoryError::Server("no".into());
                assert!(app.apply_license_answer(Err(no)).unwrap());
                assert!(
                    app.connection.borrow().is_none(),
                    "the running one is stopped"
                );
                assert!(app.ensure_connection().is_none(), "and none starts");
                assert!(app.maintenance().await.is_empty(), "nothing else goes out");
            })
            .await;
    }

    #[test]
    fn an_id_without_a_key_needs_a_license() {
        let (_dir, mut app) = licensed_app();
        app.store.delete_setting(keys::LICENSE_KEY).unwrap();
        assert_eq!(state(&mut app), "needsLicense");
    }

    #[test]
    fn status_and_offline_errors() {
        let (_dir, mut app) = app();
        let rt = rt();
        let reply: Value =
            serde_json::from_str(&rt.block_on(app.handle(r#"{"id":1,"cmd":"status"}"#))).unwrap();
        assert_eq!(reply["reply"], 1);
        assert_eq!(reply["result"]["state"], "needsLicense");

        let reply: Value =
            serde_json::from_str(&rt.block_on(app.handle(r#"{"id":2,"cmd":"createIdentity"}"#)))
                .unwrap();
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"]["kind"], "noLicense");

        let reply: Value = serde_json::from_str(
            &rt.block_on(app.handle(r#"{"id":3,"cmd":"checkLicense","licenseKey":"nope"}"#)),
        )
        .unwrap();
        assert_eq!(reply["error"]["kind"], "licenseFormat");

        let reply: Value = serde_json::from_str(
            &rt.block_on(app.handle(r#"{"id":4,"cmd":"bogus","x":"secret"}"#)),
        )
        .unwrap();
        assert_eq!(reply["reply"], 4);
        assert_eq!(reply["error"]["kind"], "internal");
        assert!(!reply.to_string().contains("secret"));
    }

    /// The page of a table, unreadable from now on.
    fn damage_table(path: &std::path::Path, key: &[u8; 32], table: &str) {
        let store = Store::open(path, key).unwrap();
        let root: usize = store
            .conn
            .query_row(
                "SELECT rootpage FROM sqlite_master WHERE name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        drop(store);
        let mut raw = std::fs::read(path).unwrap();
        raw[(root - 1) * 4096 + 100..(root - 1) * 4096 + 200].fill(0xA5);
        std::fs::write(path, raw).unwrap();
    }

    fn start_config(dir: &std::path::Path) -> StartConfig {
        serde_json::from_value(json!({
            "dataDir": dir,
            "appVersion": "0.0.0",
            "language": "en",
            "country": "CH",
            "model": "test",
            "osVersion": "5.1",
        }))
        .unwrap()
    }

    /// The core comes up and the status the bridge waits for is what fails: it gets a kind it can
    /// show, not a spinner.
    #[test]
    fn a_damaged_identity_page_makes_the_status_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let config = start_config(dir.path());
        drop(App::new(&config, &[3; 32]).unwrap());
        damage_table(&dir.path().join("threesea.db"), &[3; 32], "identity");

        let mut app = App::new(&config, &[3; 32]).unwrap();
        let reply: Value = serde_json::from_str(
            &tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(app.handle(r#"{"id":1,"cmd":"status"}"#)),
        )
        .unwrap();

        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(reply["error"]["kind"], "storeDamaged", "{reply}");
        assert_eq!(reply["error"]["message"], "sqlite 11", "{reply}");
    }

    #[test]
    fn a_store_that_fails_while_opening_gives_its_kind_and_a_code() {
        let dir = tempfile::tempdir().unwrap();
        let config = start_config(dir.path());
        drop(App::new(&config, &[3; 32]).unwrap());
        damage_table(
            &dir.path().join("threesea.db"),
            &[3; 32],
            "sqlite_autoindex_settings_1",
        );

        let error = App::new(&config, &[3; 32]).err().unwrap();
        assert_eq!(error.kind(), "storeDamaged");
        assert_eq!(error.to_string(), "sqlite 11");

        let error = App::new(&config, &[4; 32]).err().unwrap();
        assert_eq!(error.kind(), "storeKey");
    }

    /// What the user waits for between tapping the icon and seeing the chat list: opening the
    /// encrypted store (SQLCipher's key derivation and the migrations), and the first calls the
    /// UI makes. A store that is already filled, as a phone's is.
    /// `cargo test -p threesea-core --release --lib bench_start -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark"]
    fn bench_start() {
        let dir = tempfile::tempdir().unwrap();
        let config = StartConfig {
            data_dir: dir.path().to_owned(),
            app_version: "0.1.0".into(),
            language: "en".into(),
            country: "US".into(),
            model: "test".into(),
            os_version: "5.1".into(),
            pictures_dir: None,
            calls: false,
            log_level: None,
            update_check: false,
        };
        let time = |name: &str, f: &dyn Fn()| {
            let mut runs: Vec<f64> = (0..5)
                .map(|_| {
                    let start = std::time::Instant::now();
                    f();
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            runs.sort_by(f64::total_cmp);
            println!("{name}: {:.2} ms (median of 5)", runs[2]);
        };
        {
            let app = App::new(&config, &[3; 32]).unwrap();
            let (chats, per_chat) = (200_u64, 50_u64);
            let tx = app.store.conn.unchecked_transaction().unwrap();
            for chat in 0..chats {
                app.store
                    .upsert_contact(&crate::contacts::test_contact(&format!("CCCC{chat:04}")))
                    .unwrap();
                for n in 0..per_chat {
                    app.store
                        .insert_incoming(
                            &format!("CCCC{chat:04}"),
                            chat * per_chat + n,
                            "text",
                            "a message of ordinary length, as they are",
                            i64::try_from(chat * per_chat + n).unwrap(),
                        )
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        }
        println!("store: {} chats, {} messages", 200, 200 * 50,);
        time("App::new (open, key, migrations)", &|| {
            App::new(&config, &[3; 32]).unwrap();
        });
        let app = App::new(&config, &[3; 32]).unwrap();
        time("status", &|| {
            app.status().unwrap();
        });
        time("conversations (the first list)", &|| {
            app.conversations_json(false, false).unwrap();
        });
        time("contacts", &|| {
            app.contacts_json().unwrap();
        });
        time("messages (opening a chat)", &|| {
            app.messages_json("CCCC0001", Some(50), None, None, None)
                .unwrap();
        });
    }

    /// How long the chat list the UI waits for takes to build (the per-chat summary of
    /// migration 33 and the batched lookups).
    /// `cargo test -p threesea-core --release --lib bench_chat_list -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark"]
    fn bench_chat_list() {
        let (_dir, app) = app();
        let (chats, per_chat) = (1000_u64, 20_u64);
        let tx = app.store.conn.unchecked_transaction().unwrap();
        for chat in 0..chats {
            let identity = format!("C{chat:07}");
            app.store
                .upsert_contact(&crate::contacts::test_contact(&format!("CCCC{chat:04}")))
                .unwrap();
            for n in 0..per_chat {
                app.store
                    .insert_incoming(
                        &identity,
                        chat * per_chat + n,
                        "text",
                        &format!("message {n}"),
                        i64::try_from(chat * per_chat + n).unwrap(),
                    )
                    .unwrap();
            }
        }
        tx.commit().unwrap();
        let time = |name: &str, f: &dyn Fn()| {
            f();
            let mut runs: Vec<f64> = (0..5)
                .map(|_| {
                    let start = std::time::Instant::now();
                    f();
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            runs.sort_by(f64::total_cmp);
            println!("{name}: {:.2} ms (median of 5)", runs[2]);
        };
        time("conversations (store)", &|| {
            assert_eq!(app.store.conversations().unwrap().len(), 1000);
        });
        time("conversationsJson (what the UI gets)", &|| {
            let json = app.conversations_json(false, false).unwrap();
            assert_eq!(json["conversations"].as_array().unwrap().len(), 1000);
        });
    }

    /// The cover's count on its own says what the chat list's reply says, and like it never
    /// counts or names an archived or a private chat.
    #[test]
    fn unread_summary_says_what_the_chat_list_says() {
        let (_dir, app) = app();
        let same_as_the_list = |app: &App| {
            let list = app.conversations_json(false, false).unwrap();
            let summary = app.unread_summary_json().unwrap();
            assert_eq!(summary["unreadTotal"], list["unreadTotal"]);
            assert_eq!(summary["unreadFrom"], list["unreadFrom"]);
            summary
        };
        assert_eq!(same_as_the_list(&app)["unreadTotal"], 0);
        assert!(same_as_the_list(&app)["unreadFrom"].is_null());

        let mut uid = 0;
        for (at, id) in [(1, "AAAAAAAA"), (2, "BBBBBBBB"), (3, "CCCCCCCC")] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
            for _ in 0..2 {
                uid += 1;
                app.store
                    .insert_incoming(id, uid, "text", "words", at)
                    .unwrap();
            }
        }
        let summary = same_as_the_list(&app);
        assert_eq!(summary["unreadTotal"], 6);
        assert_eq!(summary["unreadFrom"], "CCCCCCCC");

        app.store.set_private("CCCCCCCC", true).unwrap();
        let summary = same_as_the_list(&app);
        assert_eq!(summary["unreadTotal"], 4);
        assert_eq!(
            summary["unreadFrom"], "BBBBBBBB",
            "a private chat isn't named"
        );

        app.store.set_archived("BBBBBBBB", true).unwrap();
        let summary = same_as_the_list(&app);
        assert_eq!(summary["unreadTotal"], 2);
        assert_eq!(summary["unreadFrom"], "AAAAAAAA", "nor an archived one");
    }

    /// The emulator's demo data (feature `demo`): a chat list to work against, and gone again
    /// on demand.
    #[cfg(feature = "demo")]
    #[test]
    fn demo_data_fills_and_clears_the_chat_list() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([5; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        app.demo_data(false, Some(20)).unwrap();

        let list = app.conversations_json(false, false).unwrap();
        let chats = list["conversations"].as_array().unwrap();
        assert!(
            chats.len() >= 8,
            "contacts, a group, the notes group, a list"
        );
        assert_eq!(list["archivedCount"], 1);
        // Bruno's one, Carla's three, Farid's one and the group's two.
        assert_eq!(list["unreadTotal"], 7);
        assert_eq!(list["unreadFrom"], "Carla Mendes");
        let summary = app.unread_summary_json().unwrap();
        assert_eq!(summary["unreadTotal"], list["unreadTotal"]);
        assert_eq!(summary["unreadFrom"], list["unreadFrom"]);
        assert_eq!(chats[0]["displayName"], "Ada Lindqvist", "pinned first");

        let long = app
            .messages_json("DEMO0005", Some(100), None, None, None)
            .unwrap();
        assert_eq!(long["messages"].as_array().unwrap().len(), 20);

        // The pictures and the voice message are on the device, not on a server.
        let media = app.chat_media("DEMO0006", None, None, None).unwrap();
        let ids: Vec<i64> = media["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_i64().unwrap())
            .collect();
        assert!(ids.len() >= 2, "two pictures at least");
        assert!(
            ids.iter()
                .any(|id| app.media.has(*id, crate::media::MediaPart::File)),
            "a picture's bytes are there"
        );

        // Nothing the connection would send: a message left in state 0 is encoded and sent
        // after every connect, and nobody is behind a demo ID (found in the audit, 2026-10-30).
        assert!(
            app.store.pending_outgoing().unwrap().is_empty(),
            "demo data must not queue anything for sending"
        );
        // And nothing automatic either: no read receipts, no typing indicators.
        for (identity, ..) in [("DEMO0001", 0), ("DEMO0006", 0)] {
            let contact = app.store.contact(identity).unwrap().unwrap();
            assert_eq!(
                contact.read_receipt_policy_override,
                Some(libthreema::protobuf::d2d_sync::ReadReceiptPolicy::DontSendReadReceipt)
            );
            assert_eq!(
                contact.typing_indicator_policy_override,
                Some(
                    libthreema::protobuf::d2d_sync::TypingIndicatorPolicy::DontSendTypingIndicator
                )
            );
        }

        app.demo_data(true, None).unwrap();
        let list = app.conversations_json(false, false).unwrap();
        assert!(list["conversations"].as_array().unwrap().is_empty());
        assert_eq!(list["unreadTotal"], 0);
        assert!(app.store.contact("DEMO0001").unwrap().is_none());
        assert!(app.store.groups().unwrap().is_empty());
    }

    /// A command's own arguments share the request's object, so none of them may be called
    /// `id`: renaming a linked device was refused as a whole ("invalid request") because the
    /// device's id was overwritten by the request's (user, 2026-10-30).
    #[test]
    fn command_arguments_never_collide_with_the_request_id() {
        let (_dir, mut app) = app();
        let rt = rt();
        let request = json!({
            "id": 7, "cmd": "setDeviceAlias",
            "device": "0011223344556677", "alias": "Laptop"
        });
        let reply: Value =
            serde_json::from_str(&rt.block_on(app.handle(&request.to_string()))).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["reply"], 7);
        assert_eq!(
            app.store
                .setting(&format!(
                    "{}0011223344556677",
                    crate::multidevice::SETTING_ALIAS_PREFIX
                ))
                .unwrap()
                .as_deref(),
            Some("Laptop")
        );

        // No command may take an argument called `id` or `cmd` again: both are the envelope's.
        let commands = std::fs::read_to_string("src/app.rs").unwrap();
        let enum_start = commands.find("pub enum Command {").unwrap();
        let enum_end = commands[enum_start..].find("\n}\n").unwrap() + enum_start;
        for line in commands[enum_start..enum_end].lines() {
            let field = line.trim();
            assert!(
                !(field.starts_with("id:") || field.starts_with("cmd:")),
                "a command takes `{field}`, which the request's own fields overwrite"
            );
        }
    }

    /// A deleted chat's notification goes too, and with it the token of its actions.
    #[test]
    fn deleting_a_chat_tells_the_ui() {
        let (_dir, mut app) = app();
        let mut events = app.take_media_events().unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        app.delete_conversation("ABCD1234").unwrap();
        let event = events.try_recv().unwrap().to_json(&app.store, true);
        assert_eq!(event["event"], "conversationRead");
        assert_eq!(event["identity"], "ABCD1234");
        // An open chat page stops keeping its draft.
        let event = events.try_recv().unwrap().to_json(&app.store, true);
        assert_eq!(event["event"], "chatDeleted");
        assert_eq!(event["identity"], "ABCD1234");
    }

    /// The picture is on the disk, whichever device deleted the group.
    #[test]
    fn a_group_deleted_on_another_device_leaves_no_picture_behind() {
        let (_dir, mut app) = app();
        let mut events = app.take_media_events().unwrap();
        let key = "g:BBBBBBBB:0000000000000007".to_owned();
        app.media.write_picture(&key, b"picture").unwrap();

        app.on_core_event(&crate::providers::CoreEvent::GroupDeleted(key.clone()));
        assert!(app.media.read_picture(&key).unwrap().is_none());
        let event = events.try_recv().unwrap().to_json(&app.store, true);
        assert_eq!(event["event"], "conversationRead");

        // A hidden private group: its name stays out of what the UI hears.
        app.store.set_private(&key, true).unwrap();
        app.unlocked.set(false);
        let event = events.try_recv().unwrap().to_json(&app.store, true);
        assert_eq!(event["event"], "chatDeleted");
        app.on_core_event(&crate::providers::CoreEvent::GroupDeleted(key));
        assert!(events.try_recv().is_err());
    }

    fn call(app: &mut App, request: serde_json::Value) -> serde_json::Value {
        let reply = rt().block_on(app.handle(&request.to_string()));
        serde_json::from_str(&reply).unwrap()
    }

    fn app_with_draft(text: &str) -> (tempfile::TempDir, App) {
        let (dir, mut app) = app();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        let reply = call(
            &mut app,
            json!({"id": 1, "cmd": "setDraft", "identity": "ABCD1234", "text": text}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        (dir, app)
    }

    fn stored_draft(app: &mut App) -> serde_json::Value {
        call(
            app,
            json!({"id": 2, "cmd": "draft", "identity": "ABCD1234"}),
        )["result"]
            .clone()
    }

    /// A draft comes back as it was written: mentions as tags with the names beside them, and
    /// what it quotes.
    #[test]
    fn a_draft_keeps_its_mentions_and_quote() {
        let (_dir, mut app) = app_with_draft("first");
        let quoted = app
            .store
            .insert_incoming("ABCD1234", 0x1234, "text", "the quoted one", 1)
            .unwrap()
            .unwrap();
        let mut named = crate::contacts::test_contact("EFGH5678");
        named.nickname = Some("Anna".into());
        app.store.upsert_contact(&named).unwrap();
        let text = "@[EFGH5678] and @[@@@@@@@@], please look";
        let reply = call(
            &mut app,
            json!({"id": 3, "cmd": "setDraft", "identity": "ABCD1234", "text": text,
                   "quote": format!("{:016x}", quoted.message_id)}),
        );
        assert_eq!(reply["ok"], true, "{reply}");

        let draft = stored_draft(&mut app);
        assert_eq!(draft["text"], text);
        assert_eq!(draft["mentions"]["EFGH5678"], "~Anna");
        assert_eq!(draft["quote"]["id"], quoted.id);

        // The chat list says the names too.
        let list = app.conversations_json(false, false).unwrap();
        let entry = list["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["identity"] == "ABCD1234")
            .unwrap();
        assert_eq!(entry["draft"], text);
        assert_eq!(entry["draftMentions"]["EFGH5678"], "~Anna");

        // Without a quote the old one goes; a draft that isn't one any more takes it along.
        call(
            &mut app,
            json!({"id": 4, "cmd": "setDraft", "identity": "ABCD1234", "text": "second"}),
        );
        assert_eq!(stored_draft(&mut app)["quote"], serde_json::Value::Null);
        call(
            &mut app,
            json!({"id": 5, "cmd": "setDraft", "identity": "ABCD1234", "text": "third",
                   "quote": format!("{:016x}", quoted.message_id)}),
        );
        call(
            &mut app,
            json!({"id": 6, "cmd": "setDraft", "identity": "ABCD1234", "text": "  "}),
        );
        let draft = stored_draft(&mut app);
        assert_eq!(draft["text"], serde_json::Value::Null);
        assert_eq!(draft["quote"], serde_json::Value::Null);
        assert_eq!(
            app.store.setting("draftQuote:ABCD1234").unwrap(),
            None,
            "nothing is left of the quote"
        );
    }

    /// A draft whose quoted message is gone is still the text.
    #[test]
    fn a_draft_outlives_its_quoted_message() {
        let (_dir, mut app) = app_with_draft("first");
        call(
            &mut app,
            json!({"id": 3, "cmd": "setDraft", "identity": "ABCD1234", "text": "kept",
                   "quote": "00000000000000ff"}),
        );
        let draft = stored_draft(&mut app);
        assert_eq!(draft["text"], "kept");
        assert_eq!(draft["quote"], serde_json::Value::Null);
    }

    /// The draft goes when the core has the message, and not before: a refused text is still
    /// there after the app ended, and another draft written meanwhile stays.
    #[test]
    fn only_an_accepted_send_takes_its_draft_away() {
        let long = "x".repeat(6001);
        let (_dir, mut app) = app_with_draft(&long);
        let reply = call(
            &mut app,
            json!({"id": 3, "cmd": "sendText", "identity": "ABCD1234", "text": long}),
        );
        assert_eq!(reply["error"]["kind"], "messageTooLong", "{reply}");
        assert_eq!(stored_draft(&mut app)["text"], long);

        call(
            &mut app,
            json!({"id": 4, "cmd": "setDraft", "identity": "ABCD1234", "text": "typed on"}),
        );
        let reply = call(
            &mut app,
            json!({"id": 5, "cmd": "sendText", "identity": "ABCD1234", "text": "something else"}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(stored_draft(&mut app)["text"], "typed on");

        call(
            &mut app,
            json!({"id": 6, "cmd": "setDraft", "identity": "ABCD1234", "text": " hello \n",
                   "quote": "00000000000000ff"}),
        );
        let reply = call(
            &mut app,
            json!({"id": 7, "cmd": "sendText", "identity": "ABCD1234", "text": "hello"}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(stored_draft(&mut app)["text"], serde_json::Value::Null);
        assert_eq!(app.store.setting("draftQuote:ABCD1234").unwrap(), None);
    }

    /// The same for a file's caption, which goes out with the file.
    #[test]
    fn only_an_accepted_file_takes_its_caption_draft_away() {
        let long = "x".repeat(6001);
        let (_dir, mut app) = app_with_draft(&long);
        let send = |app: &mut App, caption: &str| {
            let request = json!({"id": 3, "cmd": "sendFile", "identity": "ABCD1234",
                                 "rendering": "file", "mediaType": "text/plain",
                                 "name": "a.txt", "caption": caption})
            .to_string();
            // The upload it spawns goes with the set: this answers without waiting for anything.
            let local = tokio::task::LocalSet::new();
            let reply = rt().block_on(
                local.run_until(app.handle_with(&request, vec![vec![1_u8, 2, 3].into()])),
            );
            serde_json::from_str::<serde_json::Value>(&reply).unwrap()
        };
        let reply = send(&mut app, &long);
        assert_eq!(reply["error"]["kind"], "messageTooLong", "{reply}");
        assert_eq!(stored_draft(&mut app)["text"], long);

        call(
            &mut app,
            json!({"id": 4, "cmd": "setDraft", "identity": "ABCD1234", "text": "a caption"}),
        );
        let reply = send(&mut app, "");
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(stored_draft(&mut app)["text"], "a caption");
        let reply = send(&mut app, "a caption");
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(stored_draft(&mut app)["text"], serde_json::Value::Null);
    }

    /// A caption within its limit can be too long once escaped for the message's JSON: refused
    /// before anything is stored or uploaded, 1:1 and in a group, and not failing at sending.
    #[test]
    fn a_caption_too_long_once_escaped_is_refused_up_front() {
        let (_dir, mut app) = app();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        let group = "g:ABCD1234:0000000000000001";
        app.store
            .upsert_group(&crate::groups::Group {
                key: group.to_owned(),
                identity: crate::groups::parse_key(group).unwrap(),
                name: "Crew".into(),
                members: vec!["ABCD1234".to_owned()],
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        let escaped = "a\n".repeat(3000);
        assert_eq!(escaped.len(), 6000);
        for chat in ["ABCD1234", group] {
            let reply = send_file(&mut app, chat, &escaped);
            assert_eq!(reply["error"]["kind"], "messageTooLong", "{reply}");
            assert_eq!(stored_messages(&app, chat), 0);

            let reply = send_file(&mut app, chat, &"a".repeat(6000));
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(stored_messages(&app, chat), 1);
        }
    }

    /// Private chats ABCD1234 (with a message) and EFGH5678, a plain chat IJKL9012, hidden with a
    /// pass for `files` files of ABCD1234.
    fn app_with_write_pass(files: u32) -> (tempfile::TempDir, App) {
        let (dir, mut app) = app();
        let set = json!({"id": 1, "cmd": "setLock", "enabled": true, "pin": "2468"});
        assert_eq!(call(&mut app, set)["ok"], true);
        for id in ["ABCD1234", "EFGH5678", "IJKL9012"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
            app.store
                .insert_incoming(id, 1, "text", "secret words", 1)
                .unwrap();
        }
        app.store.set_private("ABCD1234", true).unwrap();
        app.store.set_private("EFGH5678", true).unwrap();
        let lock = json!({"id": 2, "cmd": "lock", "pendingFiles": {"ABCD1234": files}});
        let reply = call(&mut app, lock);
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(!app.unlocked.get(), "the lock takes effect at once");
        (dir, app)
    }

    fn send_file(app: &mut App, identity: &str, caption: &str) -> serde_json::Value {
        let request = json!({"id": 3, "cmd": "sendFile", "identity": identity,
                             "rendering": "file", "mediaType": "text/plain",
                             "name": "a.txt", "caption": caption})
        .to_string();
        let local = tokio::task::LocalSet::new();
        let reply = rt()
            .block_on(local.run_until(app.handle_with(&request, vec![vec![1_u8, 2, 3].into()])));
        serde_json::from_str(&reply).unwrap()
    }

    fn stored_messages(app: &App, identity: &str) -> usize {
        app.store.conversation_messages(identity, 10).unwrap().len()
    }

    /// A request as the core loop handles it; `None` while its reply is still to come.
    async fn from_ui(app: &mut App, request: serde_json::Value) -> Option<serde_json::Value> {
        let attachments = vec![vec![1_u8, 2, 3].into()];
        let reply = app
            .handle_from_ui(&request.to_string(), attachments)
            .await?;
        Some(serde_json::from_str(&reply).unwrap())
    }

    fn file_request(id: u64, identity: &str, name: &str) -> serde_json::Value {
        json!({"id": id, "cmd": "sendFile", "identity": identity, "rendering": "file",
               "mediaType": "text/plain", "name": name, "caption": ""})
    }

    #[test]
    fn a_file_from_the_ui_holds_up_no_command_behind_it() {
        let (_dir, mut app) = app_with_write_pass(0);
        let mut done = app.take_file_work().unwrap();
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(
                from_ui(&mut app, file_request(3, "IJKL9012", "a.txt"))
                    .await
                    .is_none()
            );
            assert_eq!(stored_messages(&app, "IJKL9012"), 1, "not stored yet");
            let next = from_ui(&mut app, json!({"id": 4, "cmd": "connectionState"})).await;
            assert_eq!(next.unwrap()["ok"], true);

            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["reply"], 3);
            assert_eq!(reply["result"]["kind"], "file", "{reply}");
            let id = reply["result"]["id"].as_i64().unwrap();
            assert_eq!(stored_messages(&app, "IJKL9012"), 2);
            let stored = app.media.read(id, crate::media::MediaPart::File);
            assert_eq!(stored.unwrap().unwrap().as_slice(), [1, 2, 3]);
            assert_eq!(app.file_work_running, 0);
        }));
    }

    #[test]
    fn files_from_the_ui_are_stored_in_the_order_they_came() {
        let (_dir, mut app) = app_with_write_pass(0);
        let mut done = app.take_file_work().unwrap();
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            for (id, name) in [(3, "first.txt"), (4, "second.txt"), (5, "third.txt")] {
                assert!(
                    from_ui(&mut app, file_request(id, "IJKL9012", name))
                        .await
                        .is_none()
                );
            }
            // All of them before the first finishes, whose upload would go to the network.
            let finished = [
                done.recv().await.unwrap(),
                done.recv().await.unwrap(),
                done.recv().await.unwrap(),
            ];
            for (finished, id) in finished.into_iter().zip([3, 4, 5]) {
                let reply: serde_json::Value =
                    serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
                assert_eq!(reply["reply"], id, "{reply}");
            }
        }));
    }

    /// Refused at once: there is nothing to wait for.
    #[test]
    fn a_file_from_the_ui_for_a_hidden_chat_is_refused_as_ever() {
        let (_dir, mut app) = app_with_write_pass(0);
        let local = tokio::task::LocalSet::new();
        let reply = rt()
            .block_on(local.run_until(from_ui(&mut app, file_request(3, "EFGH5678", "a.txt"))))
            .unwrap();
        assert_eq!(reply["error"]["kind"], "locked", "{reply}");
        assert_eq!(stored_messages(&app, "EFGH5678"), 1);
    }

    /// Under a pass, too, it goes to the blocking thread, and the answer tells nothing.
    #[test]
    fn a_file_under_a_pass_from_the_ui_answers_as_under_a_pass() {
        let (_dir, mut app) = app_with_write_pass(1);
        let mut done = app.take_file_work().unwrap();
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(
                from_ui(&mut app, file_request(3, "ABCD1234", "a.txt"))
                    .await
                    .is_none()
            );
            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(reply["result"], json!({}));
        }));
        assert_eq!(
            app.store
                .conversation_messages("ABCD1234", 10)
                .unwrap()
                .len(),
            2
        );
    }

    /// The app is locked while a file is on its way: it was sent while it could be, and it counts,
    /// but the answer says nothing of the chat and the UI's readers don't get at it.
    #[test]
    fn a_chat_hidden_while_a_file_was_on_its_way_stays_hidden() {
        let (_dir, mut app) = app_with_write_pass(0);
        let unlock = json!({"id": 2, "cmd": "unlock", "pin": "2468", "privateChats": true});
        assert_eq!(call(&mut app, unlock)["ok"], true);
        let mut done = app.take_file_work().unwrap();
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(
                from_ui(&mut app, file_request(3, "ABCD1234", "a.txt"))
                    .await
                    .is_none()
            );
            let lock = from_ui(&mut app, json!({"id": 4, "cmd": "lock"}))
                .await
                .unwrap();
            assert_eq!(lock["ok"], true);

            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(reply["result"], json!({}), "not a word of the chat");
        }));
        let messages = app.store.conversation_messages("ABCD1234", 10).unwrap();
        assert_eq!(messages.len(), 2);
        let file = messages
            .iter()
            .find(|message| message.kind == "file")
            .unwrap();
        assert!(!app.media.shown(file.id), "the readers stay out");
    }

    /// A list's members get their copies by reading its own back, which a hidden one refuses.
    #[test]
    fn a_list_hidden_while_a_file_was_on_its_way_refuses_it() {
        let (_dir, mut app) = app_with_write_pass(0);
        let unlock = json!({"id": 2, "cmd": "unlock", "pin": "2468", "privateChats": true});
        assert_eq!(call(&mut app, unlock)["ok"], true);
        let list = app
            .store
            .create_distribution_list("List", vec!["IJKL9012".to_owned()])
            .unwrap()
            .key;
        let private = json!({"id": 4, "cmd": "setPrivate", "identity": list, "private": true});
        assert_eq!(call(&mut app, private)["ok"], true);
        let mut done = app.take_file_work().unwrap();
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(
                from_ui(&mut app, file_request(5, &list, "a.txt"))
                    .await
                    .is_none()
            );
            let lock = from_ui(&mut app, json!({"id": 6, "cmd": "lock"}))
                .await
                .unwrap();
            assert_eq!(lock["ok"], true);

            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["error"]["kind"], "locked", "{reply}");
        }));
        assert_eq!(stored_messages(&app, &list), 0);
    }

    /// A contact removed while its file was prepared: nothing of the file stays on the disk.
    #[test]
    fn a_file_whose_chat_went_meanwhile_leaves_nothing() {
        let (dir, mut app) = app_with_write_pass(0);
        let mut done = app.take_file_work().unwrap();
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(
                from_ui(&mut app, file_request(3, "IJKL9012", "a.txt"))
                    .await
                    .is_none()
            );
            let finished = done.recv().await.unwrap();
            app.store.delete_contact("IJKL9012").unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["ok"], false, "{reply}");
        }));
        let left = std::fs::read_dir(dir.path().join("media")).unwrap().count();
        assert_eq!(left, 0);
    }

    #[test]
    fn saving_a_file_from_the_ui_answers_when_it_is_written() {
        let (dir, mut app) = app_with_write_pass(0);
        let downloads = dir.path().join("Downloads");
        std::fs::create_dir(&downloads).unwrap();
        let message = app
            .store
            .insert_incoming(
                "IJKL9012",
                2,
                "file",
                &json!({"name": "a.txt", "mediaType": "text/plain"}).to_string(),
                1,
            )
            .unwrap()
            .unwrap()
            .id;
        app.media
            .write(message, crate::media::MediaPart::File, b"content")
            .unwrap();
        let mut done = app.take_file_work().unwrap();
        let request = json!({"id": 3, "cmd": "saveFile", "message": message,
                             "directory": downloads});
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(from_ui(&mut app, request).await.is_none());
            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["reply"], 3);
            let path = reply["result"]["path"].as_str().unwrap();
            assert_eq!(std::fs::read(path).unwrap(), b"content");
        }));
    }

    /// Saved just as the chat was hidden: the answer would name the file.
    #[test]
    fn a_file_saved_as_its_chat_is_hidden_is_not_named() {
        let (dir, mut app) = app_with_write_pass(0);
        let unlock = json!({"id": 2, "cmd": "unlock", "pin": "2468", "privateChats": true});
        assert_eq!(call(&mut app, unlock)["ok"], true);
        let message = app
            .store
            .insert_incoming(
                "ABCD1234",
                2,
                "file",
                &json!({"name": "a.txt", "mediaType": "text/plain"}).to_string(),
                1,
            )
            .unwrap()
            .unwrap()
            .id;
        app.media
            .write(message, crate::media::MediaPart::File, b"content")
            .unwrap();
        let mut done = app.take_file_work().unwrap();
        let request = json!({"id": 3, "cmd": "saveFile", "message": message,
                             "directory": dir.path()});
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            assert!(from_ui(&mut app, request).await.is_none());
            let lock = from_ui(&mut app, json!({"id": 4, "cmd": "lock"}))
                .await
                .unwrap();
            assert_eq!(lock["ok"], true);
            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["error"]["kind"], "locked", "{reply}");
        }));
    }

    /// Waiting behind another file while the app locks: its plain text stays where it is.
    #[test]
    fn a_file_to_save_that_waited_through_the_lock_is_not_written() {
        let (dir, mut app) = app_with_write_pass(0);
        let downloads = dir.path().join("Downloads");
        std::fs::create_dir(&downloads).unwrap();
        let unlock = json!({"id": 2, "cmd": "unlock", "pin": "2468", "privateChats": true});
        assert_eq!(call(&mut app, unlock)["ok"], true);
        let message = stored_file(&app, "ABCD1234", 2);
        let mut done = app.take_file_work().unwrap();
        let request = json!({"id": 3, "cmd": "saveFile", "message": message,
                             "directory": downloads});
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            let turn = std::rc::Rc::clone(&app.file_work_turn);
            let before_us = turn.acquire().await.unwrap();
            assert!(from_ui(&mut app, request).await.is_none());
            tokio::task::yield_now().await;
            let lock = from_ui(&mut app, json!({"id": 4, "cmd": "lock"}))
                .await
                .unwrap();
            assert_eq!(lock["ok"], true);
            drop(before_us);
            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["error"]["kind"], "locked", "{reply}");
        }));
        let written = std::fs::read_dir(&downloads).unwrap().count();
        assert_eq!(written, 0, "nothing of it left the store");
    }

    /// Its message deleted while it waited, and the row id taken by another chat's message.
    #[test]
    fn a_file_to_save_whose_row_came_back_is_not_the_new_file() {
        let (dir, mut app) = app_with_write_pass(0);
        let downloads = dir.path().join("Downloads");
        std::fs::create_dir(&downloads).unwrap();
        let message = stored_file(&app, "IJKL9012", 2);
        let mut done = app.take_file_work().unwrap();
        let request = json!({"id": 3, "cmd": "saveFile", "message": message,
                             "directory": downloads});
        let local = tokio::task::LocalSet::new();
        rt().block_on(local.run_until(async {
            let turn = std::rc::Rc::clone(&app.file_work_turn);
            let before_us = turn.acquire().await.unwrap();
            assert!(from_ui(&mut app, request).await.is_none());
            app.store.remove_message(message).unwrap();
            let again = stored_file(&app, "QRST7890", 3);
            assert_eq!(again, message, "the row id came back");
            drop(before_us);
            let finished = done.recv().await.unwrap();
            let reply: serde_json::Value =
                serde_json::from_str(&app.finish_file_work(finished).await).unwrap();
            assert_eq!(reply["ok"], false, "{reply}");
        }));
        assert_eq!(std::fs::read_dir(&downloads).unwrap().count(), 0);
    }

    fn stored_file(app: &App, identity: &str, message_id: u64) -> i64 {
        let body = json!({"name": "a.txt", "mediaType": "text/plain"}).to_string();
        app.store
            .upsert_contact(&crate::contacts::test_contact(identity))
            .unwrap();
        let id = app
            .store
            .insert_incoming(identity, message_id, "file", &body, 1)
            .unwrap()
            .unwrap()
            .id;
        app.media
            .write(id, crate::media::MediaPart::File, b"content")
            .unwrap();
        id
    }

    /// With a pass running nothing but the counted files gets through: the chat is hidden from
    /// the moment of the lock, for listing, reading and every other way of writing.
    #[test]
    fn a_write_pass_opens_nothing() {
        let (_dir, mut app) = app_with_write_pass(5);
        let message = app.store.conversation_messages("ABCD1234", 1).unwrap()[0].id;
        let plain = app.store.conversation_messages("IJKL9012", 1).unwrap()[0].id;
        for request in [
            json!({"cmd": "conversations", "private": true}),
            json!({"cmd": "messages", "identity": "ABCD1234", "limit": 10}),
            json!({"cmd": "message", "message": message}),
            json!({"cmd": "draft", "identity": "ABCD1234"}),
            json!({"cmd": "setDraft", "identity": "ABCD1234", "text": "x"}),
            json!({"cmd": "quotedMessage", "identity": "ABCD1234", "messageId": "0000000000000001"}),
            json!({"cmd": "markRead", "identity": "ABCD1234"}),
            json!({"cmd": "sendText", "identity": "ABCD1234", "text": "x"}),
            json!({"cmd": "forwardMessage", "message": plain, "identity": "ABCD1234"}),
            json!({"cmd": "forwardMessage", "message": message, "identity": "IJKL9012"}),
            json!({"cmd": "deleteConversation", "identity": "ABCD1234"}),
        ] {
            let mut request = request;
            request["id"] = 4.into();
            let reply = call(&mut app, request.clone());
            assert_eq!(reply["error"]["kind"], "locked", "{request}: {reply}");
            assert!(!reply.to_string().contains("secret words"), "{reply}");
        }
        // The chat list doesn't show it, and nothing used up the pass.
        let list = app.conversations_json(false, false).unwrap();
        let chats = list["conversations"].as_array().unwrap();
        assert!(chats.iter().all(|chat| chat["identity"] != "ABCD1234"));
        assert_eq!(app.write_pass.as_ref().unwrap().files["ABCD1234"], 5);
    }

    /// Exactly the counted files, to exactly that chat; the answer says nothing of the chat, and
    /// the file is as hidden as the rest of it.
    #[test]
    fn a_write_pass_takes_only_the_counted_files() {
        let (_dir, mut app) = app_with_write_pass(2);
        for caption in ["first", "second"] {
            let reply = send_file(&mut app, "ABCD1234", caption);
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(reply["result"], json!({}), "the answer tells nothing");
        }
        assert_eq!(stored_messages(&app, "ABCD1234"), 3);
        let newest = app.store.conversation_messages("ABCD1234", 1).unwrap()[0].id;
        assert!(!app.media.shown(newest), "its media stay hidden");
        assert_eq!(
            send_file(&mut app, "ABCD1234", "third")["error"]["kind"],
            "locked"
        );
        assert_eq!(stored_messages(&app, "ABCD1234"), 3);
        assert!(app.write_pass.as_ref().unwrap().files.is_empty());
        assert!(!app.unlocked.get());
    }

    #[test]
    fn a_write_pass_is_for_one_chat() {
        let (_dir, mut app) = app_with_write_pass(2);
        let reply = send_file(&mut app, "EFGH5678", "elsewhere");
        assert_eq!(reply["error"]["kind"], "locked");
        assert_eq!(stored_messages(&app, "EFGH5678"), 1);
        assert_eq!(app.write_pass.as_ref().unwrap().files["ABCD1234"], 2);
        // A chat that isn't hidden sends as ever, and takes nothing from the pass.
        let reply = send_file(&mut app, "IJKL9012", "plain");
        assert!(reply["result"]["id"].is_i64(), "{reply}");
        assert_eq!(app.write_pass.as_ref().unwrap().files["ABCD1234"], 2);
    }

    /// A refused file uses up its place: it was that file the pass was for. It says only what is
    /// wrong with what was sent.
    #[test]
    fn a_write_pass_is_used_up_by_a_refused_file() {
        let (_dir, mut app) = app_with_write_pass(1);
        let reply = send_file(&mut app, "ABCD1234", &"x".repeat(6001));
        assert_eq!(reply["error"]["kind"], "messageTooLong");
        assert_eq!(
            send_file(&mut app, "ABCD1234", "again")["error"]["kind"],
            "locked"
        );
    }

    #[test]
    fn a_write_pass_expires() {
        let (_dir, mut app) = app_with_write_pass(2);
        assert_eq!(send_file(&mut app, "ABCD1234", "in time")["ok"], true);
        app.write_pass.as_mut().unwrap().expires = std::time::Instant::now();
        let reply = send_file(&mut app, "ABCD1234", "too late");
        assert_eq!(reply["error"]["kind"], "locked");
        assert!(app.write_pass.is_none());
    }

    #[test]
    fn unlocking_or_removing_the_data_clears_a_write_pass() {
        let unlock = |app: &mut App, private: bool| {
            let unlock = json!({"id": 5, "cmd": "unlock", "pin": "2468", "privateChats": private});
            assert_eq!(call(app, unlock)["ok"], true);
        };
        for private in [false, true] {
            let (_dir, mut app) = app_with_write_pass(2);
            unlock(&mut app, private);
            assert!(app.write_pass.is_none(), "{private}");
            call(&mut app, json!({"id": 6, "cmd": "lock"}));
            assert_eq!(
                send_file(&mut app, "ABCD1234", "x")["error"]["kind"],
                "locked"
            );
        }
        // A wrong PIN clears nothing.
        let (_dir, mut app) = app_with_write_pass(2);
        let wrong = json!({"id": 5, "cmd": "unlock", "pin": "1111", "privateChats": true});
        assert_eq!(call(&mut app, wrong)["ok"], false);
        assert!(app.write_pass.is_some());
        rt().block_on(app.delete_all_data()).unwrap();
        assert!(app.write_pass.is_none());
    }

    #[test]
    fn a_lock_without_files_takes_no_pass_away() {
        let (_dir, mut app) = app_with_write_pass(2);
        call(&mut app, json!({"id": 6, "cmd": "lock"}));
        assert_eq!(app.write_pass.as_ref().unwrap().files["ABCD1234"], 2);
    }

    /// A lock without files gives no pass; there is none for a chat that isn't hidden, or for a
    /// list, whose members' copies would have to read the hidden one.
    #[test]
    fn a_write_pass_is_given_only_for_hidden_chats_with_files() {
        let (_dir, mut app) = app();
        call(&mut app, json!({"id": 6, "cmd": "lock"}));
        assert!(app.write_pass.is_none());
        for id in ["ABCD1234", "IJKL9012"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        let key = app
            .store
            .create_distribution_list("A list", vec!["ABCD1234".into()])
            .unwrap()
            .key;
        app.store.set_private(&key, true).unwrap();
        let lock = json!({"id": 8, "cmd": "lock",
                          "pendingFiles": {"IJKL9012": 1, key.as_str(): 1, "ZZZZZZZZ": 1}});
        assert_eq!(call(&mut app, lock)["ok"], true);
        assert!(app.write_pass.is_none());
    }

    /// A deleted chat takes its draft along: nothing comes back when it is opened again.
    #[test]
    fn deleting_a_chat_takes_its_draft_along() {
        let (_dir, mut app) = app_with_draft("first");
        call(
            &mut app,
            json!({"id": 3, "cmd": "setDraft", "identity": "ABCD1234", "text": "unsent",
                   "quote": "00000000000000ff"}),
        );
        app.delete_conversation("ABCD1234").unwrap();
        assert_eq!(app.store.setting("draft:ABCD1234").unwrap(), None);
        assert_eq!(app.store.setting("draftQuote:ABCD1234").unwrap(), None);
    }

    /// Exporting a chat: the messages as text and, if wanted, the media that are here
    /// (Android's "Export chat").
    #[test]
    fn export_chat_writes_a_zip() {
        let (dir, mut app) = app();
        let rt = rt();
        app.store
            .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
            .unwrap();
        app.update_contact("ABCD1234", Some("Ada".into()), None)
            .unwrap();
        app.store
            .insert_incoming(
                "ABCD1234",
                1,
                "text",
                "when is the ferry?",
                1_700_000_000_000,
            )
            .unwrap();
        app.store
            .insert_outgoing(
                "ABCD1234",
                2,
                "text",
                "> quote #0100000000000000\n\nat nine",
            )
            .unwrap();
        app.store
            .insert_incoming(
                "ABCD1234",
                3,
                "location",
                &json!({"latitude": 59.9, "longitude": 10.7, "name": "North pier"}).to_string(),
                1_700_000_060_000,
            )
            .unwrap();
        // A picture whose bytes are on the device.
        let body = json!({
            "rendering": "media", "mediaType": "image/jpeg", "name": "pier.jpg", "size": 4,
            "thumbnail": false, "fileState": 2, "thumbnailState": 3, "uploadState": 2,
        });
        let picture = app
            .store
            .insert_incoming("ABCD1234", 4, "file", &body.to_string(), 1_700_000_120_000)
            .unwrap()
            .unwrap();
        app.media
            .write(picture.id, crate::media::MediaPart::File, b"jpeg")
            .unwrap();
        // Status lines stay out, like on Android.
        app.store
            .insert_incoming("ABCD1234", 5, "status", "group|created", 1_700_000_180_000)
            .unwrap();

        let out = dir.path().to_string_lossy().to_string();
        let reply = rt
            .block_on(app.export_chat("ABCD1234", true, &out, None, Some("Me".into()), Some(120)))
            .unwrap();
        assert_eq!(reply["messages"], 4);
        assert_eq!(reply["files"], 1);

        let file = std::fs::File::open(reply["path"].as_str().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = zip
            .file_names()
            .map(|name| name.unwrap_or_default().into_owned())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("-pier.jpg")), "{names:?}");
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("messages.txt").unwrap(), &mut text)
            .unwrap();
        // Times are the device's, two hours east of UTC here.
        assert!(
            text.contains("[2023-11-15 00:13] Ada: when is the ferry?"),
            "{text}"
        );
        assert!(text.contains("Me: at nine"), "quotes are stripped: {text}");
        assert!(text.contains("North pier <geo:59.9,10.7>"), "{text}");
        assert!(text.contains("-pier.jpg>"), "{text}");
        assert!(!text.contains("group|created"), "no status lines: {text}");

        // More than one page (PAGE = 500): the order must hold across them.
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBB2222"))
            .unwrap();
        let tx = app.store.conn.unchecked_transaction().unwrap();
        for n in 0..600_u64 {
            app.store
                .insert_incoming(
                    "BBBB2222",
                    n,
                    "text",
                    &format!("line {n}"),
                    1_700_000_000_000 + i64::try_from(n).unwrap() * 1000,
                )
                .unwrap();
        }
        tx.commit().unwrap();
        let reply = rt
            .block_on(app.export_chat("BBBB2222", false, &out, None, None, Some(0)))
            .unwrap();
        assert_eq!(reply["messages"], 600);
        let file = std::fs::File::open(reply["path"].as_str().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("messages.txt").unwrap(), &mut text)
            .unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| l.contains("line ")).collect();
        assert_eq!(lines.len(), 600);
        assert!(lines[0].ends_with("line 0"), "oldest first: {}", lines[0]);
        assert!(
            lines[599].ends_with("line 599"),
            "newest last: {}",
            lines[599]
        );

        // Without the media the file is only named, and nothing else goes in.
        let reply = rt
            .block_on(app.export_chat("ABCD1234", false, &out, None, None, None))
            .unwrap();
        assert_eq!(reply["files"], 0);
        let file = std::fs::File::open(reply["path"].as_str().unwrap()).unwrap();
        let zip = zip::ZipArchive::new(file).unwrap();
        assert_eq!(zip.len(), 1, "messages.txt only");

        // A password is a real one or none at all; nothing is written for a short one.
        let before = std::fs::read_dir(&out).unwrap().count();
        let short = Some(Zeroizing::new("1234567".to_owned()));
        assert!(matches!(
            rt.block_on(app.export_chat("ABCD1234", false, &out, short, None, None)),
            Err(ApiError::PasswordTooShort)
        ));
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), before);
        // Counted in characters, not bytes.
        let short = Some(Zeroizing::new("äöüäöüä".to_owned()));
        assert!(matches!(
            rt.block_on(app.export_chat("ABCD1234", false, &out, short, None, None)),
            Err(ApiError::PasswordTooShort)
        ));
        let long = Some(Zeroizing::new("12345678".to_owned()));
        let reply = rt
            .block_on(app.export_chat("ABCD1234", false, &out, long, None, None))
            .unwrap();
        let file = std::fs::File::open(reply["path"].as_str().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        assert!(matches!(
            zip.by_name("messages.txt").map(|_| ()),
            Err(zip::result::ZipError::UnsupportedArchive(_))
        ));
        let empty = Some(Zeroizing::new(String::new()));
        assert!(
            rt.block_on(app.export_chat("ABCD1234", false, &out, empty, None, None))
                .is_ok()
        );

        // What an export killed half way left behind goes with the next one, and nothing else.
        let folder = std::path::Path::new(&out);
        let leftover = folder.join("threesea-chat-20261104-120000.zip.part");
        let kept = [
            folder.join("threesea-chat-notes.zip"),
            folder.join("holiday.zip.part"),
        ];
        std::fs::write(&leftover, b"messages").unwrap();
        for file in &kept {
            std::fs::write(file, b"the user's").unwrap();
        }
        rt.block_on(app.export_chat("ABCD1234", false, &out, None, None, None))
            .unwrap();
        assert!(!leftover.exists());
        assert!(kept.iter().all(|file| file.exists()));
    }

    /// The cover names the chat that still has something unread, not whoever wrote last: reading
    /// the newer chat hands the name back to the older one. The archive stays out of it.
    #[test]
    fn cover_names_the_unread_chat() {
        let (_dir, mut app) = app();
        for (identity, name) in [
            ("AAAAAAAA", "Ada"),
            ("BBBBBBBB", "Ben"),
            ("CCCCCCCC", "Cem"),
        ] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
            app.update_contact(identity, Some(name.to_owned()), None)
                .unwrap();
        }
        app.store
            .insert_incoming("AAAAAAAA", 1, "text", "answer me later", 1_000)
            .unwrap();
        app.store
            .insert_incoming("BBBBBBBB", 2, "text", "hello", 2_000)
            .unwrap();
        app.store
            .insert_incoming("CCCCCCCC", 3, "text", "newest of all", 3_000)
            .unwrap();
        app.store.set_archived("CCCCCCCC", true).unwrap();

        let list = app.conversations_json(false, false).unwrap();
        assert_eq!(list["unreadTotal"], 2, "the archive doesn't count");
        assert_eq!(list["unreadFrom"], "Ben");

        // Reading Ben's chat leaves Ada's message unread: she is the one still waiting.
        app.mark_read("BBBBBBBB").unwrap();
        let list = app.conversations_json(false, false).unwrap();
        assert_eq!(list["unreadTotal"], 1);
        assert_eq!(list["unreadFrom"], "Ada");

        // Every list carries the chat list's numbers, so opening the archive can't skew the cover.
        let archive = app.conversations_json(true, false).unwrap();
        assert_eq!(archive["conversations"].as_array().unwrap().len(), 1);
        assert_eq!(archive["unreadTotal"], 1);
        assert_eq!(archive["unreadFrom"], "Ada");

        app.mark_read("AAAAAAAA").unwrap();
        let list = app.conversations_json(false, false).unwrap();
        assert_eq!(list["unreadTotal"], 0);
        assert_eq!(list["unreadFrom"], Value::Null);
    }

    #[test]
    fn block_list() {
        let (_dir, mut app) = app();
        assert_eq!(app.set_blocked("echoecho", true).unwrap(), json!({}));
        app.set_blocked("ECHOECHO", true).unwrap();
        assert_eq!(
            app.app_settings().unwrap().blocked_contacts,
            vec!["ECHOECHO"]
        );
        assert_eq!(
            app.blocked_contacts_json().unwrap(),
            json!({ "blocked": [{ "identity": "ECHOECHO", "displayName": "ECHOECHO" }] })
        );
        assert!(app.set_blocked("nope", true).is_err());
        app.set_blocked("ECHOECHO", false).unwrap();
        assert!(app.app_settings().unwrap().blocked_contacts.is_empty());
    }

    #[test]
    fn export_and_restore_backup_roundtrip_offline_part() {
        let (_dir, mut app) = app();
        let rt = rt();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([5; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let reply: Value = serde_json::from_str(
            &rt.block_on(app.handle(r#"{"id":1,"cmd":"exportIdentityBackup","password":"short"}"#)),
        )
        .unwrap();
        assert_eq!(reply["error"]["kind"], "passwordTooShort");

        let reply: Value = serde_json::from_str(&rt.block_on(
            app.handle(r#"{"id":2,"cmd":"exportIdentityBackup","password":"correct horse"}"#),
        ))
        .unwrap();
        let backup = reply["result"]["backup"].as_str().unwrap().to_owned();
        let (_, data) = decrypt_identity_backup("correct horse", &backup).unwrap();
        assert_eq!(data.threema_id.to_string(), "ECHOECHO");
        assert_eq!(data.client_key.as_bytes(), &[5; 32]);

        // Restoring needs a license first.
        let (_fresh_dir, mut app) = self::app();
        let reply: Value = serde_json::from_str(&rt.block_on(app.handle(&json!({
            "id": 3, "cmd": "restoreIdentityBackup", "backup": backup, "password": "correct horse"
        }).to_string())))
        .unwrap();
        assert_eq!(reply["error"]["kind"], "noLicense");
    }
}
