// SPDX-License-Identifier: AGPL-3.0-only
//! Files and media in conversations: receiving file messages, downloading their blobs into the
//! encrypted media store, and removing media together with their messages.
//!
//! A file message's `body` (UI JSON) carries `fileState` / `thumbnailState`:
//! 0 on the server, 1 downloading, 2 on the device, 3 failed; outgoing ones also `uploadState`:
//! 1 uploading, 2 uploaded (the message can be sent), 3 failed.
//!
//! Sending: the file is first stored in the (encrypted) media store, then uploaded from there,
//! so a failed upload can be retried after a restart without the original file.

use std::{rc::Rc, sync::Arc};

use libthreema::model::message::{FileMessage, FileRenderingType};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App},
    app_messages::Conversation,
    https::HttpsClient,
    media::{self, FileKeys, MediaPart, MediaStore},
    messages::{Message, now_ms},
    providers::CoreEvent,
    store::{Store, StoreError, keys},
};

pub const STATE_REMOTE: i64 = 0;
pub const STATE_LOADING: i64 = 1;
pub const STATE_LOCAL: i64 = 2;
pub const STATE_FAILED: i64 = 3;

fn state_field(part: MediaPart) -> &'static str {
    match part {
        MediaPart::File => "$.fileState",
        MediaPart::Thumbnail => "$.thumbnailState",
    }
}

/// The largest file name / caption the protocol allows (bytes).
const NAME_LENGTH_MAX: usize = 256;
const CAPTION_LENGTH_MAX: usize = 6000;

/// A file the user wants to send (see `sendFile` in docs/ffi.md).
pub struct OutgoingFile {
    pub identity: String,
    pub path: Option<String>,
    pub name: Option<String>,
    pub media_type: String,
    pub caption: Option<String>,
    pub rendering: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub encode: Option<String>,
}

/// The file message to send for a stored outgoing file message, once uploaded.
pub fn outgoing_file_message(body: &Value, keys: &FileKeys) -> Option<FileMessage> {
    if body["uploadState"].as_i64() != Some(STATE_LOCAL) {
        return None;
    }
    let number = |field: &str| body[field].as_u64().and_then(|n| u32::try_from(n).ok());
    let text = |field: &str| {
        body[field]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Some(FileMessage {
        rendering_type: match body["rendering"].as_str() {
            Some("media") => FileRenderingType::Media,
            Some("sticker") => FileRenderingType::Sticker,
            _ => FileRenderingType::File,
        },
        key: *keys.key,
        blob_id: keys.blob_id?,
        media_type: body["mediaType"]
            .as_str()
            .unwrap_or("application/octet-stream")
            .to_owned(),
        name: text("name"),
        size: body["size"].as_u64()?,
        thumbnail_blob_id: keys.thumbnail_blob_id,
        thumbnail_media_type: keys.thumbnail_blob_id.map(|_| "image/jpeg".to_owned()),
        caption: text("caption"),
        correlation_id: None,
        metadata: libthreema::model::message::FileMetadata {
            width: number("width"),
            height: number("height"),
            animated: body["animated"].as_bool().filter(|animated| *animated),
            duration: body["duration"]
                .as_f64()
                .filter(|d| d.is_finite() && *d >= 0.0),
        },
    })
}

/// The UI JSON of a received file message (blob keys go to the `files` table, not here).
pub fn file_body(file: &FileMessage) -> Value {
    json!({
        "rendering": match file.rendering_type {
            FileRenderingType::File => "file",
            FileRenderingType::Media => "media",
            FileRenderingType::Sticker => "sticker",
        },
        "mediaType": file.media_type,
        "name": file.name.as_deref().map(crate::file_types::without_bidi_controls),
        "size": file.size,
        "caption": file.caption,
        "width": file.metadata.width,
        "height": file.metadata.height,
        "duration": file.metadata.duration,
        "animated": file.metadata.animated,
        "thumbnail": file.thumbnail_blob_id.is_some(),
        "thumbnailMediaType": file.thumbnail_media_type.clone().unwrap_or_else(|| "image/jpeg".into()),
        "fileState": STATE_REMOTE,
        "thumbnailState": if file.thumbnail_blob_id.is_some() { STATE_REMOTE } else { STATE_FAILED },
    })
}

/// The blob keys of a received file message.
pub fn file_keys(file: &FileMessage) -> FileKeys {
    FileKeys {
        key: Zeroizing::new(file.key),
        blob_id: Some(file.blob_id),
        thumbnail_blob_id: file.thumbnail_blob_id,
    }
}

impl Store {
    /// File messages created before `before` (ms) whose file is on the device, not starred.
    pub fn files_before(&self, before: i64) -> Result<Vec<i64>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM messages WHERE kind = 'file' AND created_at < ?1 AND starred = 0
               AND json_extract(body, '$.fileState') = ?2",
        )?;
        let rows = statement.query_map(params![before, STATE_LOCAL], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Delete messages created before `before` (ms), except starred ones and status lines.
    /// Returns how many.
    /// Messages older than `before`, except starred ones, status lines, messages still on
    /// their way out and polls that are still open (Android's auto-delete keeps those two as
    /// well: a message nobody has seen and a vote still running are not old news).
    pub fn delete_messages_before(&self, before: i64) -> Result<usize, StoreError> {
        Ok(self.conn.execute(
            "DELETE FROM messages
             WHERE created_at < ?1 AND starred = 0 AND kind != 'status'
               AND NOT (outgoing = 1 AND state = ?2)
               AND NOT (kind = 'poll' AND json_extract(body, '$.state') = 0)",
            params![before, crate::messages::OutgoingState::Pending as i64],
        )?)
    }

    /// Set `fileState`/`thumbnailState` of a file message; returns the changed message.
    pub fn set_media_state(
        &self,
        message: i64,
        part: MediaPart,
        state: i64,
    ) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, ?2, ?3), updated_at = ?4
             WHERE id = ?1 AND kind = 'file'",
            params![message, state_field(part), state, now_ms()],
        )?;
        self.message(message)
    }

    /// Mark a voice message as listened to (`body.listened`): the chat shows which voice
    /// messages are still new and starts ear playback at the first one not heard yet.
    pub fn set_listened(&self, message: i64) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, '$.listened', json('true'))
             WHERE id = ?1 AND kind = 'file'",
            [message],
        )?;
        self.message(message)
    }

    /// Set `uploadState` of an outgoing file message.
    pub fn set_upload_state(
        &self,
        message: i64,
        state: i64,
    ) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, '$.uploadState', ?2), updated_at = ?3
             WHERE id = ?1 AND kind = 'file' AND outgoing = 1",
            params![message, state, now_ms()],
        )?;
        self.message(message)
    }

    /// Outgoing file messages whose upload didn't finish (interrupted or failed earlier).
    pub fn unfinished_uploads(&self) -> Result<Vec<i64>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM messages WHERE kind = 'file' AND outgoing = 1
                 AND json_extract(body, '$.uploadState') = ?1",
        )?;
        let rows = statement.query_map([STATE_LOADING], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Outgoing file messages whose upload failed and whose message never went out.
    pub fn failed_uploads(&self) -> Result<Vec<i64>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM messages WHERE kind = 'file' AND outgoing = 1 AND state = 0
                 AND json_extract(body, '$.uploadState') = ?1",
        )?;
        let rows = statement.query_map([STATE_FAILED], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// After a restart nothing is loading any more.
    pub fn reset_loading_media(&self) -> Result<(), StoreError> {
        for part in [MediaPart::File, MediaPart::Thumbnail] {
            self.conn.execute(
                "UPDATE messages SET body = json_set(body, ?1, ?2)
                 WHERE kind = 'file' AND json_extract(body, ?1) = ?3",
                params![state_field(part), STATE_REMOTE, STATE_LOADING],
            )?;
        }
        Ok(())
    }

    /// File messages whose thumbnail (or, of the `auto` download kinds, file) is not here yet:
    /// received ones, and our own from another device (reflected, Android's
    /// `ReflectedOutgoingFileTask`) — what this device sent itself has its parts already.
    /// `limit`: largest file that comes by itself, in bytes (0: any size, our default; Android
    /// has a fixed 5 MB). Thumbnails are never held back by it.
    pub fn media_to_download(
        &self,
        auto: &[String],
        limit: u64,
    ) -> Result<Vec<(i64, MediaPart)>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, json_extract(body, '$.thumbnailState'), json_extract(body, '$.fileState'),
                    body
             FROM messages WHERE kind = 'file'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut parts = Vec::new();
        for row in rows {
            let (id, thumbnail, file, body) = row?;
            if thumbnail == Some(STATE_REMOTE) {
                parts.push((id, MediaPart::Thumbnail));
            }
            let size = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|body| body["size"].as_u64())
                .unwrap_or(0);
            if file == Some(STATE_REMOTE)
                && auto.iter().any(|k| k == download_kind(&body))
                && (limit == 0 || size <= limit)
            {
                parts.push((id, MediaPart::File));
            }
        }
        Ok(parts)
    }
}

