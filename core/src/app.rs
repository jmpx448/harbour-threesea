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
    directory::{self, ClientVersion, DirectoryError},
    https::HttpsClient,
    store::{Store, StoreError, StoredIdentity, keys},
};

/// Minimum password length for ID backups (same as Android).
pub const MIN_BACKUP_PASSWORD_LENGTH: usize = 8;

/// How often a running app asks the directory about its license.
const LICENSE_CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

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
    /// A chat's media gallery, newest first: `{messages (with mediaKind), kinds, counts}`.
    /// `kinds` filters (default: the saved filter, see `setMediaKinds`).
    ChatMedia {
        identity: String,
        #[serde(default)]
        kinds: Option<Vec<String>>,
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
    /// Several, in the given order: `{forwarded, failed}`.
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
    /// Reconnect now if disconnected.
    Reconnect,
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
            Self::FileTooLarge => "fileTooLarge",
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

/// The core. Lives on the core thread only (libthreema state is not `Send`).
pub struct App {
    pub(crate) store: std::rc::Rc<Store>,
    pub(crate) https: HttpsClient,
    pub(crate) version: ClientVersion,
    pub(crate) config: Arc<Config>,
    /// Channel to the running chat server connection, if any.
    pub(crate) connection:
        Option<tokio::sync::mpsc::UnboundedSender<crate::connection::ToConnection>>,
    /// Last known connection state, for `connectionState`.
    pub connection_state: &'static str,
    /// Encrypted files of messages (shared with the bridge's image provider).
    pub(crate) media: Arc<crate::media::MediaStore>,
    /// Media downloads report their progress here (into the core loop).
    pub(crate) media_events: tokio::sync::mpsc::UnboundedSender<crate::providers::CoreEvent>,
    media_events_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::providers::CoreEvent>>,
    /// Files of hidden private chats still on their way to the core.
    pub(crate) write_pass: Option<crate::app_media::WritePass>,
    /// Binary attachments of the request being handled (e.g. `sendFile`: file, thumbnail).
    pub(crate) attachments: Vec<zeroize::Zeroizing<Vec<u8>>>,
    /// Contacts the request being handled made direct, for the other devices.
    pub(crate) made_direct: std::cell::RefCell<Vec<String>>,
    /// Chats a message brought back from the archive during the current command.
    pub(crate) unarchived: std::cell::RefCell<Vec<String>>,
    /// Calls (ADR 0012): this build has a media engine; the current call, its SDP rules and
    /// cached TURN servers (with their expiry, ms).
    pub(crate) calls_supported: bool,
    /// The build may check for updates (ADR 0014); a check is under way.
    pub(crate) update_check_built: bool,
    pub(crate) update_check_running: std::rc::Rc<std::cell::Cell<bool>>,
    pub(crate) calls: crate::calls::Calls,
    pub(crate) call_patcher: Option<crate::sdp::SdpPatcher>,
    pub(crate) call_media: crate::app_calls::CallMedia,
    /// Group calls: the SFU credentials while they last.
    pub(crate) sfu: std::rc::Rc<std::cell::RefCell<crate::app_group_calls::SfuCache>>,
    pub(crate) group_call: Option<crate::app_group_calls::ActiveGroupCall>,
    /// Threema Web sessions running (ADR 0013).
    pub(crate) web: crate::app_web::WebSessions,
    pub(crate) turn_servers: Option<(crate::directory::TurnServers, i64)>,
    /// Device linking between the scan and the emoji confirmation.
    pub(crate) pending_link: Option<crate::rendezvous::RendezvousLink>,
    /// Events for the UI caused by handling a command (e.g. a Safe import the command ran);
    /// the core loop emits and clears them after every command.
    pub(crate) pending_ui_events: Vec<&'static str>,
    /// A running Android history import's cancel flag (set true when it ends).
    pub(crate) android_import: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    /// A running Android-format export's done flag.
    pub(crate) android_export: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    /// Tells the running export to give up (the ID is being removed).
    pub(crate) android_export_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// When the pending Safe import was last tried automatically (reconnect rate limit).
    pub(crate) last_import_attempt: Option<tokio::time::Instant>,
    /// When the core loop next looks for failed transfers to try again.
    pub(crate) transfer_retry_at: Option<tokio::time::Instant>,
    /// When this device last looked whether it is alone in its device group.
    pub(crate) last_group_check: Option<tokio::time::Instant>,
    /// Private chats readable (ADR 0009); memory only, starts locked.
    pub(crate) unlocked: bool,
    /// On an unmetered network (Wi-Fi/Ethernet, from the bridge): auto-download policy.
    pub(crate) on_wifi: bool,
    pub(crate) pictures_dir: Option<PathBuf>,
    /// The app's data directory (store, media, the in-app log).
    pub(crate) data_dir: PathBuf,
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
        let store = Store::open_with(&start.data_dir.join("threesea.db"), db_key, upgrading)
            .map(std::rc::Rc::new)
            .map_err(|error| ApiError::store_failed(&error))?;
        forget_false_refusal(&store).map_err(|error| ApiError::store_failed(&error))?;
        let app = Self {
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
            connection: None,
            connection_state: "disconnected",
            media,
            media_events,
            media_events_rx: Some(media_events_rx),
            write_pass: None,
            attachments: Vec::new(),
            made_direct: std::cell::RefCell::default(),
            unarchived: std::cell::RefCell::default(),
            pending_link: None,
            calls_supported: start.calls,
            update_check_built: start.update_check,
            update_check_running: std::rc::Rc::default(),
            calls: crate::calls::Calls::default(),
            call_patcher: None,
            call_media: crate::app_calls::CallMedia::default(),
            sfu: std::rc::Rc::default(),
            group_call: None,
            web: crate::app_web::WebSessions::default(),
            turn_servers: None,
            pending_ui_events: Vec::new(),
            android_import: None,
            android_export: None,
            android_export_cancel: Arc::default(),
            last_import_attempt: None,
            transfer_retry_at: None,
            last_group_check: None,
            unlocked: false,
            on_wifi: false,
            pictures_dir: start.pictures_dir.clone(),
            data_dir: start.data_dir.clone(),
            identity_removed: false,
        };
        app.apply_display_settings()?;
        app.apply_network_settings()?;
        app.apply_multi_device();
        Ok(app)
    }

    /// Turn forward security off or on again, like [`Self::activate_multi_device`] does for
    /// a device group: off announces it (feature mask without FS), terminates every session
    /// with DisabledByLocal once connected, and reconnects so the running session follows.
    async fn set_forward_security(&mut self, on: bool) -> Result<Value, ApiError> {
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
            self.connection_state,
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

    /// Remember events the current command caused, for [`Self::take_ui_events`].
    pub(crate) fn note_ui_events(&mut self, events: &[&'static str]) {
        for event in events {
            if !self.pending_ui_events.contains(event) {
                self.pending_ui_events.push(event);
            }
        }
    }

    /// The events the last command caused (emitted by the core loop after the reply).
    pub fn take_ui_events(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.pending_ui_events)
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
        let change = sync_change(&self.store, &request.command);
        let outcome = self.execute(request.command).await;
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
                json!({"reply": request.id, "ok": true, "result": result})
            }
            Err(error) => {
                if matches!(error, ApiError::RateLimited) {
                    self.store.rest_directory_calls();
                }
                tracing::info!(kind = error.kind(), "command failed");
                json!({"reply": request.id, "ok": false,
                    "error": {"kind": error.kind(), "message": error.to_string()}})
            }
        }
        .to_string()
    }

    async fn execute(&mut self, command: Command) -> Result<Value, ApiError> {
        match command {
            Command::Status => self.status(),
            Command::CheckLicense { license_key } => self.check_license(&license_key).await,
            Command::LicenseStatus => self.license_status(),
            Command::ForgetLicense => self.forget_license(),
            Command::SpamPotential { identity } => self.spam_potential(&identity),
            Command::DismissSpam { identity } => self.dismiss_spam(&identity),
            Command::ReportSpam { identity, block } => self.report_spam(&identity, block).await,
            Command::CreateIdentity => self.create_identity().await,
            Command::RestoreIdentityBackup { backup, password } => {
                self.restore_identity_backup(backup, password).await
            }
            Command::ExportIdentityBackup { password, pin } => {
                self.check_pin(pin)?;
                self.export_identity_backup(password).await
            }
            Command::SafeCheckPassword { password } => Ok(Self::safe_check_password(&password)),
            Command::SafeStatus => self.safe_status(),
            Command::SafeEnable { password, pin } => {
                self.check_pin(pin)?;
                self.safe_enable(password).await
            }
            Command::SafeBackupNow => self.safe_backup_now().await,
            Command::SafeDisable { pin } => {
                self.check_pin(pin)?;
                self.safe_disable().await
            }
            Command::SafeRestore { identity, password } => {
                self.safe_restore(&identity, password).await
            }
            Command::SafeRetryImport => {
                let events = self.try_pending_import().await;
                self.note_ui_events(&events);
                self.safe_status()
            }
            Command::AndroidBackupInspect { path, password } => {
                self.android_backup_inspect(path, password).await
            }
            Command::AndroidBackupImport {
                path,
                password,
                scope,
            } => self.android_backup_import(path, password, &scope).await,
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
            Command::AddContact { identity } => self.add_contact(&identity).await,
            Command::UpdateContact {
                identity,
                first_name,
                last_name,
            } => self.update_contact(&identity, first_name, last_name),
            Command::DeleteContact { identity } => self.delete_contact(&identity),
            Command::SetContactPolicies {
                identity,
                read_receipts,
                typing,
            } => self.set_contact_policies(&identity, read_receipts, typing),
            Command::ClearContactPolicies => self.clear_contact_policies(),
            Command::Profile => {
                // A pending e-mail link may have been confirmed meanwhile.
                let _ = self.check_email_link().await;
                self.profile_json()
            }
            Command::RevocationStatus { cached } => self.revocation_status(cached).await,
            Command::LinkEmail { email } => self.link_email(&email).await,
            Command::LinkMobile { number } => self.link_mobile(&number).await,
            Command::NormalizeMobile { number } => Ok(json!({
                "number": crate::address_book::international_phone(&number),
            })),
            Command::VerifyMobile { code, from_link } => self.verify_mobile(&code, from_link).await,
            Command::CallMobile => self.call_mobile().await,
            Command::SetRevocationPassword { password, pin } => {
                self.check_pin(pin)?;
                self.set_revocation_password(&password).await
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
                    self.unlocked = true;
                    self.update_media_gate();
                }
                self.lock_status()
            }
            Command::Lock { pending_files } => {
                self.unlocked = false;
                self.update_media_gate();
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
                let changed = self.refresh_contacts(true).await?;
                Ok(json!({ "changed": changed }))
            }
            Command::ForwardMessage { message, identity } => {
                self.forward_message(message, &identity)
            }
            Command::SetDraft {
                identity,
                text,
                quote,
            } => self.set_draft(&identity, &text, quote.as_deref()),
            Command::Draft { identity } => self.draft(&identity),
            Command::ChatMedia { identity, kinds } => self.chat_media(&identity, kinds),
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
                Ok(self
                    .store
                    .set_listened(message)?
                    .map_or(Value::Null, |m| self.store.message_json(&m)))
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
                for message in messages {
                    match self.forward_message(message, &identity) {
                        Ok(_) => forwarded += 1,
                        Err(error) => {
                            tracing::warn!(%error, "forwarding failed");
                            failed += 1;
                        }
                    }
                }
                Ok(json!({ "forwarded": forwarded, "failed": failed }))
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
                self.match_address_book(entries).await
            }
            Command::VerifyQrCode { text } => self.verify_qr_code(&text).await,
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
            Command::RemoveGroupPicture { key } => self.remove_group_picture(&key),
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
                };
                // Only this command: nothing else that ends in sending a file uses a pass.
                if self.take_write_pass(&file.identity) {
                    self.send_file_under_pass(file)
                } else {
                    self.send_file(file)
                }
            }
            Command::RetryUpload { message } => self.retry_upload(message),
            Command::SaveFile { message, directory } => self.save_file(message, &directory),
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
            Command::LinkedDevices => self.linked_devices_checked().await,
            Command::DropDevices { ids } => self.drop_devices(&ids).await,
            Command::DeactivateMultiDevice { leave_locally } => {
                self.deactivate_multi_device(leave_locally).await
            }
            Command::StartLinking { code, pin } => {
                self.check_pin(pin)?;
                self.start_linking(&code).await
            }
            Command::ConfirmLinking => self.confirm_linking().await,
            Command::CancelLinking => {
                if let Some(link) = self.pending_link.take() {
                    link.close().await;
                }
                Ok(json!({}))
            }
            Command::CallTurnServers { identity } => self.call_turn_servers(&identity).await,
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
            Command::GroupCallRefresh { conversation } => {
                Ok(self.group_call_refresh(&conversation))
            }
            Command::GroupCallJoin {
                conversation,
                fingerprint,
                create,
            } => {
                self.group_call_join(&conversation, &fingerprint, create)
                    .await
            }
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
            } => self.send_gif_favourite(identity, favourite),
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
            Command::SetLogLevel { level } => {
                crate::runtime::set_log_level(&level);
                Ok(json!({}))
            }
            Command::ForwardSecurity => Ok(json!({
                "on": !self.store.forward_security_disabled()?
                    && self.store.multi_device()?.is_none(),
                "multiDevice": self.store.multi_device()?.is_some(),
            })),
            Command::SetForwardSecurity { on } => self.set_forward_security(on).await,
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
                self.export_chat(&identity, media, &directory, password, me, offset_minutes)
                    .await
            }
            Command::StorageInfo => Ok(json!({
                "mediaBytes": self.media.size(),
                "databaseBytes": self.store.size()?,
            })),
            Command::CleanUp { days, messages } => self.clean_up(days, messages),
            Command::EmptyConversation { identity } => {
                let key = self.readable_conversation(&identity)?.0;
                self.check_readable(&key)?;
                self.store.empty_conversation(&key)?;
                let store = std::rc::Rc::clone(&self.store);
                self.media
                    .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
                Ok(json!({}))
            }
            Command::ConnectionState => Ok(self.connection_json()),
            Command::Reconnect => Ok(self.wake_connection()),
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

    fn device_id(&self) -> Result<String, ApiError> {
        if let Some(id) = self.store.setting(keys::DEVICE_ID)? {
            return Ok(id);
        }
        let id = random_device_id();
        self.store.set_setting(keys::DEVICE_ID, &id)?;
        Ok(id)
    }

    async fn check_license(&mut self, input: &str) -> Result<Value, ApiError> {
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
    pub(crate) async fn recheck_license(&mut self) -> Result<bool, ApiError> {
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
        self.apply_license_answer(answer)
    }

    /// Only the directory's own "no" counts, like Android: being offline or a server having a bad
    /// day must not cost anyone their messages. Those try again at the next maintenance.
    fn apply_license_answer(
        &mut self,
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
                self.web_stop_all(crate::app_web::StopReason::Stop);
                self.stop_connection();
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

    async fn create_identity(&mut self) -> Result<Value, ApiError> {
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
        &mut self,
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
        &mut self,
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

    async fn export_identity_backup(
        &mut self,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
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
fn sync_change(
    store: &crate::store::Store,
    command: &Command,
) -> Option<crate::app_multidevice::SyncChange> {
    use crate::app_multidevice::SyncChange;
    Some(match command {
        Command::AddContact { identity }
        | Command::UpdateContact { identity, .. }
        | Command::SetContactPolicies { identity, .. } => {
            SyncChange::Contacts(vec![identity.clone()])
        }
        Command::SetPrivate { identity, .. }
        | Command::SetArchived { identity, .. }
        | Command::SetPinned { identity, .. }
        | Command::SetChatNotifications { identity, .. } => {
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
        | Command::SetProfilePictureRecipient { .. } => SyncChange::Profile { picture: false },
        Command::SetProfilePicture | Command::RemoveProfilePicture => {
            SyncChange::Profile { picture: true }
        }
        Command::SetGroupPicture { key } | Command::RemoveGroupPicture { key } => {
            SyncChange::GroupPicture(key.clone())
        }
        _ => return None,
    })
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
        let (_dir, mut app) = app();
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
                assert!(app.connection.is_some(), "still running");
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
        let (_dir, mut app) = licensed_app();
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

    #[tokio::test(flavor = "current_thread")]
    async fn no_connection_after_the_directorys_no() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = licensed_app();
                assert!(app.ensure_connection().is_some(), "licensed: connects");
                let no = DirectoryError::Server("no".into());
                assert!(app.apply_license_answer(Err(no)).unwrap());
                assert!(app.connection.is_none(), "the running one is stopped");
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
        assert_eq!(chats[0]["displayName"], "Ada Lindqvist", "pinned first");

        let long = app
            .messages_json("DEMO0005", Some(100), None, None, None)
            .unwrap();
        assert_eq!(long["messages"].as_array().unwrap().len(), 20);

        // The pictures and the voice message are on the device, not on a server.
        let media = app.chat_media("DEMO0006", None).unwrap();
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
        assert!(!app.unlocked, "the lock takes effect at once");
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
        assert!(!app.unlocked);
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
                &json!({"lat": 59.9, "lon": 10.7, "name": "North pier"}).to_string(),
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
