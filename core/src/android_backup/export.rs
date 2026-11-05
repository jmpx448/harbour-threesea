// SPDX-License-Identifier: AGPL-3.0-only
//! Writing an Android-format data backup (format 27): the same container
//! (AES-256/WinZip AE entries, DEFLATE) and CSV dialect Android writes, so Threema for
//! Android — and our own importer — can restore it. Store reads happen on the core thread
//! (bounded, yielding between chats); the ZIP writer, the identity-backup key derivation and
//! the media encryption run on a blocking thread, with the writer moving back and forth.
//!
//! Not written yet (reported in the summary instead of silently dropped): status lines other
//! than 1:1 calls, and polls in distribution lists (Android links polls to a contact or a
//! group only). Nonces go out hashed, exactly like Android's (`nonces.rs`).
//!
//! A chat restores only with its contact's row, for the key. Contacts deleted today keep theirs;
//! a chat without one, left by an older version, gets it back from the directory if it can, and
//! otherwise is counted as left out, because the page says so (a private one only while the
//! private chats are open, as the core hides them).

use std::{
    io::Write as _,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use libthreema::common::config::Config;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::csv::write_row;
use crate::{
    app::ApiError,
    connection::ToConnection,
    https::HttpsClient,
    media::{MediaPart, MediaStore},
    messages::{Message, OutgoingState},
    providers::CoreEvent,
    store::{Store, StoreError},
};

/// File names of scheduled and manual backups: sortable by time, recognisable for rotation.
pub fn backup_file_name(now_ms: i64) -> String {
    format!("threema-backup-{}.zip", time_stamp(now_ms))
}

/// `YYYYMMDD-HHMMSS` (UTC) for file names, here and for the log package.
pub fn time_stamp(now_ms: i64) -> String {
    let secs = now_ms / 1000;
    let days = secs.div_euclid(86_400);
    let (h, m, s) = (
        secs.rem_euclid(86_400) / 3600,
        secs.rem_euclid(3600) / 60,
        secs.rem_euclid(60),
    );
    // Civil date from the day number (Howard Hinnant's algorithm), no chrono dependency.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

/// Is this one of our backup files (rotation only ever touches these)?
pub fn is_backup_file_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("threema-backup-") else {
        return false;
    };
    let Some(stamp) = rest.strip_suffix(".zip") else {
        return false;
    };
    stamp.len() == 15
        && stamp.as_bytes()[8] == b'-'
        && stamp
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 8 || b.is_ascii_digit())
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub contacts: usize,
    pub groups: usize,
    pub lists: usize,
    pub chats: usize,
    pub messages: usize,
    pub media_files: usize,
    pub thumbnails: usize,
    pub pictures: usize,
    pub polls: usize,
    pub nonces: usize,
    /// Messages with no representation in the Android format yet (status lines, list polls).
    /// Also whole chats, under `CHATS_WITHOUT_CONTACT`, and files, under `FILES_UNREADABLE`.
    pub skipped: std::collections::BTreeMap<&'static str, usize>,
    /// Private chats without a contact row: never in what the page is told while they are hidden.
    #[serde(skip)]
    pub private_chats_left_out: usize,
    /// The same for files of private chats that can't be read.
    #[serde(skip)]
    pub private_files_left_out: usize,
    pub removed_old_backups: usize,
}

/// In `ExportSummary::skipped`: chats that have no contact row, so no key to restore them with.
pub const CHATS_WITHOUT_CONTACT: &str = "chatsWithoutContact";

/// In `ExportSummary::skipped`: media files that are noted as on the device and can't be read.
pub const FILES_UNREADABLE: &str = "filesUnreadable";

/// Everything the export job owns.
pub struct Export {
    pub store: Rc<Store>,
    pub media: Arc<MediaStore>,
    pub events: tokio::sync::mpsc::UnboundedSender<CoreEvent>,
    pub me: String,
    pub client_key: Zeroizing<[u8; 32]>,
    pub directory: PathBuf,
    pub password: Zeroizing<String>,
    /// Keep the newest this many backups in the folder (0: keep everything).
    pub keep: usize,
    /// Set true when the job ends (one export at a time).
    pub done: Rc<std::cell::Cell<bool>>,
    /// Set true to give up (the ID is being removed): the partial file goes, nothing else is
    /// touched. Atomic: the blocking thread writing an entry looks at it too.
    pub cancel: Arc<AtomicBool>,
    /// How much a folder's storage has free (`free_bytes`).
    pub free_space: FreeSpace,
    /// The directory, to ask for the keys of chats without a contact row.
    pub lookup: Option<ContactLookup>,
}

