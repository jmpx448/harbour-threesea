// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Safe commands and the automatic backup, on top of [`crate::safe`].
//! Behaviour follows upstream `ThreemaSafeServiceImpl.java`; see docs/protocol-notes.md.

use data_encoding::{BASE64, HEXLOWER};
use libthreema::common::keys::ClientKey;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App, Shared},
    safe::{
        self, MasterKey, PasswordCheck, SafeError,
        backup::{self, Backup},
        server::{self, ServerError},
    },
    store::{StoreError, keys},
};

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        match error {
            ServerError::Network(e) => Self::Network(e.to_string()),
            ServerError::NotFound => Self::SafeNotFound,
            ServerError::TooLarge { .. } => Self::SafeTooLarge(error.to_string()),
            other @ (ServerError::Status(_) | ServerError::InvalidResponse(_)) => {
                Self::ServerTrouble(other.to_string())
            }
        }
    }
}

impl From<SafeError> for ApiError {
    fn from(error: SafeError) -> Self {
        match error {
            // The backup's address comes from the key, so a wrong password finds none at all
            // (`SafeNotFound`): one that is there but won't decrypt is damaged.
            SafeError::Decrypt | SafeError::Format(_) => Self::SafeDamaged(error.to_string()),
            SafeError::UnsupportedVersion(version) => Self::BackupVersion(i64::from(version)),
            other @ (SafeError::Encrypt | SafeError::Kdf) => Self::Internal(other.to_string()),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// How long a queued group message waits for someone whose lookup keeps failing, with the
/// directory otherwise answering (it would say who doesn't exist). Then it goes without them.
const UNRESOLVED_OUTBOX_MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// How many restored contacts one round (connect, maintenance) may ask for their picture.
const PICTURE_REQUEST_BATCH: usize = 20;

/// How many contacts one round of a restore's setup steps writes to.
const RESTORE_SETUP_BATCH: usize = 20;

/// [`keys::SAFE_RESTORE_SETUP`] before the groups have been dealt with.
const RESTORE_SETUP_START: &str = "start";

/// Threema IDs: 8 characters, `*` allowed first (gateway IDs). Normalized to upper case.
pub fn normalize_identity(input: &str) -> Option<String> {
    let id = input.trim().to_ascii_uppercase();
    let valid = id.len() == 8
        && id
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_uppercase() || b.is_ascii_digit() || (i == 0 && b == b'*'));
    valid.then_some(id)
}

/// Safe's 8-byte ids (group ids, distribution list ids) are lowercase hex of the little-endian
/// bytes (Android `GroupId.toString()`, `toByteArray(id, LITTLE_ENDIAN)`; the wire order too).
pub(crate) fn safe_id(value: u64) -> String {
    HEXLOWER.encode(&value.to_le_bytes())
}

pub(crate) fn parse_safe_id(hex: &str) -> Option<u64> {
    let bytes: [u8; 8] = HEXLOWER
        .decode(hex.to_ascii_lowercase().as_bytes())
        .ok()?
        .try_into()
        .ok()?;
    Some(u64::from_le_bytes(bytes))
}

/// When each chat last changed, for the backup's `lastUpdate` (ms; Android lists a restored
/// contact's chat only if it has one).
type LastUpdates = std::collections::HashMap<String, u64>;

fn last_update(updates: &LastUpdates, chat: &str) -> Option<i64> {
    updates
        .get(chat)
        .and_then(|at| i64::try_from(*at).ok())
        .filter(|at| *at > 0)
}

/// A group as Android writes it: `members` are all participants, the user included while a member.
fn group_to_backup(group: &crate::groups::Group, me: &str) -> backup::Group {
    let mut members = group.members.clone();
    if !group.left && !members.iter().any(|m| m == me) {
        members.push(me.to_owned());
    }
    backup::Group {
        id: safe_id(group.identity.group_id),
        creator: group.identity.creator_identity.to_string(),
        groupname: Some(group.name.clone()),
        created_at: Some(group.created_at),
        members,
        deleted: false,
        last_update: None,
        is_private: false,
    }
}

/// Why a backed-up group is not imported (Android skips these too); the restore summary's
/// categories.
const SKIP_DELETED: &str = "deleted";
const SKIP_INVALID: &str = "invalid";
const SKIP_UNKNOWN_CREATOR: &str = "unknownCreator";

/// A backed-up group to store (Android `restoreGroups`): skipped (with the reason) for the
/// legacy `deleted` flag or an invalid id. `known` says whether a member exists as a contact
/// (others are dropped, as on Android); the creator must be known (or us). If the creator is no
/// member while we are, the group counts as left (Android treats it as kicked).
fn group_from_backup(
    entry: &backup::Group,
    me: &str,
    known: impl Fn(&str) -> bool,
) -> Result<crate::groups::Group, &'static str> {
    if entry.deleted {
        return Err(SKIP_DELETED);
    }
    let creator = normalize_identity(&entry.creator).ok_or(SKIP_INVALID)?;
    if creator != me && !known(&creator) {
        return Err(SKIP_UNKNOWN_CREATOR);
    }
    let identity = libthreema::common::GroupIdentity {
        creator_identity: libthreema::common::ThreemaId::try_from(creator.as_str())
            .map_err(|_| SKIP_INVALID)?,
        group_id: parse_safe_id(&entry.id).ok_or(SKIP_INVALID)?,
    };
    let listed: Vec<String> = entry
        .members
        .iter()
        .filter_map(|m| normalize_identity(m))
        .collect();
    let user_is_member = listed.iter().any(|m| m == me);
    let creator_is_member = creator == me || listed.contains(&creator);
    let mut members: Vec<String> = Vec::new();
    for member in listed {
        if member != me && !members.contains(&member) && known(&member) {
            members.push(member);
        }
    }
    Ok(crate::groups::Group {
        key: crate::groups::key(&identity),
        identity,
        name: entry.groupname.clone().unwrap_or_default(),
        members,
        left: !user_is_member || !creator_is_member,
        created_at: entry.created_at.unwrap_or(0).max(0),
        picture: None,
        picture_version: 0,
    })
}

fn list_to_backup(list: &crate::distribution_lists::DistributionList) -> backup::DistributionList {
    backup::DistributionList {
        id: list
            .key
            .strip_prefix("d:")
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .map(safe_id),
        name: list.name.clone(),
        created_at: Some(list.created_at),
        members: list.members.clone(),
        last_update: None,
        is_private: false,
    }
}

/// A backed-up distribution list to store; members that are no contacts are dropped (Android).
fn list_from_backup(
    entry: &backup::DistributionList,
    known: impl Fn(&str) -> bool,
) -> crate::distribution_lists::DistributionList {
    let mut members: Vec<String> = Vec::new();
    for member in entry.members.iter().filter_map(|m| normalize_identity(m)) {
        if !members.contains(&member) && known(&member) {
            members.push(member);
        }
    }
    crate::distribution_lists::DistributionList {
        key: entry
            .id
            .as_deref()
            .and_then(parse_safe_id)
            .map_or_else(crate::distribution_lists::new_key, |id| {
                format!("d:{id:016x}")
            }),
        name: entry.name.clone(),
        members,
        created_at: entry.created_at.unwrap_or(0).max(0),
    }
}

/// A list carried over from a restored backup that is not imported yet.
fn carried<T: serde::de::DeserializeOwned>(pending: &Value, name: &str) -> Option<Vec<T>> {
    serde_json::from_value(pending[name].clone()).ok().flatten()
}

async fn derive(identity: String, password: Zeroizing<String>) -> Result<MasterKey, ApiError> {
    // scrypt with 64 MiB takes seconds on a phone: keep it off the core loop.
    tokio::task::spawn_blocking(move || MasterKey::derive(&identity, &password))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(ApiError::from)
}

/// How long a failing Threema Safe stays quiet: a short outage isn't news (Android's 7 days).
const SAFE_WARNING_AFTER: i64 = 7 * 24 * 60 * 60;

/// Backups have kept failing for a week: the days since the last one that worked (since the first
/// failure if none ever did, where Android says nothing though nothing is protected at all).
fn safe_warning_days(
    first_failure: Option<i64>,
    last_upload: Option<i64>,
    now: i64,
) -> Option<i64> {
    let first_failure = first_failure?;
    if now - first_failure < SAFE_WARNING_AFTER {
        return None;
    }
    Some((now - last_upload.unwrap_or(first_failure)) / (24 * 60 * 60)).filter(|days| *days > 0)
}

/// A new ID without any backup: the first reminder after a month, then every three months.
const BACKUP_REMINDER_FIRST: i64 = 30 * 24 * 60 * 60;
const BACKUP_REMINDER_AGAIN: i64 = 90 * 24 * 60 * 60;

impl App {
    pub(crate) fn safe_check_password(password: &str) -> Value {
        let result = match safe::check_password(password) {
            PasswordCheck::TooShort => "tooShort",
            PasswordCheck::TooLong => "tooLong",
            PasswordCheck::Weak => "weak",
            PasswordCheck::Ok => "ok",
        };
        json!({ "result": result })
    }
}

impl Shared {
    fn safe_key(&self) -> Result<Option<MasterKey>, ApiError> {
        let Some(hex) = self.store.setting(keys::SAFE_MASTER_KEY)? else {
            return Ok(None);
        };
        let bytes = Zeroizing::new(
            HEXLOWER
                .decode(hex.as_bytes())
                .map_err(|_| ApiError::Internal("stored Safe key invalid".into()))?,
        );
        let bytes: [u8; 64] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| ApiError::Internal("stored Safe key invalid".into()))?;
        Ok(Some(MasterKey::from_bytes(bytes)))
    }

    fn setting_i64(&self, key: &str) -> Result<Option<i64>, ApiError> {
        Ok(self.store.setting(key)?.and_then(|v| v.parse().ok()))
    }

    /// An ID was just created: it is on this phone only. Asks once about Threema Safe (the
    /// chat list, `askSetup` in the status), and reminds until some backup exists.
    pub(crate) fn note_new_identity(&self) -> Result<(), ApiError> {
        self.store.set_setting(keys::SAFE_SETUP_UNANSWERED, "ask")?;
        self.store.set_setting(
            keys::BACKUP_REMINDER_AT,
            &(now() + BACKUP_REMINDER_FIRST).to_string(),
        )?;
        Ok(())
    }

    /// `safeDeclineSetup`: the user chose to go on without a backup, so no more reminders either.
    pub(crate) fn safe_decline_setup(&self) -> Result<Value, ApiError> {
        self.store.delete_setting(keys::SAFE_SETUP_UNANSWERED)?;
        self.end_backup_reminder()?;
        self.safe_status()
    }

    /// A backup of the ID exists, or the user wants none: nothing to remind of any more, and a
    /// notification still on the screen goes (the window reads `backupReminder` in the status).
    /// A Data backup's job ends it by itself; its `androidExportDone` tells the window.
    pub(crate) fn end_backup_reminder(&self) -> Result<(), ApiError> {
        if self.store.setting(keys::BACKUP_REMINDER_AT)?.is_some() {
            self.store.delete_setting(keys::BACKUP_REMINDER_AT)?;
            self.note_ui_events(&["safeChanged"]);
        }
        Ok(())
    }

