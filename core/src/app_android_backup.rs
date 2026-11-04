// SPDX-License-Identifier: AGPL-3.0-only
//! Commands around Android data backups (docs/android-backup-design.md):
//! for now the local inspection; the import itself follows in later steps.

use libthreema::common::keys::ClientKey;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{
    android_backup::{self, Archive, ArchiveError},
    app::{ApiError, App},
};

impl From<ArchiveError> for ApiError {
    fn from(error: ArchiveError) -> Self {
        match error {
            ArchiveError::WrongPassword => Self::BackupDecrypt,
            ArchiveError::UnsupportedVersion(version) => Self::BackupVersion(version),
            ArchiveError::Io(e) => Self::BackupInvalid(e.to_string()),
            ArchiveError::Corrupt(e) => Self::BackupInvalid(e),
            ArchiveError::Invalid(e) => Self::BackupInvalid(e),
            ArchiveError::TooLarge => Self::BackupInvalid("entry too large".into()),
        }
    }
}

impl App {
    /// `androidBackupInspect {path, password}`: open the archive, check the format version
    /// and whose identity it belongs to, and count what it holds. Entirely local; nothing is
    /// imported, sent or changed. The sizes are the archive's declarations — estimates until
    /// each entry is read and authenticated by an import. Runs on a blocking thread (ZIP I/O
    /// and the ID backup's key derivation).
    pub(crate) async fn android_backup_inspect(
        &mut self,
        path: String,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let own_id = identity.threema_id.clone();
        let own_public_key = ClientKey::from(*identity.client_key)
            .public_key()
            .to_string();
        tokio::task::spawn_blocking(move || -> Result<Value, ApiError> {
            let mut archive = Archive::open(std::path::Path::new(&path), &password)?;
            let version = android_backup::read_version(&mut archive)?;
            let inventory = android_backup::inventory(&archive);
            // The identity entry is an encrypted ID backup (same password). Only a backup of
            // the running identity (same ID, same key) can ever be merged here.
            let identity_state = if !archive.has_entry("identity") {
                "missing"
            } else {
                match android_backup::read_identity_backup(&mut archive)
                    .map_err(ApiError::from)
                    .and_then(|encrypted| {
                        libthreema::id_backup::decrypt_identity_backup(&password, &encrypted)
                            .map_err(|_| ApiError::BackupDecrypt)
                    }) {
                    Ok((_, data)) if data.threema_id.to_string() != own_id => "otherIdentity",
                    Ok((_, data)) if data.client_key.public_key().to_string() != own_public_key => {
                        "keyMismatch"
                    }
                    Ok(_) => "match",
                    Err(ApiError::BackupDecrypt) => "unreadable",
                    Err(error) => return Err(error),
                }
            };
            // Fixed category strings (match/otherIdentity/…), never an ID or a name.
            tracing::info!(
                version,
                id_check = identity_state,
                "Android backup inspected"
            );
            Ok(json!({
                "version": version,
                "identity": identity_state,
                "inventory": inventory,
            }))
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    }
}

/// Reading bounds for the metadata import (an archive beyond them reports instead of OOMing).
const MAX_TABLE_BYTES: usize = 64 << 20;
const MAX_PICTURE_BYTES: usize = 8 << 20;
const MAX_PICTURES_TOTAL: usize = 256 << 20;

/// What the metadata import lifts off the archive on the blocking thread, applied afterwards
/// on the core thread (which owns the store).
struct Metadata {
    contacts: Vec<crate::android_backup::records::Contact>,
    groups: Vec<crate::android_backup::records::Group>,
    lists: Vec<crate::android_backup::records::DistributionList>,
    invalid_rows: usize,
    own_picture: Option<Zeroizing<Vec<u8>>>,
    /// Contact-defined pictures (`contact_profile_pic_<alias>`), by alias.
    contact_pictures: Vec<(String, Zeroizing<Vec<u8>>)>,
    group_pictures: Vec<(String, Zeroizing<Vec<u8>>)>,
    /// `contact_avatar_<alias>` entries: the user's local overrides. Our core has one picture
    /// per contact (the contact-defined one), so these are counted, not imported.
    local_avatars: usize,
    pictures_skipped: usize,
    /// Used nonces, hashed (CSP, D2D), and cells that were neither a nonce nor a hash.
    csp_nonces: Vec<[u8; 32]>,
    d2d_nonces: Vec<[u8; 32]>,
    nonces_skipped: usize,
    /// The archive has no `nonces.csv`: old, or someone removed it (entry sets are not
    /// authenticated). Told to the user, the import goes on.
    nonces_missing: bool,
}

/// A picture entry must at least look like an image (Android writes JPEG or PNG).
fn looks_like_image(data: &[u8]) -> bool {
    data.starts_with(&[0xff, 0xd8]) || data.starts_with(&[0x89, b'P', b'N', b'G'])
}

fn load_metadata(
    path: &std::path::Path,
    password: &str,
    own_id: &str,
    own_public_key: &str,
) -> Result<Metadata, ApiError> {
    use crate::android_backup::{EntryKind, read_identity_backup, read_version, records};
    let mut archive = Archive::open(path, password)?;
    let _version = read_version(&mut archive)?;
    // Only a backup of the running identity (same ID, same key) is merged; everything else
    // is refused before any change.
    let encrypted = read_identity_backup(&mut archive)?;
    let (_, identity) = libthreema::id_backup::decrypt_identity_backup(password, &encrypted)
        .map_err(|_| ApiError::BackupDecrypt)?;
    if identity.threema_id.to_string() != own_id
        || identity.client_key.public_key().to_string() != own_public_key
    {
        return Err(ApiError::BackupOtherIdentity);
    }

    let mut invalid_rows = 0;
    let table = |archive: &mut Archive,
                 name: &str|
     -> Result<Option<crate::android_backup::csv::Table>, ApiError> {
        if !archive.has_entry(name) {
            return Ok(None);
        }
        Ok(Some(archive.read_table(name, MAX_TABLE_BYTES)?))
    };
    let (contacts, bad) = table(&mut archive, "contacts.csv")?
        .map(|t| records::contacts(&t))
        .unwrap_or_default();
    invalid_rows += bad;
    let (groups, bad) = table(&mut archive, "groups.csv")?
        .map(|t| records::groups(&t))
        .unwrap_or_default();
    invalid_rows += bad;
    let (lists, bad) = table(&mut archive, "distribution_list.csv")?
        .map(|t| records::distribution_lists(&t))
        .unwrap_or_default();
    invalid_rows += bad;

    // Used nonces: replay protection for what Android already received.
    let mut nonces_skipped = 0;
    let mut nonces = |archive: &mut Archive, name: &str| -> Result<Vec<[u8; 32]>, ApiError> {
        let Some(table) = table(archive, name)? else {
            return Ok(Vec::new());
        };
        let (hashed, skipped) = crate::android_backup::nonces::read(&table, own_id);
        nonces_skipped += skipped;
        Ok(hashed)
    };
    let nonces_missing = !archive.has_entry("nonces.csv");
    let csp_nonces = nonces(&mut archive, "nonces.csv")?;
    let d2d_nonces = nonces(&mut archive, "nonces_d2d.csv")?;

    // Pictures, each authenticated by reading it to its end, within bounds.
    let entries: Vec<(String, EntryKind)> = archive
        .entries()
        .iter()
        .map(|entry| (entry.name.clone(), entry.kind.clone()))
        .collect();
    let mut own_picture = None;
    let mut contact_pictures = Vec::new();
    let mut group_pictures = Vec::new();
    let mut local_avatars = 0;
    let mut pictures_skipped = 0;
    let mut total = 0_usize;
    for (name, kind) in entries {
        let alias = match kind {
            EntryKind::OwnAvatar => None,
            EntryKind::ContactProfilePicture(alias) | EntryKind::GroupAvatar(alias) => Some(alias),
            EntryKind::ContactAvatar(_) => {
                local_avatars += 1;
                continue;
            }
            _ => continue,
        };
        if total >= MAX_PICTURES_TOTAL {
            pictures_skipped += 1;
            continue;
        }
        let data = match archive.read(&name, MAX_PICTURE_BYTES) {
            Ok(data) if looks_like_image(&data) => data,
            Ok(_) | Err(ArchiveError::TooLarge) => {
                pictures_skipped += 1;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        total += data.len();
        match (&name, alias) {
            (_, None) => own_picture = Some(data),
            (name, Some(alias)) if name.starts_with("contact_profile_pic_") => {
                contact_pictures.push((alias, data));
            }
            (_, Some(alias)) => group_pictures.push((alias, data)),
        }
    }
    Ok(Metadata {
        contacts,
        groups,
        lists,
        invalid_rows,
        own_picture,
        contact_pictures,
        group_pictures,
        local_avatars,
        pictures_skipped,
        csp_nonces,
        d2d_nonces,
        nonces_skipped,
        nonces_missing,
    })
}

impl App {
    /// `androidBackupImport {path, password, scope: "metadata"}`: merge the archive's
    /// contacts, groups, distribution lists, pictures and used nonces (Android's hashes, so
    /// messages Android already received are refused as replays) into the running identity,
    /// entirely offline and without any sending side effect — dedicated store writes only, no message
    /// is queued, no lookup made, nothing distributed. Existing local data wins: a contact
    /// whose stored key differs is a counted conflict, pictures only fill empty slots, an
    /// existing group's membership and left state stay untouched. Running it again adds
    /// nothing. Message history is not part of this scope.
    pub(crate) async fn android_backup_import(
        &mut self,
        path: String,
        password: Zeroizing<String>,
        scope: &str,
    ) -> Result<Value, ApiError> {
        if scope == "history" {
            return self.android_backup_import_history(path, password);
        }
        if scope != "metadata" {
            return Err(ApiError::Internal(format!("unknown scope {scope}")));
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let own_id = identity.threema_id.clone();
        let own_public_key = ClientKey::from(*identity.client_key)
            .public_key()
            .to_string();
        let blocking_id = own_id.clone();
        let data = tokio::task::spawn_blocking(move || {
            load_metadata(
                std::path::Path::new(&path),
                &password,
                &blocking_id,
                &own_public_key,
            )
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;
        self.apply_android_metadata(&own_id, data)
    }

    /// `scope: "history"`: messages, media and reactions as a background job on the core
    /// thread (heavy ZIP work on a blocking thread), so commands and live messages keep
    /// flowing; progress and the final summary arrive as `androidBackup…` events. One job at
    /// a time; the chat and contact lists reload from the bulk events it ends with.
    fn android_backup_import_history(
        &mut self,
        path: String,
        password: Zeroizing<String>,
    ) -> Result<Value, ApiError> {
        if self
            .android_import
            .as_ref()
            .is_some_and(|cancel| !cancel.get())
        {
            return Err(ApiError::Internal("an import is already running".into()));
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let cancel = std::rc::Rc::new(std::cell::Cell::new(false));
        self.android_import = Some(std::rc::Rc::clone(&cancel));
        let job = crate::android_backup::import::HistoryImport {
            store: std::rc::Rc::clone(&self.store),
            media: std::sync::Arc::clone(&self.media),
            events: self.media_events.clone(),
            cancel,
            me: identity.threema_id.clone(),
            own_public_key: ClientKey::from(*identity.client_key)
                .public_key()
                .to_string(),
            path: std::path::PathBuf::from(path),
            password,
        };
        tokio::task::spawn_local(job.run());
        Ok(json!({ "started": true }))
    }

    #[allow(clippy::too_many_lines)]
    fn apply_android_metadata(&mut self, me: &str, data: Metadata) -> Result<Value, ApiError> {
        use libthreema::{
            common::FeatureMask,
            model::contact::Contact,
            protobuf::d2d_sync::{ConversationCategory, ConversationVisibility, contact as pc},
        };
        let mut alias_to_identity: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let (mut added, mut existing, mut updated, mut key_conflicts) = (0, 0, 0, 0);
        for c in &data.contacts {
            // Both naming schemes: identity_id (19+) and the identity itself (older).
            alias_to_identity.insert(
                c.alias.clone().unwrap_or_else(|| c.identity.clone()),
                c.identity.clone(),
            );
            alias_to_identity.insert(c.identity.clone(), c.identity.clone());
            if c.identity == me {
                continue;
            }
            match self.store.contact(&c.identity)? {
                Some(mut stored) => {
                    let stored_key = crate::contacts::hex_bytes(&stored.public_key.to_string());
                    if stored_key.as_slice() != c.public_key {
                        // Never overwrite a key: the archive may be old or wrong.
                        key_conflicts += 1;
                        continue;
                    }
                    // The backup knows them as a direct contact: show them in the contact
                    // list here too. Everything else local stays (it may be newer).
                    if stored.acquaintance_level == pc::AcquaintanceLevel::GroupOrDeleted
                        && !c.hidden
                    {
                        stored.acquaintance_level = pc::AcquaintanceLevel::Direct;
                        self.store.upsert_contact(&stored)?;
                        updated += 1;
                    } else {
                        existing += 1;
                    }
                }
                None => {
                    let mut contact = Contact {
                        identity: c
                            .identity
                            .parse()
                            .map_err(|_| ApiError::Internal("identity".into()))?,
                        public_key: libthreema::common::keys::PublicKey::from(c.public_key),
                        created_at: u64::try_from(crate::messages::now_ms()).unwrap_or(0),
                        first_name: c.first_name.clone(),
                        last_name: c.last_name.clone(),
                        nickname: c.nickname.clone(),
                        // The archive is the key's source, like on Android's own restore.
                        verification_level: match c.verification {
                            2 => pc::VerificationLevel::FullyVerified,
                            1 => pc::VerificationLevel::ServerVerified,
                            _ => pc::VerificationLevel::Unverified,
                        },
                        work_verification_level: pc::WorkVerificationLevel::None,
                        identity_type: pc::IdentityType::Regular,
                        acquaintance_level: if c.hidden {
                            pc::AcquaintanceLevel::GroupOrDeleted
                        } else {
                            pc::AcquaintanceLevel::Direct
                        },
                        activity_state: pc::ActivityState::Active,
                        // Unknown offline; the periodic contact refresh fills it in.
                        feature_mask: FeatureMask(0),
                        sync_state: pc::SyncState::Initial,
                        work_last_full_sync_at: None,
                        work_availability_status: None,
                        read_receipt_policy_override: None,
                        typing_indicator_policy_override: None,
                        notification_trigger_policy_override: None,
                        conversation_category: ConversationCategory::Default,
                        conversation_visibility: ConversationVisibility::Normal,
                    };
                    if !crate::contacts::pin_predefined(&self.config, &mut contact) {
                        key_conflicts += 1;
                        continue;
                    }
                    self.store.upsert_contact(&contact)?;
                    if c.archived {
                        self.store.set_archived(&c.identity, true)?;
                    }
                    added += 1;
                }
            }
        }
        let contacts_summary = json!({
            "added": added, "existing": existing, "updated": updated,
            "keyConflicts": key_conflicts,
        });

        let known = |store: &crate::store::Store, id: &str| {
            id == me || store.contact(id).ok().flatten().is_some()
        };
        let mut group_keys: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let (mut added, mut existing, mut skipped_deleted, mut skipped_creator) = (0, 0, 0, 0);
        for g in &data.groups {
            if g.deleted {
                skipped_deleted += 1;
                continue;
            }
            if g.creator != me && !known(&self.store, &g.creator) {
                skipped_creator += 1;
                continue;
            }
            let identity = libthreema::common::GroupIdentity {
                creator_identity: match g.creator.parse() {
                    Ok(creator) => creator,
                    Err(_) => {
                        skipped_creator += 1;
                        continue;
                    }
                },
                group_id: g.group_id,
            };
            let key = crate::groups::key(&identity);
            for alias in g.aliases() {
                group_keys.insert(alias, key.clone());
            }
            if self.store.group(&key)?.is_some() {
                // Local membership, name and left state may be newer: only the picture can
                // still be filled below.
                existing += 1;
                continue;
            }
            let members = known_members(&g.members, me, |id| known(&self.store, id));
            let user_listed = g.members.iter().any(|m| m == me);
            let mut left = match g.user_state {
                Some(state) => state != 0,
                // Older backups: derived from the member list, like Android.
                None => !user_listed,
            };
            // An orphaned group (creator gone) counts as kicked, like Android's restore.
            if !left && g.creator != me && !g.members.contains(&g.creator) {
                left = true;
            }
            self.store.upsert_group(&crate::groups::Group {
                key: key.clone(),
                identity,
                name: g.name.clone(),
                members,
                left,
                created_at: g.created_at,
                picture: None,
                picture_version: 0,
            })?;
            if g.archived {
                self.store.set_archived(&key, true)?;
            }
            added += 1;
        }
        let groups_summary = json!({
            "added": added, "existing": existing,
            "skippedDeleted": skipped_deleted, "skippedUnknownCreator": skipped_creator,
        });

        let (mut added, mut existing) = (0, 0);
        for l in &data.lists {
            let key = format!("d:{:016x}", l.id.cast_unsigned());
            if self.store.distribution_list(&key)?.is_some() {
                existing += 1;
                continue;
            }
            let members = known_members(&l.members, me, |id| known(&self.store, id));
            self.store
                .upsert_distribution_list(&crate::distribution_lists::DistributionList {
                    key: key.clone(),
                    name: l.name.clone(),
                    members,
                    created_at: l.created_at,
                })?;
            if l.archived {
                self.store.set_archived(&key, true)?;
            }
            added += 1;
        }
        let lists_summary = json!({ "added": added, "existing": existing });

        // Pictures fill empty slots only; a picture already on the device stays (it may be
        // newer). Nothing is uploaded or distributed by the import; our own picture follows
        // the normal distribution policy with the next messages.
        let mut events: Vec<&'static str> = Vec::new();
        let mut own_imported = false;
        if let Some(picture) = &data.own_picture
            && self
                .store
                .setting(crate::store::keys::PROFILE_PICTURE_VERSION)?
                .is_none()
            && self
                .media
                .write_picture(crate::profile_pictures::OWN_PICTURE, picture)
                .is_ok()
        {
            self.store.bump_profile_picture_version()?;
            own_imported = true;
            events.push("profileChanged");
        }
        let (mut contact_pictures, mut kept) = (0, 0);
        for (alias, picture) in &data.contact_pictures {
            let Some(identity) = alias_to_identity.get(alias) else {
                continue;
            };
            if self.store.contact(identity)?.is_none() {
                continue;
            }
            if self.store.contact_picture_version(identity)? != 0 {
                kept += 1;
                continue;
            }
            if self
                .media
                .write_picture(
                    &crate::profile_pictures::contact_picture_name(identity),
                    picture,
                )
                .is_ok()
            {
                self.store
                    .set_contact_picture(identity, Some(crate::messages::now_ms()))?;
                contact_pictures += 1;
            }
        }
        let mut group_pictures = 0;
        for (alias, picture) in &data.group_pictures {
            let Some(key) = group_keys.get(alias) else {
                continue;
            };
            let Some(mut group) = self.store.group(key)? else {
                continue;
            };
            if group.picture_version != 0 || group.picture.is_some() {
                kept += 1;
                continue;
            }
            if self.media.write_picture(key, picture).is_ok() {
                // Local bytes only: no blob descriptor is invented and nothing is sent.
                group.picture_version =
                    crate::profile_pictures::next_version(group.picture_version);
                self.store.upsert_group(&group)?;
                group_pictures += 1;
            }
        }
        // Nonces: hashes beside our raw nonces, checked on every receive (idempotent).
        let csp_nonces = self.store.add_hashed_nonces(false, &data.csp_nonces)?;
        let d2d_nonces = self.store.add_hashed_nonces(true, &data.d2d_nonces)?;

        events.push("contactsChanged");
        events.push("groupsChanged");
        self.note_ui_events(&events);
        // log-ok: counts
        tracing::info!(
            contacts = contacts_summary["added"].as_i64().unwrap_or(0),
            groups = groups_summary["added"].as_i64().unwrap_or(0),
            pictures = contact_pictures + group_pictures,
            "Android backup metadata imported"
        );
        Ok(json!({
            "contacts": contacts_summary,
            "groups": groups_summary,
            "lists": lists_summary,
            "pictures": {
                "own": own_imported,
                "contacts": contact_pictures,
                "groups": group_pictures,
                "kept": kept,
                "localAvatarsNotImported": data.local_avatars,
                "skipped": data.pictures_skipped,
            },
            "nonces": {
                "csp": csp_nonces,
                "d2d": d2d_nonces,
                "existing": (data.csp_nonces.len() + data.d2d_nonces.len())
                    .saturating_sub(csp_nonces + d2d_nonces),
                "skipped": data.nonces_skipped,
                "missing": data.nonces_missing,
            },
            "invalidRows": data.invalid_rows,
        }))
    }
}

/// The listed people known here, each once and without the user, in the file's order. A set,
/// because this runs on the core thread and the file decides how long the list is.
fn known_members(listed: &[String], me: &str, known: impl Fn(&str) -> bool) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    listed
        .iter()
        .filter(|member| *member != me && seen.insert(member.as_str()) && known(member))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity};

    pub(super) fn fixture() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/android-backup.zip")
            .to_string_lossy()
            .into_owned()
    }

    pub(super) fn app_with_identity(dir: &std::path::Path, client_key: [u8; 32]) -> App {
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
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new(client_key),
                server_group: "a1".into(),
            })
            .unwrap();
        app
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inspects_the_fixture() {
        let dir = tempfile::tempdir().unwrap();
        // The fixture's identity: ABCD1234 with client key 0x07 * 32.
        let mut app = app_with_identity(dir.path(), [7; 32]);
        let preview = app
            .android_backup_inspect(fixture(), Zeroizing::new("test-password".into()))
            .await
            .unwrap();
        assert_eq!(preview["version"], 27);
        assert_eq!(preview["identity"], "match");
        assert_eq!(preview["inventory"]["contactChats"], 1);
        assert_eq!(preview["inventory"]["mediaBytes"], 4096);
        assert_eq!(preview["inventory"]["ownAvatar"], true);

        // Same ID with another key: never merged.
        let other_dir = tempfile::tempdir().unwrap();
        let mut other = app_with_identity(other_dir.path(), [8; 32]);
        let preview = other
            .android_backup_inspect(fixture(), Zeroizing::new("test-password".into()))
            .await
            .unwrap();
        assert_eq!(preview["identity"], "keyMismatch");

        // Wrong password: told apart from damage.
        let error = app
            .android_backup_inspect(fixture(), Zeroizing::new("wrong".into()))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "backupDecrypt");

        // Not an archive at all.
        let junk = dir.path().join("junk.zip");
        std::fs::write(&junk, b"junk").unwrap();
        let error = app
            .android_backup_inspect(
                junk.to_string_lossy().into_owned(),
                Zeroizing::new("test-password".into()),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "backupInvalid");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn imports_metadata_offline() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path(), [7; 32]);
        let password = || Zeroizing::new("test-password".to_owned());
        let summary = app
            .android_backup_import(fixture(), password(), "metadata")
            .await
            .unwrap();
        assert_eq!(summary["contacts"]["added"], 3);
        assert_eq!(summary["contacts"]["keyConflicts"], 0);
        assert_eq!(summary["groups"]["added"], 2);
        assert_eq!(summary["lists"]["added"], 1);
        assert_eq!(summary["pictures"]["own"], true);
        assert_eq!(summary["pictures"]["contacts"], 1);
        assert_eq!(summary["pictures"]["groups"], 1);
        assert_eq!(summary["pictures"]["localAvatarsNotImported"], 1);
        assert_eq!(summary["invalidRows"], 0);
        // Two raw CSP nonces (hashed on import), one hashed D2D nonce, one bad cell.
        assert_eq!(summary["nonces"]["csp"], 2);
        assert_eq!(summary["nonces"]["d2d"], 1);
        assert_eq!(summary["nonces"]["skipped"], 1);
        assert_eq!(summary["nonces"]["missing"], false);
        assert!(
            app.store.has_nonce(&[0x11; 24]).unwrap(),
            "a replay is refused"
        );
        assert!(app.store.has_d2d_nonce(&[0x22; 24]).unwrap());
        assert!(!app.store.has_nonce(&[0x22; 24]).unwrap());

        use libthreema::protobuf::d2d_sync::contact as pc;
        let bob = app.store.contact("B0B0B0B0").unwrap().unwrap();
        assert_eq!(bob.acquaintance_level, pc::AcquaintanceLevel::Direct);
        assert_eq!(
            bob.verification_level,
            pc::VerificationLevel::ServerVerified
        );
        assert!(bob.nickname.is_some());
        let hidden = app.store.contact("CCCCCCCC").unwrap().unwrap();
        assert_eq!(
            hidden.acquaintance_level,
            pc::AcquaintanceLevel::GroupOrDeleted
        );
        assert!(
            app.store
                .archived_chats()
                .unwrap()
                .contains(&"ANNANNAN".to_owned())
        );

        // The groups are stored with the archive's membership semantics and show up in the
        // chat list without any message.
        let team = app
            .store
            .group("g:B0B0B0B0:0102030405060708")
            .unwrap()
            .unwrap();
        assert!(!team.left);
        assert_eq!(team.members.len(), 3);
        assert!(team.picture_version > 0, "group picture imported");
        let old = app
            .store
            .group("g:ABCD1234:1111111111111111")
            .unwrap()
            .unwrap();
        assert!(old.left, "user_state 2");
        let listed: Vec<String> = app
            .store
            .conversations()
            .unwrap()
            .into_iter()
            .map(|(m, _)| m.conversation)
            .collect();
        assert!(listed.contains(&team.key));

        let list = app
            .store
            .distribution_list("d:0000000000000007")
            .unwrap()
            .unwrap();
        assert_eq!(list.members, vec!["B0B0B0B0", "ANNANNAN"]);

        // Pictures landed in the media store; the own one has a version now.
        assert!(app.store.contact_picture_version("B0B0B0B0").unwrap() > 0);
        assert!(
            app.media
                .read_picture(&crate::profile_pictures::contact_picture_name("B0B0B0B0"))
                .unwrap()
                .is_some()
        );
        assert!(
            app.store
                .setting(crate::store::keys::PROFILE_PICTURE_VERSION)
                .unwrap()
                .is_some()
        );

        // No sending side effects: nothing queued anywhere.
        assert!(app.store.pending_outgoing().unwrap().is_empty());
        let outbox: i64 = app
            .store
            .conn
            .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(outbox, 0);

        // Idempotent: a second run adds nothing and keeps every picture.
        let again = app
            .android_backup_import(fixture(), password(), "metadata")
            .await
            .unwrap();
        assert_eq!(again["contacts"]["added"], 0);
        assert_eq!(again["contacts"]["existing"], 3);
        assert_eq!(again["groups"]["added"], 0);
        assert_eq!(again["groups"]["existing"], 2);
        assert_eq!(again["lists"]["added"], 0);
        assert_eq!(again["pictures"]["own"], false);
        assert_eq!(again["pictures"]["contacts"], 0);
        assert_eq!(again["pictures"]["kept"], 2);
        assert_eq!(again["nonces"]["csp"], 0);
        assert_eq!(again["nonces"]["existing"], 3);

        // A stored contact whose key differs is a conflict and stays untouched.
        let other_dir = tempfile::tempdir().unwrap();
        let mut other = app_with_identity(other_dir.path(), [7; 32]);
        other
            .store
            .upsert_contact(&crate::contacts::test_contact("B0B0B0B0"))
            .unwrap();
        let summary = other
            .android_backup_import(fixture(), password(), "metadata")
            .await
            .unwrap();
        assert_eq!(summary["contacts"]["keyConflicts"], 1);
        assert_eq!(summary["contacts"]["added"], 2);

        // Another identity's backup is refused before any change.
        let foreign_dir = tempfile::tempdir().unwrap();
        let mut foreign = app_with_identity(foreign_dir.path(), [8; 32]);
        let error = foreign
            .android_backup_import(fixture(), password(), "metadata")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "backupIdentity");
        assert!(foreign.store.contact("B0B0B0B0").unwrap().is_none());
    }
}

#[cfg(test)]
mod member_list_tests {
    use super::known_members;