/// What asking the directory for a contact takes, and who then has something to reflect.
pub struct ContactLookup {
    pub https: HttpsClient,
    pub config: Arc<Config>,
    pub connection: Option<tokio::sync::mpsc::UnboundedSender<ToConnection>>,
}

/// How many bytes can be written below a folder; `None` when that can't be told.
pub type FreeSpace = fn(&std::path::Path) -> Option<u64>;

/// What a backup leaves free on its storage. The chats are often stored there too, and what can't
/// be written then is a message that just came in.
pub const KEEP_FREE_BYTES: u64 = 100 * 1024 * 1024;

/// How many bytes an ordinary app can write below `path`.
#[must_use]
pub fn free_bytes(path: &std::path::Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Refuses what would leave less than `KEEP_FREE_BYTES` on the folder's storage.
fn check_room(free_space: FreeSpace, folder: &std::path::Path, bytes: u64) -> Result<(), ApiError> {
    match free_space(folder) {
        Some(free) if free < bytes.saturating_add(KEEP_FREE_BYTES) => Err(ApiError::BackupNoRoom),
        _ => Ok(()),
    }
}

/// The size of the newest backup in the folder: the best guess of how much the next one needs.
fn last_backup_size(directory: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(is_backup_file_name))
        .max_by_key(std::fs::DirEntry::file_name)
        .and_then(|entry| entry.metadata().ok())
        .map_or(0, |metadata| metadata.len())
}

type Writer = zip::ZipWriter<std::fs::File>;

/// How much of an entry is compressed before looking at the cancel flag again.
const ADD_CHUNK_BYTES: usize = 1 << 20;

/// Whom to ask the directory about for a backup: the chats without a contact row. Nobody while the
/// directory has told us to rest; a backup runs by itself.
fn contacts_to_look_up(store: &Store, me: &str) -> Result<Vec<String>, ApiError> {
    if store.directory_resting() {
        return Ok(Vec::new());
    }
    let mut missing = Vec::new();
    for chat in store.chats_with_messages()? {
        let is_identity = crate::contacts::parse_identity(&chat).is_some();
        if is_identity && chat != me && store.contact(&chat)?.is_none() {
            missing.push(chat);
        }
    }
    Ok(missing)
}