    /// Is a reminder due at `now`? Then the next one is set.
    fn backup_reminder_due(&self, now: i64) -> Result<bool, ApiError> {
        let due = self
            .setting_i64(keys::BACKUP_REMINDER_AT)?
            .is_some_and(|at| at <= now);
        if due {
            self.store.set_setting(
                keys::BACKUP_REMINDER_AT,
                &(now + BACKUP_REMINDER_AGAIN).to_string(),
            )?;
        }
        Ok(due)
    }

    /// See [`safe_warning_days`]; `None` too with Safe off.
    fn safe_warning(&self) -> Result<Option<i64>, ApiError> {
        if self.store.setting(keys::SAFE_MASTER_KEY)?.is_none() {
            return Ok(None);
        }
        Ok(safe_warning_days(
            self.setting_i64(keys::SAFE_FIRST_FAILURE)?,
            self.setting_i64(keys::SAFE_LAST_UPLOAD)?,
            now(),
        ))
    }

    pub(crate) fn safe_status(&self) -> Result<Value, ApiError> {
        let key = self.safe_key()?;
        let (pending_contacts, pending_groups, pending_lists) = self.pending_import_counts()?;
        let summary: Value = self
            .store
            .setting(keys::SAFE_IMPORT_SUMMARY)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        Ok(json!({
            "enabled": key.is_some(),
            "askSetup": key.is_none()
                && self.store.setting(keys::SAFE_SETUP_UNANSWERED)?.is_some(),
            "server": key.as_ref().map(server::base_url),
            "lastBackup": self.setting_i64(keys::SAFE_LAST_UPLOAD)?,
            "lastError": self.store.setting(keys::SAFE_LAST_ERROR)?,
            "warningDays": self.safe_warning()?,
            "backupReminder": self.store.setting(keys::BACKUP_REMINDER_AT)?.is_some(),
            "size": self.setting_i64(keys::SAFE_SIZE)?,
            "maxBytes": self.setting_i64(keys::SAFE_MAX_BYTES)?,
            "retentionDays": self.setting_i64(keys::SAFE_RETENTION_DAYS)?,
            // What a restore still has to import, why the last attempt failed, what it did.
            "restore": {
                "pendingContacts": pending_contacts,
                "pendingGroups": pending_groups,
                "pendingLists": pending_lists,
                "importError": self.store.setting(keys::SAFE_IMPORT_ERROR)?,
                "summary": summary,
            },
        }))
    }

