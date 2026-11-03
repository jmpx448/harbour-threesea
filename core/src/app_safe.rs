// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Safe commands and the automatic backup, on top of [`crate::safe`].
//! Behaviour follows upstream `ThreemaSafeServiceImpl.java`; see docs/protocol-notes.md.

use data_encoding::{BASE64, HEXLOWER};
use libthreema::common::keys::ClientKey;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App},
    safe::{
        self, MasterKey, PasswordCheck, SafeError,
        backup::{self, Backup},
        server::{self, ServerError},
    },
    store::keys,
};

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        match error {
            ServerError::Network(e) => Self::Network(e.to_string()),
            ServerError::NotFound => Self::SafeNotFound,
            ServerError::TooLarge { .. } => Self::SafeTooLarge(error.to_string()),
            other => Self::Server(other.to_string()),
        }
    }
}

impl From<SafeError> for ApiError {
    fn from(error: SafeError) -> Self {
        match error {
            SafeError::Decrypt => Self::BackupDecrypt,
            other => Self::Internal(other.to_string()),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// How many missing contact pictures one recovery round (connect, maintenance) may request.
const PICTURE_REQUEST_BATCH: usize = 20;

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
            "server": key.as_ref().map(server::base_url),
            "lastBackup": self.setting_i64(keys::SAFE_LAST_UPLOAD)?,
            "lastError": self.store.setting(keys::SAFE_LAST_ERROR)?,
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

    pub(crate) async fn safe_enable(
        &mut self,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
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
            keys::SAFE_SIZE,
        ] {
            self.store.delete_setting(k)?;
        }
        self.safe_upload(true).await?;
        self.safe_status()
    }

    pub(crate) async fn safe_backup_now(&mut self) -> Result<Value, ApiError> {
        if self.safe_key()?.is_none() {
            return Err(ApiError::Internal("Threema Safe is disabled".into()));
        }
        self.safe_upload(true).await?;
        self.safe_status()
    }

    pub(crate) fn safe_disable(&mut self) -> Result<Value, ApiError> {
        for k in [
            keys::SAFE_MASTER_KEY,
            keys::SAFE_LAST_UPLOAD,
            keys::SAFE_HASH,
            keys::SAFE_LAST_ERROR,
            keys::SAFE_SIZE,
            keys::SAFE_MAX_BYTES,
            keys::SAFE_RETENTION_DAYS,
        ] {
            self.store.delete_setting(k)?;
        }
        self.safe_status()
    }

    pub(crate) async fn safe_restore(
        &mut self,
        identity: &str,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
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
                .map_err(|_| ApiError::Internal("backup private key invalid".into()))?,
        );
        let client_key: [u8; 32] = client_key
            .as_slice()
            .try_into()
            .map_err(|_| ApiError::Internal("backup private key invalid".into()))?;
        let client_key = ClientKey::from(client_key);
        self.adopt_identity(&identity, &client_key).await?;

        match backup.user.nickname.as_deref().filter(|n| !n.is_empty()) {
            Some(nickname) => self.store.set_setting(keys::NICKNAME, nickname)?,
            None => self.store.delete_setting(keys::NICKNAME)?,
        }
        self.restore_profile_picture(&backup.user)?;
        if let Some(release) = &backup.user.profile_pic_release {
            let (share, recipients) = crate::profile_pictures::from_safe_release(release);
            self.store.set_picture_share(share)?;
            self.store.set_picture_recipients(&recipients)?;
        }
        self.save_app_settings(&backup.settings)?;
        // Contacts, then groups and lists, are imported below (lookups needed); until then they
        // wait here and are carried into our own backups.
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
        if let Ok(config) = server::config(&self.https, &key).await {
            self.store_safe_limits(config)?;
        }
        // A fresh restore starts a fresh import record.
        self.store.delete_setting(keys::SAFE_IMPORT_SUMMARY)?;
        self.store.delete_setting(keys::SAFE_IMPORT_ERROR)?;
        // Best effort now; failures are recorded and retried on reconnect, by maintenance and
        // on request.
        let events = self.try_pending_import().await;
        self.note_ui_events(&events);
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

    fn store_safe_limits(&self, config: server::ServerConfig) -> Result<(), ApiError> {
        self.store
            .set_setting(keys::SAFE_MAX_BYTES, &config.max_backup_bytes.to_string())?;
        self.store.set_setting(
            keys::SAFE_RETENTION_DAYS,
            &config.retention_days.to_string(),
        )?;
        Ok(())
    }

    /// Stored contacts plus restored ones that are not imported yet (never drop any). The own ID
    /// is excluded, like on Android.
    fn backup_contacts(&self) -> Result<Vec<backup::Contact>, ApiError> {
        let mut list: Vec<backup::Contact> = self
            .store
            .contacts()?
            .iter()
            .map(|contact| {
                let mut entry = crate::contacts::to_backup(contact);
                entry.is_private |= self.store.is_private(&entry.identity).unwrap_or(false);
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

    /// Our picture from a backup (Android `restoreUser`): stored as the picture and its Safe copy;
    /// if it can't be decoded, forget about it.
    fn restore_profile_picture(&self, user: &backup::User) -> Result<(), ApiError> {
        if let Some(jpeg) = user
            .profile_pic
            .as_deref()
            .and_then(|b64| BASE64.decode(b64.as_bytes()).ok())
            .filter(|jpeg| jpeg.starts_with(&[0xff, 0xd8]))
        {
            let stored = [
                crate::profile_pictures::OWN_PICTURE,
                crate::profile_pictures::OWN_PICTURE_SAFE,
            ]
            .iter()
            .all(|name| self.media.write_picture(name, &jpeg).is_ok());
            if stored {
                self.store.bump_profile_picture_version()?;
            }
        }
        Ok(())
    }

    /// Our groups, and those of a restored backup not imported yet (so a failed import loses
    /// nothing).
    fn backup_groups(&self, me: &str, pending: &Value) -> Result<Vec<backup::Group>, ApiError> {
        let mut list: Vec<backup::Group> = self
            .store
            .groups()?
            .iter()
            .map(|group| {
                let mut entry = group_to_backup(group, me);
                entry.is_private = self.store.is_private(&group.key).unwrap_or(false);
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

    fn backup_lists(&self, pending: &Value) -> Result<Vec<backup::DistributionList>, ApiError> {
        let mut list: Vec<backup::DistributionList> = self
            .store
            .distribution_lists()?
            .iter()
            .map(|list| {
                let mut entry = list_to_backup(list);
                entry.is_private = self.store.is_private(&list.key).unwrap_or(false);
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
    pub(crate) async fn import_pending_groups(&mut self) -> Result<bool, ApiError> {
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
            for (id, result) in crate::contacts::lookup(&self.https, &ids).await? {
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
                if self.store.contact(&contact.identity.to_string())?.is_none() {
                    self.store.upsert_contact(&contact)?;
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
                    self.store.set_private(&group.key, true)?;
                }
                imported_groups += 1;
            }
        }
        for entry in &lists {
            let list = list_from_backup(entry, |id| known(&self.store, id));
            if self.store.distribution_list(&list.key)?.is_none() {
                self.store.upsert_distribution_list(&list)?;
                if entry.is_private {
                    self.store.set_private(&list.key, true)?;
                }
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
    pub(crate) async fn try_pending_import(&mut self) -> Vec<&'static str> {
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

    /// The connection is up: try a pending Safe import right away — `maintenance` alone would
    /// leave restored groups waiting up to an hour after a network failure — and pick up
    /// picture recovery (kept downloads, missing-picture requests in a bounded batch).
    /// Rate-limited so flapping connections don't hammer the servers. Returns the UI events to
    /// emit.
    pub async fn on_connected(&mut self) -> Vec<&'static str> {
        let now = tokio::time::Instant::now();
        if self
            .last_import_attempt
            .is_some_and(|at| now.duration_since(at) < std::time::Duration::from_secs(60))
        {
            return Vec::new();
        }
        self.last_import_attempt = Some(now);
        let mut events = Vec::new();
        let pending = self
            .pending_import_counts()
            .map(|(contacts, groups, lists)| contacts + groups + lists)
            .unwrap_or(0);
        if pending > 0 {
            events = self.try_pending_import().await;
        }
        self.resume_picture_downloads();
        self.request_missing_pictures(PICTURE_REQUEST_BATCH);
        self.retry_failed_uploads();
        events
    }

    /// The backup as it would be uploaded now.
    fn build_backup(&self) -> Result<Backup, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let pending: Value = self
            .store
            .setting(keys::SAFE_PENDING_IMPORT)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let mut links = Vec::new();
        if let Some(mobile) = self.store.setting(keys::LINKED_MOBILE)? {
            links.push(backup::Link {
                kind: "mobile".into(),
                value: mobile.trim_start_matches('+').to_owned(),
                name: None,
            });
        }
        if let Some(email) = self.store.setting(keys::LINKED_EMAIL)? {
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
            contacts: Some(self.backup_contacts()?),
            groups: Some(self.backup_groups(&identity.threema_id, &pending)?),
            distributionlists: Some(self.backup_lists(&pending)?),
            settings: self.app_settings()?,
        })
    }

    /// Upload if the policy says so (or `force`). Returns whether an upload happened. Failures
    /// are recorded in `SAFE_LAST_ERROR` and returned.
    async fn safe_upload(&mut self, force: bool) -> Result<bool, ApiError> {
        let Some(key) = self.safe_key()? else {
            return Ok(false);
        };
        let result = self.safe_upload_with(&key, force).await;
        match &result {
            Ok(_) => self.store.delete_setting(keys::SAFE_LAST_ERROR)?,
            Err(error) => {
                tracing::warn!(kind = error.kind(), "Threema Safe backup failed");
                self.store
                    .set_setting(keys::SAFE_LAST_ERROR, error.kind())?;
            }
        }
        result
    }

    async fn safe_upload_with(&mut self, key: &MasterKey, force: bool) -> Result<bool, ApiError> {
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

    /// Periodic work on the core thread (called at start and then hourly): pending Safe contact
    /// import and the automatic Safe backup. Returns the events to emit to the UI.
    pub async fn maintenance(&mut self) -> Vec<&'static str> {
        let mut events = Vec::new();
        match self.store.identity() {
            Ok(Some(_)) => {}
            _ => return events,
        }
        // Refused, and nothing goes to Threema's servers until a key is accepted again.
        if !self.license_valid().unwrap_or(false) {
            return events;
        }
        match self.recheck_license().await {
            Ok(false) => {}
            Ok(true) => {
                events.push("licenseChanged");
                return events;
            }
            Err(error) => tracing::warn!(kind = error.kind(), "license check failed"),
        }
        events.extend(self.try_pending_import().await);
        self.resume_picture_downloads();
        self.request_missing_pictures(PICTURE_REQUEST_BATCH);
        if self.check_email_link().await.unwrap_or(false) {
            events.push("profileChanged");
        }
        if self.refresh_contacts(false).await.unwrap_or(false) {
            events.push("contactsChanged");
        }
        if let Err(error) = self.publish_feature_mask().await {
            tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
        }
        self.android_export_scheduled();
        if self.auto_delete_scheduled() {
            events.push("chatsChanged");
        }
        let before = self.store.setting(keys::SAFE_LAST_ERROR).ok().flatten();
        let uploaded = self.safe_upload(false).await.unwrap_or(false);
        let after = self.store.setting(keys::SAFE_LAST_ERROR).ok().flatten();
        if uploaded || before != after {
            events.push("safeChanged");
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity};

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
        assert!(new.import_pending_groups().await.unwrap());
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
        new.restore_profile_picture(&backup.user).unwrap();
        assert_eq!(
            new.media
                .read_picture(crate::profile_pictures::OWN_PICTURE)
                .unwrap()
                .as_deref()
                .map(Vec::as_slice),
            Some(&jpeg[..])
        );
        assert!(
            new.store
                .setting(keys::PROFILE_PICTURE_VERSION)
                .unwrap()
                .is_some()
        );
        // Not a JPEG: ignored.
        let other_dir = tempfile::tempdir().unwrap();
        let other = new_app(other_dir.path());
        let mut user = backup.user.clone();
        user.profile_pic = Some(BASE64.encode(b"GIF89a"));
        other.restore_profile_picture(&user).unwrap();
        assert!(
            other
                .store
                .setting(keys::PROFILE_PICTURE_VERSION)
                .unwrap()
                .is_none()
        );
        assert_eq!(new.build_backup().unwrap().groups.unwrap().len(), 3);
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