/// What an export left behind when the phone restarted or the app was killed during it: gigabytes
/// nothing else ever takes away. Only one export runs at a time, and this runs before it creates
/// its own, so none of these is in use.
fn remove_leftovers(directory: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_ours = name
            .to_str()
            .and_then(|name| name.strip_suffix(".part"))
            .is_some_and(is_backup_file_name);
        if is_ours {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A folder's entries onto the disk. File systems without that (some answer it this way) lose
/// nothing to it: anything else is a real failure.
fn sync_directory(directory: &std::path::Path) -> std::io::Result<()> {
    use std::io::ErrorKind;
    match std::fs::File::open(directory).and_then(|folder| folder.sync_all()) {
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::Unsupported | ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        other => other,
    }
}

/// What a failed write means for the user. Never `BackupInvalid`: that is about reading a file.
fn write_error(error: &std::io::Error) -> ApiError {
    use std::io::ErrorKind;
    if matches!(
        error.kind(),
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded
    ) {
        ApiError::BackupNoRoom
    } else {
        ApiError::BackupWrite(error.to_string())
    }
}

fn zip_error(error: zip::result::ZipError) -> ApiError {
    match error {
        zip::result::ZipError::Io(error) => write_error(&error),
        other => ApiError::BackupWrite(other.to_string()),
    }
}

fn export_cancelled() -> ApiError {
    ApiError::Internal("backup cancelled".into())
}

impl Export {
    fn emit(&self, value: Value) {
        let _ = self.events.send(CoreEvent::AndroidBackup(value));
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub async fn run(self) {
        let result = self.run_inner().await;
        self.done.set(true);
        if self.cancelled() {
            tracing::info!("Android-format backup cancelled");
            return;
        }
        match result {
            Ok((summary, path)) => {
                tracing::info!(
                    chats = summary.chats,
                    messages = summary.messages,
                    media = summary.media_files,
                    "Android-format backup written"
                );
                let _ = self.store.set_setting(
                    crate::store::keys::ANDROID_EXPORT_LAST,
                    &crate::messages::now_ms().to_string(),
                );
                let _ = self
                    .store
                    .delete_setting(crate::store::keys::ANDROID_EXPORT_ERROR);
                let left_out = summary.skipped.get(CHATS_WITHOUT_CONTACT).copied();
                let _ = self.store.set_setting(
                    crate::store::keys::ANDROID_EXPORT_LEFT_OUT,
                    &left_out.unwrap_or(0).to_string(),
                );
                let _ = self.store.set_setting(
                    crate::store::keys::ANDROID_EXPORT_LEFT_OUT_PRIVATE,
                    &summary.private_chats_left_out.to_string(),
                );
                let files = summary.skipped.get(FILES_UNREADABLE).copied();
                let _ = self.store.set_setting(
                    crate::store::keys::ANDROID_EXPORT_FILES_LEFT_OUT,
                    &files.unwrap_or(0).to_string(),
                );
                let _ = self.store.set_setting(
                    crate::store::keys::ANDROID_EXPORT_FILES_LEFT_OUT_PRIVATE,
                    &summary.private_files_left_out.to_string(),
                );
                self.emit(json!({
                    "event": "androidExportDone",
                    "ok": true,
                    "path": path.to_string_lossy(),
                    "summary": serde_json::to_value(&summary).unwrap_or(Value::Null),
                }));
            }
            Err(error) => {
                tracing::warn!(kind = error.kind(), "Android-format backup failed");
                let _ = self
                    .store
                    .set_setting(crate::store::keys::ANDROID_EXPORT_ERROR, error.kind());
                self.emit(json!({
                    "event": "androidExportDone",
                    "ok": false,
                    "kind": error.kind(),
                }));
            }
        }
    }

    async fn run_inner(&self) -> Result<(ExportSummary, PathBuf), ApiError> {
        let mut summary = ExportSummary::default();
        let name = backup_file_name(crate::messages::now_ms());
        let path = self.directory.join(&name);
        // A temporary name until the archive is complete: a crashed export never leaves a
        // file that looks like a finished backup.
        let temporary = self.directory.join(format!("{name}.part"));
        let create = temporary.clone();
        let folder = self.directory.clone();
        let free_space = self.free_space;
        let writer = tokio::task::spawn_blocking(move || -> Result<Writer, ApiError> {
            remove_leftovers(&folder);
            // Before minutes of decrypting and encrypting all media for nothing. Older backups
            // only go after the new one stands, so they can't make room for it.
            check_room(free_space, &folder, last_backup_size(&folder))?;
            let file = std::fs::File::create(&create).map_err(|e| write_error(&e))?;
            Ok(zip::ZipWriter::new(file))
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

        let result = self.write_entries(writer, &mut summary).await;
        let finished = match result {
            Ok(writer) => tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
                let file = writer.finish().map_err(zip_error)?;
                // On the disk before it takes the place of an older backup.
                file.sync_all().map_err(|e| write_error(&e))
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
            Err(error) => Err(error),
        };
        // Last look: past this the archive is a finished backup.
        let finished = finished.and_then(|()| {
            if self.cancelled() {
                Err(export_cancelled())
            } else {
                Ok(())
            }
        });
        if let Err(error) = finished {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        // A rename to a new name is only lasting once the folder is on the disk too, and rotating
        // may delete the only other backup: the battery can die seconds after this.
        let (from, to, folder) = (temporary.clone(), path.clone(), self.directory.clone());
        let placed = tokio::task::spawn_blocking(move || -> Result<bool, ApiError> {
            std::fs::rename(&from, &to).map_err(|e| write_error(&e))?;
            Ok(sync_directory(&folder).is_ok())
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        match placed {
            Ok(true) => summary.removed_old_backups = self.rotate(&name),
            Ok(false) => tracing::warn!("backup folder not synced, older backups stay"),
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        }
        Ok((summary, path))
    }

    /// Chats without a contact row: only what the directory has for them makes them restorable.
    /// The way of everyone looked up by itself (`resolve_members`): the row stays out of the
    /// contact list, a linked device hears of it in the same store step, the hourly allowance and
    /// the directory's rate limit hold. Without an answer they are counted as left out, and the
    /// next backup asks again.
    async fn fetch_missing_contacts(&self) -> Result<(), ApiError> {
        let Some(lookup) = &self.lookup else {
            return Ok(());
        };
        let missing = contacts_to_look_up(&self.store, &self.me)?;
        if missing.is_empty() {
            return Ok(());
        }
        let until_cancelled = async {
            while !self.cancelled() {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        let resolve = crate::app_groups::resolve_members(
            &lookup.https,
            &lookup.config,
            &self.store,
            &missing,
        );
        let resolved = tokio::select! {
            resolved = resolve => resolved,
            () = until_cancelled => return Err(export_cancelled()),
        };
        let Some(resolved) = resolved else {
            return Ok(());
        };
        if resolved.synced
            && let Some(connection) = &lookup.connection
        {
            let _ = connection.send(ToConnection::ReflectSync);
        }
        if resolved.groups_changed {
            let _ = self.events.send(CoreEvent::GroupsChanged);
        }
        for message in resolved.lines {
            let _ = self.events.send(CoreEvent::MessageAdded { message });
        }
        let _ = self.events.send(CoreEvent::ContactsChanged);
        Ok(())
    }

    /// Delete the oldest of our backup files beyond `keep` (never anything else).
    fn rotate(&self, just_written: &str) -> usize {
        if self.keep == 0 {
            return 0;
        }
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return 0;
        };
        let mut backups: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_backup_file_name(name))
            .collect();
        backups.sort();
        let mut removed = 0;
        while backups.len() > self.keep {
            let oldest = backups.remove(0);
            if oldest == just_written {
                break;
            }
            if std::fs::remove_file(self.directory.join(&oldest)).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// One entry into the archive (on the blocking thread; the writer round-trips).
    async fn add(
        &self,
        mut writer: Writer,
        name: String,
        data: Zeroizing<Vec<u8>>,
    ) -> Result<Writer, ApiError> {
        if self.cancelled() {
            return Err(export_cancelled());
        }
        let password = self.password.clone();
        let cancel = Arc::clone(&self.cancel);
        let (free_space, folder) = (self.free_space, self.directory.clone());
        tokio::task::spawn_blocking(move || -> Result<Writer, ApiError> {
            check_room(free_space, &folder, data.len() as u64)?;
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .with_aes_encryption(zip::AesMode::Aes256, &password);
            writer.start_file(name, options).map_err(zip_error)?;
            // In pieces: one big video must not hold up whoever waits for the cancel.
            for piece in data.chunks(ADD_CHUNK_BYTES) {
                if cancel.load(Ordering::Relaxed) {
                    return Err(export_cancelled());
                }
                writer.write_all(piece).map_err(|e| write_error(&e))?;
            }
            Ok(writer)
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    }

    #[allow(clippy::too_many_lines)]
    async fn write_entries(
        &self,
        mut writer: Writer,
        summary: &mut ExportSummary,
    ) -> Result<Writer, ApiError> {
        macro_rules! add {
            ($name:expr, $data:expr) => {{
                writer = self.add(writer, $name, $data).await?;
            }};
        }

        self.fetch_missing_contacts().await?;

        // settings and identity (an ordinary encrypted ID backup, same password).
        add!(
            "settings".into(),
            Zeroizing::new(write_row(&["version", "27"]).into_bytes())
        );
        let data = libthreema::id_backup::IdentityBackupData {
            threema_id: self
                .me
                .parse()
                .map_err(|_| ApiError::Internal("identity".into()))?,
            client_key: libthreema::common::keys::ClientKey::from(*self.client_key),
        };
        let password = self.password.clone();
        let identity_backup = tokio::task::spawn_blocking(move || {
            libthreema::id_backup::encrypt_identity_backup(&password, &data)
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        add!(
            "identity".into(),
            Zeroizing::new(identity_backup.as_bytes().to_vec())
        );

        // Android lists a restored chat only with a last update, and only recalculates it below
        // format 22; groups and lists sort by it. As in Threema Safe: the chat's newest message.
        let last_updates = crate::join::last_updates(&self.store)?;
        let last_update = |chat: &str| {
            last_updates
                .get(chat)
                .map_or_else(String::new, ToString::to_string)
        };

        // Contacts (the identity doubles as the archive alias).
        let contacts = self.store.contacts()?;
        let mut rows = write_row(&[
            "identity",
            "publickey",
            "verification",
            "acid",
            "firstname",
            "lastname",
            "nick_name",
            "last_update",
            "hidden",
            "archived",
            "identity_id",
        ]);
        let archived = self.store.archived_chats()?;
        // Android doesn't list the chat of a hidden contact, so it would restore unreachable. There
        // a chat always has a contact in the list (deleting one empties the chat), and a restore
        // of this file, there or here, makes it one. Status lines alone are no reason to: a group
        // member can have those.
        let spoken_with: std::collections::HashSet<String> =
            self.store.chats_with_conversation()?.into_iter().collect();
        for contact in &contacts {
            use libthreema::protobuf::d2d_sync::contact as pc;
            let identity = contact.identity.to_string();
            rows.push_str(&write_row(&[
                &identity,
                &crate::contacts::hex_bytes(&contact.public_key.to_string())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                match contact.verification_level {
                    pc::VerificationLevel::FullyVerified => "FULLY_VERIFIED",
                    pc::VerificationLevel::ServerVerified => "SERVER_VERIFIED",
                    pc::VerificationLevel::Unverified => "UNVERIFIED",
                },
                "",
                contact.first_name.as_deref().unwrap_or(""),
                contact.last_name.as_deref().unwrap_or(""),
                contact.nickname.as_deref().unwrap_or(""),
                &last_update(&identity),
                if contact.acquaintance_level == pc::AcquaintanceLevel::GroupOrDeleted
                    && !spoken_with.contains(&identity)
                {
                    "1"
                } else {
                    "0"
                },
                if archived.contains(&identity) {
                    "1"
                } else {
                    "0"
                },
                &identity,
            ]));
            summary.contacts += 1;
        }
        add!("contacts.csv".into(), Zeroizing::new(rows.into_bytes()));

        // Groups; the uid is our conversation key (any string works as an archive alias).
        let groups = self.store.groups()?;
        let mut rows = write_row(&[
            "id",
            "creator",
            "groupname",
            "created_at",
            "last_update",
            "members",
            "archived",
            "groupDesc",
            "groupDescTimestamp",
            "group_uid",
            "user_state",
        ]);
        for group in &groups {
            let mut members = group.members.clone();
            if !group.left && !members.contains(&self.me) {
                members.push(self.me.clone());
            }
            rows.push_str(&write_row(&[
                &crate::app_safe::safe_id(group.identity.group_id),
                &group.identity.creator_identity.to_string(),
                &group.name,
                &group.created_at.to_string(),
                &last_update(&group.key),
                &members.join(";"),
                if archived.contains(&group.key) {
                    "1"
                } else {
                    "0"
                },
                "",
                "",
                &group.key,
                if group.left { "2" } else { "0" },
            ]));
            summary.groups += 1;
        }
        add!("groups.csv".into(), Zeroizing::new(rows.into_bytes()));

        // Distribution lists (local ids; ours are the key's number).
        let lists = self.store.distribution_lists()?;
        let mut rows = write_row(&[
            "id",
            "distribution_list_name",
            "created_at",
            "last_update",
            "distribution_members",
            "archived",
        ]);
        for list in &lists {
            let id = list
                .key
                .strip_prefix("d:")
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                .unwrap_or_default();
            rows.push_str(&write_row(&[
                &id.cast_signed().to_string(),
                &list.name,
                &list.created_at.to_string(),
                &last_update(&list.key),
                &list.members.join(";"),
                if archived.contains(&list.key) {
                    "1"
                } else {
                    "0"
                },
            ]));
            summary.lists += 1;
        }
        add!(
            "distribution_list.csv".into(),
            Zeroizing::new(rows.into_bytes())
        );

        // Chats: every conversation of a contact, group or list, under its family's file name,
        // and what a poll in it links to.
        let mut chats: Vec<(
            String,
            String,
            &'static str,
            &'static str,
            Option<&'static str>,
        )> = Vec::new();
        for contact in &contacts {
            let identity = contact.identity.to_string();
            chats.push((
                format!("message_{identity}.csv"),
                identity,
                "message_media_",
                "message_thumbnail_",
                Some("IdentityBallotModel"),
            ));
        }
        for group in &groups {
            chats.push((
                format!("group_message_{}.csv", group.key),
                group.key.clone(),
                "group_message_media_",
                "group_message_thumbnail_",
                Some("GroupBallotModel"),
            ));
        }
        for list in &lists {
            let id = list
                .key
                .strip_prefix("d:")
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                .unwrap_or_default()
                .cast_signed();
            chats.push((
                format!("distribution_list_message_{id}.csv"),
                list.key.clone(),
                "distribution_list_message_media_",
                "distribution_list_thumbnail_",
                None,
            ));
        }
        // What that doesn't reach wouldn't restore: said on the page.
        let listed: std::collections::HashSet<&str> =
            chats.iter().map(|chat| chat.1.as_str()).collect();
        let mut left_out = 0;
        for chat in self.store.chats_with_messages()? {
            if listed.contains(chat.as_str()) {
                continue;
            }
            if self.store.is_private(&chat)? {
                summary.private_chats_left_out += 1;
            } else {
                left_out += 1;
            }
        }
        if left_out > 0 {
            summary.skipped.insert(CHATS_WITHOUT_CONTACT, left_out);
        }
        let total = chats.len();
        let mut polls = super::polls::PollWriter::default();
        for (done, (file_name, conversation, media_prefix, thumbnail_prefix, poll_reference)) in
            chats.into_iter().enumerate()
        {
            self.emit(json!({
                "event": "androidExportProgress",
                "done": done,
                "total": total,
            }));
            let messages = self.store.conversation_messages(&conversation, u32::MAX)?;
            if messages.is_empty() {
                continue;
            }
            let mut rows = write_row(&MESSAGE_HEADER);
            let mut media: Vec<(i64, MediaPart, String)> = Vec::new();
            for message in &messages {
                let reference = poll_reference.map(|model| (model, conversation.as_str()));
                match self.message_row(message, &mut polls, reference, summary)? {
                    Some(row) => rows.push_str(&row),
                    None => continue,
                }
                summary.messages += 1;
                if message.kind == "file" {
                    let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
                    if body["fileState"] == crate::app_media::STATE_LOCAL {
                        media.push((
                            message.id,
                            MediaPart::File,
                            format!("{media_prefix}{}", message.id),
                        ));
                    }
                    if body["thumbnailState"] == crate::app_media::STATE_LOCAL {
                        media.push((
                            message.id,
                            MediaPart::Thumbnail,
                            format!("{thumbnail_prefix}{}", message.id),
                        ));
                    }
                }
            }
            add!(file_name, Zeroizing::new(rows.into_bytes()));
            summary.chats += 1;
            for (row, part, entry_name) in media {
                // Decrypt and re-encrypt on the blocking thread.
                let media_store = Arc::clone(&self.media);
                let password = self.password.clone();
                let (free_space, folder) = (self.free_space, self.directory.clone());
                let (returned, written) =
                    tokio::task::spawn_blocking(move || -> (Writer, Result<bool, ApiError>) {
                        let mut writer = writer;
                        let result = (|| {
                            // One bad file must not fail this and every later backup; it is
                            // counted below. Not being able to write, on the other hand, ends it.
                            let Ok(Some(data)) = media_store.read(row, part) else {
                                return Ok(false);
                            };
                            check_room(free_space, &folder, data.len() as u64)?;
                            let options = zip::write::SimpleFileOptions::default()
                                .compression_method(zip::CompressionMethod::Deflated)
                                .with_aes_encryption(zip::AesMode::Aes256, &password);
                            writer
                                .start_file(entry_name, options)
                                .and_then(|()| {
                                    writer.write_all(&data).map_err(zip::result::ZipError::Io)
                                })
                                .map_err(zip_error)?;
                            Ok(true)
                        })();
                        (writer, result)
                    })
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                writer = returned;
                if written? {
                    if matches!(part, MediaPart::File) {
                        summary.media_files += 1;
                    } else {
                        summary.thumbnails += 1;
                    }
                } else {
                    tracing::warn!(row, "media file left out of the backup");
                    if self.store.is_private(&conversation)? {
                        summary.private_files_left_out += 1;
                    } else {
                        *summary.skipped.entry(FILES_UNREADABLE).or_default() += 1;
                    }
                }
            }
            tokio::task::yield_now().await;
        }
        self.emit(json!({
            "event": "androidExportProgress",
            "done": total,
            "total": total,
        }));
        summary.polls = polls.count;
        for (name, text) in polls.finish() {
            add!(name.into(), Zeroizing::new(text.into_bytes()));
        }

        // Pictures: our own, the contact-defined ones, the group pictures.
        if let Ok(Some(own)) = self
            .media
            .read_picture(crate::profile_pictures::OWN_PICTURE)
        {
            add!("contact_avatar_me".into(), Zeroizing::new(own.to_vec()));
            summary.pictures += 1;
        }
        for contact in &contacts {
            let identity = contact.identity.to_string();
            if self.store.contact_picture_version(&identity)? == 0 {
                continue;
            }
            if let Ok(Some(picture)) = self
                .media
                .read_picture(&crate::profile_pictures::contact_picture_name(&identity))
            {
                add!(
                    format!("contact_profile_pic_{identity}"),
                    Zeroizing::new(picture.to_vec())
                );
                summary.pictures += 1;
            }
        }
        for group in &groups {
            if group.picture_version == 0 {
                continue;
            }
            if let Ok(Some(picture)) = self.media.read_picture(&group.key) {
                add!(
                    format!("group_avatar_{}", group.key),
                    Zeroizing::new(picture.to_vec())
                );
                summary.pictures += 1;
            }
        }

        // Reactions, in the format-26 files.
        let (contact_rows, group_rows) = self.store.reactions_for_backup(&self.me)?;
        let mut rows = write_row(&[
            "identity",
            "api_message_id",
            "sender_identity",
            "emoji_sequence",
            "reacted_at",
        ]);
        let contact_count = contact_rows.len();
        for (conversation, message_id, sender, emoji, at) in contact_rows {
            rows.push_str(&write_row(&[
                &conversation,
                &api_id_hex(message_id),
                &sender,
                &emoji,
                &at.to_string(),
            ]));
        }
        add!(
            "contact_reactions.csv".into(),
            Zeroizing::new(rows.into_bytes())
        );
        let mut rows = write_row(&[
            "api_group_id",
            "group_creator_identity",
            "api_message_id",
            "sender_identity",
            "emoji_sequence",
            "reacted_at",
        ]);
        let group_count = group_rows.len();
        for (conversation, message_id, sender, emoji, at) in group_rows {
            let Some(identity) = crate::groups::parse_key(&conversation) else {
                continue;
            };
            rows.push_str(&write_row(&[
                &crate::app_safe::safe_id(identity.group_id),
                &identity.creator_identity.to_string(),
                &api_id_hex(message_id),
                &sender,
                &emoji,
                &at.to_string(),
            ]));
        }
        add!(
            "group_reactions.csv".into(),
            Zeroizing::new(rows.into_bytes())
        );
        add!(
            "reaction_counts.csv".into(),
            Zeroizing::new(
                (write_row(&["contactReactions", "groupReactions"])
                    + &write_row(&[&contact_count.to_string(), &group_count.to_string()]))
                    .into_bytes()
            )
        );

        // Used nonces, hashed like Android's, so a restore keeps refusing replays.
        let csp = self.store.all_nonces()?;
        let d2d = self.store.all_d2d_nonces()?;
        add!(
            "nonces.csv".into(),
            Zeroizing::new(super::nonces::write(&self.me, &csp).into_bytes())
        );
        add!(
            "nonces_d2d.csv".into(),
            Zeroizing::new(super::nonces::write(&self.me, &d2d).into_bytes())
        );
        add!(
            "nonce_counts.csv".into(),
            Zeroizing::new(super::nonces::write_counts(csp.len(), d2d.len()).into_bytes())
        );
        summary.nonces = csp.len() + d2d.len();
        Ok(writer)
    }

    /// One message as an Android CSV row; `None` when the kind has no representation yet.
    fn message_row(
        &self,
        message: &Message,
        polls: &mut super::polls::PollWriter,
        poll_reference: Option<(&str, &str)>,
        summary: &mut ExportSummary,
    ) -> Result<Option<String>, ApiError> {
        let (kind, body, deleted_at) = match message.kind.as_str() {
            "text" => {
                let (quote, text) = crate::messages::split_quote(&message.body);
                return Ok(Some(self.row(
                    message,
                    "TEXT",
                    text,
                    quote.map(api_id_hex).as_deref().unwrap_or(""),
                    None,
                )));
            }
            "deleted" => ("TEXT", String::new(), message.edited_at),
            "location" => {
                let value: Value = serde_json::from_str(&message.body).unwrap_or_default();
                let body = json!([
                    value["latitude"],
                    value["longitude"],
                    value["accuracy"],
                    value["address"],
                    value["name"],
                ])
                .to_string();
                ("LOCATION", body, None)
            }
            "file" => {
                let value: Value = serde_json::from_str(&message.body).unwrap_or_default();
                let keys = self.store.file_keys(message.id)?;
                let hex =
                    |bytes: Option<&[u8]>| bytes.map(|bytes| data_encoding::HEXLOWER.encode(bytes));
                let mut metadata = serde_json::Map::new();
                for (ours, theirs) in [("duration", "d"), ("width", "w"), ("height", "h")] {
                    if let Some(number) = value[ours].as_f64() {
                        metadata.insert(theirs.into(), number.into());
                    }
                }
                if value["animated"].as_bool() == Some(true) {
                    metadata.insert("a".into(), true.into());
                }
                let body = json!([
                    hex(keys
                        .as_ref()
                        .and_then(|k| k.blob_id.as_ref())
                        .map(|b| &b.0[..])),
                    hex(keys.as_ref().map(|k| &k.key[..])),
                    value["mediaType"],
                    value["size"],
                    value["name"],
                    match value["rendering"].as_str() {
                        Some("media") => 1,
                        Some("sticker") => 2,
                        _ => 0,
                    },
                    value["fileState"] == crate::app_media::STATE_LOCAL,
                    value["caption"],
                    value["thumbnailMediaType"],
                    Value::Object(metadata),
                ])
                .to_string();
                ("FILE", body, None)
            }
            "status" if message.body.starts_with("call|") => {
                let mut parts = message.body.split('|').skip(1);
                let status = match parts.next() {
                    Some("missed") => 1,
                    Some("finished") => 2,
                    Some("rejected" | "busy" | "unanswered" | "disabled") => 3,
                    _ => 4,
                };
                let _outgoing = parts.next();
                let duration: i64 = parts.next().and_then(|d| d.parse().ok()).unwrap_or(0);
                (
                    "VOIP_STATUS",
                    json!([1, {"status": status, "duration": duration}]).to_string(),
                    None,
                )
            }
            "status" => {
                *summary.skipped.entry("status").or_default() += 1;
                return Ok(None);
            }
            "poll" | "pollClosed" => {
                let state: Value = serde_json::from_str(&message.body).unwrap_or_default();
                let body = poll_reference.and_then(|reference| {
                    polls.message_body(
                        &message.conversation,
                        reference,
                        &message.kind,
                        &state,
                        message.created_at,
                        message.updated_at,
                    )
                });
                let Some(body) = body else {
                    *summary.skipped.entry("poll").or_default() += 1;
                    return Ok(None);
                };
                ("BALLOT", body, None)
            }
            _ => {
                *summary.skipped.entry("other").or_default() += 1;
                return Ok(None);
            }
        };
        Ok(Some(self.row(message, kind, &body, "", deleted_at)))
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        message: &Message,
        kind: &str,
        body: &str,
        quoted: &str,
        deleted_at: Option<i64>,
    ) -> String {
        let ms = |value: Option<i64>| value.map(|v| v.to_string()).unwrap_or_default();
        let state = if message.outgoing {
            match message.state {
                s if s == OutgoingState::Pending as i64 => "PENDING",
                s if s == OutgoingState::Sent as i64 => "SENT",
                s if s == OutgoingState::Delivered as i64 => "DELIVERED",
                s if s == OutgoingState::Failed as i64 => "SENDFAILED",
                _ => "READ",
            }
        } else {
            "READ"
        };
        write_row(&[
            // Local-only rows (1:1 call lines) carry their random local message id: it is
            // stable across exports, so importing a backup of this device back into it (or a
            // newer backup of the same store) finds the row instead of duplicating it.
            // Android ignores the api id of status rows. (A backup written by Android has an
            // empty cell here; those rows get fingerprint-keyed synthetic ids on import.)
            &api_id_hex(message.message_id),
            &message.id.to_string(),
            if message.outgoing { "1" } else { "0" },
            if message.outgoing || message.state == crate::messages::INCOMING_READ {
                "1"
            } else {
                "0"
            },
            "1",
            state,
            message.sender.as_deref().unwrap_or(""),
            kind,
            body,
            &message.created_at.to_string(),
            &message.created_at.to_string(),
            &message.updated_at.to_string(),
            "0",
            "",
            quoted,
            &ms(message.delivered_at),
            &ms(message.read_at),
            "",
            if message.starred { "1" } else { "0" },
            &ms(if message.kind == "deleted" {
                None
            } else {
                message.edited_at
            }),
            &ms(deleted_at),
        ])
    }
}

/// The same header the importer reads (Android's order).
const MESSAGE_HEADER: [&str; 21] = [
    "apiid",
    "uid",
    "isoutbox",
    "isread",
    "issaved",
    "messagestae",
    "identity",
    "type",
    "body",
    "posted_at",
    "created_at",
    "modified_at",
    "isstatusmessage",
    "caption",
    "quoted_message_apiid",
    "delivered_at",
    "read_at",
    "g_msg_states",
    "display_tags",
    "edited_at",
    "deleted_at",
];

/// Threema's message-id string: the 8 bytes in wire order as lowercase hex.
fn api_id_hex(id: u64) -> String {
    data_encoding::HEXLOWER.encode(&id.to_le_bytes())
}

impl Store {
    /// All reactions as the backup's rows: `(conversation, message id, sender identity,
    /// emoji, reacted at)`, our own reactions under our identity. Split 1:1 vs groups.
    #[allow(clippy::type_complexity)]
    pub fn reactions_for_backup(
        &self,
        me: &str,
    ) -> Result<
        (
            Vec<(String, u64, String, String, i64)>,
            Vec<(String, u64, String, String, i64)>,
        ),
        StoreError,
    > {
        let mut statement = self.conn.prepare(
            "SELECT m.conversation, m.message_id, r.sender, r.emoji, r.reacted_at
             FROM reactions r JOIN messages m ON m.id = r.message",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.cast_unsigned(),
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut contact_rows = Vec::new();
        let mut group_rows = Vec::new();
        for row in rows {
            let (conversation, message_id, sender, emoji, at) = row?;
            let sender = if sender.is_empty() {
                me.to_owned()
            } else {
                sender
            };
            let entry = (conversation.clone(), message_id, sender, emoji, at);
            if crate::groups::parse_key(&conversation).is_some() {
                group_rows.push(entry);
            } else if !crate::distribution_lists::is_key(&conversation) {
                contact_rows.push(entry);
            }
        }
        Ok((contact_rows, group_rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only chats without a row are asked about, and nobody while the directory says to rest.
    #[test]
    fn who_is_looked_up_for_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store"), &[1; 32]).unwrap();
        store
            .upsert_contact(&crate::contacts::test_contact("BBBB2222"))
            .unwrap();
        for (id, chat) in [(1, "AAAA1111"), (2, "BBBB2222"), (3, "MEME0000")] {
            store
                .insert_incoming(chat, id, "text", "words", 1_800_000_000_000)
                .unwrap();
        }
        assert_eq!(
            contacts_to_look_up(&store, "MEME0000").unwrap(),
            ["AAAA1111"]
        );
        store.rest_directory_calls();
        assert!(contacts_to_look_up(&store, "MEME0000").unwrap().is_empty());
    }
}