    pub(crate) async fn safe_enable(&self, password: Zeroizing<String>) -> Result<Value, ApiError> {
        match safe::check_password(&password) {
            PasswordCheck::TooShort => return Err(ApiError::PasswordTooShort),
            PasswordCheck::TooLong => return Err(ApiError::PasswordTooLong),
            PasswordCheck::Weak | PasswordCheck::Ok => {}
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let key = derive(identity.threema_id.clone(), password).await?;
        // Test the server before committing to it.
        let config = server::config(&self.https, &key).await?;
        self.store_safe_limits(config)?;
        if let Some(old) = self.safe_key()? {
            // New password: the old backup is unreachable now, remove it (like Android).
            if old.backup_id() != key.backup_id() {
                let _ = server::delete(&self.https, &old).await;
            }
        }
        self.store
            .set_setting(keys::SAFE_MASTER_KEY, &HEXLOWER.encode(key.as_bytes()))?;
        for k in [
            keys::SAFE_LAST_UPLOAD,
            keys::SAFE_HASH,
            keys::SAFE_LAST_ERROR,
            keys::SAFE_FIRST_FAILURE,
            keys::SAFE_SIZE,
            keys::SAFE_SETUP_UNANSWERED,
        ] {
            self.store.delete_setting(k)?;
        }
        self.end_backup_reminder()?;
        // On from here, also if the first upload fails below: the window's warning goes by this.
        self.note_ui_events(&["safeChanged"]);
        self.safe_upload(true).await?;
        self.safe_status()
    }

    pub(crate) async fn safe_backup_now(&self) -> Result<Value, ApiError> {
        if self.safe_key()?.is_none() {
            return Err(ApiError::Internal("Threema Safe is disabled".into()));
        }
        // Whichever way it goes, also with the page closed by then.
        self.note_ui_events(&["safeChanged"]);
        self.safe_upload(true).await?;
        self.safe_status()
    }

    /// Turn Safe off: first the backup goes from the server, as the key to it goes with the
    /// settings (Android's `disableSafe`). Off whatever the server says; the reply tells.
    pub(crate) async fn safe_disable(&self) -> Result<Value, ApiError> {
        let deleted = match self.safe_key() {
            Ok(Some(key)) => server::delete(&self.https, &key)
                .await
                .map_err(ApiError::from),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        self.forget_safe(deleted)
    }

    /// The local half of [`Self::safe_disable`]: `deleted` is how the server copy fared.
    fn forget_safe(&self, deleted: Result<(), ApiError>) -> Result<Value, ApiError> {
        if let Err(error) = &deleted {
            tracing::warn!(kind = error.kind(), "Threema Safe backup not deleted");
            // The wait can outlast the page, and with the key gone nobody can delete it later.
            self.note_ui_events(&["safeServerCopyLeft"]);
        }
        for k in [
            keys::SAFE_MASTER_KEY,
            keys::SAFE_LAST_UPLOAD,
            keys::SAFE_HASH,
            keys::SAFE_LAST_ERROR,
            keys::SAFE_FIRST_FAILURE,
            keys::SAFE_SIZE,
            keys::SAFE_MAX_BYTES,
            keys::SAFE_RETENTION_DAYS,
        ] {
            self.store.delete_setting(k)?;
        }
        // The window's warning goes with it.
        self.note_ui_events(&["safeChanged"]);
        let mut status = self.safe_status()?;
        status["serverCopyDeleted"] = json!(deleted.is_ok());
        Ok(status)
    }

    pub(crate) async fn safe_restore(
        &self,
        identity: &str,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
        self.check_no_identity()?;
        let identity = normalize_identity(identity).ok_or(ApiError::IdentityFormat)?;
        // Libre: a valid license comes first, as in the Android wizard.
        let _ = self.license()?;
        let key = derive(identity.clone(), password).await?;
        let blob = server::download(&self.https, &key).await?;
        let json = key.decrypt(&blob)?;
        let backup = safe::parse_backup(&json)?;

        let client_key = Zeroizing::new(
            BASE64
                .decode(backup.user.privatekey.as_bytes())
                .map_err(|_| ApiError::SafeDamaged("backup private key invalid".into()))?,
        );
        let client_key: [u8; 32] = client_key
            .as_slice()
            .try_into()
            .map_err(|_| ApiError::SafeDamaged("backup private key invalid".into()))?;
        let client_key = ClientKey::from(client_key);
        let private =
            crate::directory::fetch_identity_private(&self.https, &identity, &client_key).await?;
        let picture = self.restore_profile_picture(&backup.user);
        self.store_restored(&identity, &client_key, private, &backup, &key, picture)?;
        if let Ok(config) = server::config(&self.https, &key).await {
            self.store_safe_limits(config)?;
        }
        // Best effort now; failures are recorded and retried on reconnect, by maintenance and
        // on request.
        let events = self.try_pending_import().await;
        self.note_ui_events(&events);
        // log-ok: counts
        tracing::info!(
            contacts = backup.contacts.as_ref().map_or(0, Vec::len),
            groups = backup.groups.as_ref().map_or(0, Vec::len),
            "identity restored from Threema Safe"
        );
        Ok(json!({
            "identity": identity,
            "contacts": backup.contacts.as_ref().map_or(0, Vec::len),
            "groups": backup.groups.as_ref().map_or(0, Vec::len),
        }))
    }

    /// `safeSearchId {phone, email}`: which IDs are linked to a number or address, for someone who
    /// knows the Safe password but not the ID (Android's "forgot ID"). Before there is an ID: the
    /// only moment it is for. Both are hashed here and never stored or logged.
    pub(crate) async fn safe_search_id(&self, phone: &str, email: &str) -> Result<Value, ApiError> {
        self.check_no_identity()?;
        // Libre: a valid license comes first, as for the restore itself.
        let _ = self.license()?;
        let (email_hashes, phone_hashes) =
            crate::address_book::search_hashes(phone, email, self.address_book_region()?);
        if email_hashes.is_empty() && phone_hashes.is_empty() {
            return Err(ApiError::IdSearchInput);
        }
        let found =
            crate::directory::search_identities(&self.https, &email_hashes, &phone_hashes).await?;
        let mut identities: Vec<String> = found
            .into_iter()
            .filter_map(|found| normalize_identity(&found.identity))
            .collect();
        identities.sort();
        identities.dedup();
        // log-ok: a count
        tracing::info!(found = identities.len(), "ID search answered");
        Ok(json!({ "identities": identities }))
    }

    fn store_safe_limits(&self, config: server::ServerConfig) -> Result<(), ApiError> {
        self.store
            .set_setting(keys::SAFE_MAX_BYTES, &config.max_backup_bytes.to_string())?;
        self.store.set_setting(
            keys::SAFE_RETENTION_DAYS,
            &config.retention_days.to_string(),
        )?;
        Ok(())
    }

    /// Stored contacts plus restored ones that are not imported yet (never drop any; those keep
    /// the backup's own `lastUpdate`). The own ID is excluded, like on Android.
    fn backup_contacts(&self, updates: &LastUpdates) -> Result<Vec<backup::Contact>, ApiError> {
        let mut list: Vec<backup::Contact> = self
            .store
            .contacts()?
            .iter()
            .map(|contact| {
                let mut entry = crate::contacts::to_backup(contact);
                entry.is_private |= self.store.is_private(&entry.identity).unwrap_or(false);
                entry.last_update = last_update(updates, &entry.identity);
                entry
            })
            .collect();
        for pending in self.pending_backup_contacts()? {
            if !list
                .iter()
                .any(|c| c.identity.eq_ignore_ascii_case(&pending.identity))
            {
                list.push(pending);
            }
        }
        Ok(list)
    }

    /// Our picture, base64: the Safe-sized copy, or (pictures set before the bridge made one) the
    /// stored 512 px JPEG, which every reader decodes too.
    fn backup_profile_picture(&self) -> Option<String> {
        self.store
            .setting(keys::PROFILE_PICTURE_VERSION)
            .ok()
            .flatten()?;
        let read = |name| self.media.read_picture(name).ok().flatten();
        read(crate::profile_pictures::OWN_PICTURE_SAFE)
            .or_else(|| read(crate::profile_pictures::OWN_PICTURE))
            .map(|jpeg| BASE64.encode(&jpeg))
    }

    /// Our picture from a backup (Android `restoreUser`), as the picture and its Safe copy: whether
    /// that worked. If it can't be decoded, forget about it. Without a stored version the files
    /// mean nothing, so a kill right after this leaves no picture behind.
    fn restore_profile_picture(&self, user: &backup::User) -> bool {
        user.profile_pic
            .as_deref()
            .and_then(|b64| BASE64.decode(b64.as_bytes()).ok())
            .filter(|jpeg| jpeg.starts_with(&[0xff, 0xd8]))
            .is_some_and(|jpeg| {
                [
                    crate::profile_pictures::OWN_PICTURE,
                    crate::profile_pictures::OWN_PICTURE_SAFE,
                ]
                .iter()
                .all(|name| self.media.write_picture(name, &jpeg).is_ok())
            })
    }

    /// Everything a restore keeps, all or nothing. A kill or a full disk half way would leave an
    /// ID without its contacts and with Safe off: nothing would say so, the restore page is gone
    /// once there is an ID, and turning Safe on again would overwrite the good copy on the server.
    fn store_restored(
        &self,
        identity: &str,
        client_key: &ClientKey,
        private: crate::directory::PrivateIdentityData,
        backup: &Backup,
        key: &MasterKey,
        picture_stored: bool,
    ) -> Result<(), ApiError> {
        let transaction = self
            .store
            .conn
            .unchecked_transaction()
            .map_err(StoreError::from)?;
        self.store_adopted_identity(identity, client_key, private)?;
        match backup.user.nickname.as_deref().filter(|n| !n.is_empty()) {
            Some(nickname) => self.store.set_setting(keys::NICKNAME, nickname)?,
            // None in the backup: the same question as for an ID backup, which never holds one.
            None => {
                self.store.delete_setting(keys::NICKNAME)?;
                self.store.set_setting(keys::NICKNAME_UNANSWERED, "ask")?;
            }
        }
        if picture_stored {
            self.store.bump_profile_picture_version()?;
        }
        if let Some(release) = &backup.user.profile_pic_release {
            let (share, recipients) = crate::profile_pictures::from_safe_release(release);
            self.store.set_picture_share(share)?;
            self.store.set_picture_recipients(&recipients)?;
        }
        self.save_app_settings(&backup.settings)?;
        // Contacts, then groups and lists, are imported afterwards (lookups needed); until then
        // they wait here and are carried into our own backups.
        let pending = json!({
            "contacts": backup.contacts,
            "groups": backup.groups,
            "distributionlists": backup.distributionlists,
        });
        self.store
            .set_setting(keys::SAFE_PENDING_IMPORT, &pending.to_string())?;
        // Keep Safe on with the same key; the server copy is current, so no immediate upload.
        self.store
            .set_setting(keys::SAFE_MASTER_KEY, &HEXLOWER.encode(key.as_bytes()))?;
        self.store
            .set_setting(keys::SAFE_LAST_UPLOAD, &now().to_string())?;
        self.store.delete_setting(keys::SAFE_LAST_ERROR)?;
        self.store.delete_setting(keys::SAFE_FIRST_FAILURE)?;
        // A fresh restore starts a fresh import record.
        self.store.delete_setting(keys::SAFE_IMPORT_SUMMARY)?;
        self.store.delete_setting(keys::SAFE_IMPORT_ERROR)?;
        self.store
            .set_setting(keys::SAFE_RESTORE_SETUP, RESTORE_SETUP_START)?;
        transaction.commit().map_err(StoreError::from)?;
        Ok(())
    }

    /// Our groups, and those of a restored backup not imported yet (so a failed import loses
    /// nothing).
    fn backup_groups(
        &self,
        me: &str,
        pending: &Value,
        updates: &LastUpdates,
    ) -> Result<Vec<backup::Group>, ApiError> {
        let mut list: Vec<backup::Group> = self
            .store
            .groups()?
            .iter()
            .map(|group| {
                let mut entry = group_to_backup(group, me);
                entry.is_private = self.store.is_private(&group.key).unwrap_or(false);
                entry.last_update = last_update(updates, &group.key);
                entry
            })
            .collect();
        for entry in carried::<backup::Group>(pending, "groups").unwrap_or_default() {
            if !list
                .iter()
                .any(|g| g.id == entry.id.to_ascii_lowercase() && g.creator == entry.creator)
            {
                list.push(entry);
            }
        }
        Ok(list)
    }

    fn backup_lists(
        &self,
        pending: &Value,
        updates: &LastUpdates,
    ) -> Result<Vec<backup::DistributionList>, ApiError> {
        let mut list: Vec<backup::DistributionList> = self
            .store
            .distribution_lists()?
            .iter()
            .map(|list| {
                let mut entry = list_to_backup(list);
                entry.is_private = self.store.is_private(&list.key).unwrap_or(false);
                entry.last_update = last_update(updates, &list.key);
                entry
            })
            .collect();
        for entry in
            carried::<backup::DistributionList>(pending, "distributionlists").unwrap_or_default()
        {
            if entry.id.is_none() || !list.iter().any(|l| l.id == entry.id) {
                list.push(entry);
            }
        }
        Ok(list)
    }

    /// Import restored Safe groups and distribution lists, after the contacts (Android
    /// `restoreGroups` / `restoreDistributionLists`). Members we have no contact for are looked
    /// up first (group members stored as _group_ acquaintances, list members as direct contacts,
    /// like Android); IDs that don't exist any more are dropped. Groups and lists that exist
    /// already are left alone. On network errors everything stays pending for the next
    /// maintenance. Returns whether anything was imported.
    pub(crate) async fn import_pending_groups(&self) -> Result<bool, ApiError> {
        let mut pending: Value = self
            .store
            .setting(keys::SAFE_PENDING_IMPORT)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        // Contacts first: they decide who is known.
        if pending
            .get("contacts")
            .is_some_and(|c| c.as_array().is_some_and(|a| !a.is_empty()))
        {
            return Ok(false);
        }
        let groups: Vec<backup::Group> = carried(&pending, "groups").unwrap_or_default();
        let lists: Vec<backup::DistributionList> =
            carried(&pending, "distributionlists").unwrap_or_default();
        if groups.is_empty() && lists.is_empty() {
            return Ok(false);
        }
        let me = self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id;
        let known = |store: &crate::store::Store, id: &str| {
            id == me || store.contact(id).ok().flatten().is_some()
        };

        // Unknown people, once: list members become direct contacts, the rest group members.
        let mut unknown: Vec<(String, bool)> = Vec::new();
        let people = groups
            .iter()
            .filter(|g| !g.deleted)
            .flat_map(|g| g.members.iter().chain(std::iter::once(&g.creator)))
            .map(|m| (m, false))
            .chain(
                lists
                    .iter()
                    .flat_map(|l| l.members.iter())
                    .map(|m| (m, true)),
            );
        for (member, direct) in people {
            let Some(id) = normalize_identity(member) else {
                continue;
            };
            if known(&self.store, &id) {
                continue;
            }
            match unknown.iter_mut().find(|(u, _)| *u == id) {
                Some(entry) => entry.1 |= direct,
                None => unknown.push((id, direct)),
            }
        }
        if !unknown.is_empty() {
            let ids: Vec<String> = unknown.iter().map(|(id, _)| id.clone()).collect();
            for (id, result) in crate::contacts::lookup(&self.https, &self.config, &ids).await? {
                let crate::contacts::Lookup::Found(contact) = result else {
                    continue;
                };
                let mut contact = *contact;
                if unknown
                    .iter()
                    .any(|(u, direct)| *direct && u.eq_ignore_ascii_case(&id))
                {
                    contact.acquaintance_level =
                        libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct;
                }
                let id = contact.identity.to_string();
                if self.store.contact(&id)?.is_none() {
                    self.store.upsert_contact(&contact)?;
                    self.store.mark_picture_restored(&id)?;
                }
            }
        }

        let (mut imported_groups, mut imported_lists) = (0_usize, 0_usize);
        let mut skipped: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for entry in &groups {
            let group = match group_from_backup(entry, &me, |id| known(&self.store, id)) {
                Ok(group) => group,
                Err(reason) => {
                    *skipped.entry(reason).or_default() += 1;
                    continue;
                }
            };
            if self.store.group(&group.key)?.is_none() {
                self.store.upsert_group(&group)?;
                if entry.is_private {
                    self.mark_private(&group.key, true)?;
                }
                self.store
                    .note_restored_chat(&group.key, entry.last_update)?;
                imported_groups += 1;
            }
        }
        for entry in &lists {
            let list = list_from_backup(entry, |id| known(&self.store, id));
            if self.store.distribution_list(&list.key)?.is_none() {
                self.store.upsert_distribution_list(&list)?;
                if entry.is_private {
                    self.mark_private(&list.key, true)?;
                }
                self.store
                    .note_restored_chat(&list.key, entry.last_update)?;
                imported_lists += 1;
            }
        }
        if let Some(object) = pending.as_object_mut() {
            object.remove("groups");
            object.remove("distributionlists");
        }
        self.store
            .set_setting(keys::SAFE_PENDING_IMPORT, &pending.to_string())?;
        let skipped_total: usize = skipped.values().sum();
        self.update_import_summary(json!({
            "groups": imported_groups,
            "lists": imported_lists,
            "skippedGroups": skipped,
        }))?;
        tracing::info!(
            groups = imported_groups,
            lists = imported_lists,
            skipped = skipped_total,
            "Safe groups and lists imported"
        );
        Ok(true)
    }

    /// Merge `fields` into the stored restore summary (`SAFE_IMPORT_SUMMARY`), stamped with the
    /// time: the imports write their counts here for the status page.
    pub(crate) fn update_import_summary(&self, fields: Value) -> Result<(), ApiError> {
        let mut summary: Value = self
            .store
            .setting(keys::SAFE_IMPORT_SUMMARY)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}));
        if let (Some(object), Some(new)) = (summary.as_object_mut(), fields.as_object()) {
            for (name, value) in new {
                object.insert(name.clone(), value.clone());
            }
            object.insert("at".into(), json!(crate::messages::now_ms()));
        }
        self.store
            .set_setting(keys::SAFE_IMPORT_SUMMARY, &summary.to_string())?;
        Ok(())
    }

    /// How much of a restored backup still waits to be imported: contacts, groups, lists.
    fn pending_import_counts(&self) -> Result<(usize, usize, usize), ApiError> {
        let pending: Value = self
            .store
            .setting(keys::SAFE_PENDING_IMPORT)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let count = |name: &str| pending[name].as_array().map_or(0, Vec::len);
        Ok((
            count("contacts"),
            count("groups"),
            count("distributionlists"),
        ))
    }

    /// Run the pending Safe import (contacts first, then groups and lists). A failure is kept in
    /// `SAFE_IMPORT_ERROR` for the status page and cleared by the next successful attempt; the
    /// import is retried when the connection comes back, by `maintenance`, and on request
    /// (`safeRetryImport`). Returns the UI events the attempt caused.
    pub(crate) async fn try_pending_import(&self) -> Vec<&'static str> {
        let mut events = Vec::new();
        let result = async {
            let contacts = self.import_pending_contacts().await?;
            let groups = self.import_pending_groups().await?;
            Ok::<_, ApiError>((contacts, groups))
        }
        .await;
        let had_error = self
            .store
            .setting(keys::SAFE_IMPORT_ERROR)
            .ok()
            .flatten()
            .is_some();
        match result {
            Ok((contacts, groups)) => {
                if contacts {
                    events.push("contactsChanged");
                }
                if groups {
                    events.push("contactsChanged");
                    events.push("groupsChanged");
                }
                if had_error {
                    let _ = self.store.delete_setting(keys::SAFE_IMPORT_ERROR);
                }
                if had_error || contacts || groups {
                    events.push("safeChanged");
                }
            }
            Err(error) => {
                if matches!(error, ApiError::RateLimited) {
                    self.store.rest_directory_calls();
                }
                tracing::warn!(kind = error.kind(), "pending Safe import failed");
                if self
                    .store
                    .set_setting(keys::SAFE_IMPORT_ERROR, error.kind())
                    .is_ok()
                    && !had_error
                {
                    events.push("safeChanged");
                }
            }
        }
        events.dedup();
        events
    }

    /// The network's part of [`App::on_connected`].
    pub(crate) async fn on_connected_away(&self) -> Vec<&'static str> {
        let mut events = Vec::new();
        let pending = self
            .pending_import_counts()
            .map(|(contacts, groups, lists)| contacts + groups + lists)
            .unwrap_or(0);
        if pending > 0 && !self.store.directory_resting() {
            events = self.try_pending_import().await;
        }
        self.run_restore_setup().await;
        self.resume_picture_downloads();
        self.request_restored_pictures(PICTURE_REQUEST_BATCH);
        events
    }

    /// A restored device's _Application Setup Steps_ (Android `ApplicationSetupSteps.kt`), once
    /// the import is done and the connection is up: its groups and sessions are as old as the
    /// backup. Upstream's first step comes first here too: nothing is said about forward secrecy
    /// before the directory knows our feature mask; if that fails, the next round tries again.
    async fn run_restore_setup(&self) {
        let waiting = self.store.setting(keys::SAFE_RESTORE_SETUP);
        let imported = self
            .pending_import_counts()
            .is_ok_and(|counts| counts == (0, 0, 0));
        // The feature mask goes first, and during a rest it stays home without saying so.
        if !matches!(waiting, Ok(Some(_)))
            || !imported
            || self.connection_state.get() != "connected"
            || self.store.directory_resting()
        {
            return;
        }
        if let Err(error) = self.publish_feature_mask().await {
            tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
            return;
        }
        if let Err(error) = self.restore_setup_step() {
            tracing::warn!(kind = error.kind(), "restore setup step failed");
        }
    }

    /// One round of the setup steps. The first queues what the groups need, in the outbox, and
    /// picks the contacts; each then starts forward secrecy with a bounded number of them and asks
    /// them for their picture (their next message would do too, so nobody has to hurry, and the
    /// servers don't get a burst). What is left waits in the setting for the next round.
    fn restore_setup_step(&self) -> Result<(), ApiError> {
        let Some(state) = self.store.setting(keys::SAFE_RESTORE_SETUP)? else {
            return Ok(());
        };
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let mut contacts: Vec<String> = if state == RESTORE_SETUP_START {
            let transaction = self
                .store
                .conn
                .unchecked_transaction()
                .map_err(StoreError::from)?;
            let mut queued = 0_usize;
            for group in self.store.groups()? {
                if crate::groups::resync_restored(&self.store, &group, &identity.threema_id)? {
                    queued += 1;
                }
            }
            let contacts = self.solicited_contacts(&identity.threema_id)?;
            self.store.set_setting(
                keys::SAFE_RESTORE_SETUP,
                &serde_json::to_string(&contacts).map_err(|e| ApiError::Internal(e.to_string()))?,
            )?;
            transaction.commit().map_err(StoreError::from)?;
            tracing::info!(
                groups = queued,
                total = contacts.len(),
                "restore setup started"
            );
            contacts
        } else {
            serde_json::from_str(&state).unwrap_or_default()
        };
        let rest = contacts.split_off(contacts.len().min(RESTORE_SETUP_BATCH));
        // As upstream: no sessions with multi-device (or forward secrecy off, the same thing here).
        let fs_on =
            self.store.multi_device()?.is_none() && !self.store.forward_security_disabled()?;
        let own: libthreema::common::ThreemaId = identity
            .threema_id
            .parse()
            .map_err(|_| ApiError::Internal("stored identity invalid".into()))?;
        let client_key = ClientKey::from(*identity.client_key);
        for id in contacts {
            let Some(contact) = self.store.contact(&id)? else {
                continue;
            };
            if self.store.is_explicitly_blocked(&id)? {
                continue;
            }
            if fs_on && crate::contacts::has_fs_support(&contact) {
                match crate::fs_store::start_session(
                    &self.store,
                    own,
                    &client_key,
                    contact.identity,
                    &contact.public_key,
                    u64::try_from(crate::messages::now_ms()).unwrap_or(0),
                ) {
                    Ok(inits) => {
                        for envelope in inits {
                            self.tell_connection(crate::connection::ToConnection::FsControl {
                                peer: id.clone(),
                                peer_key: Some(contact.public_key),
                                envelope,
                            });
                        }
                    }
                    Err(error) => tracing::warn!(%error, "cannot start a forward secrecy session"),
                }
            }
            self.store.request_restored_picture(&id)?;
        }
        if rest.is_empty() {
            self.store.delete_setting(keys::SAFE_RESTORE_SETUP)?;
            tracing::info!("restore setup done");
        } else {
            self.store.set_setting(
                keys::SAFE_RESTORE_SETUP,
                &serde_json::to_string(&rest).map_err(|e| ApiError::Internal(e.to_string()))?,
            )?;
        }
        self.tell_connection(crate::connection::ToConnection::SendPending);
        Ok(())
    }
}