    #[test]
    fn keeps_known_people_once_without_the_user() {
        let listed = ["B0B0B0B0", "ABCD1234", "C0C0C0C0", "B0B0B0B0", "D0D0D0D0"].map(String::from);
        let members = known_members(&listed, "ABCD1234", |id| id != "D0D0D0D0");
        assert_eq!(members, ["B0B0B0B0", "C0C0C0C0"]);
    }

    #[test]
    fn a_huge_list_stays_linear() {
        // Comparing each new member with those before it took minutes at this size.
        const SIZE: usize = 300_000;
        let listed: Vec<String> = (0..SIZE).map(|i| format!("M{i:07}")).collect();
        let start = std::time::Instant::now();
        assert_eq!(known_members(&listed, "ABCD1234", |_| true).len(), SIZE);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "{:?}",
            start.elapsed()
        );
    }
}

#[cfg(test)]
mod history_tests {
    use serde_json::Value;
    use zeroize::Zeroizing;

    use super::tests::{app_with_identity, fixture};
    use crate::{android_backup::import::HistoryImport, app::App, media::MediaPart};

    /// Run one history import to completion and return the Done event's summary.
    pub(super) async fn run_import(app: &App) -> Value {
        run_import_of(app, fixture().into()).await
    }

    async fn run_import_of(app: &App, path: std::path::PathBuf) -> Value {
        let identity = app.store.identity().unwrap().unwrap();
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let job = HistoryImport {
            store: std::rc::Rc::clone(&app.store),
            media: std::sync::Arc::clone(&app.media),
            events: events_tx,
            cancel: std::rc::Rc::new(std::cell::Cell::new(false)),
            me: identity.threema_id.clone(),
            own_public_key: libthreema::common::keys::ClientKey::from(*identity.client_key)
                .public_key()
                .to_string(),
            path,
            password: Zeroizing::new("test-password".into()),
        };
        job.run().await;
        let mut done = Value::Null;
        while let Ok(event) = events_rx.try_recv() {
            let json = event.to_json(&app.store, true);
            if json["event"] == "androidBackupDone" {
                done = json;
            }
        }
        assert_eq!(done["ok"], true, "{done}");
        done["summary"].clone()
    }

