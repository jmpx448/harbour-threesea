// SPDX-License-Identifier: AGPL-3.0-only
//! Files and media: the blob server client, the encrypted on-device media store and the blob
//! keys of file messages.
//!
//! - Blob server (as `BlobUploader`/`BlobLoader` on Android, without multi-device mirror):
//!   upload `POST https://ds-blobp-upload.threema.ch/upload?persist=0` (`1` for a group's files;
//!   multipart field `blob`, answer: blob id in hex), download
//!   `GET https://ds-blobp-{first byte hex}.threema.ch/{id}`, then `POST …/{id}/done` (received
//!   1:1 messages only) so the server can delete it.
//! - Blobs are encrypted with the message's random key: XSalsa20-Poly1305, nonce `00..01` for the
//!   file and `00..02` for the thumbnail.
//! - On the device every file is stored encrypted (CLAUDE.md §7: nothing readable at rest), with
//!   a key derived from the database key, a random nonce per file. The bridge reads them through
//!   [`MediaStore::read`], which is thread-safe (Qt image providers run on their own threads).

use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use crypto_secretbox::{
    KeyInit as _, XSalsa20Poly1305,
    aead::{Aead as _, AeadCore as _, OsRng},
};
use data_encoding::HEXLOWER;
use libthreema::{
    common::BlobId,
    https::{HttpsHeader, HttpsMethod, HttpsRequest},
};
use rusqlite::{OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    https::HttpsClient,
    store::{Store, StoreError},
};

/// Schema for the blob keys of file messages (store migration 7).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE files (
        message            INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
        key                BLOB NOT NULL,
        blob_id            BLOB,
        thumbnail_blob_id  BLOB
    );";

/// Largest blob we download or upload (Threema's limit).
pub const MAX_BLOB_SIZE: usize = 100 * 1024 * 1024;

/// Largest encrypted message file fetched: its size plus the secretbox tag.
pub const MAX_FILE_DOWNLOAD: usize = MAX_BLOB_SIZE + 16;

/// Largest contact or group picture, or message thumbnail, fetched. These come without anyone
/// asking, also on mobile data, and senders scale them down.
pub const MAX_PICTURE_DOWNLOAD: usize = 10 * 1024 * 1024;

/// Contact and group pictures downloading at once; apart from the message media's own slots, so
/// a stranger's pictures can't hold those up.
pub static PICTURE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Wait for one of `slots`; `None` if the work isn't `wanted` any more by then (a newer picture
/// came while it waited).
pub async fn slot_if_wanted(
    slots: &tokio::sync::Semaphore,
    wanted: impl FnOnce() -> bool,
) -> Option<tokio::sync::SemaphorePermit<'_>> {
    let permit = slots.acquire().await.ok()?;
    wanted().then_some(permit)
}

const UPLOAD_URL: &str = "https://ds-blobp-upload.threema.ch/upload";
const MULTIPART_BOUNDARY: &str = "---------------------------Boundary_Line";
/// How long a blob's transfer may go without moving, as Android's read timeout. The whole may take
/// as long as the link needs.
const BLOB_STALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(100);
const DONE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Nonce of the file blob.
pub const FILE_NONCE: [u8; 24] = {
    let mut nonce = [0; 24];
    nonce[23] = 1;
    nonce
};
/// Nonce of the thumbnail blob.
pub const THUMBNAIL_NONCE: [u8; 24] = {
    let mut nonce = [0; 24];
    nonce[23] = 2;
    nonce
};

/// Errors of the media subsystem.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("blob server: {0}")]
    Server(String),
    #[error("blob server: status {0}")]
    Status(u16),
    #[error("network: {0}")]
    Network(String),
    #[error("decryption failed")]
    Decrypt,
    #[error("file too large")]
    TooLarge,
    /// The file to upload is no longer on the device: no later try can bring it back.
    #[error("file is gone")]
    FileGone,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl MediaError {
    /// Whether the same transfer may work later: not what the server refused (but for its "not
    /// now"s), nor what is wrong on this side.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::Server(_) => true,
            Self::Status(status) => matches!(status, 408 | 429 | 500..=599),
            Self::Decrypt | Self::TooLarge | Self::FileGone | Self::Io(_) => false,
        }
    }
}