impl Shared {
    /// Upstream's _solicited contacts_: the people in groups we are in and those we have a chat
    /// with, without revoked IDs. Blocked contacts aren't written to either.
    pub(crate) fn solicited_contacts(&self, me: &str) -> Result<Vec<String>, ApiError> {
        let mut candidates = std::collections::BTreeSet::new();
        for group in self.store.groups()?.into_iter().filter(|group| !group.left) {
            candidates.insert(group.identity.creator_identity.to_string());
            candidates.extend(group.members);
        }
        for (last, _) in self.store.conversations()? {
            if normalize_identity(&last.conversation).is_some() {
                candidates.insert(last.conversation);
            }
        }
        let mut contacts = Vec::new();
        for id in candidates {
            let valid = self.store.contact(&id)?.is_some_and(|contact| {
                contact.activity_state
                    != libthreema::protobuf::d2d_sync::contact::ActivityState::Invalid
            });
            if valid && id != me && !self.store.is_explicitly_blocked(&id)? {
                contacts.push(id);
            }
        }
        Ok(contacts)
    }

    /// The backup as it would be uploaded now.
    fn build_backup(&self) -> Result<Backup, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let pending: Value = self
            .store
            .setting(keys::SAFE_PENDING_IMPORT)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let updates = crate::join::last_updates(&self.store)?;
        let (email, mobile) = self.store.confirmed_links()?;
        let mut links = Vec::new();
        if let Some(mobile) = mobile {
            links.push(backup::Link {
                kind: "mobile".into(),
                value: mobile.trim_start_matches('+').to_owned(),
                name: None,
            });
        }
        if let Some(email) = email {
            links.push(backup::Link {
                kind: "email".into(),
                value: email,
                name: None,
            });
        }
        Ok(Backup {
            info: backup::Info {
                version: backup::VERSION,
                device: Some(format!(
                    "{}{}/{}_{}",
                    self.version.app_version,
                    crate::directory::PLATFORM_CODE,
                    self.version.language,
                    self.version.country
                )),
            },
            user: backup::User {
                privatekey: BASE64.encode(&identity.client_key[..]),
                nickname: self.store.setting(keys::NICKNAME)?,
                profile_pic: self.backup_profile_picture(),
                profile_pic_release: Some(crate::profile_pictures::safe_release(
                    self.store.picture_share()?,
                    &self.store.picture_recipients()?,
                )),
                links,
            },
            contacts: Some(self.backup_contacts(&updates)?),
            groups: Some(self.backup_groups(&identity.threema_id, &pending, &updates)?),
            distributionlists: Some(self.backup_lists(&pending, &updates)?),
            settings: self.app_settings()?,
        })
    }
}

impl Shared {
    /// Upload if the policy says so (or `force`). Returns whether an upload happened. Failures
    /// are recorded in `SAFE_LAST_ERROR` and returned.
    async fn safe_upload(&self, force: bool) -> Result<bool, ApiError> {
        let Some(key) = self.safe_key()? else {
            return Ok(false);
        };
        let result = self.safe_upload_with(&key, force).await;
        self.note_safe_upload(result.as_ref().err())?;
        result
    }

    /// How an upload attempt went: the last error, and since when they have kept failing.
    fn note_safe_upload(&self, error: Option<&ApiError>) -> Result<(), ApiError> {
        let Some(error) = error else {
            self.store.delete_setting(keys::SAFE_LAST_ERROR)?;
            self.store.delete_setting(keys::SAFE_FIRST_FAILURE)?;
            return Ok(());
        };
        tracing::warn!(kind = error.kind(), "Threema Safe backup failed");
        self.store
            .set_setting(keys::SAFE_LAST_ERROR, error.kind())?;
        if self.store.setting(keys::SAFE_FIRST_FAILURE)?.is_none() {
            self.store
                .set_setting(keys::SAFE_FIRST_FAILURE, &now().to_string())?;
        }
        Ok(())
    }

    async fn safe_upload_with(&self, key: &MasterKey, force: bool) -> Result<bool, ApiError> {
        let json = Zeroizing::new(
            serde_json::to_vec(&self.build_backup()?)
                .map_err(|e| ApiError::Internal(e.to_string()))?,
        );
        let hash = HEXLOWER.encode(&Sha256::digest(json.as_slice()));
        let changed = self.store.setting(keys::SAFE_HASH)?.as_deref() != Some(hash.as_str());
        let due = safe::upload_due(
            changed,
            self.setting_i64(keys::SAFE_LAST_UPLOAD)?,
            self.store.setting(keys::SAFE_LAST_ERROR)?.is_some(),
            self.setting_i64(keys::SAFE_RETENTION_DAYS)?.unwrap_or(0),
            now(),
            force,
        );
        if !due {
            return Ok(false);
        }
        let config = server::config(&self.https, key).await?;
        self.store_safe_limits(config)?;
        let blob = key.encrypt(&json)?;
        if u64::try_from(blob.len()).unwrap_or(u64::MAX) > config.max_backup_bytes {
            return Err(ServerError::TooLarge {
                size: blob.len(),
                max: config.max_backup_bytes,
            }
            .into());
        }
        let size = blob.len();
        server::upload(&self.https, key, blob).await?;
        self.store.set_setting(keys::SAFE_HASH, &hash)?;
        self.store
            .set_setting(keys::SAFE_LAST_UPLOAD, &now().to_string())?;
        self.store.set_setting(keys::SAFE_SIZE, &size.to_string())?;
        tracing::info!(size, "Threema Safe backup uploaded");
        Ok(true)
    }
}