/// Is a part of this file message still on the blob server (or the mirror)?
fn awaits_download(body: &str) -> bool {
    let body: Value = serde_json::from_str(body).unwrap_or_default();
    ["thumbnailState", "fileState"]
        .iter()
        .any(|field| body[field].as_i64() == Some(STATE_REMOTE))
}

/// The auto-download category of a file message (Android's choices): `image` (also GIFs),
/// `audio` (voice messages), `video` or `file` (also audio sent as a file).
pub fn download_kind(body: &str) -> &'static str {
    match crate::messages::media_kind(body) {
        "image" | "gif" => "image",
        "voice" => "audio",
        "video" => "video",
        _ => "file",
    }
}

/// Auto-download defaults (Android): everything on Wi-Fi, only pictures on mobile data.
pub const AUTO_DOWNLOAD_WIFI: [&str; 4] = ["image", "audio", "video", "file"];
pub const AUTO_DOWNLOAD_MOBILE: [&str; 1] = ["image"];

/// Download one part into the media store, updating its state (events go to the UI).
/// Media downloads running at once (each holds its blob and the decrypted data in memory).
static DOWNLOAD_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

async fn download(
    store: Rc<Store>,
    https: HttpsClient,
    media: Arc<MediaStore>,
    events: mpsc::UnboundedSender<CoreEvent>,
    message: i64,
    part: MediaPart,
    save_to: Option<std::path::PathBuf>,
) {
    let changed = |result: Result<Option<Message>, StoreError>| {
        if let Ok(Some(message)) = result {
            let _ = events.send(CoreEvent::MessagesChanged(vec![message]));
        }
    };
    let keys = match store.file_keys(message) {
        Ok(Some(keys)) => keys,
        _ => return,
    };
    let Some(blob_id) = (match part {
        MediaPart::File => keys.blob_id,
        MediaPart::Thumbnail => keys.thumbnail_blob_id,
    }) else {
        return;
    };
    // Android's scopes: our own (reflected) messages' blobs are only on the mirror; others'
    // are public; "done" in public scope only for incoming 1:1 messages (group members and the
    // other devices still need it).
    let (outgoing, group) = store
        .message(message)
        .ok()
        .flatten()
        .map_or((false, false), |m| {
            (m.outgoing, m.conversation.starts_with("g:"))
        });
    let download_scope = if outgoing {
        media::BlobScope::Local
    } else {
        media::BlobScope::Public
    };
    let done_scope = if !outgoing && !group {
        media::BlobScope::Public
    } else {
        media::BlobScope::Local
    };
    changed(store.set_media_state(message, part, STATE_LOADING));
    let result = async {
        // A few at a time: each can hold a blob of up to 100 MiB, twice (S06).
        let _slot = DOWNLOAD_SLOTS.acquire().await;
        let limit = match part {
            MediaPart::File => media::MAX_FILE_DOWNLOAD,
            MediaPart::Thumbnail => media::MAX_PICTURE_DOWNLOAD,
        };
        let blob = media::download(&https, &blob_id, download_scope, limit).await?;
        let data = media::decrypt_blob(&keys.key, part, &blob)?;
        media.write(message, part, &data)?;
        if let Some(directory) = save_to {
            auto_save(&store, message, &data, &directory);
        }
        Ok::<_, media::MediaError>(())
    }
    .await;
    match result {
        Ok(()) => {
            changed(store.set_media_state(message, part, STATE_LOCAL));
            media::mark_done(&https, &blob_id, done_scope).await;
        }
        Err(error) => {
            tracing::warn!(%error, ?part, "media download failed");
            changed(store.set_media_state(message, part, STATE_FAILED));
        }
    }
}

/// A received photo or video in `<Pictures>/Threesea` (Android's "Auto-save to gallery"),
/// unencrypted, as the user asked. Voice messages and other files stay in the app, and so does
/// everything of a private chat (ADR 0009): checked here, after the download, so a chat marked
/// private meanwhile counts too; if the mark can't be read, nothing is written.
fn auto_save(store: &Store, message: i64, data: &[u8], pictures: &std::path::Path) {
    let Ok(Some(message)) = store.message(message) else {
        return;
    };
    if !matches!(store.is_private(&message.conversation), Ok(false)) {
        return;
    }
    if message.outgoing
        || !matches!(
            crate::messages::media_kind(&message.body),
            "image" | "gif" | "video"
        )
    {
        return;
    }
    let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
    let Some(name) = crate::file_types::gallery_file_name(
        body["name"].as_str(),
        body["mediaType"].as_str().unwrap_or(""),
    )
    .filter(|_| crate::file_types::looks_like_gallery_media(data)) else {
        return;
    };
    let directory = pictures.join("Threesea");
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let target = save_target(&directory, &name, data);
    if !target.exists()
        && let Err(error) = std::fs::write(&target, data)
    {
        tracing::warn!(%error, "auto-saving media failed");
    }
}