/// Which of a message's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPart {
    File,
    Thumbnail,
}

impl MediaPart {
    fn extension(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Thumbnail => "thumb",
        }
    }

    /// The blob nonce of this part.
    pub fn nonce(self) -> &'static [u8; 24] {
        match self {
            Self::File => &FILE_NONCE,
            Self::Thumbnail => &THUMBNAIL_NONCE,
        }
    }
}

/// Encrypted files on the device, one per message and part.
pub struct MediaStore {
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
    /// While private chats are hidden: their messages, whose media the UI may not read
    /// (`None`: nothing hidden). The core keeps it current; the readers for the UI, which run
    /// on other threads without the database, consult it (ADR 0009; deep audit D02).
    hidden: std::sync::RwLock<Option<std::collections::HashSet<i64>>>,
    /// "Show profile pictures": whether the UI may read contacts' pictures. The same readers.
    contact_pictures_shown: std::sync::atomic::AtomicBool,
}

const MEDIA_MAGIC: &[u8; 4] = b"3SM1";
const NONCE_LENGTH: usize = 24;

impl MediaStore {
    /// Media live in `<data_dir>/media`; the key is derived from the database key.
    pub fn open(data_dir: &Path, db_key: &[u8; 32]) -> std::io::Result<Self> {
        let dir = data_dir.join("media");
        std::fs::create_dir_all(&dir)?;
        let mut hash = Sha256::new();
        hash.update(b"threesea media key v1");
        hash.update(db_key);
        let key = Zeroizing::new(hash.finalize().into());
        Ok(Self {
            dir,
            key,
            hidden: std::sync::RwLock::new(None),
            contact_pictures_shown: std::sync::atomic::AtomicBool::new(true),
        })
    }

    /// Private chats hidden (with these messages) or shown (`None`).
    pub fn set_hidden(&self, messages: Option<std::collections::HashSet<i64>>) {
        *self
            .hidden
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = messages;
    }