impl Shared {
    /// The part of periodic work that waits for the network, beside the core loop:
    /// [`App::finish_maintenance`] does the rest.
    pub(crate) async fn maintenance_away(&self) -> MaintenanceRound {
        let mut round = MaintenanceRound::default();
        match self.store.identity() {
            Ok(Some(_)) => {}
            _ => return round,
        }
        // Refused, and nothing goes to Threema's servers until a key is accepted again.
        if !self.license_valid().unwrap_or(false) {
            return round;
        }
        match self.ask_about_license().await {
            Ok(false) => {}
            Ok(true) => {
                round.refused = true;
                return round;
            }
            Err(error) => tracing::warn!(kind = error.kind(), "license check failed"),
        }
        if !self.store.directory_resting() {
            round.events.extend(self.try_pending_import().await);
        }
        self.run_restore_setup().await;
        self.resume_picture_downloads();
        self.request_restored_pictures(PICTURE_REQUEST_BATCH);
        match self
            .store
            .expire_unresolved_outbox(crate::messages::now_ms() - UNRESOLVED_OUTBOX_MAX_AGE_MS)
        {
            Ok(0) => {}
            Ok(receivers) => {
                tracing::warn!(receivers, "gave up on group receivers with no contact")
            }
            Err(error) => tracing::warn!(%error, "expiring the group outbox failed"),
        }
        if self.check_email_link().await.unwrap_or(false) {
            round.events.push("profileChanged");
        }
        if self.refresh_contacts(false).await.unwrap_or(false) {
            round.events.push("contactsChanged");
        }
        self.check_device_group();
        if let Err(error) = self.publish_feature_mask().await {
            tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
        }
        let before = self.store.setting(keys::SAFE_LAST_ERROR).ok().flatten();
        let uploaded = self.safe_upload(false).await.unwrap_or(false);
        let after = self.store.setting(keys::SAFE_LAST_ERROR).ok().flatten();
        // Also while the warning stands: its days go up, and it may have just come due.
        if uploaded || before != after || self.safe_warning().ok().flatten().is_some() {
            round.events.push("safeChanged");
        }
        if self.backup_reminder_due(now()).unwrap_or(false) {
            round.events.push("backupReminder");
        }
        round.licensed = true;
        round
    }
}

/// How far a round of maintenance got beside the core loop.
#[derive(Default)]
pub(crate) struct MaintenanceRound {
    events: Vec<&'static str>,
    /// There is an ID with a license, and it wasn't refused during the round.
    licensed: bool,
    /// The directory refused the license just now.
    refused: bool,
}

impl App {
    /// The connection is up: try a pending Safe import right away — `maintenance` alone would
    /// leave restored groups waiting up to an hour after a network failure — and pick up
    /// picture recovery (kept downloads, restored contacts' requests in a bounded batch).
    /// Rate-limited so flapping connections don't hammer the servers. Whether the part that can
    /// wait for the network ([`Shared::on_connected_away`]) is to run now.
    pub(crate) fn on_connected_here(&mut self) -> bool {
        self.check_device_group();
        self.retry_failed_transfers();
        let now = tokio::time::Instant::now();
        if self
            .last_import_attempt
            .get()
            .is_some_and(|at| now.duration_since(at) < std::time::Duration::from_secs(60))
        {
            return false;
        }
        self.last_import_attempt.set(Some(now));
        true
    }