/// Where a message's blob goes: a notes group (a group whose only member is us) is read by our
/// own devices alone, so its blob stays on the device group's mirror, as Android does
/// (`isNotesGroup ? BlobScope.Local : BlobScope.Public`). Without multi-device there is no
/// mirror and both go to the usual blob server.
fn upload_scope(store: &Store, message: i64) -> media::BlobScope {
    match store.message(message).ok().flatten() {
        Some(message) => match store.group(&message.conversation) {
            Ok(Some(group)) if group.members.is_empty() && !group.left => media::BlobScope::Local,
            _ => media::BlobScope::Public,
        },
        None => media::BlobScope::Public,
    }
}

/// Upload a stored outgoing file (and thumbnail), then record the blob ids.
async fn upload(
    store: Rc<Store>,
    https: HttpsClient,
    media: Arc<MediaStore>,
    events: mpsc::UnboundedSender<CoreEvent>,
    message: i64,
) {
    let changed = |result: Result<Option<Message>, StoreError>| {
        if let Ok(Some(message)) = result {
            let _ = events.send(CoreEvent::MessagesChanged(vec![message]));
        }
    };
    let Ok(Some(mut keys)) = store.file_keys(message) else {
        return;
    };
    changed(store.set_upload_state(message, STATE_LOADING));
    let scope = upload_scope(&store, message);
    let result = async {
        let file = media
            .read(message, MediaPart::File)?
            .ok_or_else(|| media::MediaError::Server("file is gone".into()))?;
        keys.blob_id = Some(
            media::upload_scoped(
                &https,
                &media::encrypt_blob(&keys.key, MediaPart::File, &file),
                false,
                scope,
            )
            .await?,
        );
        if let Some(thumbnail) = media.read(message, MediaPart::Thumbnail)? {
            keys.thumbnail_blob_id = Some(
                media::upload_scoped(
                    &https,
                    &media::encrypt_blob(&keys.key, MediaPart::Thumbnail, &thumbnail),
                    false,
                    scope,
                )
                .await?,
            );
        }
        Ok::<_, media::MediaError>(())
    }
    .await;
    match result.map(|()| store.set_file_keys(message, &keys)) {
        Ok(Ok(())) => changed(store.set_upload_state(message, STATE_LOCAL)),
        Ok(Err(error)) => {
            tracing::warn!(%error, "storing blob ids failed");
            changed(store.set_upload_state(message, STATE_FAILED));
        }
        Err(error) => {
            tracing::warn!(%error, "upload failed");
            changed(store.set_upload_state(message, STATE_FAILED));
        }
    }
}

/// How long files may take to arrive once their chat is hidden.
const WRITE_PASS_LIFETIME: std::time::Duration = std::time::Duration::from_secs(60);

/// Files of private chats on their way to the core when those chats were hidden: the UI prepares
/// a file on another thread, so what a chat being left hands over arrives after the lock. A pass
/// lets exactly those be stored and sent. It opens nothing: the chats are hidden from the lock
/// on, every other command answers `locked`, and the answer to such a file says nothing of the
/// chat (ADR 0009).
#[derive(Debug)]
pub(crate) struct WritePass {
    /// By chat: how many files may still come.
    pub(crate) files: std::collections::HashMap<String, u32>,
    pub(crate) expires: std::time::Instant,
}

impl App {
    /// A `lock` naming files on their way: those of hidden private chats get a pass, which replaces
    /// any earlier one. A list has none: its members' copies are made by reading its own back.
    pub(crate) fn grant_write_pass(&mut self, files: std::collections::HashMap<String, u32>) {
        let files: std::collections::HashMap<String, u32> = files
            .into_iter()
            .filter_map(|(identity, count)| {
                let (key, conversation) = self.conversation(&identity).ok()?;
                let hidden = self.store.is_private(&key).unwrap_or(false);
                (count > 0 && hidden && !matches!(conversation, Conversation::List(_)))
                    .then_some((key, count))
            })
            .collect();
        self.write_pass = (!files.is_empty()).then(|| WritePass {
            files,
            expires: std::time::Instant::now() + WRITE_PASS_LIFETIME,
        });
    }