    /// One more message of a hidden private chat.
    pub fn hide(&self, message: i64) {
        if let Some(hidden) = self
            .hidden
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            hidden.insert(message);
        }
    }

    /// Whether the UI may read `message`'s media now.
    pub fn shown(&self, message: i64) -> bool {
        self.hidden
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_none_or(|hidden| !hidden.contains(&message))
    }

    fn path(&self, message: i64, part: MediaPart) -> PathBuf {
        self.dir.join(format!("{message}.{}", part.extension()))
    }

    /// Pictures that don't belong to a message (group pictures): file named by a hash of `name`.
    fn picture_path(&self, name: &str) -> PathBuf {
        let hash = Sha256::digest(name.as_bytes());
        self.dir
            .join(format!("pic-{}.picture", HEXLOWER.encode(&hash[..16])))
    }

    pub fn write_picture(&self, name: &str, data: &[u8]) -> Result<(), MediaError> {
        self.write_to(&self.picture_path(name), data)
    }

    pub fn read_picture(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, MediaError> {
        self.read_from(&self.picture_path(name))
    }

    /// What the UI reads of a picture: contacts' only while they are shown (Settings; our own
    /// and groups' always). What the core does with them isn't affected.
    pub fn read_picture_for_ui(
        &self,
        name: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, MediaError> {
        let shown = self
            .contact_pictures_shown
            .load(std::sync::atomic::Ordering::Relaxed);
        if !shown && crate::profile_pictures::is_contact_picture(name) {
            return Ok(None);
        }
        self.read_picture(name)
    }

    pub fn set_contact_pictures_shown(&self, shown: bool) {
        self.contact_pictures_shown
            .store(shown, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn remove_picture(&self, name: &str) {
        let _ = std::fs::remove_file(self.picture_path(name));
    }

    fn cipher(&self) -> XSalsa20Poly1305 {
        XSalsa20Poly1305::new(self.key.as_ref().into())
    }

    /// Store `data` encrypted (atomically: temporary file, then rename).
    pub fn write(&self, message: i64, part: MediaPart, data: &[u8]) -> Result<(), MediaError> {
        self.write_to(&self.path(message, part), data)
    }

    fn write_to(&self, path: &Path, data: &[u8]) -> Result<(), MediaError> {
        let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher()
            .encrypt(&nonce, data)
            .map_err(|_| MediaError::Decrypt)?;
        let temporary = path.with_extension("tmp");
        let written = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(MEDIA_MAGIC)?;
            file.write_all(&nonce)?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        // A full disk would stay full by what the failed write left.
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        Ok(written?)
    }

    /// The decrypted content, `None` if not (yet) on the device.
    pub fn read(
        &self,
        message: i64,
        part: MediaPart,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, MediaError> {
        self.read_from(&self.path(message, part))
    }

    fn read_from(&self, path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, MediaError> {
        let stored = match std::fs::read(path) {
            Ok(stored) => stored,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let header = MEDIA_MAGIC.len() + NONCE_LENGTH;
        if stored.len() < header || &stored[..MEDIA_MAGIC.len()] != MEDIA_MAGIC {
            return Err(MediaError::Decrypt);
        }
        let nonce = &stored[MEDIA_MAGIC.len()..header];
        self.cipher()
            .decrypt(nonce.into(), &stored[header..])
            .map(|data| Some(Zeroizing::new(data)))
            .map_err(|_| MediaError::Decrypt)
    }

    pub fn has(&self, message: i64, part: MediaPart) -> bool {
        self.path(message, part).exists()
    }

    /// Remove both parts of a message.
    /// One part of a message's media (storage clean-up keeps the thumbnail).
    pub fn remove_part(&self, message: i64, part: MediaPart) {
        let _ = std::fs::remove_file(self.path(message, part));
    }

    /// Bytes used by all media and pictures.
    pub fn size(&self) -> u64 {
        std::fs::read_dir(&self.dir).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .filter(std::fs::Metadata::is_file)
                .map(|m| m.len())
                .sum()
        })
    }

    pub fn remove(&self, message: i64) {
        for part in [MediaPart::File, MediaPart::Thumbnail] {
            let _ = std::fs::remove_file(self.path(message, part));
        }
    }

    /// Remove every stored file — all message media, pictures and GIFs ("Delete ID and data").
    pub fn remove_all(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }

    /// Remove what writes cut short by the end of the process. Only at startup, before anything
    /// writes: later on a temporary file may be a write in progress.
    pub fn remove_temporary_files(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "tmp") {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Remove files whose message no longer exists (deleted while we were not looking).
    pub fn remove_orphans(&self, exists: impl Fn(i64) -> bool) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.split('.').next())
                .and_then(|id| id.parse::<i64>().ok())
            else {
                continue;
            };
            if !exists(id) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Encrypt a blob with a file message key.
pub fn encrypt_blob(key: &[u8; 32], part: MediaPart, data: &[u8]) -> Vec<u8> {
    XSalsa20Poly1305::new(key.into())
        .encrypt(part.nonce().into(), data)
        .expect("XSalsa20Poly1305 encryption cannot fail")
}

/// Decrypt a blob with a file message key.
/// Decrypt a blob with an explicit nonce (sync messages carry theirs).
pub fn decrypt_blob_with_nonce(
    key: &[u8; 32],
    nonce: &[u8; 24],
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    XSalsa20Poly1305::new(key.into())
        .decrypt(nonce.into(), data)
        .map(Zeroizing::new)
        .map_err(|_| MediaError::Decrypt)
}

pub fn decrypt_blob(
    key: &[u8; 32],
    part: MediaPart,
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    XSalsa20Poly1305::new(key.into())
        .decrypt(part.nonce().into(), data)
        .map(Zeroizing::new)
        .map_err(|_| MediaError::Decrypt)
}

/// `https://ds-blobp-{prefix}.threema.ch/{id}`: the prefix is the first byte as two hex digits
/// (Android's `BlobUrl`; libthreema's template drops the leading zero).
pub fn download_url(blob_id: &BlobId) -> String {
    let hex = HEXLOWER.encode(&blob_id.0);
    format!("https://ds-blobp-{}.threema.ch/{hex}", &hex[..2])
}

/// Blob mirror scope (multi-device; Android's `BlobScope`): `Local` blobs stay on the mirror for
/// the device group, `Public` ones are also on the usual blob server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobScope {
    Local,
    Public,
}

impl BlobScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Public => "public",
        }
    }
}

/// The device group's blob mirror (multi-device active), set by the app.
struct Mirror {
    device_group_id: [u8; 32],
    device_id: u64,
}

static MIRROR: std::sync::RwLock<Option<Mirror>> = std::sync::RwLock::new(None);

/// Multi-device on (device group key and our mediator device id) or off: from now on every blob
/// goes through the device group's mirror, or the usual blob server.
pub fn set_mirror(device_group: Option<(&libthreema::common::keys::DeviceGroupKey, u64)>) {
    let mirror = device_group.map(|(key, device_id)| Mirror {
        device_group_id: key.device_group_id(),
        device_id,
    });
    if let Ok(mut current) = MIRROR.write() {
        *current = mirror;
    }
}

/// Is multi-device on, so that blobs go through the device group's mirror?
pub fn mirror_active() -> bool {
    MIRROR.read().is_ok_and(|mirror| mirror.is_some())
}

/// The device group whose mirror is in use.
#[cfg(test)]
pub(crate) fn mirror_device_group() -> Option<[u8; 32]> {
    MIRROR
        .read()
        .ok()?
        .as_ref()
        .map(|mirror| mirror.device_group_id)
}

/// `path` on the mirror (`upload`, `{blob}`, `{blob}/done`) with Android's query parameters,
/// or `None` without multi-device.
fn mirror_url(path: &str, scope: BlobScope) -> Option<String> {
    let mirror = MIRROR.read().ok()?;
    let mirror = mirror.as_ref()?;
    let prefix = mirror.device_group_id[0];
    Some(format!(
        "https://blob-mirror-{:x}.threema.ch/{prefix:02x}/{path}?deviceId={}&deviceGroupId={}&scope={}",
        prefix >> 4,
        HEXLOWER.encode(&mirror.device_id.to_le_bytes()),
        HEXLOWER.encode(&mirror.device_group_id),
        scope.as_str()
    ))
}

fn network(error: libthreema::https::HttpsError) -> MediaError {
    MediaError::Network(error.to_string())
}

/// Download a blob of at most `limit` bytes (through the mirror with multi-device, in `scope`).
pub async fn download(
    https: &HttpsClient,
    blob_id: &BlobId,
    scope: BlobScope,
    limit: usize,
) -> Result<Vec<u8>, MediaError> {
    let url =
        mirror_url(&HEXLOWER.encode(&blob_id.0), scope).unwrap_or_else(|| download_url(blob_id));
    let request = HttpsRequest {
        timeout: BLOB_STALL_LIMIT,
        url,
        method: HttpsMethod::Get,
        headers: Vec::new(),
        body: Vec::new(),
    };
    let response = https.transfer(request, limit).await.map_err(|error| {
        if crate::https::is_too_large(&error) {
            MediaError::TooLarge
        } else {
            network(error)
        }
    })?;
    if response.status != 200 {
        return Err(MediaError::Status(response.status));
    }
    Ok(response.body)
}

/// Tell the blob server the blob was downloaded (it may delete it; the mirror keeps it for the
/// device group as `scope` says).
pub async fn mark_done(https: &HttpsClient, blob_id: &BlobId, scope: BlobScope) {
    let url = mirror_url(&format!("{}/done", HEXLOWER.encode(&blob_id.0)), scope)
        .unwrap_or_else(|| format!("{}/done", download_url(blob_id)));
    let result = https
        .send(HttpsRequest {
            timeout: DONE_TIMEOUT,
            url,
            method: HttpsMethod::Post,
            headers: Vec::new(),
            body: Vec::new(),
        })
        .await;
    match result {
        // The mirror answers 204 No Content.
        Ok(response) if response.status == 200 || response.status == 204 => {}
        Ok(response) => tracing::warn!(status = response.status, "marking blob done failed"),
        Err(error) => tracing::warn!(%error, "marking blob done failed"),
    }
}

/// Upload an (encrypted) blob; with multi-device, to the mirror in `scope` (sync pictures:
/// local, only the device group fetches them). `persist` as in [`upload`].
pub async fn upload_scoped(
    https: &HttpsClient,
    data: &[u8],
    persist: bool,
    scope: BlobScope,
) -> Result<BlobId, MediaError> {
    upload_to(https, data, persist, scope).await
}

/// Upload an (encrypted) blob; returns its id.
/// `persist`: keep the blob after downloads (group pictures, which every member downloads and
/// which are re-sent on group syncs); a 1:1 message's blob is removed once fetched.
pub async fn upload(https: &HttpsClient, data: &[u8], persist: bool) -> Result<BlobId, MediaError> {
    upload_to(https, data, persist, BlobScope::Public).await
}

async fn upload_to(
    https: &HttpsClient,
    data: &[u8],
    persist: bool,
    scope: BlobScope,
) -> Result<BlobId, MediaError> {
    if data.len() > MAX_BLOB_SIZE + 16 {
        return Err(MediaError::TooLarge);
    }
    let mut body = format!(
        "--{MULTIPART_BOUNDARY}\r\nContent-Disposition: form-data; name=\"blob\"; \
         filename=\"blob.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{MULTIPART_BOUNDARY}--\r\n").as_bytes());
    let response = https
        .transfer(
            HttpsRequest {
                timeout: BLOB_STALL_LIMIT,
                // With multi-device through the mirror (public: contacts fetch it too).
                url: mirror_url("upload", scope).map_or_else(
                    || format!("{UPLOAD_URL}?persist={}", u8::from(persist)),
                    |url| format!("{url}&persist={}", u8::from(persist)),
                ),
                method: HttpsMethod::Post,
                headers: vec![HttpsHeader {
                    name: "content-type".into(),
                    value: format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}"),
                }],
                body,
            },
            crate::https::MAX_RESPONSE,
        )
        .await
        .map_err(network)?;
    if response.status != 200 {
        return Err(MediaError::Status(response.status));
    }
    let hex = String::from_utf8_lossy(&response.body);
    data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.trim().as_bytes())
        .ok()
        .and_then(|id| <[u8; BlobId::LENGTH]>::try_from(id).ok())
        .map(BlobId)
        .ok_or_else(|| MediaError::Server("invalid blob id".into()))
}