    /// All of what follows connecting, in place. Returns the UI events to emit.
    #[cfg(test)]
    pub async fn on_connected(&mut self) -> Vec<&'static str> {
        if self.on_connected_here() {
            self.shared.on_connected_away().await
        } else {
            Vec::new()
        }
    }

    /// The end of a round of maintenance, on the core loop. Returns the events to emit to the UI.
    pub(crate) fn finish_maintenance(&mut self, mut round: MaintenanceRound) -> Vec<&'static str> {
        if round.refused {
            self.license_refused();
            round.events.push("licenseChanged");
        }
        if round.licensed {
            self.android_export_scheduled();
            if self.expire_messages() {
                round.events.push("chatsChanged");
            }
        }
        round.events
    }

    /// Periodic work (a while after the start, then hourly): pending Safe contact import and the
    /// automatic Safe backup, all in place. Returns the events to emit to the UI.
    #[cfg(test)]
    pub async fn maintenance(&mut self) -> Vec<&'static str> {
        let round = self.shared.maintenance_away().await;
        self.finish_maintenance(round)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity};

    fn app_without_identity(dir: &std::path::Path) -> App {
        App::new(
            &StartConfig {
                data_dir: dir.to_owned(),
                app_version: "0.1.0".into(),
                language: "de".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[7; 32],
        )
        .unwrap()
    }

    /// An app with the ID `AAAAAAAA`.
    fn app_with_identity(dir: &std::path::Path) -> App {
        let app = App::new(
            &StartConfig {
                data_dir: dir.to_owned(),
                app_version: "0.1.0".into(),
                language: "de".into(),
                country: "CH".into(),
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
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "AAAAAAAA".into(),
                client_key: Zeroizing::new([5; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        app
    }

    #[test]
    fn identity_format() {
        assert_eq!(
            normalize_identity(" echoecho ").as_deref(),
            Some("ECHOECHO")
        );
        assert_eq!(normalize_identity("*THREEMA").as_deref(), Some("*THREEMA"));
        assert!(normalize_identity("ECHO*CHO").is_none());
        assert!(normalize_identity("ECHOECH").is_none());
    }

    #[test]
    fn backup_carries_pending_import_and_profile() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "de".into(),
                country: "CH".into(),
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
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([5; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        app.store.set_setting(keys::NICKNAME, "julia").unwrap();
        app.store
            .set_setting(keys::LINKED_MOBILE, "+41791234567")
            .unwrap();
        app.store
            .set_setting(
                keys::SAFE_PENDING_IMPORT,
                r#"{"contacts":[{"identity":"ABCDEFGH","verification":1}],"groups":null,
                    "distributionlists":[]}"#,
            )
            .unwrap();
        app.store
            .set_setting(
                keys::APP_SETTINGS,
                r#"{"syncContacts":true,"readReceipts":false}"#,
            )
            .unwrap();
        let backup = app.build_backup().unwrap();
        assert_eq!(backup.user.privatekey, BASE64.encode(&[5; 32]));
        assert_eq!(backup.user.nickname.as_deref(), Some("julia"));
        assert_eq!(backup.user.links[0].value, "41791234567");
        assert_eq!(backup.info.device.as_deref(), Some("0.1.0S/de_CH"));
        assert_eq!(backup.contacts.unwrap()[0].identity, "ABCDEFGH");
        assert_eq!(backup.groups.unwrap().len(), 0);
        assert!(backup.settings.sync_contacts);
        assert!(!backup.settings.read_receipts);

        // What we write must be readable by our own (Android-compatible) reader.
        let key = MasterKey::from_bytes([1; 64]);
        let blob = key
            .encrypt(&serde_json::to_vec(&app.build_backup().unwrap()).unwrap())
            .unwrap();
        let parsed = safe::parse_backup(&key.decrypt(&blob).unwrap()).unwrap();
        assert_eq!(parsed.contacts.unwrap().len(), 1);
    }

    #[test]
    fn server_and_backup_failures_have_kinds_of_their_own() {
        let kind = |error: ServerError| ApiError::from(error).kind();
        assert_eq!(kind(ServerError::Status(503)), "serverTrouble");
        assert_eq!(
            kind(ServerError::InvalidResponse("line 1".into())),
            "serverTrouble"
        );
        assert_eq!(kind(ServerError::NotFound), "safeNotFound");
        let kind = |error: crate::directory::DirectoryError| ApiError::from(error).kind();
        assert_eq!(
            kind(crate::directory::DirectoryError::Status(503)),
            "serverTrouble"
        );
        assert_eq!(
            kind(crate::directory::DirectoryError::InvalidResponse(
                "line 1".into()
            )),
            "serverTrouble"
        );
        assert_eq!(
            kind(crate::directory::DirectoryError::Server("Nein".into())),
            "server"
        );
        // A backup from a newer version, and one that isn't JSON.
        let error = safe::parse_backup(br#"{"info":{"version":2}}"#).unwrap_err();
        assert_eq!(ApiError::from(error).kind(), "backupVersion");
        let error = safe::parse_backup(b"not json").unwrap_err();
        assert_eq!(ApiError::from(error).kind(), "safeDamaged");
        // Found by the key, but it won't decrypt.
        let key = MasterKey::from_bytes([1; 64]);
        let mut damaged = key.encrypt(b"{}").unwrap();
        damaged[30] ^= 1;
        let error = key.decrypt(&damaged).unwrap_err();
        assert_eq!(ApiError::from(error).kind(), "safeDamaged");
    }

    #[test]
    fn only_confirmed_links_leave_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        let links = |app: &App| -> Vec<String> {
            let backup = app.build_backup().unwrap();
            let synced = crate::join::profile_sync(&app.store)
                .unwrap()
                .identity_links
                .unwrap()
                .links;
            assert_eq!(synced.len(), backup.user.links.len(), "the same for both");
            backup.user.links.into_iter().map(|l| l.value).collect()
        };
        app.store
            .set_setting(keys::LINKED_EMAIL, "ab@example.org")
            .unwrap();
        app.store
            .set_setting(keys::LINKED_MOBILE, "41791234567")
            .unwrap();
        assert_eq!(links(&app), ["41791234567", "ab@example.org"]);
        // Both waiting for their confirmation: neither may go anywhere.
        app.store
            .set_setting(keys::LINKED_EMAIL_PENDING, "1")
            .unwrap();
        app.store
            .set_setting(keys::LINKED_MOBILE_PENDING, "41790000000 v1")
            .unwrap();
        assert!(links(&app).is_empty());
        app.store
            .delete_setting(keys::LINKED_EMAIL_PENDING)
            .unwrap();
        assert_eq!(links(&app), ["ab@example.org"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_setup_waits_for_a_directory_rest_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        app.store
            .set_setting(keys::SAFE_RESTORE_SETUP, RESTORE_SETUP_START)
            .unwrap();
        app.connection_state.set("connected");
        app.store.rest_directory_calls();
        app.run_restore_setup().await;
        assert_eq!(
            app.store
                .setting(keys::SAFE_RESTORE_SETUP)
                .unwrap()
                .as_deref(),
            Some(RESTORE_SETUP_START),
            "no step before the feature mask is out"
        );
    }

    /// An app without an ID, and what it restores.
    fn restore_material(dir: &std::path::Path) -> (App, Backup, MasterKey) {
        let app = app_with_identity(dir);
        let mut backup = app.build_backup().unwrap();
        backup.user.nickname = Some("julia".into());
        backup.contacts = Some(vec![crate::contacts::to_backup(
            &crate::contacts::test_contact("BBBBBBBB"),
        )]);
        app.store.wipe().unwrap();
        (app, backup, MasterKey::from_bytes([1; 64]))
    }

    fn private_data() -> crate::directory::PrivateIdentityData {
        crate::directory::PrivateIdentityData {
            server_group: "a1".into(),
            email: None,
            mobile_no: None,
        }
    }

    #[test]
    fn a_restore_is_stored_all_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let (app, backup, key) = restore_material(dir.path());
        let client_key = ClientKey::from([5; 32]);
        app.store_restored("AAAAAAAA", &client_key, private_data(), &backup, &key, true)
            .unwrap();
        assert!(app.store.identity().unwrap().is_some());
        let status = app.safe_status().unwrap();
        assert_eq!(status["enabled"], true);
        assert_eq!(status["restore"]["pendingContacts"], 1);
        assert!(status["lastBackup"].is_number());
        assert!(
            app.store
                .setting(keys::PROFILE_PICTURE_VERSION)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            app.store.setting(keys::NICKNAME).unwrap().as_deref(),
            Some("julia")
        );
    }

    #[test]
    fn a_restore_without_a_nickname_asks_instead_of_removing_it() {
        let dir = tempfile::tempdir().unwrap();
        let (app, mut backup, key) = restore_material(dir.path());
        backup.user.nickname = None;
        let client_key = ClientKey::from([5; 32]);
        app.store_restored("AAAAAAAA", &client_key, private_data(), &backup, &key, true)
            .unwrap();
        assert_eq!(app.store.setting(keys::NICKNAME).unwrap(), None);
        assert_eq!(app.profile_json().unwrap()["askNickname"], true);

        let dir = tempfile::tempdir().unwrap();
        let (app, backup, key) = restore_material(dir.path());
        app.store_restored("AAAAAAAA", &client_key, private_data(), &backup, &key, true)
            .unwrap();
        assert_eq!(app.store.setting(keys::NICKNAME_UNANSWERED).unwrap(), None);
        assert_eq!(app.profile_json().unwrap()["askNickname"], false);
    }

    #[test]
    fn a_restore_that_cant_finish_leaves_no_id_so_it_can_be_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let (app, backup, key) = restore_material(dir.path());
        // The last write fails, as with a full disk.
        app.store
            .conn
            .execute_batch(
                "CREATE TRIGGER no_more BEFORE INSERT ON settings WHEN NEW.key = 'safe_last_upload'
                 BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
            )
            .unwrap();
        let client_key = ClientKey::from([5; 32]);
        app.store_restored("AAAAAAAA", &client_key, private_data(), &backup, &key, true)
            .unwrap_err();
        assert!(app.store.conn.is_autocommit(), "no transaction left open");
        assert!(app.store.identity().unwrap().is_none());
        for name in [
            keys::NICKNAME,
            keys::SAFE_PENDING_IMPORT,
            keys::SAFE_MASTER_KEY,
            keys::PROFILE_PICTURE_VERSION,
        ] {
            assert!(app.store.setting(name).unwrap().is_none(), "{name}");
        }
        // Once it works, the same restore goes through.
        app.store
            .conn
            .execute_batch("DROP TRIGGER no_more;")
            .unwrap();
        app.store_restored(
            "AAAAAAAA",
            &client_key,
            private_data(),
            &backup,
            &key,
            false,
        )
        .unwrap();
        assert!(app.store.identity().unwrap().is_some());
    }

    #[test]
    fn a_backup_says_when_each_chat_last_changed() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        for id in ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        app.store
            .insert_incoming("BBBBBBBB", 1, "text", "hi", 5000)
            .unwrap();
        // A restored chat without messages, and a contact who never had a chat.
        app.store
            .note_restored_chat("CCCCCCCC", Some(3000))
            .unwrap();
        // Waiting to be imported: the backup's own value stays.
        app.store
            .set_setting(
                keys::SAFE_PENDING_IMPORT,
                r#"{"contacts":[{"identity":"EEEEEEEE","verification":0,"lastUpdate":7000}]}"#,
            )
            .unwrap();
        let contacts = app.build_backup().unwrap().contacts.unwrap();
        let of = |id: &str| {
            contacts
                .iter()
                .find(|c| c.identity == id)
                .unwrap()
                .last_update
        };
        // When it arrived here, not the sender's clock (5000).
        let arrived = |id: &str| {
            let newest = app.store.conversation_messages(id, 1).unwrap();
            Some(newest[0].arrived_at())
        };
        assert_eq!(of("BBBBBBBB"), arrived("BBBBBBBB"));
        assert_ne!(of("BBBBBBBB"), Some(5000));
        assert_eq!(of("CCCCCCCC"), Some(3000));
        assert_eq!(of("DDDDDDDD"), None);
        assert_eq!(of("EEEEEEEE"), Some(7000));

        // A later message wins over the restored time, and deleting the chat ends it.
        app.store
            .insert_incoming("CCCCCCCC", 2, "text", "hi", 9000)
            .unwrap();
        app.store.delete_conversation("BBBBBBBB").unwrap();
        let contacts = app.build_backup().unwrap().contacts.unwrap();
        let of = |id: &str| {
            contacts
                .iter()
                .find(|c| c.identity == id)
                .unwrap()
                .last_update
        };
        assert_eq!(of("BBBBBBBB"), None);
        assert_eq!(of("CCCCCCCC"), arrived("CCCCCCCC"));
        // Android's old negative values are no time at all.
        app.store.note_restored_chat("DDDDDDDD", Some(-5)).unwrap();
        assert_eq!(
            last_update(&crate::join::last_updates(&app.store).unwrap(), "DDDDDDDD"),
            None
        );
    }

    #[test]
    fn a_restored_private_chat_without_messages_stays_out_of_the_chat_list() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        for id in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
            app.store.note_restored_chat(id, Some(3000)).unwrap();
        }
        app.mark_private("CCCCCCCC", true).unwrap();
        let list = app.conversations_json(false, false).unwrap();
        let listed: Vec<&str> = list["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| chat["identity"].as_str().unwrap())
            .collect();
        assert_eq!(listed, ["BBBBBBBB"]);
    }

    /// A restored app, connected: a group of ours, one of someone else's, one left; `DDDDDDDD`
    /// is only in the left group and `FFFFFFFF` is blocked.
    fn restored_app() -> (
        tempfile::TempDir,
        App,
        tokio::sync::mpsc::UnboundedReceiver<crate::connection::ToConnection>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.connection.replace(Some(tx));
        app.connection_state.set("connected");
        for id in ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD", "EEEEEEEE", "FFFFFFFF"] {
            let mut contact = crate::contacts::test_contact(id);
            contact.feature_mask = libthreema::common::FeatureMask(
                libthreema::common::FeatureMask::FORWARD_SECURITY_SUPPORT,
            );
            app.store.upsert_contact(&contact).unwrap();
        }
        let group = |creator: &str, id: u64, members: &[&str], left: bool| {
            let identity = libthreema::common::GroupIdentity {
                creator_identity: libthreema::common::ThreemaId::try_from(creator).unwrap(),
                group_id: id,
            };
            crate::groups::Group {
                key: crate::groups::key(&identity),
                identity,
                name: format!("Group {id}"),
                members: members.iter().map(|m| (*m).to_owned()).collect(),
                left,
                created_at: 1000,
                picture: None,
                picture_version: 0,
            }
        };
        for g in [
            group("AAAAAAAA", 1, &["BBBBBBBB"], false),
            group("CCCCCCCC", 2, &["CCCCCCCC"], false),
            group("DDDDDDDD", 3, &["DDDDDDDD"], true),
        ] {
            app.store.upsert_group(&g).unwrap();
        }
        app.store
            .note_restored_chat("EEEEEEEE", Some(3000))
            .unwrap();
        app.store
            .note_restored_chat("FFFFFFFF", Some(3000))
            .unwrap();
        app.store
            .set_setting(keys::APP_SETTINGS, r#"{"blockedContacts":["FFFFFFFF"]}"#)
            .unwrap();
        app.store
            .set_setting(keys::SAFE_RESTORE_SETUP, RESTORE_SETUP_START)
            .unwrap();
        (dir, app, rx)
    }

    fn inits_to(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::connection::ToConnection>,
    ) -> Vec<String> {
        let mut peers = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let crate::connection::ToConnection::FsControl { peer, .. } = message {
                peers.push(peer);
            }
        }
        peers
    }

    #[test]
    fn a_restore_brings_groups_and_sessions_up_to_date_once() {
        let (_dir, app, mut rx) = restored_app();
        let queued = |key: &str| -> Vec<String> {
            let mut statement = app
                .store
                .conn
                .prepare("SELECT kind FROM outbox WHERE conversation = ?1 ORDER BY id")
                .unwrap();
            statement
                .query_map([key], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        app.restore_setup_step().unwrap();
        // Our group: its state, without saying anything about a picture we don't have.
        let ours = app.store.groups().unwrap();
        let key_of = |creator: &str| {
            ours.iter()
                .find(|g| g.identity.creator_identity.as_str() == creator)
                .unwrap()
                .key
                .clone()
        };
        assert_eq!(
            queued(&key_of("AAAAAAAA")),
            [
                crate::groups::OUTBOX_GROUP_SETUP,
                crate::groups::OUTBOX_GROUP_NAME
            ]
        );
        // Someone else's: a request. One we left: nothing.
        assert_eq!(
            queued(&key_of("CCCCCCCC")),
            [crate::groups::OUTBOX_GROUP_SYNC_REQUEST]
        );
        assert!(queued(&key_of("DDDDDDDD")).is_empty());
        // Sessions: group people and those with a chat; not the blocked, not a left group's.
        assert_eq!(inits_to(&mut rx), ["BBBBBBBB", "CCCCCCCC", "EEEEEEEE"]);
        assert!(app.store.fs_peers().unwrap().len() == 3);
        assert!(
            app.store
                .setting(keys::SAFE_RESTORE_SETUP)
                .unwrap()
                .is_none(),
            "done, and not again"
        );
        app.restore_setup_step().unwrap();
        assert!(inits_to(&mut rx).is_empty());
        assert_eq!(queued(&key_of("CCCCCCCC")).len(), 1);
    }

    #[test]
    fn a_restores_contacts_go_in_batches() {
        let (_dir, app, mut rx) = restored_app();
        let many: Vec<String> = (0..RESTORE_SETUP_BATCH + 5)
            .map(|n| format!("ZZZZ{n:04}"))
            .collect();
        for id in &many {
            let mut contact = crate::contacts::test_contact(id);
            contact.feature_mask = libthreema::common::FeatureMask(
                libthreema::common::FeatureMask::FORWARD_SECURITY_SUPPORT,
            );
            app.store.upsert_contact(&contact).unwrap();
        }
        app.store
            .set_setting(
                keys::SAFE_RESTORE_SETUP,
                &serde_json::to_string(&many).unwrap(),
            )
            .unwrap();
        app.restore_setup_step().unwrap();
        assert_eq!(inits_to(&mut rx).len(), RESTORE_SETUP_BATCH);
        assert!(
            app.store
                .setting(keys::SAFE_RESTORE_SETUP)
                .unwrap()
                .is_some()
        );
        app.restore_setup_step().unwrap();
        assert_eq!(inits_to(&mut rx).len(), 5);
        assert!(
            app.store
                .setting(keys::SAFE_RESTORE_SETUP)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn with_forward_secrecy_off_a_restore_starts_no_sessions() {
        let (_dir, app, mut rx) = restored_app();
        app.store.set_setting(keys::FORWARD_SECURITY, "0").unwrap();
        app.restore_setup_step().unwrap();
        assert!(inits_to(&mut rx).is_empty());
        assert!(app.store.fs_peers().unwrap().is_empty());
        assert!(
            app.store
                .setting(keys::SAFE_RESTORE_SETUP)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_setup_steps_wait_for_the_import_and_the_connection() {
        let (_dir, app, mut rx) = restored_app();
        app.store
            .set_setting(
                keys::SAFE_PENDING_IMPORT,
                r#"{"contacts":[{"identity":"ABCD1234","verification":0}]}"#,
            )
            .unwrap();
        app.run_restore_setup().await;
        assert!(inits_to(&mut rx).is_empty(), "the import comes first");
        assert_eq!(
            app.store
                .setting(keys::SAFE_RESTORE_SETUP)
                .unwrap()
                .as_deref(),
            Some(RESTORE_SETUP_START)
        );
        app.store.delete_setting(keys::SAFE_PENDING_IMPORT).unwrap();
        app.connection_state.set("connecting");
        app.run_restore_setup().await;
        assert!(inits_to(&mut rx).is_empty(), "the connection too");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_id_search_is_for_a_device_without_an_id_and_needs_something_to_look_for() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        assert!(matches!(
            app.safe_search_id("+41791234567", "").await,
            Err(ApiError::IdentityExists)
        ));

        let dir = tempfile::tempdir().unwrap();
        let app = app_without_identity(dir.path());
        assert!(matches!(
            app.safe_search_id("+41791234567", "").await,
            Err(ApiError::NoLicense)
        ));
        app.store
            .set_setting(keys::LICENSE_KEY, "XXXXX-XXXXX")
            .unwrap();
        for (phone, email) in [("", ""), ("12", "not an address")] {
            assert!(matches!(
                app.safe_search_id(phone, email).await,
                Err(ApiError::IdSearchInput)
            ));
        }
    }

    #[test]
    fn an_id_search_sends_hashes_only() {
        let (emails, phones) =
            crate::address_book::search_hashes("+41 79 123 45 67", " Someone@Example.org ", "");
        assert_eq!(
            emails,
            crate::address_book::email_hashes("someone@example.org")
        );
        assert_eq!(phones, [crate::address_book::phone_hash("41791234567")]);
        for hash in emails.iter().chain(&phones) {
            assert!(
                !hash.contains("example") && !hash.contains("4179"),
                "{hash}"
            );
        }
        // A number of its own country without one: by the region's rules.
        let (_, phones) = crate::address_book::search_hashes("079 123 45 67", "", "CH");
        assert_eq!(phones, [crate::address_book::phone_hash("41791234567")]);
        assert!(
            crate::address_book::search_hashes("079 123 45 67", "", "")
                .1
                .is_empty()
        );
    }

    #[test]
    fn a_new_id_is_asked_about_safe_until_it_answers() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        let asked = |app: &App| app.safe_status().unwrap()["askSetup"].clone();
        // Restored or older IDs aren't asked.
        assert_eq!(asked(&app), false);
        app.note_new_identity().unwrap();
        assert_eq!(asked(&app), true);
        app.safe_decline_setup().unwrap();
        assert_eq!(asked(&app), false);

        // Turned on some other way: no more question either.
        app.note_new_identity().unwrap();
        app.store
            .set_setting(keys::SAFE_MASTER_KEY, &"00".repeat(64))
            .unwrap();
        assert_eq!(asked(&app), false);
    }

    #[test]
    fn a_new_id_without_a_backup_is_reminded_now_and_then() {
        const DAY: i64 = 24 * 60 * 60;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path());
        // Restored or older IDs: never.
        assert!(!app.backup_reminder_due(now() + 1000 * DAY).unwrap());

        app.note_new_identity().unwrap();
        assert!(!app.backup_reminder_due(now() + 29 * DAY).unwrap());
        let first = now() + 31 * DAY;
        assert!(app.backup_reminder_due(first).unwrap());
        assert!(!app.backup_reminder_due(first + 89 * DAY).unwrap());
        assert!(app.backup_reminder_due(first + 90 * DAY).unwrap());

        // Choosing to go on without a backup ends it, and the window hears.
        app.take_ui_events();
        assert_eq!(app.safe_status().unwrap()["backupReminder"], true);
        app.safe_decline_setup().unwrap();
        assert_eq!(app.safe_status().unwrap()["backupReminder"], false);
        assert_eq!(app.take_ui_events(), ["safeChanged"]);
        assert!(!app.backup_reminder_due(first + 1000 * DAY).unwrap());

        // Nothing left to end: nothing to hear.
        app.end_backup_reminder().unwrap();
        assert!(app.take_ui_events().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_exported_id_backup_ends_the_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path());
        app.note_new_identity().unwrap();
        // Refused: nothing was made.
        assert!(
            app.export_identity_backup(Zeroizing::new("short".into()))
                .await
                .is_err()
        );
        assert!(
            app.store
                .setting(keys::BACKUP_REMINDER_AT)
                .unwrap()
                .is_some()
        );
        app.export_identity_backup(Zeroizing::new("long enough here".into()))
            .await
            .unwrap();
        assert_eq!(app.store.setting(keys::BACKUP_REMINDER_AT).unwrap(), None);
        assert_eq!(app.take_ui_events(), ["safeChanged"]);
    }

    #[test]
    fn a_failing_safe_warns_after_a_week_with_the_days_since_the_last_backup() {
        const DAY: i64 = 24 * 60 * 60;
        let now = 100 * DAY;
        assert_eq!(safe_warning_days(None, Some(now - 50 * DAY), now), None);
        // Not before the week is over, however old the last backup is.
        assert_eq!(
            safe_warning_days(Some(now - 7 * DAY + 1), Some(now - 50 * DAY), now),
            None
        );
        assert_eq!(
            safe_warning_days(Some(now - 7 * DAY), Some(now - 50 * DAY), now),
            Some(50)
        );
        // No backup ever: since the first failure.
        assert_eq!(safe_warning_days(Some(now - 9 * DAY), None, now), Some(9));
    }

    #[test]
    fn only_the_first_failure_starts_the_week_and_success_ends_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        let first = |app: &App| app.store.setting(keys::SAFE_FIRST_FAILURE).unwrap();
        let failure = ApiError::Network("no route".into());
        app.note_safe_upload(Some(&failure)).unwrap();
        assert!(first(&app).is_some());
        app.store
            .set_setting(keys::SAFE_FIRST_FAILURE, "1000")
            .unwrap();
        app.note_safe_upload(Some(&failure)).unwrap();
        assert_eq!(first(&app).as_deref(), Some("1000"));
        assert_eq!(
            app.store.setting(keys::SAFE_LAST_ERROR).unwrap().as_deref(),
            Some("network")
        );

        // Safe off: nothing to warn of, whatever was noted.
        assert_eq!(app.safe_status().unwrap()["warningDays"], json!(null));
        app.store
            .set_setting(keys::SAFE_MASTER_KEY, &"00".repeat(64))
            .unwrap();
        assert!(app.safe_status().unwrap()["warningDays"].as_i64().unwrap() > 7);

        app.note_safe_upload(None).unwrap();
        assert_eq!(first(&app), None);
        assert_eq!(app.store.setting(keys::SAFE_LAST_ERROR).unwrap(), None);
        assert_eq!(app.safe_status().unwrap()["warningDays"], json!(null));
    }

    #[test]
    fn turning_safe_off_says_whether_the_server_copy_went() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path());
        let enable = |app: &App| {
            app.store
                .set_setting(keys::SAFE_MASTER_KEY, &HEXLOWER.encode(&[1; 64]))
                .unwrap();
            app.store.set_setting(keys::SAFE_LAST_UPLOAD, "1").unwrap();
        };
        enable(&app);
        assert_eq!(app.safe_status().unwrap()["enabled"], true);
        let status = app.forget_safe(Ok(())).unwrap();
        assert_eq!(status["enabled"], false);
        assert_eq!(status["serverCopyDeleted"], true);
        assert_eq!(app.take_ui_events(), ["safeChanged"]);

        // A failed deletion turns it off all the same, and both the reply and an event say so.
        enable(&app);
        let status = app
            .forget_safe(Err(ApiError::Network("no route".into())))
            .unwrap();
        assert_eq!(status["enabled"], false);
        assert_eq!(status["serverCopyDeleted"], false);
        assert_eq!(app.take_ui_events(), ["safeServerCopyLeft", "safeChanged"]);
        assert!(app.store.setting(keys::SAFE_MASTER_KEY).unwrap().is_none());
        assert_eq!(app.safe_status().unwrap().get("serverCopyDeleted"), None);
    }

    #[test]
    fn safe_ids_are_little_endian_hex() {
        assert_eq!(safe_id(0x0102_0304_0506_0708), "0807060504030201");
        assert_eq!(
            parse_safe_id("0807060504030201"),
            Some(0x0102_0304_0506_0708)
        );
        assert_eq!(
            parse_safe_id("0807060504030201".to_uppercase().as_str()),
            Some(0x0102_0304_0506_0708)
        );
        assert_eq!(parse_safe_id("08070605"), None);
    }

    #[test]
    fn group_restore_rules() {
        let me = "AAAAAAAA";
        let known = |id: &str| id != "UNKNOWN1";
        let entry = |creator: &str, members: &[&str]| backup::Group {
            id: "0807060504030201".into(),
            creator: creator.into(),
            groupname: Some("Team".into()),
            created_at: Some(-5),
            members: members.iter().map(|m| (*m).to_owned()).collect(),
            deleted: false,
            last_update: None,
            is_private: false,
        };
        // A member: the creator stays in our members, we don't; unknown people are dropped.
        let group = group_from_backup(&entry("BBBBBBBB", &[me, "BBBBBBBB", "UNKNOWN1"]), me, known)
            .unwrap();
        assert_eq!(group.key, "g:BBBBBBBB:0102030405060708");
        assert_eq!(group.members, vec!["BBBBBBBB".to_owned()]);
        assert!(!group.left);
        assert_eq!((group.name.as_str(), group.created_at), ("Team", 0));
        // Not listed: left. Listed but the creator isn't: treated as kicked (left).
        assert!(
            group_from_backup(&entry("BBBBBBBB", &["BBBBBBBB"]), me, known)
                .unwrap()
                .left
        );
        assert!(
            group_from_backup(&entry("BBBBBBBB", &[me, "CCCCCCCC"]), me, known)
                .unwrap()
                .left
        );
        // Our own group.
        let own = group_from_backup(&entry(me, &[me, "CCCCCCCC"]), me, known).unwrap();
        assert_eq!(
            (own.members, own.left),
            (vec!["CCCCCCCC".to_owned()], false)
        );
        // Skipped: legacy deleted flag, unknown creator, invalid id — each with its reason.
        let mut deleted = entry("BBBBBBBB", &[me, "BBBBBBBB"]);
        deleted.deleted = true;
        assert_eq!(group_from_backup(&deleted, me, known), Err(SKIP_DELETED));
        assert_eq!(
            group_from_backup(&entry("UNKNOWN1", &[me]), me, known),
            Err(SKIP_UNKNOWN_CREATOR)
        );
        let mut invalid = entry("BBBBBBBB", &[me, "BBBBBBBB"]);
        invalid.id = "nonsense".into();
        assert_eq!(group_from_backup(&invalid, me, known), Err(SKIP_INVALID));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn groups_and_lists_survive_a_safe_round_trip() {
        let new_app = |dir: &std::path::Path| {
            let app = App::new(
                &StartConfig {
                    data_dir: dir.to_owned(),
                    app_version: "0.1.0".into(),
                    language: "de".into(),
                    country: "CH".into(),
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
            app.store
                .set_identity(&StoredIdentity {
                    threema_id: "AAAAAAAA".into(),
                    client_key: Zeroizing::new([5; 32]),
                    server_group: "a1".into(),
                })
                .unwrap();
            for id in ["BBBBBBBB", "CCCCCCCC"] {
                app.store
                    .upsert_contact(&crate::contacts::test_contact(id))
                    .unwrap();
            }
            app
        };
        let group = |creator: &str, id: u64, members: &[&str], left: bool| {
            let identity = libthreema::common::GroupIdentity {
                creator_identity: libthreema::common::ThreemaId::try_from(creator).unwrap(),
                group_id: id,
            };
            crate::groups::Group {
                key: crate::groups::key(&identity),
                identity,
                name: format!("Group {id}"),
                members: members.iter().map(|m| (*m).to_owned()).collect(),
                left,
                created_at: 1000 + i64::try_from(id).unwrap(),
                picture: None,
                picture_version: 0,
            }
        };
        let groups = [
            group("AAAAAAAA", 1, &["BBBBBBBB", "CCCCCCCC"], false),
            group("BBBBBBBB", 2, &["BBBBBBBB", "CCCCCCCC"], false),
            group("CCCCCCCC", 3, &["CCCCCCCC"], true),
        ];
        let list = crate::distribution_lists::DistributionList {
            key: "d:00000000000000ff".into(),
            name: "List".into(),
            members: vec!["BBBBBBBB".into()],
            created_at: 7,
        };

        let old_dir = tempfile::tempdir().unwrap();
        let old = new_app(old_dir.path());
        // Our picture: the Safe copy goes into the backup.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
        old.media
            .write_picture(crate::profile_pictures::OWN_PICTURE, &[0xff, 0xd8, 9])
            .unwrap();
        old.media
            .write_picture(crate::profile_pictures::OWN_PICTURE_SAFE, &jpeg)
            .unwrap();
        old.store
            .set_setting(keys::PROFILE_PICTURE_VERSION, "1")
            .unwrap();
        for g in &groups {
            old.store.upsert_group(g).unwrap();
        }
        old.store.upsert_distribution_list(&list).unwrap();
        old.store.set_private(&groups[1].key, true).unwrap();
        old.store.set_private(&list.key, true).unwrap();
        let backup = old.build_backup().unwrap();
        let backed_up = backup.groups.clone().unwrap();
        assert_eq!(backed_up.len(), 3);
        let first = backed_up.iter().find(|g| g.creator == "AAAAAAAA").unwrap();
        assert_eq!(first.id, "0100000000000000");
        assert!(
            first.members.contains(&"AAAAAAAA".to_owned()),
            "members include us"
        );
        let left = backed_up.iter().find(|g| g.creator == "CCCCCCCC").unwrap();
        assert!(!left.members.contains(&"AAAAAAAA".to_owned()));
        assert_eq!(
            backup.distributionlists.as_ref().unwrap()[0].id.as_deref(),
            Some("ff00000000000000")
        );

        // A restore on a new device: pending until imported, carried into backups meanwhile.
        // One more entry with the legacy deleted flag: skipped, but counted in the summary.
        let mut pending_groups = backup.groups.clone().unwrap();
        pending_groups.push(backup::Group {
            id: "0404040404040404".into(),
            creator: "BBBBBBBB".into(),
            groupname: None,
            created_at: None,
            members: Vec::new(),
            deleted: true,
            last_update: None,
            is_private: false,
        });
        let new_dir = tempfile::tempdir().unwrap();
        let mut new = new_app(new_dir.path());
        new.store
            .set_setting(
                keys::SAFE_PENDING_IMPORT,
                &json!({
                    "contacts": [],
                    "groups": pending_groups,
                    "distributionlists": backup.distributionlists,
                })
                .to_string(),
            )
            .unwrap();
        let status = new.safe_status().unwrap();
        assert_eq!(status["restore"]["pendingGroups"], 4);
        assert_eq!(status["restore"]["pendingLists"], 1);
        assert_eq!(new.build_backup().unwrap().groups.unwrap().len(), 4);
        let mut events = new.take_media_events().unwrap();
        assert!(new.import_pending_groups().await.unwrap());
        // The restored private marks reach the media gate, notifications and browsers too.
        let mut marked = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let crate::providers::CoreEvent::PrivateChanged {
                conversation,
                private: true,
            } = event
            {
                marked.push(conversation);
            }
        }
        assert_eq!(marked, [groups[1].key.clone(), list.key.clone()]);
        // The status now reports the counts and that nothing waits any more.
        let status = new.safe_status().unwrap();
        assert_eq!(status["restore"]["pendingGroups"], 0);
        assert_eq!(status["restore"]["summary"]["groups"], 3);
        assert_eq!(status["restore"]["summary"]["lists"], 1);
        assert_eq!(status["restore"]["summary"]["skippedGroups"]["deleted"], 1);
        assert_eq!(status["restore"]["importError"], Value::Null);
        for g in &groups {
            let restored = new.store.group(&g.key).unwrap().unwrap();
            assert_eq!(
                (
                    &restored.name,
                    &restored.members,
                    restored.left,
                    restored.created_at
                ),
                (&g.name, &g.members, g.left, g.created_at)
            );
        }
        assert!(new.store.is_private(&groups[1].key).unwrap());
        assert!(!new.store.is_private(&groups[0].key).unwrap());
        assert!(new.store.is_private(&list.key).unwrap());
        let restored = new.store.distribution_list(&list.key).unwrap().unwrap();
        assert_eq!((restored.name, restored.members), (list.name, list.members));
        assert!(!new.import_pending_groups().await.unwrap(), "nothing left");

        assert_eq!(backup.user.profile_pic, Some(BASE64.encode(&jpeg)));
        assert!(new.restore_profile_picture(&backup.user));
        assert_eq!(
            new.media
                .read_picture(crate::profile_pictures::OWN_PICTURE)
                .unwrap()
                .as_deref()
                .map(Vec::as_slice),
            Some(&jpeg[..])
        );
        // Not a JPEG: ignored.
        let mut user = backup.user.clone();
        user.profile_pic = Some(BASE64.encode(b"GIF89a"));
        assert!(!new.restore_profile_picture(&user));
        assert_eq!(new.build_backup().unwrap().groups.unwrap().len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_groups_last_update_survives_a_restore() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_identity(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let entry = backup::Group {
            id: "0807060504030201".into(),
            creator: "BBBBBBBB".into(),
            groupname: Some("Team".into()),
            created_at: Some(1000),
            members: vec!["AAAAAAAA".into(), "BBBBBBBB".into()],
            deleted: false,
            last_update: Some(4000),
            is_private: false,
        };
        let list = backup::DistributionList {
            id: Some("ff00000000000000".into()),
            name: "List".into(),
            created_at: Some(1000),
            members: vec!["BBBBBBBB".into()],
            last_update: Some(5000),
            is_private: false,
        };
        app.store
            .set_setting(
                keys::SAFE_PENDING_IMPORT,
                &json!({ "groups": [entry], "distributionlists": [list] }).to_string(),
            )
            .unwrap();
        assert!(app.import_pending_groups().await.unwrap());
        let backup = app.build_backup().unwrap();
        assert_eq!(backup.groups.unwrap()[0].last_update, Some(4000));
        assert_eq!(backup.distributionlists.unwrap()[0].last_update, Some(5000));
    }

    /// The backup on the server of the test ID (`THREESEA_TEST_SAFE_ID`,
    /// `THREESEA_TEST_SAFE_PASSWORD`): readable, and what it holds.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "downloads a real Threema Safe backup"]
    async fn live_safe_backup_parses() {
        let (Ok(id), Ok(password)) = (
            std::env::var("THREESEA_TEST_SAFE_ID"),
            std::env::var("THREESEA_TEST_SAFE_PASSWORD"),
        ) else {
            panic!("set THREESEA_TEST_SAFE_ID and THREESEA_TEST_SAFE_PASSWORD");
        };
        let key = derive(id, Zeroizing::new(password)).await.unwrap();
        let client = crate::https::HttpsClient::new().unwrap();
        let blob = server::download(&client, &key).await.unwrap();
        let backup = safe::parse_backup(&key.decrypt(&blob).unwrap()).unwrap();
        for group in backup.groups.unwrap_or_default() {
            eprintln!(
                "group {} by {}: {} members",
                group.id,
                group.creator,
                group.members.len()
            );
        }
        for list in backup.distributionlists.unwrap_or_default() {
            eprintln!("list {:?}: {} members", list.id, list.members.len());
        }
        eprintln!("profilePicRelease {:?}", backup.user.profile_pic_release);
        if let Some(jpeg) = backup
            .user
            .profile_pic
            .and_then(|b64| BASE64.decode(b64.as_bytes()).ok())
        {
            // Size from the first start-of-frame marker (baseline or progressive).
            let size = jpeg
                .windows(9)
                .find(|w| w[0] == 0xff && (w[1] == 0xc0 || w[1] == 0xc2))
                .map(|w| {
                    (
                        u16::from_be_bytes([w[7], w[8]]),
                        u16::from_be_bytes([w[5], w[6]]),
                    )
                });
            eprintln!("profilePic {} bytes, {size:?} (width, height)", jpeg.len());
        }
    }
}