    /// Whether a file for `identity` comes under the pass, which then counts it. A chat that isn't
    /// hidden needs none.
    pub(crate) fn take_write_pass(&mut self, identity: &str) -> bool {
        let Some(pass) = &self.write_pass else {
            return false;
        };
        if std::time::Instant::now() >= pass.expires {
            self.write_pass = None;
            return false;
        }
        let Ok((key, _)) = self.conversation(identity) else {
            return false;
        };
        if self.unlocked || !self.store.is_private(&key).unwrap_or(false) {
            return false;
        }
        let Some(pass) = &mut self.write_pass else {
            return false;
        };
        let Some(count) = pass.files.get_mut(&key) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            pass.files.remove(&key);
        }
        true
    }

    /// A file of a hidden private chat, under the pass: stored and sent, and all the answer tells
    /// is what is wrong with what was sent. Anything about the chat reads as the lock.
    pub(crate) fn send_file_under_pass(&mut self, file: OutgoingFile) -> Result<Value, ApiError> {
        let (key, conversation) = self.conversation(&file.identity)?;
        match self.store_outgoing_file(key, conversation, file) {
            Ok(message) => {
                if let Some(id) = message["id"].as_i64() {
                    self.media.hide(id);
                }
                Ok(json!({}))
            }
            Err(error @ (ApiError::MessageTooLong | ApiError::FileTooLarge)) => Err(error),
            Err(error) => {
                tracing::warn!(kind = error.kind(), "file under a write pass refused");
                Err(ApiError::Locked)
            }
        }
    }

    /// `sendFile`: store the file encrypted, show it right away, upload in the background; the
    /// message is sent once the upload is done.
    pub(crate) fn send_file(&mut self, file: OutgoingFile) -> Result<Value, ApiError> {
        let (identity, conversation) = self.readable_conversation(&file.identity)?;
        self.store_outgoing_file(identity, conversation, file)
    }

    fn store_outgoing_file(
        &mut self,
        identity: String,
        conversation: Conversation,
        file: OutgoingFile,
    ) -> Result<Value, ApiError> {
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        let mut attachments = std::mem::take(&mut self.attachments).into_iter();
        let mut data = attachments.next().unwrap_or_default();
        let thumbnail = attachments.next().filter(|thumbnail| !thumbnail.is_empty());
        if data.is_empty() {
            let path = file
                .path
                .as_deref()
                .ok_or_else(|| ApiError::Internal("no file".into()))?;
            let size = std::fs::metadata(path)
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .len();
            if size > media::MAX_BLOB_SIZE as u64 {
                return Err(ApiError::FileTooLarge);
            }
            data =
                Zeroizing::new(std::fs::read(path).map_err(|e| ApiError::Internal(e.to_string()))?);
        }
        let mut file = file;
        if file.encode.as_deref() == Some("opus") {
            let (encoded, seconds) =
                crate::voice::encode(&data).map_err(|e| ApiError::Internal(e.to_string()))?;
            data = Zeroizing::new(encoded);
            file.media_type = "audio/ogg".into();
            file.duration = Some(seconds);
            file.name = file.name.map(|name| match name.rsplit_once('.') {
                Some((stem, _)) => format!("{stem}.ogg"),
                None => format!("{name}.ogg"),
            });
        }
        if data.len() > media::MAX_BLOB_SIZE {
            return Err(ApiError::FileTooLarge);
        }
        let name = file.name.filter(|name| !name.is_empty());
        if name
            .as_ref()
            .is_some_and(|name| name.len() > NAME_LENGTH_MAX)
        {
            return Err(ApiError::Internal("file name too long".into()));
        }
        let caption = file
            .caption
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty());
        if caption
            .as_ref()
            .is_some_and(|c| c.len() > CAPTION_LENGTH_MAX)
        {
            return Err(ApiError::MessageTooLong);
        }
        let body = json!({
            "rendering": match file.rendering.as_str() {
                "media" | "sticker" => file.rendering.as_str(),
                _ => "file",
            },
            "mediaType": file.media_type,
            "name": name,
            "size": data.len(),
            "caption": caption,
            "width": file.width,
            "height": file.height,
            "duration": file.duration,
            // Like Android: GIFs shown as media are marked animated (they play in the chat).
            "animated": (file.media_type == "image/gif" && file.rendering == "media").then_some(true),
            "thumbnail": thumbnail.is_some(),
            "thumbnailMediaType": "image/jpeg",
            "fileState": STATE_LOCAL,
            "thumbnailState": if thumbnail.is_some() { STATE_LOCAL } else { STATE_FAILED },
            // A list's copy is not uploaded (each member's copy is).
            "uploadState": if matches!(conversation, Conversation::List(_)) { STATE_LOCAL } else { STATE_LOADING },
        });
        let message = self.store.insert_outgoing(
            &identity,
            libthreema::common::MessageId::random().0,
            "file",
            &body.to_string(),
        )?;
        let mut key = Zeroizing::new([0_u8; 32]);
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, key.as_mut());
        let stored = (|| {
            self.media.write(message.id, MediaPart::File, &data)?;
            if let Some(thumbnail) = &thumbnail {
                self.media
                    .write(message.id, MediaPart::Thumbnail, thumbnail)?;
            }
            Ok::<_, media::MediaError>(())
        })();
        if let Err(error) = stored {
            self.store.remove_message(message.id)?;
            self.media.remove(message.id);
            return Err(ApiError::Internal(error.to_string()));
        }
        self.store.set_file_keys(
            message.id,
            &FileKeys {
                key,
                blob_id: None,
                thumbnail_blob_id: None,
            },
        )?;
        self.prepare_sending(&conversation, &message)?;
        if let Some(caption) = &caption {
            self.forget_sent_draft(&identity, caption);
        }
        // Its state may have changed (a list's copy counts as sent).
        let message = self.store.message(message.id)?.unwrap_or(message);
        if let Conversation::List(list) = &conversation {
            for member in self.list_receivers(list)? {
                if let Err(error) = self.forward_message(message.id, &member) {
                    tracing::warn!(%error, "sending a list's file to a member failed");
                }
            }
            return Ok(self.store.message_json(&message));
        }
        self.spawn_upload(message.id);
        Ok(self.store.message_json(&message))
    }

    /// `retryUpload`: upload a failed outgoing file again.
    pub(crate) fn retry_upload(&mut self, id: i64) -> Result<Value, ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|message| message.kind == "file" && message.outgoing)
            .ok_or_else(|| ApiError::Internal("no such file".into()))?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        if body["uploadState"].as_i64() == Some(STATE_FAILED) {
            self.spawn_upload(id);
        }
        Ok(json!({}))
    }

    /// Online (again): uploads that failed get another try by themselves, so a voice message
    /// sent on a bad connection does not sit behind its clock until it is tapped (user report
    /// 2026-10-28). Each reconnect is one new attempt per file.
    pub(crate) fn retry_failed_uploads(&self) {
        match self.store.failed_uploads() {
            Ok(uploads) => {
                for message in uploads {
                    self.spawn_upload(message);
                }
            }
            Err(error) => tracing::warn!(%error, "listing failed uploads failed"),
        }
    }

    fn spawn_upload(&self, message: i64) {
        tokio::task::spawn_local(upload(
            Rc::clone(&self.store),
            self.https.clone(),
            Arc::clone(&self.media),
            self.media_events.clone(),
            message,
        ));
    }

    /// A media task reported progress: an upload may have finished, so send what's pending.
    pub(crate) fn on_media_event(&mut self) {
        self.send_pending_messages();
    }

    /// Side effects of core events: download new media, drop media of deleted messages.
    pub(crate) fn on_core_event(&mut self, event: &CoreEvent) {
        match event {
            CoreEvent::MessageAdded { message, .. } => {
                // A new message brings an archived chat back (Android).
                if !message.outgoing && message.kind != "status" {
                    let _ = self.store.set_archived(&message.conversation, false);
                }
                if message.kind == "file" {
                    self.download_pending_media();
                }
            }
            CoreEvent::UnknownMembers(identities) => self.fetch_members(identities.clone()),
            CoreEvent::WebSessionResume(hash) => self.web_resume(hash),
            CoreEvent::DownloadGroupPicture(key) => self.download_group_picture(key.clone()),
            CoreEvent::DownloadContactPicture(identity) => {
                self.download_contact_picture(identity.clone());
            }
            CoreEvent::GroupCallStart {
                conversation,
                start,
            } => self.on_group_call_start(conversation, start),
            CoreEvent::CallSignal { peer, kind, json } => self.on_call_signal(peer, *kind, json),
            CoreEvent::SyncPicture { target, blob } => {
                self.apply_synced_picture(target.clone(), blob.clone());
            }
            CoreEvent::MessagesChanged(messages) => {
                for message in messages.iter().filter(|m| m.kind == "deleted") {
                    self.media.remove(message.id);
                }
                // Our own file message from another device (reflected): it arrives as a change,
                // not as an added message, and its blobs are still on the mirror.
                if messages
                    .iter()
                    .any(|m| m.kind == "file" && m.outgoing && awaits_download(&m.body))
                {
                    self.download_pending_media();
                }
            }
            _ => {}
        }
    }

    /// The kinds that download by themselves on the current network (Settings → Media).
    pub(crate) fn auto_download_kinds(&self) -> Vec<String> {
        let (key, default): (&str, &[&str]) = if self.on_wifi {
            (keys::AUTO_DOWNLOAD_WIFI, &AUTO_DOWNLOAD_WIFI)
        } else {
            (keys::AUTO_DOWNLOAD_MOBILE, &AUTO_DOWNLOAD_MOBILE)
        };
        match self.store.setting(key).ok().flatten() {
            Some(list) => list
                .split(',')
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .collect(),
            None => default.iter().map(|k| (*k).to_owned()).collect(),
        }
    }

    /// Settings → Media: the largest file that downloads by itself, in bytes (0: any size).
    pub(crate) fn auto_download_limit(&self) -> u64 {
        self.store
            .setting(keys::AUTO_DOWNLOAD_LIMIT)
            .ok()
            .flatten()
            .and_then(|mib| mib.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
            * 1024
    }

    /// Start downloads for everything that should be on the device but isn't.
    pub(crate) fn download_pending_media(&mut self) {
        let auto = self.auto_download_kinds();
        match self
            .store
            .media_to_download(&auto, self.auto_download_limit())
        {
            Ok(parts) => {
                for (message, part) in parts {
                    self.spawn_download(message, part);
                }
            }
            Err(error) => tracing::warn!(%error, "listing media downloads failed"),
        }
    }

    fn spawn_download(&self, message: i64, part: MediaPart) {
        // Settings → Media: received photos and videos also go to the Gallery's folder.
        let save_to = (part == MediaPart::File
            && self
                .store
                .setting(keys::AUTO_SAVE_MEDIA)
                .ok()
                .flatten()
                .as_deref()
                == Some("1"))
        .then(|| self.pictures_dir.clone())
        .flatten();
        tokio::task::spawn_local(download(
            Rc::clone(&self.store),
            self.https.clone(),
            Arc::clone(&self.media),
            self.media_events.clone(),
            message,
            part,
            save_to,
        ));
    }

    /// `downloadFile`: fetch a file (or retry a thumbnail) on request.
    pub(crate) fn download_file(&mut self, id: i64) -> Result<Value, ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|message| message.kind == "file")
            .ok_or_else(|| ApiError::Internal("no such file".into()))?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        for (part, field) in [
            (MediaPart::Thumbnail, "thumbnailState"),
            (MediaPart::File, "fileState"),
        ] {
            let state = body[field].as_i64();
            if matches!(state, Some(STATE_REMOTE | STATE_FAILED))
                && (part == MediaPart::File || body["thumbnail"] == true)
            {
                self.spawn_download(message.id, part);
            }
        }
        Ok(json!({}))
    }

    pub(crate) fn media_settings_json(&self) -> Result<Value, ApiError> {
        let list = |key, default: &[&str]| -> Result<Vec<String>, ApiError> {
            Ok(match self.store.setting(key)? {
                Some(list) => list
                    .split(',')
                    .filter(|k| !k.is_empty())
                    .map(str::to_owned)
                    .collect(),
                None => default.iter().map(|k| (*k).to_owned()).collect(),
            })
        };
        Ok(json!({
            "autoDownloadWifi": list(keys::AUTO_DOWNLOAD_WIFI, &AUTO_DOWNLOAD_WIFI)?,
            "autoDownloadMobile": list(keys::AUTO_DOWNLOAD_MOBILE, &AUTO_DOWNLOAD_MOBILE)?,
            "autoDownloadLimit": self.auto_download_limit() / (1024 * 1024),
            "autoSave": self.store.setting(keys::AUTO_SAVE_MEDIA)?.as_deref() == Some("1"),
            "autoDeleteDays": self.auto_delete_days(),
            "picturesAvailable": self.pictures_dir.is_some(),
        }))
    }

    /// `cleanUp` (Android's storage management): older than `days` (0: everything), starred
    /// messages kept. Media only: the files go (thumbnails stay, the file can be downloaded
    /// again while the server still has it); messages: the messages and their media go.
    pub(crate) fn clean_up(&mut self, days: u32, messages: bool) -> Result<Value, ApiError> {
        let before = if days == 0 {
            i64::MAX
        } else {
            crate::messages::now_ms() - i64::from(days) * 86_400_000
        };
        let removed = if messages {
            let removed = self.store.delete_messages_before(before)?;
            let store = Rc::clone(&self.store);
            self.media
                .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
            removed
        } else {
            let ids = self.store.files_before(before)?;
            for id in &ids {
                self.media.remove_part(*id, MediaPart::File);
                self.store
                    .set_media_state(*id, MediaPart::File, STATE_REMOTE)?;
            }
            ids.len()
        };
        tracing::info!(removed, messages, "storage cleaned up");
        Ok(json!({ "removed": removed }))
    }

    /// After how many days messages go by themselves (0: they stay).
    pub(crate) fn auto_delete_days(&self) -> u32 {
        self.store
            .setting(keys::AUTO_DELETE_DAYS)
            .ok()
            .flatten()
            .and_then(|days| days.parse().ok())
            .unwrap_or(0)
    }

    /// Maintenance: with auto-delete set, throw away what is older than that, twice a day like
    /// Android's worker. `true` if something went (the lists need reloading).
    pub(crate) fn auto_delete_scheduled(&mut self) -> bool {
        let days = self.auto_delete_days();
        if days == 0 {
            return false;
        }
        let now = crate::messages::now_ms();
        let last: i64 = self
            .store
            .setting(keys::AUTO_DELETE_LAST)
            .ok()
            .flatten()
            .and_then(|at| at.parse().ok())
            .unwrap_or(0);
        if now - last < 12 * 3_600_000 {
            return false;
        }
        let removed = match self.clean_up(days, true) {
            Ok(result) => result["removed"].as_u64().unwrap_or(0),
            Err(error) => {
                tracing::warn!(kind = error.kind(), "auto-delete failed");
                return false;
            }
        };
        let _ = self
            .store
            .set_setting(keys::AUTO_DELETE_LAST, &now.to_string());
        tracing::info!(removed, days, "auto-delete ran");
        removed > 0
    }

    /// Remove the media of a message that is gone (deleted, removed).
    pub(crate) fn forget_media(&self, id: i64) {
        self.media.remove(id);
    }

    /// Startup: clean up and resume.
    pub(crate) fn resume_media(&mut self) {
        match self.store.unfinished_uploads() {
            Ok(uploads) => {
                for message in uploads {
                    self.spawn_upload(message);
                }
            }
            Err(error) => tracing::warn!(%error, "listing uploads failed"),
        }
        if let Err(error) = self.store.reset_loading_media() {
            tracing::warn!(%error, "resetting media states failed");
        }
        let store = Rc::clone(&self.store);
        self.media
            .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
        self.download_pending_media();
    }
}