    /// A backup of the fixture's identity with the given tables, written here.
    fn backup_with(dir: &std::path::Path, tables: &[(&str, String)]) -> std::path::PathBuf {
        use std::io::Write as _;
        let mut fixture =
            crate::android_backup::Archive::open(std::path::Path::new(&fixture()), "test-password")
                .unwrap();
        let identity = fixture.read("identity", 1 << 16).unwrap();
        let path = dir.join("backup.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .with_aes_encryption(zip::AesMode::Aes256, "test-password");
        let settings = b"\"version\",\"27\"\n".as_slice();
        for (name, data) in [("settings", settings), ("identity", &identity)]
            .into_iter()
            .chain(tables.iter().map(|(name, text)| (*name, text.as_bytes())))
        {
            writer.start_file(name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    fn message_rows(rows: impl IntoIterator<Item = (String, Option<&'static str>)>) -> String {
        use crate::android_backup::csv::write_row;
        let mut text = write_row(&["uid", "identity", "type", "body", "posted_at"]);
        for (index, (uid, sender)) in rows.into_iter().enumerate() {
            let at = (1_700_000_000_000 + index).to_string();
            text.push_str(&write_row(&[
                &uid,
                sender.unwrap_or(""),
                "TEXT",
                "hello",
                &at,
            ]));
        }
        text
    }

    /// A backup without media, years of history: nothing of it goes to the blob server by itself.
    /// The keys are kept, so a tap can still try one whose blob is there.
    #[tokio::test(flavor = "current_thread")]
    async fn a_backup_without_media_downloads_nothing_by_itself() {
        use crate::android_backup::csv::write_row;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path(), [7; 32]);
        let contacts = write_row(&["identity", "publickey", "identity_id"])
            + &write_row(&["B0B0B0B0", &"ab".repeat(32), "17"]);
        let body = format!(
            r#"["{}","{}","image/jpeg",1000,"a.jpg",1,true,"","image/jpeg",{{}}]"#,
            "01".repeat(16),
            "02".repeat(32),
        );
        let messages = write_row(&["uid", "identity", "isoutbox", "type", "body", "posted_at"])
            + &write_row(&["u-1", "B0B0B0B0", "0", "FILE", &body, "1700000000000"]);
        let path = backup_with(
            dir.path(),
            &[("contacts.csv", contacts), ("message_17.csv", messages)],
        );
        app.android_backup_import(
            path.to_string_lossy().into_owned(),
            Zeroizing::new("test-password".into()),
            "metadata",
        )
        .await
        .unwrap();
        let summary = run_import_of(&app, path).await;
        assert_eq!(summary["imported"], 1, "{summary}");

        let page = app.store.conversation_messages("B0B0B0B0", 50).unwrap();
        let file = page.iter().find(|m| m.kind == "file").unwrap();
        let parsed: Value = serde_json::from_str(&file.body).unwrap();
        assert_eq!(parsed["fileState"], crate::app_media::STATE_FAILED);
        assert_eq!(parsed["thumbnailState"], crate::app_media::STATE_FAILED);
        let every_kind = ["image", "audio", "video", "file"].map(str::to_owned);
        assert!(
            app.store
                .media_to_download(&every_kind, 0)
                .unwrap()
                .is_empty()
        );
        let keys = app.store.file_keys(file.id).unwrap().unwrap();
        assert_eq!(keys.blob_id.unwrap().0, [1; 16]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hints_at_files_that_were_moved_around() {
        use crate::android_backup::csv::write_row;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path(), [7; 32]);
        let contacts = write_row(&["identity", "publickey", "identity_id"])
            + &write_row(&["B0B0B0B0", &"aa".repeat(32), "17"])
            + &write_row(&["C0C0C0C0", &"bb".repeat(32), "18"])
            + &write_row(&["D0D0D0D0", &"cc".repeat(32), "19"]);
        let groups = write_row(&["creator", "id", "groupname", "members", "group_uid"])
            + &write_row(&[
                "ABCD1234",
                "0102030405060708",
                "Ours",
                "ABCD1234;B0B0B0B0",
                "ours",
            ])
            + &write_row(&[
                "ABCD1234",
                "0202030405060708",
                "Changing",
                "ABCD1234;B0B0B0B0",
                "changing",
            ])
            + &write_row(&[
                "B0B0B0B0",
                "0302030405060708",
                "Theirs",
                "ABCD1234;B0B0B0B0",
                "theirs",
            ]);
        let mut theirs = write_row(&["uid", "identity", "isoutbox", "type", "body", "posted_at"]);
        for (index, (sender, outgoing)) in std::iter::repeat_n(("ABCD1234", "1"), 10)
            .chain(std::iter::repeat_n(("B0B0B0B0", "0"), 4))
            .enumerate()
        {
            let at = (1_700_000_000_000 + index).to_string();
            let uid = format!("theirs-{index}");
            theirs.push_str(&write_row(&[&uid, sender, outgoing, "TEXT", "hello", &at]));
        }
        let copied =
            |range: std::ops::Range<u32>| message_rows(range.map(|i| (format!("u-{i}"), None)));
        let from = |sender: &'static str, count: u32| {
            (0..count)
                .map(|i| (format!("{sender}-{i}"), Some(sender)))
                .collect::<Vec<_>>()
        };
        let path = backup_with(
            dir.path(),
            &[
                ("contacts.csv", contacts),
                ("groups.csv", groups),
                ("message_17.csv", copied(0..5)),
                // Holds a copy of three of the first chat's rows.
                ("message_18.csv", copied(2..7)),
                // Nobody who is in the group wrote here.
                ("group_message_ours.csv", message_rows(from("D0D0D0D0", 10))),
                // The user's own rows carry the user's ID in polls, which no member list holds.
                ("group_message_theirs.csv", theirs),
                // Someone left; most words are a member's.
                (
                    "group_message_changing.csv",
                    message_rows([from("E0E0E0E0", 4), from("B0B0B0B0", 6)].concat()),
                ),
            ],
        );
        let metadata = app
            .android_backup_import(
                path.to_string_lossy().into_owned(),
                Zeroizing::new("test-password".into()),
                "metadata",
            )
            .await
            .unwrap();
        assert_eq!(metadata["nonces"]["missing"], true, "{metadata}");
        assert_eq!(metadata["contacts"]["added"], 3, "{metadata}");

        let summary = run_import_of(&app, path).await;
        assert_eq!(summary["chats"], 5, "{summary}");
        assert_eq!(summary["uidConflicts"], 3, "{summary}");
        assert_eq!(summary["foreignGroupChats"], 1, "{summary}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn imports_history_between_live_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_identity(dir.path(), [7; 32]);
        // Metadata first (contacts/groups must exist), then a live message that is newer
        // than everything in the archive.
        app.android_backup_import(
            fixture(),
            Zeroizing::new("test-password".into()),
            "metadata",
        )
        .await
        .unwrap();
        let live = app
            .store
            .insert_incoming("B0B0B0B0", 0xAAAA, "text", "live now", 1_800_000_000_000)
            .unwrap()
            .unwrap();
        app.store.mark_conversation_read("B0B0B0B0").unwrap();

        let summary = run_import(&app).await;
        assert_eq!(summary["chats"], 2, "{summary}");
        assert_eq!(summary["skippedChats"], 0);
        assert_eq!(summary["uidConflicts"], 0);
        assert_eq!(summary["foreignGroupChats"], 0);
        // 9 contact rows (one a poll), 6 group rows: 3 texts and a poll's created, modified
        // (not shown here) and closed lines.
        assert_eq!(summary["imported"], 14, "{summary}");
        assert_eq!(summary["polls"], 3);
        assert_eq!(summary["unsupported"]["poll"], 1, "the modified line");
        assert_eq!(summary["unsent"], 1);
        assert_eq!(summary["mediaFiles"], 2);
        assert_eq!(summary["thumbnails"], 1);
        assert_eq!(summary["reactions"], 2);
        assert_eq!(summary["invalidRows"], 0);

        // History order: the legacy image (oldest) first, the live message last.
        let page = app.store.conversation_messages("B0B0B0B0", 50).unwrap();
        let kinds: Vec<(&str, i64)> = page
            .iter()
            .map(|m| (m.kind.as_str(), m.created_at))
            .collect();
        assert_eq!(page.last().unwrap().id, live.id, "{kinds:?}");
        assert_eq!(page.first().unwrap().created_at, 1_700_000_000_500);
        let created: Vec<i64> = page.iter().map(|m| m.created_at).collect();
        let mut sorted = created.clone();
        sorted.sort_unstable();
        assert_eq!(created, sorted, "history reads chronologically here");

        // The quote survives with Threema's id format; the unread row stays unread without
        // notifications; the unsent one is failed, never queued.
        let quote = page.iter().find(|m| m.body.contains("Good point")).unwrap();
        assert_eq!(
            crate::messages::split_quote(&quote.body).0,
            crate::android_backup::records::parse_api_id("0123456789abcdef")
        );
        assert_eq!(quote.state, crate::messages::INCOMING_UNREAD);
        assert_eq!(app.store.conversations().unwrap()[0].1, 1, "one unread");
        let unsent = page.iter().find(|m| m.body.contains("never left")).unwrap();
        assert_eq!(unsent.state, crate::messages::OutgoingState::Failed as i64);
        assert!(app.store.pending_outgoing().unwrap().is_empty());

        // Tombstone, call record, starred flag.
        assert!(page.iter().any(|m| m.kind == "deleted"));
        assert!(
            page.iter()
                .any(|m| m.kind == "status" && m.body == "call|finished|1|65")
        );
        assert!(page.iter().any(|m| m.starred && m.body.contains("Tricky")));

        // Media: bytes are on the device and the states say so.
        let photo = page.iter().find(|m| m.body.contains("photo.jpg")).unwrap();
        let body: Value = serde_json::from_str(&photo.body).unwrap();
        assert_eq!(body["fileState"], crate::app_media::STATE_LOCAL);
        assert_eq!(body["thumbnailState"], crate::app_media::STATE_LOCAL);
        assert_eq!(body["uploadState"], crate::app_media::STATE_LOCAL);
        assert!(app.media.read(photo.id, MediaPart::File).unwrap().is_some());
        let legacy = page
            .iter()
            .find(|m| m.kind == "file" && m.created_at == 1_700_000_000_500)
            .unwrap();
        let body: Value = serde_json::from_str(&legacy.body).unwrap();
        assert_eq!(body["mediaType"], "image/jpeg");
        assert_eq!(body["fileState"], crate::app_media::STATE_LOCAL);

        // Thumbs and reaction CSVs: the partner's ack on our outgoing text, both group
        // states, and the ❤️/🎉 from the reaction files.
        let acked = page.iter().find(|m| m.body == "agreed?").unwrap();
        assert_eq!(acked.reactions[0]["sender"], "B0B0B0B0");
        assert_eq!(acked.reactions[0]["emoji"], "\u{1F44D}");
        let tricky = page.iter().find(|m| m.body.contains("Tricky")).unwrap();
        assert!(
            tricky
                .reactions
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["emoji"] == "\u{2764}\u{FE0F}")
        );
        let group_page = app
            .store
            .conversation_messages("g:B0B0B0B0:0102030405060708", 50)
            .unwrap();
        let thumbs = group_page.iter().find(|m| m.body == "thumbs?").unwrap();
        let reactions = thumbs.reactions.as_array().unwrap();
        assert!(
            reactions
                .iter()
                .any(|r| r["sender"] == "ANNANNAN" && r["emoji"] == "\u{1F44D}")
        );
        assert!(
            reactions
                .iter()
                .any(|r| r["sender"] == "" && r["emoji"] == "\u{1F44E}"),
            "our own USERDEC"
        );
        assert_eq!(
            thumbs.sender.as_deref(),
            Some("B0B0B0B0"),
            "group sender kept"
        );

        // Polls: Bob's open poll is registered, so a live vote lands in it.
        let lunch = page.iter().find(|m| m.kind == "poll").unwrap();
        let state: Value = serde_json::from_str(&lunch.body).unwrap();
        assert_eq!(state["description"], "Lunch?");
        assert_eq!(
            state["pollId"], "0807060504030201",
            "wire-order hex read back"
        );
        assert_eq!(state["announce"], true);
        assert_eq!(state["votes"], serde_json::json!({"B0B0B0B0": [0]}));
        let voted = crate::polls::receive_vote(
            &app.store,
            "B0B0B0B0",
            "B0B0B0B0",
            "B0B0B0B0",
            0x0807_0605_0403_0201,
            "[[0,0],[1,1]]",
        )
        .unwrap()
        .unwrap();
        assert_eq!(voted.id, lunch.id);
        // Ann's closed group poll: the poll and its closing line, results per voter, choices
        // in Android's order; the draft is not imported.
        let polls: Vec<_> = group_page
            .iter()
            .filter(|m| m.kind.starts_with("poll"))
            .collect();
        assert_eq!(polls.len(), 2);
        assert_eq!(polls[0].kind, "poll");
        assert_eq!(polls[1].kind, crate::polls::KIND_CLOSED);
        assert_eq!(polls[0].sender.as_deref(), Some("ANNANNAN"));
        let meeting: Value = serde_json::from_str(&polls[1].body).unwrap();
        assert_eq!(meeting["state"], 1);
        assert_eq!(meeting["multiple"], true);
        assert_eq!(meeting["choices"][0]["name"], "Tuesday");
        assert_eq!(
            meeting["votes"],
            serde_json::json!({"ANNANNAN": [1, 0], "ABCD1234": [1]}),
            "in the choices' order"
        );
        assert!(
            app.store
                .poll_message(
                    "g:B0B0B0B0:0102030405060708",
                    "ANNANNAN",
                    0x1817_1615_1413_1211
                )
                .unwrap()
                .is_some_and(|m| m.id == polls[0].id)
        );

        // Idempotent: the same import again adds nothing and finds everything in place.
        let again = run_import(&app).await;
        assert_eq!(again["imported"], 0, "{again}");
        assert_eq!(again["existing"], 14);
        assert_eq!(
            app.store
                .conversation_messages("B0B0B0B0", 50)
                .unwrap()
                .len(),
            page.len()
        );
    }
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How long a running export may take to stop: it looks at the flag every megabyte.
const EXPORT_STOP_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

impl App {
    /// `androidBackupExportStatus`: the automatic-backup settings and the last run.
    pub(crate) fn android_export_status(&self) -> Result<Value, ApiError> {
        use crate::store::keys;
        let number =
            |key: &str| -> Option<i64> { self.store.setting(key).ok().flatten()?.parse().ok() };
        Ok(json!({
            "directory": self.store.setting(keys::ANDROID_EXPORT_DIR)?,
            "interval": self.store.setting(keys::ANDROID_EXPORT_INTERVAL)?
                .unwrap_or_else(|| "off".into()),
            "keep": number(keys::ANDROID_EXPORT_KEEP).unwrap_or(3),
            "lastBackup": number(keys::ANDROID_EXPORT_LAST),
            "lastError": self.store.setting(keys::ANDROID_EXPORT_ERROR)?,
            "passwordSet": self.store.setting(keys::ANDROID_EXPORT_PASSWORD)?.is_some(),
            "running": self.android_export.as_ref().is_some_and(|done| !done.get()),
        }))
    }

    /// `androidBackupSchedule`: persist what was given (the password only when non-empty,
    /// at least 8 characters like Android's backup wizard).
    pub(crate) fn android_backup_schedule(
        &mut self,
        directory: Option<String>,
        password: Option<Zeroizing<String>>,
        interval: Option<String>,
        keep: Option<u32>,
    ) -> Result<Value, ApiError> {
        use crate::store::keys;
        if let Some(directory) = directory.as_deref().filter(|d| !d.is_empty()) {
            self.store
                .set_setting(keys::ANDROID_EXPORT_DIR, directory)?;
        }
        if let Some(password) = password.as_ref().filter(|p| !p.is_empty()) {
            if password.chars().count() < 8 {
                return Err(ApiError::PasswordTooShort);
            }
            self.store
                .set_setting(keys::ANDROID_EXPORT_PASSWORD, password)?;
        }
        if let Some(interval) = interval.as_deref() {
            if !matches!(interval, "off" | "daily" | "weekly") {
                return Err(ApiError::Internal("unknown interval".into()));
            }
            self.store
                .set_setting(keys::ANDROID_EXPORT_INTERVAL, interval)?;
        }
        if let Some(keep) = keep {
            self.store
                .set_setting(keys::ANDROID_EXPORT_KEEP, &keep.min(50).to_string())?;
        }
        self.android_export_status()
    }

    /// `androidBackupExport`: start the export job; omitted fields use the stored settings.
    pub(crate) fn android_backup_export(
        &mut self,
        directory: Option<String>,
        password: Option<Zeroizing<String>>,
    ) -> Result<Value, ApiError> {
        use crate::store::keys;
        if self.android_export.as_ref().is_some_and(|done| !done.get()) {
            return Err(ApiError::Internal("an export is already running".into()));
        }
        let directory = directory
            .filter(|d| !d.is_empty())
            .or(self.store.setting(keys::ANDROID_EXPORT_DIR)?)
            .ok_or_else(|| ApiError::Internal("no folder chosen".into()))?;
        let password = match password.filter(|p| !p.is_empty()) {
            Some(password) => password,
            None => Zeroizing::new(
                self.store
                    .setting(keys::ANDROID_EXPORT_PASSWORD)?
                    .ok_or(ApiError::PasswordTooShort)?,
            ),
        };
        if password.chars().count() < 8 {
            return Err(ApiError::PasswordTooShort);
        }
        let keep = self
            .store
            .setting(keys::ANDROID_EXPORT_KEEP)?
            .and_then(|k| k.parse().ok())
            .unwrap_or(3);
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let done = std::rc::Rc::new(std::cell::Cell::new(false));
        self.android_export = Some(std::rc::Rc::clone(&done));
        self.android_export_cancel = std::sync::Arc::default();
        let job = crate::android_backup::export::Export {
            store: std::rc::Rc::clone(&self.store),
            media: std::sync::Arc::clone(&self.media),
            events: self.media_events.clone(),
            me: identity.threema_id.clone(),
            client_key: identity.client_key.clone(),
            directory: std::path::PathBuf::from(directory),
            password,
            keep,
            done,
            cancel: std::sync::Arc::clone(&self.android_export_cancel),
        };
        tokio::task::spawn_local(job.run());
        Ok(json!({ "started": true }))
    }

    /// The ID is being removed: no import goes on, and a running export gives up and takes its
    /// partial file with it. Waits for that: once the core stops the job can't clean up any more,
    /// and it holds the ID's key and writes to the user's own folder.
    pub(crate) async fn stop_backups(&mut self) {
        if let Some(cancel) = &self.android_import {
            cancel.set(true);
        }
        self.android_export_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let Some(done) = self.android_export.clone() else {
            return;
        };
        let finished = tokio::time::timeout(EXPORT_STOP_LIMIT, async {
            while !done.get() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        if finished.is_err() {
            tracing::warn!("the running backup did not stop in time");
        }
    }

    /// Called by maintenance: start a scheduled backup when one is due and configured.
    pub(crate) fn android_export_scheduled(&mut self) {
        use crate::store::keys;
        let interval = self
            .store
            .setting(keys::ANDROID_EXPORT_INTERVAL)
            .ok()
            .flatten();
        let period = match interval.as_deref() {
            Some("daily") => DAY_MS,
            Some("weekly") => 7 * DAY_MS,
            _ => return,
        };
        let last: i64 = self
            .store
            .setting(keys::ANDROID_EXPORT_LAST)
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if crate::messages::now_ms() - last < period {
            return;
        }
        if let Err(error) = self.android_backup_export(None, None) {
            tracing::warn!(kind = error.kind(), "scheduled backup not started");
            let _ = self
                .store
                .set_setting(keys::ANDROID_EXPORT_ERROR, error.kind());
        }
    }
}

#[cfg(test)]
mod export_tests {
    use serde_json::Value;
    use zeroize::Zeroizing;

    use super::{
        history_tests::run_import,
        tests::{app_with_identity, fixture},
    };
    use crate::{
        android_backup::export::{Export, backup_file_name, is_backup_file_name},
        app::App,
        media::MediaPart,
    };

    async fn run_export(app: &App, directory: &std::path::Path, keep: usize) -> Value {
        let identity = app.store.identity().unwrap().unwrap();
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let job = Export {
            store: std::rc::Rc::clone(&app.store),
            media: std::sync::Arc::clone(&app.media),
            events: events_tx,
            me: identity.threema_id.clone(),
            client_key: identity.client_key.clone(),
            directory: directory.to_owned(),
            password: Zeroizing::new("round-trip-pw".into()),
            keep,
            done: std::rc::Rc::new(std::cell::Cell::new(false)),
            cancel: std::sync::Arc::default(),
        };
        job.run().await;
        let mut done = Value::Null;
        while let Ok(event) = events_rx.try_recv() {
            let json = event.to_json(&app.store, true);
            if json["event"] == "androidExportDone" {
                done = json;
            }
        }
        assert_eq!(done["ok"], true, "{done}");
        done
    }

    /// The ID goes while a backup runs: no archive, no partial file, nothing written into the
    /// emptied store.
    #[test]
    fn removing_the_id_stops_a_running_backup() {
        let dir = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &crate::app::StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "test".into(),
                language: "en".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "test".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "AAAAAAAA".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a".into(),
            })
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, async {
            app.android_backup_export(
                Some(folder.path().to_str().unwrap().to_owned()),
                Some(Zeroizing::new("round-trip-pw".into())),
            )
            .unwrap();
            app.delete_all_data().await.unwrap();
        });
        assert!(app.android_export.as_ref().unwrap().get(), "the job ended");
        let left: Vec<_> = std::fs::read_dir(folder.path()).unwrap().collect();
        assert!(left.is_empty(), "{left:?}");
        use crate::store::keys;
        assert_eq!(app.store.setting(keys::ANDROID_EXPORT_LAST).unwrap(), None);
        assert_eq!(app.store.setting(keys::ANDROID_EXPORT_ERROR).unwrap(), None);
    }

    #[test]
    fn backup_file_names() {
        let name = backup_file_name(1_700_000_000_000);
        assert_eq!(name, "threema-backup-20231114-221320.zip");
        assert!(is_backup_file_name(&name));
        assert!(!is_backup_file_name("threema-backup-2023.zip"));
        assert!(!is_backup_file_name("other-20231114-221320.zip"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn export_round_trips_through_the_importer() {
        // A: everything the fixture holds, plus one live message.
        let dir_a = tempfile::tempdir().unwrap();
        let mut a = app_with_identity(dir_a.path(), [7; 32]);
        a.android_backup_import(
            fixture(),
            Zeroizing::new("test-password".into()),
            "metadata",
        )
        .await
        .unwrap();
        a.store
            .insert_incoming("B0B0B0B0", 0xAAAA, "text", "live now", 1_800_000_000_000)
            .unwrap();
        a.store.mark_conversation_read("B0B0B0B0").unwrap();
        run_import(&a).await;

        let out = tempfile::tempdir().unwrap();
        let done = run_export(&a, out.path(), 0).await;
        let path = done["path"].as_str().unwrap().to_owned();
        assert!(is_backup_file_name(
            std::path::Path::new(&path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        ));
        let summary = &done["summary"];
        assert_eq!(summary["contacts"], 3, "{summary}");
        assert_eq!(summary["groups"], 2);
        assert_eq!(summary["lists"], 1);
        assert_eq!(summary["mediaFiles"], 2);
        assert_eq!(summary["thumbnails"], 1);
        assert_eq!(summary["pictures"], 3, "own, one contact, one group");
        assert_eq!(summary["polls"], 2);
        assert_eq!(summary["nonces"], 3, "two CSP, one D2D");
        assert_eq!(summary["skipped"].get("poll"), None);

        // B: a fresh device with the same identity restores A's file completely.
        let dir_b = tempfile::tempdir().unwrap();
        let mut b = app_with_identity(dir_b.path(), [7; 32]);
        let preview = b
            .android_backup_inspect(path.clone(), Zeroizing::new("round-trip-pw".into()))
            .await
            .unwrap();
        assert_eq!(preview["identity"], "match");
        assert_eq!(preview["version"], 27);
        b.android_backup_import(
            path.clone(),
            Zeroizing::new("round-trip-pw".into()),
            "metadata",
        )
        .await
        .unwrap();
        let identity = b.store.identity().unwrap().unwrap();
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let job = crate::android_backup::import::HistoryImport {
            store: std::rc::Rc::clone(&b.store),
            media: std::sync::Arc::clone(&b.media),
            events: events_tx,
            cancel: std::rc::Rc::new(std::cell::Cell::new(false)),
            me: identity.threema_id.clone(),
            own_public_key: libthreema::common::keys::ClientKey::from(*identity.client_key)
                .public_key()
                .to_string(),
            path: path.clone().into(),
            password: Zeroizing::new("round-trip-pw".into()),
        };
        job.run().await;
        while events_rx.try_recv().is_ok() {}

        // The chats read identically on both devices.
        for conversation in ["B0B0B0B0", "g:B0B0B0B0:0102030405060708"] {
            let rows_a = a.store.conversation_messages(conversation, 500).unwrap();
            let rows_b = b.store.conversation_messages(conversation, 500).unwrap();
            let shape =
                |rows: &[crate::messages::Message]| -> Vec<(String, String, i64, i64, bool)> {
                    rows.iter()
                        .map(|m| {
                            let text = if m.kind == "text" {
                                m.body.clone()
                            } else {
                                String::new()
                            };
                            (m.kind.clone(), text, m.created_at, m.state, m.starred)
                        })
                        .collect()
                };
            assert_eq!(shape(&rows_a), shape(&rows_b), "{conversation}");
            // Polls come back with their whole state, and are registered again.
            let polls = |rows: &[crate::messages::Message]| -> Vec<Value> {
                rows.iter()
                    .filter(|m| m.kind.starts_with("poll"))
                    .map(|m| serde_json::from_str(&m.body).unwrap())
                    .collect()
            };
            assert_eq!(polls(&rows_a), polls(&rows_b), "{conversation}");
            for poll in polls(&rows_b) {
                let (creator, id) = crate::android_backup::polls::key(&poll).unwrap();
                assert!(
                    b.store
                        .poll_message(conversation, &creator, id)
                        .unwrap()
                        .is_some()
                );
            }
        }
        // The used nonces too: hashed on the way out, still recognised after the restore.
        assert!(b.store.has_nonce(&[0x11; 24]).unwrap());
        assert!(b.store.has_d2d_nonce(&[0x22; 24]).unwrap());
        // Media bytes and reactions survive.
        let photo_b = b
            .store
            .conversation_messages("B0B0B0B0", 500)
            .unwrap()
            .into_iter()
            .find(|m| m.body.contains("photo.jpg"))
            .unwrap();
        assert!(b.media.read(photo_b.id, MediaPart::File).unwrap().is_some());
        assert!(
            b.media
                .read(photo_b.id, MediaPart::Thumbnail)
                .unwrap()
                .is_some()
        );
        let tricky_b = b
            .store
            .conversation_messages("B0B0B0B0", 500)
            .unwrap()
            .into_iter()
            .find(|m| m.body.contains("Tricky"))
            .unwrap();
        assert!(
            tricky_b
                .reactions
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["emoji"] == "\u{2764}\u{FE0F}")
        );
        assert!(
            b.store
                .setting(crate::store::keys::PROFILE_PICTURE_VERSION)
                .unwrap()
                .is_some(),
            "own picture restored"
        );

        // The phone scenario: a backup created on this device, imported back into itself.
        // A second backup file has a new fingerprint, so this guards the api-id matching
        // (synthetic ids cannot carry across files): nothing may be added twice.
        let metadata = a
            .android_backup_import(
                path.clone(),
                Zeroizing::new("round-trip-pw".into()),
                "metadata",
            )
            .await
            .unwrap();
        assert_eq!(metadata["contacts"]["added"], 0, "{metadata}");
        assert_eq!(metadata["groups"]["added"], 0);
        let identity_a = a.store.identity().unwrap().unwrap();
        let before: usize = ["B0B0B0B0", "g:B0B0B0B0:0102030405060708", "d:11"]
            .iter()
            .map(|c| a.store.conversation_messages(c, 500).unwrap().len())
            .sum();
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let job = crate::android_backup::import::HistoryImport {
            store: std::rc::Rc::clone(&a.store),
            media: std::sync::Arc::clone(&a.media),
            events: events_tx,
            cancel: std::rc::Rc::new(std::cell::Cell::new(false)),
            me: identity_a.threema_id.clone(),
            own_public_key: libthreema::common::keys::ClientKey::from(*identity_a.client_key)
                .public_key()
                .to_string(),
            path: path.clone().into(),
            password: Zeroizing::new("round-trip-pw".into()),
        };
        job.run().await;
        let mut reimport = Value::Null;
        while let Ok(event) = events_rx.try_recv() {
            let json = event.to_json(&a.store, true);
            if json["event"] == "androidBackupDone" {
                reimport = json;
            }
        }
        assert_eq!(reimport["ok"], true, "{reimport}");
        assert_eq!(
            reimport["summary"]["imported"], 0,
            "{}",
            reimport["summary"]
        );
        let after: usize = ["B0B0B0B0", "g:B0B0B0B0:0102030405060708", "d:11"]
            .iter()
            .map(|c| a.store.conversation_messages(c, 500).unwrap().len())
            .sum();
        assert_eq!(after, before, "no duplicated rows");

        // Rotation: with keep = 2, only the two newest of our files stay; foreign files are
        // never touched.
        std::fs::write(out.path().join("unrelated.zip"), b"x").unwrap();
        run_export(&a, out.path(), 2).await;
        run_export(&a, out.path(), 2).await;
        let names: Vec<String> = std::fs::read_dir(out.path())
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        assert!(names.contains(&"unrelated.zip".to_owned()));
        assert!(
            names
                .iter()
                .filter(|name| is_backup_file_name(name))
                .count()
                <= 2,
            "{names:?}"
        );
    }
}