/// The blob key and ids of a file message.
#[derive(Clone)]
pub struct FileKeys {
    pub key: Zeroizing<[u8; 32]>,
    pub blob_id: Option<BlobId>,
    pub thumbnail_blob_id: Option<BlobId>,
}

fn blob_id(bytes: Option<Vec<u8>>) -> Option<BlobId> {
    bytes
        .and_then(|bytes| <[u8; BlobId::LENGTH]>::try_from(bytes).ok())
        .map(BlobId)
}

impl Store {
    pub fn set_file_keys(&self, message: i64, keys: &FileKeys) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (message, key, blob_id, thumbnail_blob_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                message,
                keys.key.as_slice(),
                keys.blob_id.map(|id| id.0.to_vec()),
                keys.thumbnail_blob_id.map(|id| id.0.to_vec())
            ],
        )?;
        Ok(())
    }

    pub fn file_keys(&self, message: i64) -> Result<Option<FileKeys>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT key, blob_id, thumbnail_blob_id FROM files WHERE message = ?1",
                [message],
                |row| {
                    // Read in place, so no copy of the key stays behind in freed memory.
                    let mut key = Zeroizing::new([0; 32]);
                    let usable = row.get_ref(0)?.as_blob().ok().and_then(|blob| {
                        let source = <&[u8; 32]>::try_from(blob).ok()?;
                        key.copy_from_slice(source);
                        Some(())
                    });
                    Ok((usable.map(|()| key), row.get(1)?, row.get(2)?))
                },
            )
            .optional()?
            .and_then(|(key, file, thumbnail)| {
                Some(FileKeys {
                    key: key?,
                    blob_id: blob_id(file),
                    thumbnail_blob_id: blob_id(thumbnail),
                })
            }))
    }

    /// Does a message with this local id still count for its media? "Message deleted" doesn't: a
    /// download that outlived its deletion once could leave its file behind.
    pub fn message_keeps_media(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM messages WHERE id = ?1 AND kind != 'deleted'",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Does a message with this local id exist?
    pub fn message_exists(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM messages WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twenty pictures in a row: two run at a time, and those superseded while waiting for a
    /// slot do nothing.
    #[tokio::test(flavor = "current_thread")]
    async fn picture_slots_bound_the_work_and_skip_what_is_superseded() {
        use std::{cell::Cell, rc::Rc};
        static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
        let newest = Rc::new(Cell::new(0));
        let running = Rc::new(Cell::new(0));
        let most = Rc::new(Cell::new(0));
        let done = Rc::new(Cell::new(0));
        let local = tokio::task::LocalSet::new();
        for generation in 1..=20 {
            newest.set(generation);
            let (newest, running, most, done) = (
                Rc::clone(&newest),
                Rc::clone(&running),
                Rc::clone(&most),
                Rc::clone(&done),
            );
            local.spawn_local(async move {
                let wanted = || newest.get() == generation;
                let Some(_slot) = slot_if_wanted(&SLOTS, wanted).await else {
                    return;
                };
                running.set(running.get() + 1);
                most.set(most.get().max(running.get()));
                tokio::task::yield_now().await;
                running.set(running.get() - 1);
                done.set(done.get() + 1);
            });
        }
        local.await;
        assert_eq!(done.get(), 1, "only the newest was worked on");
        assert_eq!(most.get(), 1);

        // Still wanted: at most two at once, then every one of them.
        let (running, most, done) = (
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0)),
        );
        let local = tokio::task::LocalSet::new();
        for _ in 0..20 {
            let (running, most, done) = (Rc::clone(&running), Rc::clone(&most), Rc::clone(&done));
            local.spawn_local(async move {
                let _slot = slot_if_wanted(&SLOTS, || true).await.unwrap();
                running.set(running.get() + 1);
                most.set(most.get().max(running.get()));
                tokio::task::yield_now().await;
                running.set(running.get() - 1);
                done.set(done.get() + 1);
            });
        }
        local.await;
        assert_eq!(done.get(), 20);
        assert_eq!(most.get(), 2);
    }

    #[test]
    fn media_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let media = MediaStore::open(dir.path(), &[7; 32]).unwrap();
        assert!(media.read(1, MediaPart::File).unwrap().is_none());
        media.write(1, MediaPart::File, b"picture").unwrap();
        media.write(1, MediaPart::Thumbnail, b"thumb").unwrap();
        assert_eq!(
            media.read(1, MediaPart::File).unwrap().unwrap().as_slice(),
            b"picture"
        );
        // Encrypted at rest.
        let raw = std::fs::read(dir.path().join("media/1.file")).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"picture"));
        // Another key can't read it.
        let other = MediaStore::open(dir.path(), &[8; 32]).unwrap();
        assert!(other.read(1, MediaPart::File).is_err());
        media.write(2, MediaPart::File, b"x").unwrap();
        media.remove_orphans(|id| id == 2);
        assert!(!media.has(1, MediaPart::File) && !media.has(1, MediaPart::Thumbnail));
        assert!(media.has(2, MediaPart::File));
        media.remove(2);
        assert!(!media.has(2, MediaPart::File));
    }

    #[test]
    fn what_may_work_later() {
        for transient in [
            MediaError::Network("timeout".into()),
            MediaError::Server("invalid blob id".into()),
            MediaError::Status(408),
            MediaError::Status(429),
            MediaError::Status(500),
            MediaError::Status(503),
        ] {
            assert!(transient.is_transient(), "{transient}");
        }
        for last in [
            MediaError::Status(307),
            MediaError::Status(400),
            MediaError::Status(404),
            MediaError::Status(413),
            MediaError::Decrypt,
            MediaError::TooLarge,
            MediaError::FileGone,
            MediaError::Io(std::io::ErrorKind::StorageFull.into()),
        ] {
            assert!(!last.is_transient(), "{last}");
        }
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let media = MediaStore::open(dir.path(), &[7; 32]).unwrap();
        // The last step of the write can't work: a directory is in the way.
        std::fs::create_dir(dir.path().join("media/1.file")).unwrap();
        assert!(media.write(1, MediaPart::File, b"picture").is_err());
        assert!(!dir.path().join("media/1.tmp").exists());
    }

    #[test]
    fn a_write_cut_short_is_removed_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let media = MediaStore::open(dir.path(), &[7; 32]).unwrap();
        media.write(1, MediaPart::File, b"picture").unwrap();
        media.write_picture("group:x", b"group picture").unwrap();
        let stored = media.size();
        std::fs::write(dir.path().join("media/1.tmp"), b"cut short").unwrap();
        std::fs::write(dir.path().join("media/pic-0123.tmp"), b"cut short").unwrap();
        assert!(media.size() > stored);
        media.remove_temporary_files();
        assert_eq!(media.size(), stored);
        assert_eq!(
            media.read(1, MediaPart::File).unwrap().unwrap().as_slice(),
            b"picture"
        );
        assert!(media.read_picture("group:x").unwrap().is_some());
    }

    #[test]
    fn remove_all_empties_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let media = MediaStore::open(dir.path(), &[7; 32]).unwrap();
        media.write(1, MediaPart::File, b"picture").unwrap();
        media.write(1, MediaPart::Thumbnail, b"thumb").unwrap();
        media.write_picture("group:x", b"group picture").unwrap();
        media.remove_all();
        assert_eq!(media.size(), 0);
        assert!(media.read(1, MediaPart::File).unwrap().is_none());
        assert!(media.read_picture("group:x").unwrap().is_none());
    }

    #[test]
    fn blob_crypto_and_urls() {
        let key = [3; 32];
        let blob = encrypt_blob(&key, MediaPart::Thumbnail, b"data");
        assert!(decrypt_blob(&key, MediaPart::File, &blob).is_err());
        assert_eq!(
            decrypt_blob(&key, MediaPart::Thumbnail, &blob)
                .unwrap()
                .as_slice(),
            b"data"
        );
        let mut id = [0xab; 16];
        id[0] = 0x0a;
        assert_eq!(
            download_url(&BlobId(id)),
            "https://ds-blobp-0a.threema.ch/0aababababababababababababababab"
        );
    }

    #[test]
    fn file_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let message = store.insert_outgoing("ECHOECHO", 1, "file", "{}").unwrap();
        assert!(store.file_keys(message.id).unwrap().is_none());
        let keys = FileKeys {
            key: Zeroizing::new([5; 32]),
            blob_id: Some(BlobId([1; 16])),
            thumbnail_blob_id: None,
        };
        store.set_file_keys(message.id, &keys).unwrap();
        let back = store.file_keys(message.id).unwrap().unwrap();
        assert_eq!(*back.key, [5; 32]);
        assert_eq!(back.blob_id, Some(BlobId([1; 16])));
        assert_eq!(back.thumbnail_blob_id, None);
        // A key of the wrong size is no key.
        store
            .conn
            .execute(
                "UPDATE files SET key = x'0505' WHERE message = ?1",
                [message.id],
            )
            .unwrap();
        assert!(store.file_keys(message.id).unwrap().is_none());
        store.remove_message(message.id).unwrap();
        assert!(store.file_keys(message.id).unwrap().is_none());
    }
}