/// A safe file name for saving: no directories, hidden files or characters that reorder how it
/// reads, not empty, at most 200 bytes before an extension from the media type (only if it has
/// none of its own).
pub fn safe_file_name(name: Option<&str>, media_type: &str) -> String {
    let cleaned: String = crate::file_types::without_bidi_controls(name.unwrap_or_default())
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | '\0') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_owned();
    if name.is_empty() {
        name = "threesea-file".to_owned();
    }
    while name.len() > 200 {
        name.pop();
    }
    // "report v1.2" and "Nr. 5" have no extension, whatever the path says.
    let has_extension = std::path::Path::new(&name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            !extension.contains(' ') && !extension.chars().all(|c| c.is_ascii_digit())
        });
    if !has_extension && let Some(extension) = crate::file_types::extension_for(media_type) {
        name = format!("{name}.{extension}");
    }
    name
}

/// Where to save `name` in `directory`: the name itself, or "name (1).ext", … if taken. An
/// existing file with exactly `data` is reused (saving twice doesn't duplicate it).
fn save_target(directory: &std::path::Path, name: &str, data: &[u8]) -> std::path::PathBuf {
    let path = std::path::Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_owned();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"));
    for n in 0..1000 {
        let candidate = if n == 0 {
            directory.join(name)
        } else {
            directory.join(format!(
                "{stem} ({n}){}",
                extension.as_deref().unwrap_or("")
            ))
        };
        match std::fs::read(&candidate) {
            Ok(existing) if existing == data => return candidate,
            Ok(_) => continue,
            Err(_) if !candidate.exists() => return candidate,
            Err(_) => continue,
        }
    }
    directory.join(format!("{stem}-{}", now_ms()))
}

impl App {
    /// `forwardMessage`: send a text or file message again, to another chat (Android's
    /// "Forward"). Quotes are dropped (the quoted message is not in that chat); files are
    /// uploaded again with a new key, since blobs are deleted once downloaded.
    pub(crate) fn forward_message(&mut self, id: i64, identity: &str) -> Result<Value, ApiError> {
        let message = self
            .store
            .message(id)?
            .ok_or_else(|| ApiError::Internal("no such message".into()))?;
        self.check_readable(&message.conversation)?;
        match message.kind.as_str() {
            "text" => {
                let text = crate::messages::split_quote(&message.body).1.to_owned();
                self.send_text(identity, &text, None)
            }
            "location" => {
                let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
                self.send_location(identity, &body)
            }
            "file" => {
                let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
                let data = self
                    .media
                    .read(id, MediaPart::File)
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .ok_or(ApiError::FileNotDownloaded)?;
                let thumbnail = self
                    .media
                    .read(id, MediaPart::Thumbnail)
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .unwrap_or_default();
                self.attachments = vec![data, thumbnail];
                let text = |field: &str| body[field].as_str().map(str::to_owned);
                let dimension = |field: &str| {
                    body[field]
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                };
                let result = self.send_file(OutgoingFile {
                    identity: identity.to_owned(),
                    path: None,
                    name: text("name"),
                    media_type: text("mediaType")
                        .unwrap_or_else(|| "application/octet-stream".into()),
                    caption: text("caption"),
                    rendering: text("rendering").unwrap_or_else(|| "file".into()),
                    width: dimension("width"),
                    height: dimension("height"),
                    duration: body["duration"].as_f64(),
                    encode: None,
                });
                self.attachments.clear();
                result
            }
            _ => Err(ApiError::NotForwardable),
        }
    }

    /// `saveFile`: write a received or sent file, decrypted, to `directory` (the user's
    /// Downloads); returns the path. This is the only way media leave the encrypted store.
    pub(crate) fn save_file(&mut self, id: i64, directory: &str) -> Result<Value, ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|message| message.kind == "file")
            .ok_or_else(|| ApiError::Internal("no such file".into()))?;
        self.check_readable(&message.conversation)?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        let data = self
            .media
            .read(id, MediaPart::File)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::Internal("file not downloaded".into()))?;
        let directory = std::path::Path::new(directory);
        if !directory.is_dir() {
            return Err(ApiError::Internal("no such directory".into()));
        }
        let name = safe_file_name(
            body["name"].as_str(),
            body["mediaType"].as_str().unwrap_or(""),
        );
        let target = save_target(directory, &name, &data);
        if !target.exists() {
            std::fs::write(&target, data.as_slice())
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
        let openable = target
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(crate::file_types::opens_externally);
        Ok(json!({ "path": target.to_string_lossy(), "openable": openable }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A photo of a private chat must not land in the Gallery folder, unencrypted, while one of
    /// an ordinary chat does.
    #[test]
    fn auto_save_leaves_private_chats_out() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let pictures = dir.path().join("Pictures");
        let photo = serde_json::json!({"mediaType": "image/jpeg", "name": "photo.jpg"}).to_string();
        let open = store
            .insert_incoming("BBBBBBBB", 1, "file", &photo, 1)
            .unwrap()
            .unwrap();
        let private = store
            .insert_incoming("CCCCCCCC", 2, "file", &photo, 1)
            .unwrap()
            .unwrap();
        store.set_private("CCCCCCCC", true).unwrap();
        let saved = || {
            std::fs::read_dir(pictures.join("Threesea"))
                .map(Iterator::count)
                .unwrap_or(0)
        };

        auto_save(&store, private.id, b"\xff\xd8\xffprivate", &pictures);
        assert_eq!(saved(), 0);

        auto_save(&store, open.id, b"\xff\xd8\xffopen", &pictures);
        assert_eq!(saved(), 1);
    }

    /// The gallery's folder is indexed without anyone asking: a peer's name can't make something
    /// else of a photo there, and what isn't a photo doesn't get there.
    #[test]
    fn auto_save_goes_by_the_checked_type() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let pictures = dir.path().join("Pictures");
        let insert = |id, body: serde_json::Value| {
            store
                .insert_incoming("BBBBBBBB", id, "file", &body.to_string(), 1)
                .unwrap()
                .unwrap()
                .id
        };
        let saved = || {
            let mut names: Vec<_> = std::fs::read_dir(pictures.join("Threesea"))
                .map(|entries| {
                    entries
                        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        };

        let renamed = insert(1, json!({"mediaType": "image/jpeg", "name": "x.desktop"}));
        auto_save(&store, renamed, b"\xff\xd8\xffphoto", &pictures);
        assert_eq!(saved(), ["x.desktop.jpg"]);

        // Bytes that aren't what the type says, and a type outside the list.
        let mislabelled = insert(2, json!({"mediaType": "image/jpeg", "name": "y.jpg"}));
        auto_save(&store, mislabelled, b"II*\0tiff", &pictures);
        let unknown = insert(3, json!({"mediaType": "image/tiff", "name": "z.tiff"}));
        auto_save(&store, unknown, b"\xff\xd8\xffphoto", &pictures);
        assert_eq!(saved(), ["x.desktop.jpg"]);
    }

    #[test]
    fn deleting_old_messages_keeps_what_is_not_old_news() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let old = 1_000;
        let now = crate::messages::now_ms();
        store
            .insert_incoming("AAAAAAAA", 1, "text", "old", old)
            .unwrap();
        let starred = store
            .insert_incoming("AAAAAAAA", 2, "text", "keep this", old)
            .unwrap()
            .unwrap();
        store.set_starred(starred.id, true).unwrap();
        store
            .insert_incoming("AAAAAAAA", 3, "text", "new", now)
            .unwrap();
        store
            .insert_incoming("AAAAAAAA", 4, "poll", r#"{"state":0}"#, old)
            .unwrap();
        store
            .insert_incoming("AAAAAAAA", 5, "poll", r#"{"state":1}"#, old)
            .unwrap();
        // An outgoing message still waiting to go out.
        store
            .insert_outgoing("AAAAAAAA", 6, "text", "unsent")
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET created_at = ?1 WHERE message_id = 6",
                [old],
            )
            .unwrap();

        assert_eq!(store.delete_messages_before(now - 1).unwrap(), 2);
        let left: Vec<String> = store
            .conversation_messages("AAAAAAAA", 50)
            .unwrap()
            .into_iter()
            .map(|message| message.body)
            .collect();
        assert_eq!(
            left,
            vec!["keep this", "new", r#"{"state":0}"#, "unsent"],
            "starred, open poll and unsent stay"
        );
    }

    /// A notes group's blob (a group whose only member is us) goes to the device group's
    /// mirror, not to the public blob server — only our own devices read it (Android's
    /// `isNotesGroup`).
    #[test]
    fn notes_group_blobs_stay_local() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let group = |key: &str, members: Vec<String>| crate::groups::Group {
            key: key.to_owned(),
            identity: libthreema::common::GroupIdentity {
                creator_identity: "ECHOECHO".parse().unwrap(),
                group_id: 1,
            },
            name: "g".into(),
            members,
            left: false,
            created_at: 0,
            picture: None,
            picture_version: 0,
        };
        let notes = group("g:ECHOECHO:0000000000000001", Vec::new());
        let shared = group("g:ECHOECHO:0000000000000002", vec!["ABCD1234".to_owned()]);
        store.upsert_group(&notes).unwrap();
        store.upsert_group(&shared).unwrap();
        let body = json!({"mediaType": "image/jpeg", "size": 1}).to_string();
        let own = store.insert_outgoing(&notes.key, 1, "file", &body).unwrap();
        let theirs = store
            .insert_outgoing(&shared.key, 2, "file", &body)
            .unwrap();
        let contact = store.insert_outgoing("ABCD1234", 3, "file", &body).unwrap();

        assert_eq!(upload_scope(&store, own.id), media::BlobScope::Local);
        assert_eq!(upload_scope(&store, theirs.id), media::BlobScope::Public);
        assert_eq!(upload_scope(&store, contact.id), media::BlobScope::Public);
    }

    /// A picture we sent from another device (Threema Desktop): the reflection carries the blob,
    /// so this device fetches it like a received one, thumbnail always and the file by the
    /// auto-download setting — Android's `ReflectedOutgoingFileTask`. What this device sent
    /// itself has its parts already and is left alone.
    #[test]
    fn reflected_outgoing_media_is_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let file = FileMessage {
            rendering_type: FileRenderingType::Media,
            key: [1; 32],
            blob_id: libthreema::common::BlobId([2; 16]),
            media_type: "image/jpeg".into(),
            name: Some("a.jpg".into()),
            size: 1000,
            thumbnail_blob_id: Some(libthreema::common::BlobId([3; 16])),
            thumbnail_media_type: None,
            caption: None,
            correlation_id: None,
            metadata: libthreema::model::message::FileMetadata::default(),
        };
        // As reflected.rs stores it: the blobs are on the mirror, the upload is long done.
        let mut body = file_body(&file);
        body["uploadState"] = json!(STATE_LOCAL);
        let reflected = store
            .insert_reflected_outgoing("ECHOECHO", 11, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap();
        assert!(awaits_download(&reflected.body));

        // Sent from this device: both parts are here.
        let own_body = json!({
            "rendering": "media", "mediaType": "image/jpeg", "name": "b.jpg", "size": 10,
            "thumbnail": true, "fileState": STATE_LOCAL, "thumbnailState": STATE_LOCAL,
            "uploadState": STATE_LOCAL,
        });
        let own = store
            .insert_outgoing("ECHOECHO", 12, "file", &own_body.to_string())
            .unwrap();
        assert!(!awaits_download(&own.body));

        assert_eq!(
            store.media_to_download(&["image".to_owned()], 0).unwrap(),
            vec![
                (reflected.id, MediaPart::Thumbnail),
                (reflected.id, MediaPart::File)
            ]
        );
        // Pictures not chosen on this network: the thumbnail still comes.
        assert_eq!(
            store.media_to_download(&[], 0).unwrap(),
            vec![(reflected.id, MediaPart::Thumbnail)]
        );
        let every_kind = ["image".to_owned(), "video".to_owned(), "file".to_owned()];
        assert!(
            store
                .media_to_download(&every_kind, 0)
                .unwrap()
                .iter()
                .all(|(id, _)| *id == reflected.id),
            "nothing of our own copy is listed"
        );
    }

    #[test]
    fn media_states_and_download_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let file = FileMessage {
            rendering_type: FileRenderingType::Media,
            key: [1; 32],
            blob_id: libthreema::common::BlobId([2; 16]),
            media_type: "image/jpeg".into(),
            name: Some("a.jpg".into()),
            size: 1000,
            thumbnail_blob_id: Some(libthreema::common::BlobId([3; 16])),
            thumbnail_media_type: None,
            caption: Some("look".into()),
            correlation_id: None,
            metadata: libthreema::model::message::FileMetadata::default(),
        };
        let message = store
            .insert_incoming("ECHOECHO", 9, "file", &file_body(&file).to_string(), 1)
            .unwrap()
            .unwrap();
        let images = vec!["image".to_owned()];
        assert_eq!(
            store.media_to_download(&images, 0).unwrap(),
            vec![
                (message.id, MediaPart::Thumbnail),
                (message.id, MediaPart::File)
            ]
        );
        // Pictures not chosen: only the thumbnail.
        assert_eq!(
            store.media_to_download(&["video".to_owned()], 0).unwrap(),
            vec![(message.id, MediaPart::Thumbnail)]
        );
        // Over the size the user allows (1000 bytes here): the preview still comes.
        assert_eq!(
            store.media_to_download(&images, 500).unwrap(),
            vec![(message.id, MediaPart::Thumbnail)]
        );
        assert_eq!(
            store.media_to_download(&images, 2000).unwrap(),
            vec![
                (message.id, MediaPart::Thumbnail),
                (message.id, MediaPart::File)
            ]
        );
        let changed = store
            .set_media_state(message.id, MediaPart::Thumbnail, STATE_LOADING)
            .unwrap()
            .unwrap();
        let body: Value = serde_json::from_str(&changed.body).unwrap();
        assert_eq!(body["thumbnailState"], STATE_LOADING);
        assert_eq!(body["caption"], "look");
        store.reset_loading_media().unwrap();
        store
            .set_media_state(message.id, MediaPart::File, STATE_LOCAL)
            .unwrap();
        assert_eq!(
            store.media_to_download(&images, 0).unwrap(),
            vec![(message.id, MediaPart::Thumbnail)]
        );
        let kind = |media_type: &str, rendering: &str| {
            download_kind(&json!({"mediaType": media_type, "rendering": rendering}).to_string())
        };
        assert_eq!(kind("image/gif", "media"), "image");
        assert_eq!(kind("audio/aac", "media"), "audio");
        assert_eq!(kind("audio/mpeg", "file"), "file");
        assert_eq!(kind("video/mp4", "media"), "video");
        assert_eq!(kind("application/pdf", "file"), "file");

        // Played voice messages are remembered as listened to.
        let heard: Value =
            serde_json::from_str(&store.set_listened(message.id).unwrap().unwrap().body).unwrap();
        assert_eq!(heard["listened"], true);
        assert_eq!(heard["caption"], "look", "the rest of the body stays");
    }

    #[test]
    fn storage_size() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        assert!(store.size().unwrap() > 0);
    }

    #[test]
    fn clean_up_lists() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let body = json!({"mediaType": "image/jpeg", "fileState": STATE_LOCAL}).to_string();
        let old = store
            .insert_incoming("ECHOECHO", 1, "file", &body, 1_000)
            .unwrap()
            .unwrap();
        let starred = store
            .insert_incoming("ECHOECHO", 2, "file", &body, 1_000)
            .unwrap()
            .unwrap();
        store.set_starred(starred.id, true).unwrap();
        store
            .insert_incoming("ECHOECHO", 3, "text", "old", 1_000)
            .unwrap();
        let recent = store
            .insert_incoming("ECHOECHO", 4, "file", &body, 9_000)
            .unwrap()
            .unwrap();
        assert_eq!(store.files_before(5_000).unwrap(), vec![old.id]);
        assert_eq!(store.delete_messages_before(5_000).unwrap(), 2);
        assert!(store.message(old.id).unwrap().is_none());
        assert!(store.message(starred.id).unwrap().is_some());
        assert!(store.message(recent.id).unwrap().is_some());
    }

    #[test]
    fn outgoing_file_message_needs_the_upload() {
        let mut body = json!({
            "rendering": "media", "mediaType": "image/jpeg", "name": "a.jpg", "size": 5,
            "caption": "", "width": 640, "height": 480, "duration": 3.5, "uploadState": STATE_LOADING,
        });
        let mut keys = FileKeys {
            key: Zeroizing::new([4; 32]),
            blob_id: None,
            thumbnail_blob_id: None,
        };
        assert!(outgoing_file_message(&body, &keys).is_none());
        body["uploadState"] = json!(STATE_LOCAL);
        assert!(
            outgoing_file_message(&body, &keys).is_none(),
            "no blob id yet"
        );
        keys.blob_id = Some(libthreema::common::BlobId([1; 16]));
        keys.thumbnail_blob_id = Some(libthreema::common::BlobId([2; 16]));
        let file = outgoing_file_message(&body, &keys).unwrap();
        assert_eq!(file.rendering_type, FileRenderingType::Media);
        assert_eq!(file.size, 5);
        assert_eq!(file.caption, None, "empty caption is left out");
        assert_eq!(file.metadata.width, Some(640));
        assert_eq!(file.metadata.duration, Some(3.5));
        assert_eq!(file.thumbnail_media_type.as_deref(), Some("image/jpeg"));
        assert_eq!(file.metadata.animated, None);
        body["animated"] = json!(true);
        let file = outgoing_file_message(&body, &keys).unwrap();
        assert_eq!(file.metadata.animated, Some(true));
    }

    #[test]
    fn saving_names() {
        assert_eq!(
            safe_file_name(Some("../../etc/passwd"), ""),
            "_.._etc_passwd"
        );
        assert_eq!(safe_file_name(Some(".hidden"), ""), "hidden");
        assert_eq!(safe_file_name(None, "application/pdf"), "threesea-file.pdf");
        assert_eq!(
            safe_file_name(Some("report"), "application/pdf"),
            "report.pdf"
        );
        assert_eq!(
            safe_file_name(Some("report."), "application/pdf"),
            "report.pdf"
        );
        assert_eq!(
            safe_file_name(Some("Rechnung Nr. 5"), "application/pdf"),
            "Rechnung Nr. 5.pdf"
        );
        assert_eq!(
            safe_file_name(Some("report v1.2"), "application/pdf"),
            "report v1.2.pdf"
        );
        // A real extension stays, and a name stays as it is when the type gives no other.
        assert_eq!(
            safe_file_name(Some("a.7z"), "application/x-7z-compressed"),
            "a.7z"
        );
        assert_eq!(safe_file_name(Some("a.001"), ""), "a.001");
        assert_eq!(
            safe_file_name(Some("invoice\u{202E}fdp.desktop"), "application/pdf"),
            "invoicefdp.desktop"
        );
        assert_eq!(
            safe_file_name(Some("  "), "image/jpeg"),
            "threesea-file.jpg"
        );
        assert!(safe_file_name(Some(&"ä".repeat(300)), "").len() <= 200);

        let dir = tempfile::tempdir().unwrap();
        let first = save_target(dir.path(), "a.pdf", b"one");
        assert_eq!(first, dir.path().join("a.pdf"));
        std::fs::write(&first, b"one").unwrap();
        // Same content: reused; other content: numbered.
        assert_eq!(save_target(dir.path(), "a.pdf", b"one"), first);
        assert_eq!(
            save_target(dir.path(), "a.pdf", b"two"),
            dir.path().join("a (1).pdf")
        );
    }
}
